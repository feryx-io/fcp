//! Push from a node (`docs/PUSH.md` phase P2): `[push]` read from the
//! node's file, the publisher's keys made ready, and a device's apps sent a
//! test push through the stand-in APNs and FCM; a token the provider no
//! longer knows costs its binding the push address. Skipped without
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

use fcp_core::storage::{CredentialStore, DeviceStore, Registration, RegistrationStore};
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::push::{StandInApns, StandInFcm, APNS_TEAM, APNS_TOPIC, FCM_PROJECT};
use fcp_testkit::smoke::loopback;
use fcp_testkit::{free_port, Seed, Service, TestDatabase};
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";

/// An app's binding at its user's address of record, as a push REGISTER
/// leaves it.
fn app_binding(
    tenant: fcp_core::TenantId,
    contact: &str,
    push: fcp_core::push::PushAddress,
) -> Registration {
    let now = SystemTime::now();
    Registration {
        tenant_id: tenant,
        aor: format!("sip:alice@{REALM}"),
        contact: contact.to_string(),
        expires_at: now + Duration::from_secs(604_800),
        call_id: format!("{}@app", uuid::Uuid::new_v4()),
        cseq: 1,
        user_agent: Some("Anvil/0.2.0".into()),
        q: None,
        registered_at: now,
        path: None,
        flow_token: None,
        source_address: None,
        instance_id: Some(format!("<urn:uuid:{}>", uuid::Uuid::new_v4())),
        reg_id: Some(1),
        push: Some(push),
        flow_node: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_is_sent_a_test_push_and_a_dead_token_is_let_go() {
    let (Some(db), Some(redis)) = (TestDatabase::create().await, private_redis_url()) else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let apns = StandInApns::start().await;
    let fcm = StandInFcm::start().await;
    let seed = Seed::new(db.url(), REALM).await;
    let (sip_port, http) = (free_port(), free_port());
    let rtp_start = allocate_port_pair().rtp_port;
    let dir = tempfile::tempdir().unwrap();
    // The keys in files, as an operator keeps them.
    let apns_key = dir.path().join("apns.p8");
    std::fs::write(&apns_key, apns.config().key.unwrap()).unwrap();
    let fcm_account = dir.path().join("fcm.json");
    std::fs::write(&fcm_account, fcm.config().service_account.unwrap()).unwrap();
    let config = format!(
        r#"
[service]
node_id = "e2e-push"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

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

[push]

[[push.apns]]
team_id = "{APNS_TEAM}"
key_id = "{apns_key_id}"
key_file = "{apns_key}"
topics = ["{APNS_TOPIC}"]
endpoint = "{apns_url}"

[[push.fcm]]
project_id = "{FCM_PROJECT}"
service_account_file = "{fcm_account}"
endpoint = "{fcm_url}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        rtp_end = rtp_start + 20,
        dir = dir.path().display(),
        apns_key_id = apns.config().key_id,
        apns_key = apns_key.display(),
        apns_url = apns.config().endpoint.unwrap(),
        fcm_account = fcm_account.display(),
        fcm_url = fcm.config().endpoint.unwrap(),
    );
    let service = Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;

    // Alice has two apps: an iPhone and an Android phone whose app was
    // removed (its token is gone at FCM), and a desk phone without push.
    seed.user("alice", "Alice-pw-2026!", "2001").await;
    let storage = seed.storage();
    let alice = storage
        .get_user_in_tenant("alice", &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let device = |name: &str, address: &str| {
        let mut d = fcp_core::Device::new(
            name.into(),
            fcp_core::device::DeviceType::Softphone,
            seed.tenant_id,
        );
        d.owner_user_id = Some(alice.id);
        d.last_ip_address = Some(address.into());
        d
    };
    let iphone = device("iPhone", "10.0.0.7:52311");
    let android = device("Android", "10.0.0.8:40000");
    let desk = device("Desk", "10.0.0.9:5060");
    for d in [&iphone, &android, &desk] {
        storage.put_device(d.clone()).await.unwrap();
    }
    storage
        .put_registration(app_binding(
            seed.tenant_id,
            "<sip:app-1@10.0.0.7:52311;transport=tls>",
            StandInApns::address("a1b2c3d4e5f6a7b8c9d0"),
        ))
        .await
        .unwrap();
    storage
        .put_registration(app_binding(
            seed.tenant_id,
            "<sip:app-2@10.0.0.8:40000;transport=tls>",
            StandInFcm::address("gone-token-0123456789"),
        ))
        .await
        .unwrap();
    fcm.forget("gone-token-0123456789");

    let http_client = reqwest::Client::new();
    let push_test = |id: String| {
        let http_client = http_client.clone();
        let tenant = seed.tenant_id;
        async move {
            let response = http_client
                .post(format!("http://127.0.0.1:{http}/devices/{id}/push-test"))
                .json(&serde_json::json!({ "tenant_id": tenant }))
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            (status, response.json::<serde_json::Value>().await.unwrap())
        }
    };

    // The iPhone: one VoIP push, taken.
    let (status, body) = push_test(iphone.id.to_string()).await;
    assert_eq!(status, 200, "{body}");
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{body}");
    assert_eq!(results[0]["provider"], "apns");
    assert_eq!(results[0]["outcome"], "sent", "{body} {:?}", apns.refused());
    let pushed = apns.pushes().pop().expect("APNs took a push");
    assert_eq!(pushed.kind(), "test");
    assert_eq!(pushed.token, "a1b2c3d4e5f6a7b8c9d0");
    assert_eq!(pushed.topic.as_deref(), Some(APNS_TOPIC));

    // The Android phone: FCM no longer knows the token, so the binding
    // keeps no push address — and stays, the app may be awake on it.
    let (status, body) = push_test(android.id.to_string()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["results"][0]["provider"], "fcm");
    assert_eq!(body["results"][0]["outcome"], "gone", "{body}");
    let rows = storage
        .get_registrations_in_tenant(&seed.tenant_id, &format!("sip:alice@{REALM}"))
        .await
        .unwrap();
    let android_row = rows
        .iter()
        .find(|r| r.contact.contains("10.0.0.8:40000"))
        .expect("the binding stays");
    assert_eq!(android_row.push, None);
    let iphone_row = rows
        .iter()
        .find(|r| r.contact.contains("10.0.0.7:52311"))
        .unwrap();
    assert!(iphone_row.push.is_some());

    // Nothing to wake: no pushes.
    let (status, body) = push_test(desk.id.to_string()).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["results"].as_array().unwrap().is_empty(), "{body}");

    // Another tenant's device is not this node's to push to.
    let (status, _) = http_client
        .post(format!(
            "http://127.0.0.1:{http}/devices/{}/push-test",
            iphone.id
        ))
        .json(&serde_json::json!({ "tenant_id": fcp_core::TenantId::new() }))
        .send()
        .await
        .map(|r| (r.status().as_u16(), ()))
        .unwrap();
    assert_eq!(status, 404);

    // Counted.
    let metrics = http_client
        .get(format!("http://127.0.0.1:{http}/metrics"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        metrics.contains(
            "fcp_push_total{node=\"e2e-push\",provider=\"apns\",kind=\"test\",outcome=\"sent\"} 1"
        ),
        "{metrics}"
    );
    assert!(
        metrics.contains("provider=\"fcm\",kind=\"test\",outcome=\"gone\"} 1"),
        "{metrics}"
    );
    drop(service);
}

// ===== Ringing a sleeping app (`docs/PUSH.md` phase P3) =====

use fcp_testkit::certs::ClusterCa;
use fcp_testkit::smoke::toml_path;
use fcp_testkit::Phone;
use std::net::SocketAddr;
use std::sync::Arc;

struct PushStack {
    service: Service,
    seed: Seed,
    udp: SocketAddr,
    tls: SocketAddr,
    ca: ClusterCa,
    apns: StandInApns,
    _db: TestDatabase,
    _dir: tempfile::TempDir,
}

/// One call manager's addresses.
struct PushNode {
    service: Service,
    udp: SocketAddr,
    tls: SocketAddr,
}

/// A call manager over UDP and TLS on `db` with `[push]` at the APNs
/// stand-in; ringing gives up after `ring_secs`. With `redis`, one node of
/// a cluster whose events go over it.
#[allow(clippy::too_many_arguments)]
async fn push_node(
    name: &str,
    db: &str,
    dir: &std::path::Path,
    ca: &ClusterCa,
    apns: &StandInApns,
    ring_secs: u64,
    redis: Option<&str>,
) -> PushNode {
    let (sip, sips, http) = (free_port(), free_port(), free_port());
    let rtp_start = allocate_port_pair().rtp_port;
    let cert = ca.node(name);
    let key = dir.join("apns.p8");
    std::fs::write(&key, apns.config().key.unwrap()).unwrap();
    let cluster = redis
        .map(|url| {
            format!(
                "redis_url = \"{url}\"\n\n[cluster]\nenabled = true\n\
                 heartbeat_interval_secs = 1\nheartbeat_ttl_secs = 3\n\n\
                 [events]\nuse_redis = true"
            )
        })
        .unwrap_or_default();
    let config = format!(
        r#"
[service]
node_id = "{name}"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
sips_bind = "127.0.0.1:{sips}"
tls_cert_path = "{cert}"
tls_key_path = "{tls_key}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[cdr]
spool_dir = "{dir}/{name}-cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 30

[b2bua.forking]
timeout = {ring_secs}

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/{name}-recordings"

[[push.apns]]
team_id = "{APNS_TEAM}"
key_id = "{key_id}"
key_file = "{key}"
topics = ["{APNS_TOPIC}"]
endpoint = "{url}"

[logging]
level = "info"
format = "compact"

[storage]
database_url = "{db}"
{cluster}
"#,
        cert = toml_path(&cert.cert_path),
        tls_key = toml_path(&cert.key_path),
        dir = toml_path(dir),
        rtp_end = rtp_start + 40,
        key_id = apns.config().key_id,
        key = toml_path(&key),
        url = apns.config().endpoint.unwrap(),
    );
    let service = Service::start(name, BIN, &config, &loopback(http, "/health")).await;
    PushNode {
        service,
        udp: SocketAddr::from(([127, 0, 0, 1], sip)),
        tls: SocketAddr::from(([127, 0, 0, 1], sips)),
    }
}

/// A call manager over UDP and TLS with `[push]` at the APNs stand-in;
/// ringing gives up after `ring_secs`.
async fn push_stack(name: &str, ring_secs: u64) -> Option<PushStack> {
    let db = TestDatabase::create().await?;
    let apns = StandInApns::start().await;
    let seed = Seed::new(db.url(), REALM).await;
    let dir = tempfile::tempdir().unwrap();
    let ca = ClusterCa::new(dir.path());
    let node = push_node(name, db.url(), dir.path(), &ca, &apns, ring_secs, None).await;
    Some(PushStack {
        service: node.service,
        seed,
        udp: node.udp,
        tls: node.tls,
        ca,
        apns,
        _db: db,
        _dir: dir,
    })
}

/// Bob's phone app: registered over TLS with its push address, then
/// asleep (its connection gone).
async fn sleeping_app(
    stack: &PushStack,
    bob: &fcp_testkit::seed::SeededUser,
    token: &str,
) -> Phone {
    let app = Phone::new_app(
        &bob.username,
        &bob.password,
        REALM,
        stack.tls,
        &stack.ca.ca_path,
        "<urn:uuid:00000000-0000-0000-0000-0000000000b1>",
    )
    .await
    .unwrap();
    app.set_push(Some(&format!(
        ";pn-provider=apns;pn-param={APNS_TEAM}.{APNS_TOPIC};pn-prid={token}"
    )))
    .unwrap();
    assert_eq!(app.register().await.unwrap(), 200);
    app.sleep().await;
    // The node notices the connection is gone.
    tokio::time::sleep(Duration::from_millis(500)).await;
    app
}

const RING: Duration = Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_wakes_a_sleeping_app_which_answers() {
    let Some(stack) = push_stack("e2e-push-wake", 20).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "Alice-pw-2026!", "2001").await;
    let bob = stack.seed.user("bob", "Bob-pw-2026!", "2002").await;
    let app = Arc::new(sleeping_app(&stack, &bob, "a1b2c3d4e5f6a7b8c9d0").await);
    app.auto_answer(true);

    // Woken by its push, the app registers again — a new connection — and
    // takes the call.
    let mut pushes = stack.apns.subscribe();
    let woken = {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            let push = StandInApns::next_push(&mut pushes, 15).await?;
            assert_eq!(app.register().await.ok()?, 200);
            Some(push)
        })
    };

    let caller = Phone::new(&alice.username, &alice.password, REALM, stack.udp)
        .await
        .unwrap();
    assert_eq!(caller.register().await.unwrap(), 200);
    let call = caller.call(&bob.extension).await.unwrap();
    if let Err(e) = call.answered(RING).await {
        let logs = stack.service.kill().await;
        panic!("the woken app should answer: {e}\n{logs}");
    }
    let push = woken.await.unwrap().expect("a push woke the app");
    assert_eq!(push.kind(), "call");
    assert_eq!(push.token, "a1b2c3d4e5f6a7b8c9d0");
    assert!(push.payload["call"].as_str().is_some_and(|c| !c.is_empty()));
    assert!(
        push.payload["from"].as_str().is_some(),
        "{:?}",
        push.payload
    );
    assert_eq!(call.hangup().await.unwrap(), 200);
    // The app came back, so no cancel.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(stack.apns.pushes().iter().all(|p| p.kind() == "call"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_desk_phone_answers_and_the_sleeping_app_is_told_the_call_is_over() {
    let Some(stack) = push_stack("e2e-push-cancel", 20).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "Alice-pw-2026!", "2001").await;
    let bob = stack.seed.user("bob", "Bob-pw-2026!", "2002").await;
    let _app = sleeping_app(&stack, &bob, "a1b2c3d4e5f6a7b8c9d0").await;
    let desk = Phone::new(&bob.username, &bob.password, REALM, stack.udp)
        .await
        .unwrap();
    assert_eq!(desk.register().await.unwrap(), 200);
    desk.auto_answer(true);

    let caller = Phone::new(&alice.username, &alice.password, REALM, stack.udp)
        .await
        .unwrap();
    assert_eq!(caller.register().await.unwrap(), 200);
    let mut pushes = stack.apns.subscribe();
    let call = caller.call(&bob.extension).await.unwrap();
    call.answered(RING).await.expect("the desk phone answers");
    let first = StandInApns::next_push(&mut pushes, 5)
        .await
        .expect("the call's push");
    let second = StandInApns::next_push(&mut pushes, 5)
        .await
        .expect("its cancel");
    assert_eq!(first.kind(), "call");
    assert_eq!(second.kind(), "cancel");
    assert_eq!(second.payload["call"], first.payload["call"]);
    assert_eq!(call.hangup().await.unwrap(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_app_that_cannot_be_woken_is_unreachable_at_once() {
    // A long ring time: the caller must not wait it out.
    let Some(stack) = push_stack("e2e-push-gone", 30).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "Alice-pw-2026!", "2001").await;
    let bob = stack.seed.user("bob", "Bob-pw-2026!", "2002").await;
    let _app = sleeping_app(&stack, &bob, "deadbeefdeadbeefdead").await;
    stack.apns.forget("deadbeefdeadbeefdead");

    let caller = Phone::new(&alice.username, &alice.password, REALM, stack.udp)
        .await
        .unwrap();
    assert_eq!(caller.register().await.unwrap(), 200);
    let started = std::time::Instant::now();
    let call = caller.call(&bob.extension).await.unwrap();
    let code = call.failed(Duration::from_secs(10)).await;
    if code != Some(480) {
        let logs = stack.service.kill().await;
        panic!("unreachable, not a ring that times out: {code:?}\n{logs}");
    }
    assert!(started.elapsed() < Duration::from_secs(10));

    // The binding lost its push address.
    let rows = stack
        .seed
        .storage()
        .get_registrations_in_tenant(&stack.seed.tenant_id, &format!("sip:bob@{REALM}"))
        .await
        .unwrap();
    assert!(rows.iter().all(|r| r.push.is_none()), "{rows:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_app_that_never_comes_back_is_no_answer_and_is_told_so() {
    let Some(stack) = push_stack("e2e-push-noanswer", 3).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "Alice-pw-2026!", "2001").await;
    let bob = stack.seed.user("bob", "Bob-pw-2026!", "2002").await;
    let _app = sleeping_app(&stack, &bob, "a1b2c3d4e5f6a7b8c9d0").await;

    let caller = Phone::new(&alice.username, &alice.password, REALM, stack.udp)
        .await
        .unwrap();
    assert_eq!(caller.register().await.unwrap(), 200);
    let mut pushes = stack.apns.subscribe();
    let call = caller.call(&bob.extension).await.unwrap();
    // Ringing while the app is woken; no answer within the ring time.
    let code = call.failed(Duration::from_secs(10)).await;
    assert_eq!(code, Some(480), "no answer");
    let first = StandInApns::next_push(&mut pushes, 2)
        .await
        .expect("the call's push");
    let second = StandInApns::next_push(&mut pushes, 5)
        .await
        .expect("its cancel");
    assert_eq!((first.kind(), second.kind()), ("call", "cancel"));
}

// ===== A phone's flow on another node (`docs/PUSH.md` phase P4) =====

struct PushCluster {
    a: PushNode,
    b: PushNode,
    seed: Seed,
    ca: ClusterCa,
    apns: StandInApns,
    _db: TestDatabase,
    _dir: tempfile::TempDir,
}

/// Two call managers on one database and one Redis, each knowing the
/// other from its heartbeats.
async fn push_cluster(name: &str) -> Option<PushCluster> {
    let redis = private_redis_url()?;
    let db = TestDatabase::create().await?;
    let apns = StandInApns::start().await;
    let seed = Seed::new(db.url(), REALM).await;
    let dir = tempfile::tempdir().unwrap();
    let ca = ClusterCa::new(dir.path());
    let (a_name, b_name) = (format!("{name}-a"), format!("{name}-b"));
    let a = push_node(&a_name, db.url(), dir.path(), &ca, &apns, 20, Some(&redis)).await;
    let b = push_node(&b_name, db.url(), dir.path(), &ca, &apns, 20, Some(&redis)).await;
    for node in [&a_name, &b_name] {
        fcp_testkit::cluster::heartbeat_when(&redis, node, Duration::from_secs(10), |_| true)
            .await
            .expect("the node heartbeats");
    }
    // Each reads the other's heartbeat on its next beat.
    tokio::time::sleep(Duration::from_secs(2)).await;
    Some(PushCluster {
        a,
        b,
        seed,
        ca,
        apns,
        _db: db,
        _dir: dir,
    })
}

/// Bob's app on `node` over TLS with a push address, awake.
async fn app_on(
    cluster: &PushCluster,
    node: &PushNode,
    bob: &fcp_testkit::seed::SeededUser,
    token: &str,
) -> Arc<Phone> {
    let app = Phone::new_app(
        &bob.username,
        &bob.password,
        REALM,
        node.tls,
        &cluster.ca.ca_path,
        "<urn:uuid:00000000-0000-0000-0000-0000000000b2>",
    )
    .await
    .unwrap();
    app.set_push(Some(&format!(
        ";pn-provider=apns;pn-param={APNS_TEAM}.{APNS_TOPIC};pn-prid={token}"
    )))
    .unwrap();
    assert_eq!(app.register().await.unwrap(), 200);
    Arc::new(app)
}

async fn alice_on(node: &PushNode, alice: &fcp_testkit::seed::SeededUser) -> Phone {
    let caller = Phone::new(&alice.username, &alice.password, REALM, node.udp)
        .await
        .unwrap();
    assert_eq!(caller.register().await.unwrap(), 200);
    caller
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_reaches_an_app_whose_flow_is_on_another_node() {
    let Some(cluster) = push_cluster("e2e-push-relay").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let alice = cluster.seed.user("alice", "Alice-pw-2026!", "2001").await;
    let bob = cluster.seed.user("bob", "Bob-pw-2026!", "2002").await;
    // Bob's app is connected to node B; Alice calls through node A, which
    // cannot write to that connection.
    let app = app_on(&cluster, &cluster.b, &bob, "a1b2c3d4e5f6a7b8c9d0").await;
    app.auto_answer(true);
    let caller = alice_on(&cluster.a, &alice).await;
    let call = caller.call(&bob.extension).await.unwrap();
    if let Err(e) = call.answered(RING).await {
        let a = cluster.a.service.kill().await;
        let b = cluster.b.service.kill().await;
        panic!("the app should answer through node B: {e}\n--- A\n{a}\n--- B\n{b}");
    }
    // Awake, so not pushed.
    assert!(
        cluster.apns.pushes().is_empty(),
        "{:?}",
        cluster.apns.pushes()
    );
    assert_eq!(call.hangup().await.unwrap(), 200);
    // One call, one record: node A's. Node B's leg is A's to record.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let records = loop {
        let records = fcp_core::storage::CdrStore::query_cdrs(
            cluster.seed.storage(),
            fcp_core::storage::CdrQuery {
                tenant_id: Some(cluster.seed.tenant_id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        if !records.is_empty() || tokio::time::Instant::now() >= deadline {
            break records;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    // Time for a second record, were one coming.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let again = fcp_core::storage::CdrStore::query_cdrs(
        cluster.seed.storage(),
        fcp_core::storage::CdrQuery {
            tenant_id: Some(cluster.seed.tenant_id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(again.len(), 1, "{again:?}");
    let b = cluster.b.service.stop().await;
    assert!(
        b.contains("A call relayed down a flow this node holds"),
        "{b}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_app_asleep_on_another_node_is_woken_and_answers_there() {
    let Some(cluster) = push_cluster("e2e-push-relay-wake").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
        return;
    };
    let alice = cluster.seed.user("alice", "Alice-pw-2026!", "2001").await;
    let bob = cluster.seed.user("bob", "Bob-pw-2026!", "2002").await;
    let app = app_on(&cluster, &cluster.b, &bob, "a1b2c3d4e5f6a7b8c9d0").await;
    app.auto_answer(true);
    app.sleep().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Node B says the flow is gone (430); node A pushes, and the app comes
    // back to node B, where node A reaches it again.
    let mut pushes = cluster.apns.subscribe();
    let woken = {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            let push = StandInApns::next_push(&mut pushes, 15).await?;
            assert_eq!(app.register().await.ok()?, 200);
            Some(push)
        })
    };
    let caller = alice_on(&cluster.a, &alice).await;
    let call = caller.call(&bob.extension).await.unwrap();
    if let Err(e) = call.answered(RING).await {
        let a = cluster.a.service.kill().await;
        let b = cluster.b.service.kill().await;
        panic!("the woken app should answer: {e}\n--- A\n{a}\n--- B\n{b}");
    }
    let push = woken.await.unwrap().expect("a push woke the app");
    assert_eq!(push.kind(), "call");
    assert_eq!(call.hangup().await.unwrap(), 200);
    // Pushed on node B's word, and reached through it once back.
    let a = cluster.a.service.stop().await;
    let b = cluster.b.service.stop().await;
    assert!(a.contains("is gone: waking it"), "{a}");
    assert!(
        b.contains("Refusing a relay down a flow this node does not hold"),
        "{b}"
    );
    assert!(
        b.contains("A call relayed down a flow this node holds"),
        "{b}"
    );
}
