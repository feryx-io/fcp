//! The soak against the compose stack built from this checkout
//! (`docs/DEEP_TESTING.md` D6): every service the compose file runs by
//! default, started from its own configuration against the test's
//! Postgres and Redis (`ComposeStack::start_local`), then
//! `fcp_loadgen::soak::run` with each service sampled as a process of
//! this user's.
//!
//! In CI it runs for `FCP_SOAK_SECONDS` (90 by default) as a check that
//! the soak itself works — the seeding, the three loads, the API traffic,
//! a sample of every service and store — since a minute is too short to
//! say anything levelled off. Given ten minutes or more, it also fails on
//! growth, which is how a soak is run on a workstation:
//!
//! ```text
//! FCP_SOAK_SECONDS=3600 FCP_SOAK_REPORT=soak.json cargo test -p fcp-loadgen --test soak_stack -- --nocapture
//! ```
//!
//! The weekly soak runs `fcp-soak` against the images instead
//! (`.github/workflows/soak.yml`). The compose file's ports are used as
//! they are, so this is the one test in its binary.

use fcp_loadgen::soak::admin::Admin;
use fcp_loadgen::soak::stack::{MetricsSource, Process, StackProbe};
use fcp_loadgen::soak::{self, Room, SoakOptions};
use fcp_testkit::compose::ComposeStack;
use fcp_testkit::{cluster, TestDatabase};
use std::time::Duration;

/// A run this long is judged for growth.
const JUDGED_FROM: u64 = 600;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_stack_holds_steady_under_load() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the soak");
        return;
    };
    let Some(redis) = cluster::private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the soak");
        return;
    };
    let seconds: u64 = std::env::var("FCP_SOAK_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);
    let judged = seconds >= JUDGED_FROM;
    // One load at a time, to tell which one a growth belongs to:
    // `FCP_SOAK_CALLERS=0` leaves only the rooms, `FCP_SOAK_ROOMS=0` only
    // the calls.
    let callers: u32 = std::env::var("FCP_SOAK_CALLERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(if judged { 10 } else { 4 });
    let with_rooms = std::env::var("FCP_SOAK_ROOMS").as_deref() != Ok("0");

    let stack = ComposeStack::load();
    let local = stack.start_local(db.url(), &redis, |_, _, _| {}).await;
    let processes = local
        .services
        .iter()
        .map(|(name, service)| Process::Local {
            service: name.clone(),
            pid: service.pid().expect("a running service"),
        })
        .collect();
    let probe = StackProbe::new(
        processes,
        Some(db.url()),
        Some(&redis),
        vec![MetricsSource {
            service: "fcp-call-manager".into(),
            url: "http://127.0.0.1:8084/metrics".into(),
            token: Some(stack.dotenv("FCP_CALL_MANAGER_API_TOKEN")),
        }],
    )
    .await;

    let duration = Duration::from_secs(seconds);
    let options = SoakOptions {
        admin: Admin {
            base: "http://127.0.0.1:8080".into(),
            token: stack.dotenv("FCP_ADMIN_AUTH_TOKEN"),
        },
        sip: "127.0.0.1:5060".parse().unwrap(),
        realm: "fcp.local".into(),
        duration,
        warmup: (duration / 5).min(Duration::from_secs(30 * 60)),
        sample_interval: Duration::from_secs(if judged { 15 } else { 5 }),
        callers,
        cps: 0.5,
        call_secs: if judged { 20 } else { 8 },
        rooms: if with_rooms {
            vec![
                Room {
                    number: "8801".into(),
                    callers: 3,
                    call_secs: if judged { 30 } else { 10 },
                },
                Room {
                    number: "8802".into(),
                    callers: 2,
                    call_secs: if judged { 15 } else { 6 },
                },
            ]
        } else {
            Vec::new()
        },
        api_every: Duration::from_secs(if judged { 5 } else { 2 }),
    };
    let report = match soak::run(options, probe).await {
        Ok(report) => report,
        Err(e) => local.fail(&format!("the soak did not run: {e:#}")).await,
    };
    println!("{}", report.summary());
    if let Ok(path) = std::env::var("FCP_SOAK_REPORT") {
        std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
        println!("report written to {path}");
    }

    // Every load answered and heard, and the API answered.
    for (name, load) in &report.loads {
        let m = &load.measurements;
        if !load.within_budget() || m.answered == 0 {
            local.fail(&format!("{name}: {}", load.summary())).await;
        }
    }
    assert_eq!(report.api_failures, 0, "the API failed: {:?}", report.api);
    assert!(
        report
            .api
            .get("sign in")
            .is_some_and(|c| c.contains_key(&200)),
        "the API user signed in: {:?}",
        report.api
    );

    // Every service and store was sampled.
    let last = report.samples.last().expect("samples");
    for (name, _) in &local.services {
        for what in ["rss_bytes", "fds", "threads"] {
            let v = last.series.get(&format!("process/{name}/{what}"));
            assert!(v.is_some_and(|v| *v > 0.0), "{name} {what}: {v:?}");
        }
    }
    assert!(
        last.series
            .get("process/fcp-call-manager/udp_sockets")
            .is_some_and(|v| *v > 0.0),
        "the call manager's UDP sockets were counted"
    );
    for series in ["postgres/connections", "redis/keys", "table/users/rows"] {
        assert!(last.series.contains_key(series), "{series} was sampled");
    }
    assert!(
        last.series
            .keys()
            .any(|k| k.starts_with("metric/fcp-call-manager/")),
        "the call manager's gauges were read with its token"
    );

    if judged {
        let leaks: Vec<String> = report.leaks().iter().map(|v| v.series.clone()).collect();
        assert!(leaks.is_empty(), "grew and kept growing: {leaks:?}");
        assert!(report.passed(), "{}", report.summary());
    }

    for (_, service) in local.services.into_iter().rev() {
        let log = service.stop().await;
        assert!(!log.contains("panicked"), "a service panicked:\n{log}");
    }
    db.drop_db().await;
}
