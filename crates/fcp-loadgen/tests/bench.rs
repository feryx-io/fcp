//! The measured runs: what this platform carries, on hardware worth
//! naming (`docs/CAPACITY_AND_SURVIVABILITY.md` §13 phase 4).
//!
//! These are **not** part of any suite. They are `#[ignore]`d, they take
//! minutes, and a number from them means nothing without the machine it
//! came from. Run them deliberately, in release, on a box that is not
//! doing anything else:
//!
//! ```bash
//! FCP_TEST_DATABASE_URL=… cargo test --release -p fcp-loadgen --test bench \
//!     -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Each writes its JSON report under `target/loadgen/` and prints its
//! summary, so the numbers in the design document can be traced back to a
//! file. The harness runs on the same box as the node here, which is
//! honest for a dev VM and wrong for a capacity claim: for a number worth
//! publishing, put the harness on another machine and point `--sip` at
//! the node.

use fcp_loadgen::profile::Profile;
use fcp_loadgen::report::Report;
use fcp_loadgen::run::RunOptions;
use fcp_loadgen::target::Target;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{bins, free_port, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const REALM: &str = "bench.local";
const PASSWORD: &str = "bench-pw";
const EXTENSION_BASE: u32 = 3000;

/// A call manager with `users` caller accounts and as many answering
/// ones, and nothing else on its database.
struct Bench {
    service: Option<Service>,
    db: Option<TestDatabase>,
    sip: SocketAddr,
    http: u16,
    users: u32,
    _dir: tempfile::TempDir,
}

impl Bench {
    async fn start(users: u32) -> Option<Self> {
        Self::start_with(users, None, "").await
    }

    /// As [`Bench::start`], in a cluster: `redis` goes inside the config's
    /// own `[storage]` table — a second one would not parse — and `extra`
    /// adds whole tables of its own.
    async fn start_with(users: u32, redis: Option<&str>, extra: &str) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        for i in 0..users {
            seed.user(&format!("load{}", i + 1), PASSWORD, &format!("1{:04}", i))
                .await;
            seed.user(
                &format!("peer{}", i + 1),
                PASSWORD,
                &(EXTENSION_BASE + i).to_string(),
            )
            .await;
        }
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = format!(
            r#"
[service]
node_id = "bench"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"
{redis}

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 60

[b2bua.forking]
timeout = 10

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "warn"
format = "compact"

{extra}
"#,
            db = db.url(),
            redis = redis
                .map(|url| format!("redis_url = \"{url}\""))
                .unwrap_or_default(),
            // Two ports a call, and room for the ramp's bursts.
            rtp_end = rtp_start + 4 * users.max(50) as u16,
            dir = dir.path().display(),
        );
        let service = Service::start(
            "call-manager",
            bins::path("fcp-call-manager", "fcp-call-manager"),
            &config,
            &loopback(http, "/health"),
        )
        .await;
        Some(Self {
            service: Some(service),
            db: Some(db),
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            http,
            users,
            _dir: dir,
        })
    }

    fn options(&self, profile: Profile) -> RunOptions {
        RunOptions {
            profile,
            target: Target {
                sip: self.sip,
                realm: REALM.to_string(),
                user_prefix: "load".to_string(),
                first_user: 1,
                password: PASSWORD.to_string(),
                destination: "{callee}".to_string(),
                metrics_url: Some(loopback(self.http, "/metrics")),
                pid: self.service.as_ref().and_then(|s| s.pid()),
            },
            users: self.users,
            answer_with: Some("peer".to_string()),
            answer_extension_base: Some(EXTENSION_BASE),
            sample_interval: Duration::from_secs(2),
            ..RunOptions::default()
        }
    }

    /// The node's resident size right now, before any load.
    fn idle_rss_mib(&self) -> f64 {
        let Some(pid) = self.service.as_ref().and_then(|s| s.pid()) else {
            return 0.0;
        };
        std::fs::read_to_string(format!("/proc/{pid}/statm"))
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<f64>().ok())
            .map(|pages| pages * 4096.0 / 1024.0 / 1024.0)
            .unwrap_or(0.0)
    }

    async fn stop(mut self) {
        if let Some(service) = self.service.take() {
            service.stop().await;
        }
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
    }
}

/// Write the report where a number in the design document can be traced
/// back to it, and print the summary.
fn record(name: &str, report: &Report) {
    let dir = std::path::Path::new("target/loadgen");
    std::fs::create_dir_all(dir).ok();
    let path = dir.join(format!("{name}.json"));
    if let Ok(json) = serde_json::to_string_pretty(report) {
        std::fs::write(&path, json).ok();
    }
    println!(
        "\n=== {name} ===\n{}  report   {}",
        report.summary(),
        path.display()
    );
}

