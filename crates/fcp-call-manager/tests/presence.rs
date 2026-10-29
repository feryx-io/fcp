//! Presence and busy lamps over SIP in any tenant (RFC 3856, RFC 4235;
//! `docs/SOFTPHONE.md` phase A3): a phone in a tenant other than the
//! node's default subscribes to a colleague's `dialog` and is told the
//! lamp at once and when the colleague is called — the handler used to
//! keep every subscription in the default tenant, where the calls'
//! presence never lands. An event nothing here takes is answered 489, not
//! the 500 of the in-dialog path. Skipped without `FCP_TEST_DATABASE_URL`.
//!
//! A person's presence on the phones (`docs/PRESENCE.md` phase 2): what
//! the admin announces as `presence.user_changed` is what a watcher's
//! NOTIFY says — the status they chose with its label, on the phone over
//! it, a new note, signed out — and the first NOTIFY reads it from the
//! database; a queue offers calls to an agent who chose Busy, and not to
//! one in do not disturb. Those also need `FCP_TEST_REDIS_URL`.

use fcp_core::presence_status::{ChosenStatus, OwnStatus, PresenceStatusStore, StatusState};
use fcp_core::storage::CredentialStore;
use fcp_events::{EventPublisher, PlatformEvent, PresenceEvent};
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

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

/// A call manager on Redis's event bus with presence and `extra`, as the
/// compose stack runs one: the admin's announcements reach it there.
async fn node_on_redis(
    db: &str,
    redis: &str,
    dir: &std::path::Path,
    extra: &str,
) -> (Service, SocketAddr) {
    let (sip_port, http) = (free_port(), free_port());
    let rtp_start = allocate_port_pair().rtp_port;
    let config = format!(
        r#"
[service]
node_id = "e2e-people"

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
redis_url = "{redis}"

[events]
use_redis = true

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
level = "info"
format = "compact"
{extra}
"#,
        rtp_end = rtp_start + 200,
        dir = dir.display(),
    );
    let service = Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
    (service, SocketAddr::from(([127, 0, 0, 1], sip_port)))
}

/// The admin, as far as the node can tell: announcements on Redis's bus
/// under a node of its own.
async fn admin_bus(redis: &str) -> fcp_storage_redis::event_bus::RedisEventBus {
    fcp_storage_redis::event_bus::RedisEventBus::new(
        fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
            url: redis.to_string(),
            ..Default::default()
        })
        .await
        .unwrap(),
        "e2e-admin".into(),
    )
}

/// `presence.user_changed` for a user, as the admin announces it.
fn user_changed(
    seed: &Seed,
    user: &fcp_core::storage::User,
    state: &str,
    label: Option<&str>,
    note: Option<&str>,
) -> PlatformEvent {
    PlatformEvent::Presence(PresenceEvent::UserChanged {
        user_id: user.id.to_string(),
        username: user.username.clone(),
        extension: user.extension.clone(),
        state: state.into(),
        label: label.map(String::from),
        note: note.map(String::from),
        idle: false,
        tenant_id: seed.tenant_id,
        timestamp: SystemTime::now(),
    })
}

