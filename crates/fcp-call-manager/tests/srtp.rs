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

/// A call manager with a TLS listener, and the way in to it.
struct Stack {
    service: Service,
    seed: Seed,
    db: TestDatabase,
    udp: SocketAddr,
    tls: SocketAddr,
    ca: ClusterCa,
    _dir: tempfile::TempDir,
}

async fn stack() -> Option<Stack> {
    let db = TestDatabase::create().await?;
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

    Some(Stack {
        service,
        seed,
        db,
        udp,
        tls,
        ca,
        _dir: dir,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_softphone_registered_over_tls_is_offered_srtp() {
    let Some(Stack {
        service,
        seed,
        db,
        udp,
        tls,
        ca,
        _dir,
    }) = stack().await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
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

/// An encrypted caller who holds and resumes is answered by the node with
/// its own key each time — not in the clear, which would tell a phone to
/// drop its encryption while the node still expects it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_encrypted_caller_on_hold_keeps_its_key() {
    let Some(Stack {
        service,
        seed,
        db,
        udp,
        _dir,
        ..
    }) = stack().await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;
    let bob_phone = Phone::new(&bob.username, &bob.password, REALM, udp)
        .await
        .unwrap();
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    let alice_phone = Phone::new(&alice.username, &alice.password, REALM, udp)
        .await
        .unwrap();
    assert_eq!(alice_phone.register().await.unwrap(), 200);

    let crypto = "1 AES_CM_128_HMAC_SHA1_80 inline:WVNfX19zZW1jdGwgKCkgewkyMjA7fQp9CnVubGVz";
    let call = alice_phone
        .call_offering_srtp(&bob.extension, crypto)
        .await
        .unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        let logs = service.kill().await;
        panic!("Bob's phone did not ring\n{logs}");
    };
    let bob_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.expect("answered");
    let key_of = |sdp: &str| {
        sdp.lines()
            .find_map(|l| l.strip_prefix("a=crypto:"))
            .map(str::to_string)
    };
    let first = call.answer_sdp().expect("the answer");
    let node_key = key_of(&first).unwrap_or_else(|| panic!("answered in the clear: {first}"));

    for (direction, answered) in [("sendonly", "recvonly"), ("sendrecv", "sendrecv")] {
        let (code, answer) = call
            .reinvite_offering_srtp(direction, crypto)
            .await
            .unwrap();
        assert_eq!(code, 200, "{direction}: {answer}");
        assert!(
            answer.contains(" RTP/SAVP "),
            "{direction} answered in the clear: {answer}"
        );
        assert_eq!(
            key_of(&answer).as_deref(),
            Some(node_key.as_str()),
            "{direction}: the node's key changed: {answer}"
        );
        assert!(
            answer.contains(&format!("a={answered}")),
            "{direction}: {answer}"
        );
    }

    call.hangup().await.unwrap();
    assert!(bob_call.ended(SOON).await);
    service.stop().await;
    db.drop_db().await;
}