/// Calls per second to the first rung that does not hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "a measured run: minutes, and only meaningful in release on a quiet box"]
async fn ramp_to_first_refusal() {
    let Some(bench) = Bench::start(60).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let options = RunOptions {
        duration: Duration::from_secs(300),
        call_secs: 10,
        cps: 2.0,
        cps_step: 2.0,
        cps_max: 40.0,
        rung_secs: 20,
        // A rung that refuses one call in a hundred, answers slower than
        // a second at p99, or drops one, is past this node's rate.
        stop_refused_fraction: 0.01,
        stop_p99_ms: 1000.0,
        ..bench.options(Profile::CallRamp)
    };
    let report = fcp_loadgen::run(options).await.expect("the run");
    record("call-ramp", &report);
    let held = report
        .rungs
        .iter()
        .filter(|r| r.stopped_because.is_none())
        .map(|r| r.offered_cps)
        .fold(0.0_f64, f64::max);
    println!("  last rung that held: {held:.0} calls/s");
    bench.stop().await;
}

/// What holding calls costs: the node's memory per call, and whether the
/// media stays clean while it holds them.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "a measured run: minutes, and only meaningful in release on a quiet box"]
async fn soak_memory_per_call() {
    let Some(bench) = Bench::start(100).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let idle = bench.idle_rss_mib();
    let options = RunOptions {
        duration: Duration::from_secs(120),
        call_secs: 110,
        ..bench.options(Profile::Soak)
    };
    let report = fcp_loadgen::run(options).await.expect("the run");
    record("soak", &report);
    let peak = report
        .samples
        .iter()
        .filter_map(|s| s.rss_bytes)
        .max()
        .unwrap_or(0) as f64
        / 1024.0
        / 1024.0;
    let calls = report.measurements.answered.max(1) as f64;
    println!(
        "  node idle {idle:.0} MiB, peak {peak:.0} MiB over {calls:.0} calls \
         → {:.2} MiB a call",
        (peak - idle).max(0.0) / calls.min(100.0)
    );
    bench.stop().await;
}

/// What a lease costs on the INVITE path
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §7, §12 Q3).
///
/// Counting a tenant's calls across the cluster is a Redis round trip as
/// each call is admitted. The same ramp is run twice against the same
/// node configuration — once counting across the cluster, once counting
/// on the node — and the difference is what the lease costs. Needs
/// `FCP_TEST_REDIS_URL`; without it there is nothing to lease against.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "a measured run: minutes, and only meaningful in release on a quiet box"]
async fn what_a_lease_costs_on_the_invite_path() {
    let Ok(redis) = std::env::var("FCP_TEST_REDIS_URL") else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    for (what, leasing) in [("per-node", false), ("cluster-leased", true)] {
        let cluster = format!(
            "[cluster]\nenabled = true\nlease_tenant_calls = {leasing}\n\
             lease_settle_secs = 2\n"
        );
        let Some(bench) = Bench::start_with(20, Some(&redis), &cluster).await else {
            eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
            return;
        };
        let options = RunOptions {
            duration: Duration::from_secs(60),
            call_secs: 5,
            cps: 5.0,
            cps_step: 5.0,
            cps_max: 10.0,
            rung_secs: 20,
            // The point is the latency, not the ladder: let it climb.
            stop_refused_fraction: 1.0,
            stop_p99_ms: f64::MAX,
            ..bench.options(Profile::CallRamp)
        };
        let report = fcp_loadgen::run(options).await.expect("the run");
        record(&format!("lease-{what}"), &report);
        let answer = &report.measurements.invite_to_answer;
        println!(
            "  {what}: INVITE→200 p50 {:.1}ms p90 {:.1}ms p99 {:.1}ms over {} calls",
            answer.p50_ms, answer.p90_ms, answer.p99_ms, answer.count
        );
        bench.stop().await;
    }
}

/// Every device registering at once, and again as they lapse.
///
/// `FCP_LOADGEN_DEVICES` sizes it: the same storm at 15 and at 150 says
/// whether a slow registration is the node's doing or the harness's,
/// since only one of them scales with the device count
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §13 phase 4).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "a measured run: minutes, and only meaningful in release on a quiet box"]
async fn register_storm() {
    let devices = std::env::var("FCP_LOADGEN_DEVICES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    let Some(bench) = Bench::start(devices).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let options = RunOptions {
        duration: Duration::from_secs(90),
        register_expiry: 30,
        ..bench.options(Profile::RegisterStorm)
    };
    let report = fcp_loadgen::run(options).await.expect("the run");
    record("register-storm", &report);
    bench.stop().await;
}