/// The first presence NOTIFY whose body has every one of `wanted`, or
/// the last one seen.
async fn pidf(phone: &Phone, wanted: &[&str]) -> Option<String> {
    let deadline = tokio::time::Instant::now() + SOON;
    let mut last = None;
    while tokio::time::Instant::now() < deadline {
        if let Some(PhoneEvent::Notify { event, body, .. }) =
            phone.next_event(Duration::from_millis(250)).await
        {
            if event.starts_with("presence") {
                if wanted.iter().all(|w| body.contains(w)) {
                    return Some(body);
                }
                last = Some(body);
            }
        }
    }
    last
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_watcher_sees_the_persons_presence_over_their_calls() {
    let (Some(db), Some(redis)) = (TestDatabase::create().await, private_redis_url()) else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let dir = tempfile::tempdir().unwrap();
    let (service, sip) = node_on_redis(db.url(), &redis, dir.path(), "").await;

    // Bob chose a status of his own, "At lunch" (away), before the node
    // started hearing of him.
    let carol = seed.user("carol", "Carol-pw-2026!", "2001").await;
    let bob = seed.user("bob", "Bob-pw-2026!", "2002").await;
    let storage = seed.storage();
    let bob_row = storage
        .get_user_in_tenant("bob", &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let lunch = OwnStatus {
        id: fcp_core::presence_status::new_status_id(),
        tenant_id: seed.tenant_id,
        user_id: bob_row.id,
        label: "At lunch".into(),
        state: StatusState::Away,
        position: 0,
        updated_at: SystemTime::now(),
        deleted_at: None,
    };
    storage.put_own_status(&lunch).await.unwrap();
    storage
        .put_chosen_status(&ChosenStatus {
            tenant_id: seed.tenant_id,
            user_id: bob_row.id,
            status: lunch.id.clone(),
            note: None,
            until: None,
            updated_at: SystemTime::now(),
        })
        .await
        .unwrap();
    let bob_phone = Phone::new(&bob.username, &bob.password, REALM, sip)
        .await
        .unwrap();
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    // Carol watches him: told at once, from the database.
    let carol_phone = Phone::new(&carol.username, &carol.password, REALM, sip)
        .await
        .unwrap();
    let sub = carol_phone
        .subscribe(&format!("sip:bob@{REALM}"), "presence", 600, None)
        .await
        .unwrap();
    assert_eq!(sub.code, 200, "the subscription is accepted");
    let first = pidf(&carol_phone, &["<rpid:away/>"])
        .await
        .expect("his presence at once");
    assert!(first.contains("<basic>open</basic>"), "{first}");
    assert!(first.contains("<rpid:away/>"), "away: {first}");
    assert!(first.contains("At lunch"), "with his label: {first}");

    // On a call: on the phone, his label kept.
    let bus = admin_bus(&redis).await;
    let say = |state, note| user_changed(&seed, &bob_row, state, Some("At lunch"), note);
    bus.publish(say("on_call", None)).await.unwrap();
    let body = pidf(&carol_phone, &["on-the-phone"])
        .await
        .expect("told he is on a call");
    assert!(body.contains("<rpid:on-the-phone/>"), "{body}");
    assert!(body.contains("On the phone · At lunch"), "{body}");

    // The call over, a note: away again, with it.
    bus.publish(say("away", Some("back at 2"))).await.unwrap();
    let body = pidf(&carol_phone, &["<rpid:away/>"])
        .await
        .expect("told he is away again");
    assert!(body.contains("<rpid:away/>"), "{body}");
    assert!(body.contains("At lunch — back at 2"), "{body}");

    // A new note alone is told too.
    bus.publish(say("away", Some("back at 3"))).await.unwrap();
    let body = pidf(&carol_phone, &["back at 3"])
        .await
        .expect("told the new note");
    assert!(body.contains("At lunch — back at 3"), "{body}");

    // Signed out everywhere: closed.
    bus.publish(user_changed(&seed, &bob_row, "offline", None, None))
        .await
        .unwrap();
    let body = pidf(&carol_phone, &["closed"])
        .await
        .expect("told he signed out");
    assert!(body.contains("<basic>closed</basic>"), "{body}");

    service.stop().await;
    db.drop_db().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_queue_offers_calls_to_an_agent_who_chose_busy_and_not_to_one_in_dnd() {
    let (Some(db), Some(redis)) = (TestDatabase::create().await, private_redis_url()) else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let dir = tempfile::tempdir().unwrap();
    // Eve is first in line, then Dana.
    let (service, sip) = node_on_redis(
        db.url(),
        &redis,
        dir.path(),
        r#"
[queue]
enabled = true

[queue.queues.support]
name = "support"
extension = "7000"
strategy = "linear"
agents = ["eve", "dana"]
"#,
    )
    .await;

    let alice = seed.user("alice", "Alice-pw-2026!", "2001").await;
    let dana = seed.user("dana", "Dana-pw-2026!", "2004").await;
    let eve = seed.user("eve", "Eve-pw-2026!", "2005").await;
    let phone = |u: &fcp_testkit::seed::SeededUser| {
        let (name, pw) = (u.username.clone(), u.password.clone());
        async move {
            let p = Phone::new(&name, &pw, REALM, sip).await.unwrap();
            assert_eq!(p.register().await.unwrap(), 200, "{name} registers");
            p
        }
    };
    let (alice_phone, dana_phone, eve_phone) =
        (phone(&alice).await, phone(&dana).await, phone(&eve).await);
    let _dana_in = dana_phone.call("*50").await.unwrap();
    let _eve_in = eve_phone.call("*50").await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Dana chose Busy; Eve is in do not disturb.
    let storage = seed.storage();
    let row = |name: &'static str| async move {
        storage
            .get_user_in_tenant(name, &seed.tenant_id)
            .await
            .unwrap()
            .unwrap()
    };
    let bus = admin_bus(&redis).await;
    bus.publish(user_changed(&seed, &row("dana").await, "busy", None, None))
        .await
        .unwrap();
    bus.publish(user_changed(&seed, &row("eve").await, "dnd", None, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Alice calls the queue: Dana is offered it, Eve is passed over.
    let call = alice_phone.call("7000").await.unwrap();
    let offered = dana_phone.incoming(Duration::from_secs(15)).await;
    let eve_rang = eve_phone
        .incoming(Duration::from_millis(200))
        .await
        .is_some();
    if offered.is_none() || eve_rang {
        let logs = service.kill().await;
        panic!(
            "Dana offered: {}, Eve rang: {eve_rang}\n{logs}",
            offered.is_some()
        );
    }
    let _ = call.hangup().await;

    service.stop().await;
    db.drop_db().await;
}
