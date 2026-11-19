//! #336: a tenant's trunk was sent OPTIONS by the call manager, the
//! conference server, the IVR and the voicemail server. On the cluster's
//! Redis, a server that is not a call manager now takes each trunk's
//! health from what the call managers publish, and probes nothing.
//!
//! A call manager and an IVR share a database and a Redis; the tenant's
//! trunk points at a carrier in the test that notes where each OPTIONS
//! came from, answers for a while, then goes quiet. Needs
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`; skipped otherwise.

use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::media::allocate_rtp_range;
use fcp_testkit::smoke::{free_port, loopback, toml_path};
use fcp_testkit::{Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::UdpSocket;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-ivr");

/// A carrier that answers OPTIONS while `answering`, and remembers the
/// source of every one it was sent.
struct Carrier {
    port: u16,
    sources: Arc<Mutex<Vec<SocketAddr>>>,
    answering: Arc<AtomicBool>,
}

impl Carrier {
    async fn start() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("carrier binds");
        let port = socket.local_addr().unwrap().port();
        let sources = Arc::new(Mutex::new(Vec::new()));
        let answering = Arc::new(AtomicBool::new(true));
        let (seen, answer) = (Arc::clone(&sources), Arc::clone(&answering));
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                let Ok((n, from)) = socket.recv_from(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                if !request.starts_with("OPTIONS ") {
                    continue;
                }
                seen.lock().unwrap().push(from);
                if answer.load(Ordering::SeqCst) {
                    let _ = socket.send_to(ok_for(&request).as_bytes(), from).await;
                }
            }
        });
        Self {
            port,
            sources,
            answering,
        }
    }

    fn probes_from(&self, port: u16) -> usize {
        self.sources
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.port() == port)
            .count()
    }

    fn all_sources(&self) -> Vec<SocketAddr> {
        self.sources.lock().unwrap().clone()
    }
}

/// `200 OK` to `request`, with the headers a response copies.
fn ok_for(request: &str) -> String {
    let mut response = String::from("SIP/2.0 200 OK\r\n");
    for line in request.lines() {
        let name = line
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "via" | "v" | "from" | "f" | "call-id" | "i" | "cseq"
        ) {
            response.push_str(line.trim_end());
            response.push_str("\r\n");
        } else if matches!(name.as_str(), "to" | "t") {
            response.push_str(line.trim_end());
            response.push_str(";tag=carrier\r\n");
        }
    }
    response.push_str("Content-Length: 0\r\n\r\n");
    response
}

/// `log` without its colours.
fn plain(log: &str) -> String {
    let mut out = String::with_capacity(log.len());
    let mut chars = log.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

async fn eventually(within: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    check()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_call_manager_probes_a_trunk_and_the_ivr_follows_it() {
    let Some(redis) = private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping");
        return;
    };
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let seed = Seed::new(db.url(), "follow.local").await;
    let carrier = Carrier::start().await;
    // In the database before either node starts: the IVR, with no event
    // bus to hear of a new trunk, reads them at startup.
    seed.trunk_with(seed.tenant_id, "tenant-carrier", carrier.port, |trunk| {
        trunk.failover.health_check_enabled = true;
        trunk.failover.health_check_interval_secs = 1;
        trunk.failover.max_failures = 2;
    })
    .await;

    let dir = fcp_testkit::smoke::temp_dir();
    let (cm_sip, cm_http) = (free_port(), free_port());
    let rtp_start = allocate_rtp_range(60);
    let cm_config = format!(
        r#"[service]
node_id = "follow-cm"

[sip]
udp_bind = "127.0.0.1:{cm_sip}"
tcp_bind = "127.0.0.1:{cm_sip}"
local_ip = "127.0.0.1"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{cm_http}"

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
        dir = toml_path(dir.path()),
        rtp_end = rtp_start + 60,
    );
    let call_manager = Service::start(
        "call-manager",
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &cm_config,
        &loopback(cm_http, "/health"),
    )
    .await;

    let (ivr_sip, ivr_http) = (free_port(), free_port());
    let ivr_config = format!(
        r#"[service]
node_id = "follow-ivr"

[sip]
udp_bind = "127.0.0.1:{ivr_sip}"
tcp_bind = "127.0.0.1:{ivr_sip}"
local_ip = "127.0.0.1"
local_uri = "sip:ivr@127.0.0.1:{ivr_sip}"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[http]
bind_address = "127.0.0.1:{ivr_http}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
    );
    let ivr = Service::start("ivr", BIN, &ivr_config, &loopback(ivr_http, "/health")).await;

    // The call manager probes the trunk; over the same while, the IVR,
    // which used to probe every 5 s, sends it nothing.
    assert!(
        eventually(Duration::from_secs(15), || carrier.probes_from(cm_sip) >= 2).await,
        "the call manager probes the tenant's trunk: {:?}",
        carrier.all_sources()
    );
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        carrier.probes_from(ivr_sip),
        0,
        "the IVR probes nothing: {:?}",
        carrier.all_sources()
    );

    // The carrier goes quiet: the call manager finds it down, publishes
    // that, and the IVR takes it as its own finding.
    carrier.answering.store(false, Ordering::SeqCst);
    let mut ivr_log = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut ivr = Some(ivr);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let (code, body) = fcp_testkit::smoke::http_get(&loopback(cm_http, "/trunks"))
            .await
            .expect("the call manager answers");
        assert_eq!(code, 200, "{body}");
        if body.contains("\"health\":\"down\"") {
            // Two of the IVR's reads later, it has the call manager's view.
            tokio::time::sleep(Duration::from_secs(12)).await;
            ivr_log = ivr.take().unwrap().stop().await;
            break;
        }
    }
    let ivr_log = match ivr {
        Some(ivr) => panic!(
            "the call manager never found the trunk down\n{}",
            ivr.stop().await
        ),
        None => ivr_log,
    };
    let ivr_log = plain(&ivr_log);
    assert!(
        ivr_log
            .lines()
            .any(|l| l.contains("as the call managers found it") && l.contains("to=\"down\"")),
        "the IVR follows the call manager's finding:\n{ivr_log}"
    );
    assert_eq!(
        carrier.probes_from(ivr_sip),
        0,
        "the IVR probed nothing throughout: {:?}",
        carrier.all_sources()
    );
    call_manager.stop().await;
    db.drop_db().await;
}
