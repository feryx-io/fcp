//! The harness against a real node — the small profile CI runs
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §8, §12 Q8).
//!
//! It asserts **behaviour, not throughput**: every call answered, every
//! leg hearing audio, nothing refused, nothing dropped, and a register
//! storm that the registrar answers. A shared runner's calls per second
//! are a number about the runner, so no rate is asserted here — those
//! come from a manual run on known hardware.
//!
//! Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_loadgen::budget::Budget;
use fcp_loadgen::profile::Profile;
use fcp_loadgen::run::RunOptions;
use fcp_loadgen::target::Target;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{bins, free_port, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const REALM: &str = "e2e.local";
const PASSWORD: &str = "load-pw";
/// Three pairs is enough for every path the harness drives and short
/// enough for CI.
const USERS: u32 = 3;
const EXTENSION_BASE: u32 = 2000;

/// A call manager with accounts for the harness to use.
struct Node {
    service: Option<Service>,
    db: Option<TestDatabase>,
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Node {
    async fn start() -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        // Callers and the phones that answer them, as the harness names
        // them: load1.. and peer1.., the peers at 2000, 2001, …
        for i in 0..USERS {
            seed.user(&format!("load{}", i + 1), PASSWORD, &format!("10{:02}", i))
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
node_id = "loadgen-ci"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 10

[b2bua.forking]
timeout = 5

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "info"
format = "compact"
"#,
            db = db.url(),
            rtp_end = rtp_start + 60,
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
            _dir: dir,
        })
    }

    /// The options every run here shares: this node, these accounts, and
    /// the node watched from outside the calls.
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
            users: USERS,
            answer_with: Some("peer".to_string()),
            answer_extension_base: Some(EXTENSION_BASE),
            sample_interval: Duration::from_millis(500),
            ..RunOptions::default()
        }
    }

    async fn stop(mut self) {
        if let Some(service) = self.service.take() {
            service.stop().await;
        }
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
    }

    async fn fail(mut self, why: &str) -> ! {
        let logs = match self.service.take() {
            Some(s) => s.kill().await,
            None => String::new(),
        };
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
        panic!("{why}\n{logs}");
    }
}

/// Calls placed, answered, held with a tone and hung up — and the node's
/// own view of the same seconds in the same report.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_small_profile_places_calls_and_measures_both_views() {
    let Some(node) = Node::start().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let options = RunOptions {
        duration: Duration::from_secs(6),
        call_secs: 2,
        cps: 2.0,
        // Behaviour, not throughput: nothing refused, nothing dropped,
        // no leg left silent.
        budget: Budget {
            max_refused_fraction: Some(0.0),
            max_dropped: Some(0),
            max_silent_legs: Some(0),
            ..Budget::default()
        },
        ..node.options(Profile::Soak)
    };

    let report = match fcp_loadgen::run(options).await {
        Ok(report) => report,
        Err(e) => node.fail(&format!("the run failed: {e}")).await,
    };
    let m = &report.measurements;
    let summary = report.summary();

    if m.offered < 4 {
        node.fail(&format!("too few calls were offered:\n{summary}"))
            .await;
    }
    if m.answered != m.offered {
        node.fail(&format!("not every call was answered:\n{summary}"))
            .await;
    }
    if m.invite_to_answer.count != m.answered || m.invite_to_answer.p50_ms <= 0.0 {
        node.fail(&format!("answer latency was not measured:\n{summary}"))
            .await;
    }
    // Both sides of every answered call are measured: the caller's leg
    // and the answering phone's.
    if m.media.legs != m.answered * 2 || m.media.silent_legs > 0 {
        node.fail(&format!(
            "both legs of every answered call should have heard audio:\n{summary}"
        ))
        .await;
    }
    if m.media.loss_fraction > 0.05 {
        node.fail(&format!("the relayed audio was lossy:\n{summary}"))
            .await;
    }
    if !report.within_budget() {
        node.fail(&format!("a budget was breached:\n{summary}"))
            .await;
    }
    // The node's own view of the same seconds, from /metrics and /proc.
    if report.samples.is_empty() {
        node.fail(&format!("the node was never sampled:\n{summary}"))
            .await;
    }
    if !report
        .samples
        .iter()
        .any(|s| s.rss_bytes.unwrap_or(0) > 0 && !s.metrics.is_empty())
    {
        node.fail(&format!(
            "no sample carried both the node's memory and its metrics:\n{summary}"
        ))
        .await;
    }
    // The report is a file, and reads back as one.
    let json = serde_json::to_string(&report).expect("the report serialises");
    let read: serde_json::Value = serde_json::from_str(&json).expect("and reads back");
    assert_eq!(read["profile"], "soak");
    assert!(read["measurements"]["answered"].as_u64().unwrap_or(0) >= 4);

    node.stop().await;
}

