//! A colleague's busy lamp reaches a user's app (`/me/events`,
//! `docs/SOFTPHONE.md`): the call manager forwards a user's status and
//! dialog changes to the events server, which the admin's stream reads.
//! Before, the forwarder dropped every presence event, so no app ever saw a
//! lamp on a node sending its events over HTTP (the compose stack's way).
//! An admin, a call manager with presence on, an events server; Bob watching
//! his stream while Alice calls Carol. Skipped without
//! `FCP_TEST_DATABASE_URL`.

use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, Seed, Service, TestDatabase};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "presence-events-static-token-0123456789";
const EVENTS_TOKEN: &str = "presence-events-events-token-0123456789";
const REALM: &str = "fcp.local";
const SOON: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_colleagues_lamp_reaches_a_users_stream() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let seed = Seed::new(db.url(), REALM).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let carol = seed.user("carol", "carol-pw", "1003").await;

    let events_http = free_port();
    let events_config = format!(
        r#"[service]
node_id = "presence-events-events"

[auth]
token = "{EVENTS_TOKEN}"

[websocket]
bind_address = "127.0.0.1:{events_http}"

[logging]
level = "info"
format = "compact"
"#
    );
    let events = Service::start(
        "events-server",
        fcp_testkit::bins::path("fcp-events-server", "fcp-events-server"),
        &events_config,
        &loopback(events_http, "/health"),
    )
    .await;

    let (sip, node_http) = (free_port(), free_port());
    let node_config = format!(
        r#"
[service]
node_id = "presence-events-node"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[presence]
enabled = true

[events]
server_url = "http://127.0.0.1:{events_http}/ingest"
auth_token = "{EVENTS_TOKEN}"

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
    let admin_config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "presence-events-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

[storage]
database_url = "{db}"

[events]
server_url = "http://127.0.0.1:{events_http}/ingest"
auth_token = "{EVENTS_TOKEN}"

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
        &loopback(http, "/api/v1/health"),
    )
    .await;

    // Bob, a colleague signed in to the web, watching his stream.
    let client = reqwest::Client::new();
    let made = client
        .post(loopback(http, "/api/v1/users"))
        .bearer_auth(TOKEN)
        .json(&json!({"username": "bob", "password": "Str0ng-Passw0rd!"}))
        .send()
        .await
        .unwrap();
    assert!(made.status().is_success(), "{:?}", made.text().await);
    let signed_in: Value = client
        .post(loopback(http, "/api/v1/auth/login"))
        .json(&json!({"username": "bob", "password": "Str0ng-Passw0rd!"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bob_token = signed_in["token"].as_str().expect("Bob signs in");
    let (mut stream, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{http}/api/v1/me/events"))
            .await
            .expect("Bob's stream");
    stream
        .send(Message::Text(
            json!({"type": "auth", "token": bob_token}).to_string(),
        ))
        .await
        .unwrap();

    // Alice calls Carol; Carol answers.
    let server = SocketAddr::from(([127, 0, 0, 1], sip));
    let alice_phone = Phone::new(&alice.username, &alice.password, REALM, server)
        .await
        .unwrap();
    let carol_phone = Phone::new(&carol.username, &carol.password, REALM, server)
        .await
        .unwrap();
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(carol_phone.register().await.unwrap(), 200);
    let alice_call = alice_phone.call(&carol.extension).await.unwrap();
    let carol_call = carol_phone
        .incoming(SOON)
        .await
        .expect("Carol's phone rings")
        .answer()
        .await
        .unwrap();
    alice_call.answered(SOON).await.unwrap();

    // Bob's stream lights Alice's lamp.
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + SOON;
    let lamp = loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let message: Value = serde_json::from_str(&text).unwrap_or_default();
                let is_lamp = message["event_type"] == "presence"
                    && message["type"] == "dialog_changed"
                    && message["aor"].as_str().is_some_and(|a| a.contains("alice"))
                    && message["dialog_state"] == "confirmed";
                seen.push(message.clone());
                if is_lamp {
                    break message;
                }
            }
            Ok(Some(Ok(_))) => {}
            other => {
                let logs = node.stop().await;
                panic!("no lamp for Alice ({other:?}); Bob saw {seen:#?}\nthe node's log:\n{logs}");
            }
        }
    };
    assert!(
        lamp["tenant_id"]
            .as_str()
            .is_some_and(|t| t.ends_with("00000000-0000-0000-0000-000000000000")),
        "the default tenant's: {lamp}"
    );

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(carol_call.ended(SOON).await);
    admin.stop().await;
    node.stop().await;
    events.stop().await;
    db.drop_db().await;
}
