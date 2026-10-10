//! An administrator's test push (`docs/PUSH.md` phase P2): the admin asks
//! the call managers in turn, a node without `[push]` passes it on (503),
//! and the one with the publisher's keys sends it through the stand-in
//! APNs. Skipped without `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

use fcp_core::storage::{CredentialStore, DeviceStore, Registration, RegistrationStore};
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::push::{StandInApns, APNS_TEAM, APNS_TOPIC};
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Seed, Service, TestDatabase};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "push-test-static-token-0123456789";
const NODE_TOKEN: &str = "push-test-node-token-0123456789";
const REALM: &str = "fcp.local";

async fn call_manager(
    db: &TestDatabase,
    dir: &Path,
    redis: &str,
    node_id: &str,
    push: &str,
) -> (Service, u16) {
    let (sip, http) = (free_port(), free_port());
    let config = format!(
        r#"
[service]
node_id = "{node_id}"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[cluster]
enabled = true

[cdr]
spool_dir = "{dir}/{node_id}-cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"
api_token = "{NODE_TOKEN}"

[media]
prompts_dir = "{dir}/{node_id}-prompts"
tts_cache_dir = "{dir}/{node_id}-tts"
recording_dir = "{dir}/{node_id}-recordings"

{push}

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir),
    );
    let service = Service::start(
        node_id,
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &config,
        &loopback(http, "/health"),
    )
    .await;
    (service, http)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_test_push_is_sent_by_a_node_with_the_keys() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let Some(redis) = private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let apns = StandInApns::start().await;
    let key = dir.path().join("apns.p8");
    std::fs::write(&key, apns.config().key.unwrap()).unwrap();
    let push = format!(
        r#"[[push.apns]]
team_id = "{APNS_TEAM}"
key_id = "{key_id}"
key_file = "{key}"
topics = ["{APNS_TOPIC}"]
endpoint = "{url}"
"#,
        key_id = apns.config().key_id,
        key = smoke::toml_path(&key),
        url = apns.config().endpoint.unwrap(),
    );
    let (without, without_http) = call_manager(&db, dir.path(), &redis, "push-a", "").await;
    let (with, _) = call_manager(&db, dir.path(), &redis, "push-b", &push).await;

    // Alice's iPhone, with its push binding.
    let seed = Seed::new(db.url(), REALM).await;
    seed.user("alice", "alice-pw", "1001").await;
    let storage = seed.storage();
    let alice = storage
        .get_user_in_tenant("alice", &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let mut iphone = fcp_core::Device::new(
        "iPhone".into(),
        fcp_core::device::DeviceType::Softphone,
        seed.tenant_id,
    );
    iphone.owner_user_id = Some(alice.id);
    iphone.last_ip_address = Some("10.0.0.7:52311".into());
    storage.put_device(iphone.clone()).await.unwrap();
    let now = SystemTime::now();
    storage
        .put_registration(Registration {
            tenant_id: seed.tenant_id,
            aor: format!("sip:alice@{REALM}"),
            contact: "<sip:app-1@10.0.0.7:52311;transport=tls>".into(),
            expires_at: now + Duration::from_secs(604_800),
            call_id: "app@x".into(),
            cseq: 1,
            user_agent: None,
            q: None,
            registered_at: now,
            path: None,
            flow_token: None,
            source_address: None,
            instance_id: None,
            reg_id: None,
            push: Some(StandInApns::address("a1b2c3d4e5f6a7b8c9d0")),
            flow_node: None,
        })
        .await
        .unwrap();

    // A node without [push] says so.
    let refused = reqwest::Client::new()
        .post(loopback(
            without_http,
            &format!("/devices/{}/push-test", iphone.id),
        ))
        .bearer_auth(NODE_TOKEN)
        .json(&json!({ "tenant_id": seed.tenant_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status().as_u16(), 503);

    let admin_http = free_port();
    let admin_config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "push-admin"

[api]
bind_address = "127.0.0.1:{admin_http}"
auth_enabled = true
auth_token = "{TOKEN}"

[storage]
database_url = "{db}"

[cluster]
redis_url = "{redis}"

[call_manager]
auth_token = "{NODE_TOKEN}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let admin = Service::start(
        "admin",
        BIN,
        &admin_config,
        &loopback(admin_http, "/api/v1/health"),
    )
    .await;

    // Once both nodes heartbeat, the admin finds the one that can push.
    let mut answer = Value::Null;
    for _ in 0..40 {
        let response = reqwest::Client::new()
            .post(loopback(
                admin_http,
                &format!("/api/v1/devices/{}/push-test", iphone.id),
            ))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        let ok = response.status().is_success();
        answer = response.json().await.unwrap_or(Value::Null);
        if ok && answer["results"][0]["outcome"] == "sent" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(answer["results"][0]["provider"], "apns", "{answer}");
    assert_eq!(answer["results"][0]["outcome"], "sent", "{answer}");
    let pushed = apns.pushes();
    assert!(!pushed.is_empty());
    assert!(pushed.iter().all(|p| p.kind() == "test"));

    admin.stop().await;
    without.stop().await;
    with.stop().await;
    db.drop_db().await;
}
