//! A phone running older firmware than its target says so
//! (`docs/PROVISIONING.md` §11): a Yealink whose tenant targets a newer
//! image registers, and the call manager publishes `device.firmware_behind`
//! on the event stream — once for the version it runs, not again on its
//! next REGISTER. Skipped without `FCP_TEST_DATABASE_URL` and
//! `FCP_TEST_REDIS_URL`.

use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, Seed, Service, TestDatabase};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "firmware-behind-static-token-0123456789";
const REALM: &str = "fcp.local";
const MAC: &str = "80:5E:C0:00:00:07";
/// It registers saying what it runs, its MAC included.
const USER_AGENT: &str = "Yealink SIP-T54W 96.86.0.70 80:5e:c0:00:00:07";

fn multipart(parts: &[(&str, Option<&str>, &[u8])]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "fcp-firmware-behind-boundary";
    let mut body = Vec::new();
    for (name, file, bytes) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        let disposition = match file {
            Some(file) => format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            ),
            None => format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n"),
        };
        body.extend_from_slice(disposition.as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

/// The `device.firmware_behind` events on the stream, as JSON.
async fn behind_events(redis: &str) -> Vec<Value> {
    let client = redis::Client::open(redis).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let entries: Vec<(String, Vec<(String, String)>)> = redis::cmd("XRANGE")
        .arg("fcp:event_stream")
        .arg("-")
        .arg("+")
        .query_async(&mut conn)
        .await
        .unwrap_or_default();
    entries
        .into_iter()
        .flat_map(|(_, fields)| fields.into_iter().map(|(_, v)| v))
        // On the stream by its serialized type; its webhook name is
        // `device.firmware_behind`.
        .filter(|v| v.contains("\"type\":\"device_firmware_behind\""))
        .filter_map(|v| serde_json::from_str(&v).ok())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_behind_its_target_is_announced_once() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let Some(redis) = private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    Seed::new(db.url(), REALM).await;

    let (sip, node_http) = (free_port(), free_port());
    let node_config = format!(
        r#"
[service]
node_id = "firmware-behind-node"

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

[events]
use_redis = true

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{node_http}"

[media]
prompts_dir = "{dir}/node-prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let node = Service::start(
        "call-manager",
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &node_config,
        &loopback(node_http, "/health"),
    )
    .await;

    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "firmware-behind-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

[api.firmware]
directory = "{dir}/firmware"

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
    let api = |method: reqwest::Method, path: &str, body: Value| {
        let request = client
            .request(method, loopback(http, path))
            .bearer_auth(TOKEN)
            .json(&body);
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            (
                status,
                response.json::<Value>().await.unwrap_or(Value::Null),
            )
        }
    };

    // Alice and her Yealink; the tenant installs 96.86.0.80 on Yealinks.
    let (status, alice) = api(
        reqwest::Method::POST,
        "/api/v1/users",
        json!({"username": "alice", "password": "Str0ng-Passw0rd!",
               "sip_provisioning_password": "Alice-Sip-Secret-1"}),
    )
    .await;
    assert_eq!(status, 201, "{alice}");
    let (status, device) = api(
        reqwest::Method::POST,
        "/api/v1/devices",
        json!({"name": "Alice's desk", "device_type": "desk_phone",
               "mac_address": MAC, "owner_user_id": alice["id"]}),
    )
    .await;
    assert_eq!(status, 201, "{device}");
    let device_id = device["id"].as_str().unwrap().to_string();
    let (content_type, body) = multipart(&[
        ("vendor", None, b"yealink"),
        ("version", None, b"96.86.0.80"),
        ("models", None, b"T5*"),
        ("file", Some("T54W-96.86.0.80.rom"), b"not really firmware"),
    ]);
    let made = client
        .post(loopback(http, "/api/v1/firmware"))
        .bearer_auth(TOKEN)
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(made.status().as_u16(), 201);
    let image: Value = made.json().await.unwrap();
    let (status, target) = api(
        reqwest::Method::POST,
        "/api/v1/firmware/targets",
        json!({"vendor": "yealink", "image_id": image["id"], "stage": "all"}),
    )
    .await;
    assert_eq!(status, 201, "{target}");

    // It registers on 96.86.0.70: behind, and said so.
    let phone = Phone::with_user_agent(
        "alice",
        "alice",
        "Alice-Sip-Secret-1",
        REALM,
        SocketAddr::from(([127, 0, 0, 1], sip)),
        USER_AGENT,
    )
    .await
    .expect("the phone binds");
    assert_eq!(phone.register().await.unwrap(), 200);
    let mut events = Vec::new();
    for _ in 0..50 {
        events = behind_events(&redis).await;
        if !events.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(events.len(), 1, "{events:?}");
    let event = events[0].to_string();
    for expected in [
        device_id.as_str(),
        "96.86.0.70",
        "96.86.0.80",
        "Alice's desk",
    ] {
        assert!(event.contains(expected), "{expected} in {event}");
    }

    // Its next REGISTER, on the same version, is not said again.
    assert_eq!(phone.register().await.unwrap(), 200);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(behind_events(&redis).await.len(), 1);

    admin.stop().await;
    node.stop().await;
    db.drop_db().await;
}
