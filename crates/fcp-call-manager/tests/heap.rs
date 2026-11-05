//! What a node keeps per call (#344): `FCP_HEAP_CALLS` answered calls
//! through a call manager, then its RSS and, when built with
//! `--features dhat-heap`, its heap profile in `FCP_HEAP_DIR`.
//! A probe, not a gate: skipped unless `FCP_HEAP_CALLS` is set.

use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
const PAIRS: usize = 4;

fn rss_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_a_node_keeps_per_call() {
    let Some(calls) = std::env::var("FCP_HEAP_CALLS")
        .ok()
        .and_then(|n| n.parse::<usize>().ok())
    else {
        eprintln!("skipped: FCP_HEAP_CALLS is not set");
        return;
    };
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let out = std::env::var("FCP_HEAP_DIR").unwrap_or_else(|_| ".".into());
    let seed = Seed::new(db.url(), REALM).await;
    let sip_port = free_port();
    let http = free_port();
    let rtp_start = allocate_port_pair().rtp_port;
    let dir = tempfile::tempdir().expect("tempdir");
    let config = format!(
        r#"
[service]
node_id = "e2e-heap"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"
max_messages_per_source = 0

[security]
max_invite_cps_per_source = 0

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "warn"
format = "compact"
"#,
        db = db.url(),
        rtp_end = rtp_start + 200,
        dir = dir.path().display(),
    );
    // SAFETY: set before the node is spawned; the test is single-purpose.
    std::env::set_var("FCP_DHAT_FILE", format!("{out}/dhat-{calls}.json"));
    let service = Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
    let pid = service.pid().expect("pid");
    let sip = SocketAddr::from(([127, 0, 0, 1], sip_port));

    let mut pairs = Vec::new();
    for i in 0..PAIRS {
        let a = seed
            .user(&format!("a{i}"), "pw", &format!("{}", 1100 + i))
            .await;
        let b = seed
            .user(&format!("b{i}"), "pw", &format!("{}", 1200 + i))
            .await;
        let pa = Phone::new(&a.username, "pw", REALM, sip).await.unwrap();
        let pb = Phone::new(&b.username, "pw", REALM, sip).await.unwrap();
        assert_eq!(pa.register().await.unwrap(), 200);
        assert_eq!(pb.register().await.unwrap(), 200);
        pairs.push((Arc::new(pa), Arc::new(pb), b.extension.clone()));
    }

    let warm = calls / 10;
    let mut rss_after_warm = 0;
    let per_pair = calls / PAIRS;
    let warm_per_pair = warm / PAIRS;
    let barrier = Arc::new(tokio::sync::Barrier::new(PAIRS + 1));
    let mut tasks = Vec::new();
    for (pa, pb, ext) in pairs.clone() {
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            for n in 0..per_pair {
                if n == warm_per_pair {
                    barrier.wait().await;
                    barrier.wait().await;
                }
                let call = pa.call(&ext).await.unwrap();
                let Some(incoming) = pb.incoming(SOON).await else {
                    panic!(
                        "call {n} to {ext} never rang: {:?}",
                        call.failed(SOON).await
                    );
                };
                let answered = incoming.answer().await.expect("answers");
                call.answered(SOON).await.expect("answered");
                call.media().play_tone(1000.0, 200);
                answered.media().play_tone(500.0, 200);
                tokio::time::sleep(Duration::from_millis(300)).await;
                if let Err(e) = call.hangup().await {
                    panic!("call {n}: BYE: {e:#}");
                }
                assert!(answered.ended(SOON).await);
            }
        }));
    }
    barrier.wait().await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    rss_after_warm = rss_after_warm.max(rss_kb(pid));
    barrier.wait().await;
    for t in tasks {
        t.await.unwrap();
    }
    // Let anything kept for a while after a call (timers, linger) go.
    tokio::time::sleep(Duration::from_secs(40)).await;
    let rss_end = rss_kb(pid);
    eprintln!(
        "calls={calls} warm={warm} rss_after_warm_kb={rss_after_warm} rss_end_kb={rss_end} \
         per_call_bytes={}",
        (rss_end.saturating_sub(rss_after_warm) * 1024) / (calls - warm).max(1) as u64
    );
    let logs = service.stop().await;
    if std::env::var("FCP_HEAP_LOGS").is_ok() {
        eprintln!("{logs}");
    }
    db.drop_db().await;
}
