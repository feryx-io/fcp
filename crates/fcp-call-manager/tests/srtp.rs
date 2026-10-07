//! SDES-SRTP toward a softphone (`docs/SOFTPHONE.md` phase A5): a device
//! registered over TLS is offered `RTP/SAVP` with the node's key for the
//! call, a device of the same user registered over UDP is offered plain RTP,
//! and the encrypting device's answer is taken. Skipped without
//! `FCP_TEST_DATABASE_URL`.

use fcp_testkit::certs::ClusterCa;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{loopback, toml_path};
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_softphone_registered_over_tls_is_offered_srtp() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let (sip, sips, http) = (free_port(), free_port(), free_port());
    let rtp_start = allocate_port_pair().rtp_port;
    let dir = tempfile::tempdir().unwrap();
    let ca = ClusterCa::new(dir.path());
    let cert = ca.node("e2e-srtp");
    let config = format!(
        r#"
[service]
node_id = "e2e-srtp"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
sips_bind = "127.0.0.1:{sips}"
tls_cert_path = "{cert}"
tls_key_path = "{key}"
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
call_timeout_seconds = 5

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
        cert = toml_path(&cert.cert_path),
        key = toml_path(&cert.key_path),
        db = db.url(),
        dir = toml_path(dir.path()),
        rtp_end = rtp_start + 20,
    );
    let service = Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
    let (udp, tls) = (
        SocketAddr::from(([127, 0, 0, 1], sip)),
        SocketAddr::from(([127, 0, 0, 1], sips)),
    );

    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;
    // Bob's softphone over TLS, his desk phone over UDP.
    let softphone = Phone::new_tls(&bob.username, &bob.password, REALM, tls, &ca.ca_path)
        .await
        .unwrap();
    assert_eq!(softphone.register().await.unwrap(), 200);
    let desk = Phone::new(&bob.username, &bob.password, REALM, udp)
        .await
        .unwrap();
    assert_eq!(desk.register().await.unwrap(), 200);
    let alice_phone = Phone::new(&alice.username, &alice.password, REALM, udp)
        .await
        .unwrap();
    assert_eq!(alice_phone.register().await.unwrap(), 200);

    let call = alice_phone.call(&bob.extension).await.unwrap();
    let (Some(at_soft), Some(at_desk)) =
        (softphone.incoming(SOON).await, desk.incoming(SOON).await)
    else {
        let logs = service.kill().await;
        panic!("both of Bob's phones should ring\n{logs}");
    };
    let soft_offer = String::from_utf8_lossy(at_soft.body()).to_string();
    let desk_offer = String::from_utf8_lossy(at_desk.body()).to_string();
    assert!(soft_offer.contains(" RTP/SAVP "), "TLS: {soft_offer}");
    assert!(
        soft_offer.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:"),
        "TLS: {soft_offer}"
    );
    assert!(desk_offer.contains(" RTP/AVP "), "UDP: {desk_offer}");
    assert!(!desk_offer.contains("a=crypto"), "UDP: {desk_offer}");

    // The softphone answers; the call is up.
    let bob_call = at_soft.answer().await.unwrap();
    call.answered(SOON).await.expect("answered");
    call.hangup().await.unwrap();
    assert!(bob_call.ended(SOON).await);

    service.stop().await;
    db.drop_db().await;
}
