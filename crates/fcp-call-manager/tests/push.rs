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
