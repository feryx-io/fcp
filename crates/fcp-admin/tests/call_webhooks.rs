//! A tenant's webhook hears its calls (`docs/WEBHOOKS.md`): call events
//! carry the call's tenant, so the dispatcher finds the tenant's endpoints.
//! Before, `call.ringing`, `call.answered`, `call.ended` and the rest named
//! no tenant and were never delivered. An admin, a call manager and an
//! events server running the dispatcher; a webhook on `call.*`; one
//! answered call. Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_testkit::sinks::HttpSink;
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, Seed, Service, TestDatabase};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "call-webhooks-static-token-0123456789";
const EVENTS_TOKEN: &str = "call-webhooks-events-token-0123456789";
const REALM: &str = "fcp.local";
const SOON: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_webhook_hears_its_calls() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let seed = Seed::new(db.url(), REALM).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;

    // The events server, delivering webhooks.
    let events_http = free_port();
    let events_config = format!(
        r#"[service]
node_id = "call-webhooks-events"

[auth]
token = "{EVENTS_TOKEN}"

[websocket]
bind_address = "127.0.0.1:{events_http}"

[webhook]
enabled = true
database_url = "{db}"
allow_private_targets = true

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
    );
    let events = Service::start(
        "events-server",
        fcp_testkit::bins::path("fcp-events-server", "fcp-events-server"),
        &events_config,
        &loopback(events_http, "/health"),
    )
    .await;

    // The call manager, sending its events there.
    let (sip, node_http) = (free_port(), free_port());
    let node_config = format!(
        r#"
[service]
node_id = "call-webhooks-node"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

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

    // The admin, where the tenant makes its webhook.
    let http = free_port();
    let admin_config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "call-webhooks-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

[webhooks]
allow_private_targets = true

[storage]
database_url = "{db}"

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
    let receiver = HttpSink::start().await;
    let made = reqwest::Client::new()
        .post(loopback(http, "/api/v1/webhooks"))
        .bearer_auth(TOKEN)
        .json(&json!({"url": format!("{}/hook", receiver.url()), "events": ["call.*"]}))
        .send()
        .await
        .unwrap();
    let status = made.status().as_u16();
    let made: Value = made.json().await.unwrap();
    assert_eq!(status, 201, "{made}");

    // Alice calls Bob, Bob answers, Alice hangs up.
    let server = SocketAddr::from(([127, 0, 0, 1], sip));
    let alice_phone = Phone::new(&alice.username, &alice.password, REALM, server)
        .await
        .unwrap();
    let bob_phone = Phone::new(&bob.username, &bob.password, REALM, server)
        .await
        .unwrap();
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let bob_call = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob's phone rings")
        .answer()
        .await
        .unwrap();
    alice_call.answered(SOON).await.unwrap();
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);

    // The tenant's webhook heard the call, answered and ended, as its own.
    let mut types = Vec::new();
    for _ in 0..100 {
        types = receiver
            .received()
            .iter()
            .filter_map(|d| d["event_type"].as_str().map(String::from))
            .collect::<Vec<_>>();
        if types.iter().any(|t| t == "call.answered") && types.iter().any(|t| t == "call.ended") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !(types.iter().any(|t| t == "call.answered") && types.iter().any(|t| t == "call.ended")) {
        let logs = events.stop().await;
        panic!("delivered: {types:?}\nthe events server's log:\n{logs}");
    }
    for delivery in receiver.received() {
        assert_eq!(
            delivery["tenant_id"], "tenant-00000000-0000-0000-0000-000000000000",
            "{delivery}"
        );
    }

    admin.stop().await;
    node.stop().await;
    events.stop().await;
    db.drop_db().await;
}
