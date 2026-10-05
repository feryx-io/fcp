//! An app install's SIP account works, and stops working when the device is
//! removed (`docs/SOFTPHONE.md` §2.1): the device an app's sign-in makes
//! registers at a call manager as its owner with credentials of its own,
//! rings for her, and once she removes it its registration is gone and its
//! credentials refused. Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, Seed, Service, TestDatabase};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "app-devices-static-token-0123456789";
const REALM: &str = "fcp.local";
const SOON: Duration = Duration::from_secs(5);

async fn call_manager(db: &TestDatabase, dir: &Path) -> (Service, u16) {
    let (sip, http) = (free_port(), free_port());
    let config = format!(
        r#"
[service]
node_id = "app-devices-node"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"
# No cache: this node hears no events, so it would keep a binding the admin
# removed until its TTL ran out (with [events] use_redis it is told).
cache_ttl_secs = 0

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[media]
prompts_dir = "{dir}/node-prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir),
    );
    let service = Service::start(
        "call-manager",
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &config,
        &loopback(http, "/health"),
    )
    .await;
    (service, sip)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_app_devices_sip_account_rings_until_the_device_is_removed() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let seed = Seed::new(db.url(), REALM).await;
    let bob = seed.user("bob", "bob-pw", "1002").await;
    let (node, sip) = call_manager(&db, dir.path()).await;
    let server = SocketAddr::from(([127, 0, 0, 1], sip));

    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "app-devices-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

[sip]
realm = "{REALM}"

[storage]
database_url = "{db}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let admin = Service::start("admin", BIN, &config, &loopback(http, "/api/v1/health")).await;
    let client = reqwest::Client::new();
    let post = |path: &str, token: &str, body: Value| {
        client
            .post(loopback(http, path))
            .bearer_auth(token)
            .json(&body)
            .send()
    };

    // Alice, with extension 1001, and her app signing in: a device with its
    // own SIP account.
    let made: Value = post(
        "/api/v1/users",
        TOKEN,
        json!({"username": "alice", "password": "Str0ng-Passw0rd!"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let extension = post(
        "/api/v1/extensions",
        TOKEN,
        json!({"number": "1001", "target": {"type": "user", "id": made["id"]}}),
    )
    .await
    .unwrap();
    assert_eq!(
        extension.status().as_u16(),
        201,
        "{:?}",
        extension.text().await
    );
    let signed_in: Value = post(
        "/api/v1/auth/login",
        "",
        json!({"username": "alice", "password": "Str0ng-Passw0rd!"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let web = signed_in["token"].as_str().expect("signed in").to_string();
    let app: Value = post(
        "/api/v1/auth/app/sessions",
        &web,
        json!({"client_name": "Alice's laptop", "install_id": "laptop-1"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let device = &app["device"];
    let (device_id, sip_username, sip_password) = (
        device["id"].as_str().unwrap().to_string(),
        device["sip_username"].as_str().unwrap().to_string(),
        device["sip_password"].as_str().unwrap().to_string(),
    );
    assert_eq!(device["realm"], REALM);
    let app_access = app["access_token"].as_str().unwrap().to_string();

    // The app registers as Alice with its own credentials, and rings for her.
    let softphone = Phone::with_user_agent(
        "alice",
        &sip_username,
        &sip_password,
        REALM,
        server,
        "Anvil/0.2.0",
    )
    .await
    .unwrap();
    assert_eq!(softphone.register().await.unwrap(), 200);
    let bob_phone = Phone::new(&bob.username, &bob.password, REALM, server)
        .await
        .unwrap();
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    let call = bob_phone.call("1001").await.unwrap();
    let Some(incoming) = softphone.incoming(SOON).await else {
        let node = node;
        let logs = node.stop().await;
        let tail: String = logs
            .lines()
            .filter(|l| l.contains("WARN") || l.contains("alice") || l.contains("Destination"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!("the app did not ring for Alice:\n{tail}");
    };
    let answered = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(answered.hangup().await.unwrap(), 200);
    assert!(call.ended(SOON).await);

    // The registration lands on the device.
    let mut registered = false;
    for _ in 0..50 {
        let mine: Value = client
            .get(loopback(http, "/api/v1/me/devices"))
            .bearer_auth(&app_access)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        registered = mine["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"] == device_id.as_str() && d["registration_state"] == "registered");
        if registered {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(registered, "the device shows its registration");

    // Alice removes it: the app's session ends, its registration goes, and
    // its credentials are refused.
    let removed = client
        .delete(loopback(http, &format!("/api/v1/me/devices/{device_id}")))
        .bearer_auth(&web)
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status().as_u16(), 204);
    let me = client
        .get(loopback(http, "/api/v1/auth/me"))
        .bearer_auth(&app_access)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status().as_u16(), 401, "the app's session ended");
    let call = bob_phone.call("1001").await.unwrap();
    assert!(
        softphone.incoming(Duration::from_secs(3)).await.is_none(),
        "the removed app does not ring"
    );
    let _ = call.ended(SOON).await;
    let again = softphone.register().await.unwrap();
    assert_eq!(again, 401, "its credentials are refused");

    admin.stop().await;
    let node = node;
    node.stop().await;
    db.drop_db().await;
}
