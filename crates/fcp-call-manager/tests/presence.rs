//! Presence and busy lamps over SIP in any tenant (RFC 3856, RFC 4235;
//! `docs/SOFTPHONE.md` phase A3): a phone in a tenant other than the
//! node's default subscribes to a colleague's `dialog` and is told the
//! lamp at once and when the colleague is called — the handler used to
//! keep every subscription in the default tenant, where the calls'
//! presence never lands. An event nothing here takes is answered 489, not
//! the 500 of the in-dialog path. Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
const GLOBEX: &str = "globex.e2e.local";

/// The next dialog NOTIFY's body.
async fn lamp(phone: &Phone) -> Option<String> {
    let deadline = tokio::time::Instant::now() + SOON;
    while tokio::time::Instant::now() < deadline {
        if let Some(PhoneEvent::Notify { event, body, .. }) =
            phone.next_event(Duration::from_millis(250)).await
        {
            if event.starts_with("dialog") {
                return Some(body);
            }
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_in_another_tenant_watches_a_colleagues_lamp() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let (sip_port, http) = (free_port(), free_port());
    let rtp_start = allocate_port_pair().rtp_port;
    let dir = tempfile::tempdir().unwrap();
    let config = format!(
        r#"
[service]
node_id = "e2e-presence"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[presence]
enabled = true

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
        db = db.url(),
        rtp_end = rtp_start + 20,
        dir = dir.path().display(),
    );
    let service = Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
    let sip = SocketAddr::from(([127, 0, 0, 1], sip_port));

    // Three people in a tenant that is not the node's default.
    let globex = seed.tenant("globex", GLOBEX).await;
    let carol = seed
        .user_in(globex, "carol-globex", "carol-pw", "2101")
        .await;
    let bob = seed.user_in(globex, "bob-globex", "bob-pw", "2102").await;
    let alice = seed
        .user_in(globex, "alice-globex", "alice-pw", "2103")
        .await;
    let phone = |u: &fcp_testkit::seed::SeededUser| {
        let (name, pw) = (u.username.clone(), u.password.clone());
        async move {
            let p = Phone::new(&name, &pw, GLOBEX, sip).await.unwrap();
            assert_eq!(p.register().await.unwrap(), 200, "{name} registers");
            p
        }
    };
    let (carol_phone, bob_phone, alice_phone) =
        (phone(&carol).await, phone(&bob).await, phone(&alice).await);

    // Carol watches Bob's lamp: subscribed, and told at once.
    let bob_aor = format!("sip:{}@{GLOBEX}", bob.username);
    let sub = carol_phone
        .subscribe(&bob_aor, "dialog", 600, None)
        .await
        .unwrap();
    if sub.code != 200 {
        let logs = service.kill().await;
        panic!("the dialog subscription was answered {}\n{logs}", sub.code);
    }
    let first = lamp(&carol_phone).await.expect("the lamp at once");
    assert!(first.contains(&bob.username), "{first}");
    assert!(!first.contains("<state>confirmed"), "idle: {first}");

    // Carol's own device, signing in with credentials of its own (an app
    // install, a hot desk), may watch as her.
    {
        use fcp_core::storage::{CredentialStore, DeviceStore};
        let owner = seed
            .storage()
            .get_user_in_tenant(&carol.username, &globex)
            .await
            .unwrap()
            .unwrap();
        let mut device = fcp_core::Device::new(
            "Carol's app".into(),
            fcp_core::DeviceType::Softphone,
            globex,
        );
        device.sip_username = Some("carol-app".into());
        device.sip_uri = Some(format!("sip:carol-app@{GLOBEX}"));
        device.sip_ha1_hash = Some(fcp_core::storage::password::compute_ha1(
            "carol-app",
            REALM,
            "carol-app-pw",
        ));
        device.owner_user_id = Some(owner.id);
        seed.storage().put_device(device).await.unwrap();
        let app =
            Phone::with_credentials(&carol.username, "carol-app", "carol-app-pw", GLOBEX, sip)
                .await
                .unwrap();
        let sub = app.subscribe(&bob_aor, "dialog", 600, None).await.unwrap();
        assert_eq!(sub.code, 200, "a device of Carol's watches as her");
        assert!(lamp(&app).await.is_some(), "and is told the lamp");
    }

    // A watcher whose phone has gone off the network: its NOTIFYs go
    // unanswered for half a minute, and nobody else's lamp waits on them.
    let gone = phone(&alice).await;
    assert_eq!(
        gone.subscribe(&bob_aor, "dialog", 600, None)
            .await
            .unwrap()
            .code,
        200
    );
    assert!(lamp(&gone).await.is_some());
    gone.fall_silent();

    // Alice calls Bob: Carol sees it.
    let call = alice_phone.call(&bob.extension).await.unwrap();
    let incoming = bob_phone.incoming(SOON).await.expect("Bob rings");
    let mut lit = false;
    let deadline = tokio::time::Instant::now() + SOON;
    while tokio::time::Instant::now() < deadline && !lit {
        if let Some(body) = lamp(&carol_phone).await {
            lit = body.contains("<dialog ");
        }
    }
    assert!(lit, "Carol is told Bob has a call");
    let bob_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);
    // And told when it ends: no live dialog left.
    let mut out = false;
    let deadline = tokio::time::Instant::now() + SOON;
    while tokio::time::Instant::now() < deadline && !out {
        if let Some(body) = lamp(&carol_phone).await {
            out = !body.contains("<state>early") && !body.contains("<state>confirmed");
        }
    }
    assert!(out, "Carol is told Bob's call is over");

    // An event nothing here takes: 489, with what is.
    let odd = carol_phone
        .subscribe(&bob_aor, "x-no-such-package", 600, None)
        .await
        .unwrap();
    assert_eq!(odd.code, 489);

    service.stop().await;
    db.drop_db().await;
}