/// Every device registering at once, then again as their registrations
/// lapse: the registrar answers all of them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_register_storm_is_answered() {
    let Some(node) = Node::start().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let options = RunOptions {
        // Two rounds of registrations inside the run: the storm, then
        // the refresh a quarter before the lifetime lapses.
        duration: Duration::from_secs(5),
        register_expiry: 4,
        budget: Budget {
            max_p99_register_ms: Some(5_000.0),
            ..Budget::default()
        },
        ..node.options(Profile::RegisterStorm)
    };

    let report = match fcp_loadgen::run(options).await {
        Ok(report) => report,
        Err(e) => node.fail(&format!("the run failed: {e}")).await,
    };
    let m = &report.measurements;
    let summary = report.summary();
    // Callers and their answering phones, registered at least twice.
    if m.registers < u64::from(USERS) * 4 {
        node.fail(&format!(
            "the devices did not re-register within the run:\n{summary}"
        ))
        .await;
    }
    if m.register_outcomes.get("200").copied().unwrap_or(0) != m.registers {
        node.fail(&format!("a REGISTER was not answered 200:\n{summary}"))
            .await;
    }
    if !m.outcomes.is_empty() {
        node.fail(&format!(
            "a storm places no calls, so no call outcomes:\n{summary}"
        ))
        .await;
    }
    if m.register_latency.count != m.registers || !report.within_budget() {
        node.fail(&format!("register latency was not measured:\n{summary}"))
            .await;
    }
    node.stop().await;
}

/// The ramp's ladder: each rung is measured on its own, the run's totals
/// are every rung together, and the climb stops where it is told to.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ramp_climbs_and_reports_every_rung() {
    let Some(node) = Node::start().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let options = RunOptions {
        duration: Duration::from_secs(10),
        call_secs: 1,
        cps: 1.0,
        cps_step: 1.0,
        // Two rungs, and the ladder's top is the reason it stops.
        cps_max: 2.0,
        rung_secs: 3,
        ..node.options(Profile::CallRamp)
    };

    let report = match fcp_loadgen::run(options).await {
        Ok(report) => report,
        Err(e) => node.fail(&format!("the run failed: {e}")).await,
    };
    let summary = report.summary();
    if report.rungs.len() != 2 {
        node.fail(&format!("two rungs were asked for:\n{summary}"))
            .await;
    }
    if report.rungs[0].offered_cps != 1.0 || report.rungs[1].offered_cps != 2.0 {
        node.fail(&format!("the ladder did not climb:\n{summary}"))
            .await;
    }
    if report.rungs.iter().any(|r| r.stopped_because.is_some()) {
        node.fail(&format!(
            "a rung of two calls a second should hold:\n{summary}"
        ))
        .await;
    }
    // The run's totals are the rungs added up — the second rung offers
    // more than the first, so neither is the whole run.
    let from_rungs: u64 = report.rungs.iter().map(|r| r.measurements.offered).sum();
    if report.measurements.offered != from_rungs || from_rungs == 0 {
        node.fail(&format!(
            "the totals are every rung together ({from_rungs} offered):\n{summary}"
        ))
        .await;
    }
    if report.measurements.invite_to_answer.count != report.measurements.answered {
        node.fail(&format!(
            "every rung's latencies belong to the run:\n{summary}"
        ))
        .await;
    }
    node.stop().await;
}

/// Calls and registrations at once, which is what a PBX sees.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mixed_profile_does_both_at_once() {
    let Some(node) = Node::start().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let options = RunOptions {
        duration: Duration::from_secs(6),
        call_secs: 1,
        cps: 2.0,
        register_expiry: 4,
        ..node.options(Profile::Mixed)
    };

    let report = match fcp_loadgen::run(options).await {
        Ok(report) => report,
        Err(e) => node.fail(&format!("the run failed: {e}")).await,
    };
    let m = &report.measurements;
    let summary = report.summary();
    if m.offered == 0 || m.answered != m.offered {
        node.fail(&format!("calls should have been placed:\n{summary}"))
            .await;
    }
    // The devices registered at the start and again as they lapsed,
    // while the calls were going on.
    if m.registers <= u64::from(USERS) * 2 {
        node.fail(&format!(
            "the devices should have re-registered under load:\n{summary}"
        ))
        .await;
    }
    node.stop().await;
}
