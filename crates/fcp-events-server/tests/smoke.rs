//! Smoke test for the `fcp-events-server` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::cluster;
use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::Service;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-events-server");

#[test]
fn version_and_help() {
    smoke::assert_version_and_help(BIN, env!("CARGO_PKG_VERSION"));
}

#[test]
fn rejects_malformed_config() {
    smoke::assert_rejects_malformed_config(BIN);
}

#[tokio::test]
async fn starts_serves_health_and_stops() {
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-events"

[auth]
token = "smoke-token"

[websocket]
bind_address = "127.0.0.1:{http}"

[event_bus]
bus_type = "in-memory"

[logging]
level = "info"
format = "compact"
"#,
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();
}

const TOKEN: &str = "smoke-token";

fn events_server_config(
    node_id: &str,
    region: &str,
    http: u16,
    redis: &str,
    peer_region: &str,
    peer_redis: &str,
) -> String {
    format!(
        r#"[service]
node_id = "{node_id}"

[auth]
token = "{TOKEN}"

[websocket]
bind_address = "127.0.0.1:{http}"

[cluster]
region = "{region}"

[event_bus]
bus_type = "redis"
redis_url = "{redis}"
consumer_name = "{node_id}"

[[event_bus.peer_regions]]
region = "{peer_region}"
redis_url = "{peer_redis}"

[logging]
level = "info"
format = "compact"
"#
    )
}

async fn get_json(url: &str) -> serde_json::Value {
    let bearer = format!("Bearer {TOKEN}");
    let (code, body) = smoke::http_request("GET", url, &[("Authorization", &bearer)], None)
        .await
        .expect("answers");
    assert_eq!(code, 200, "{url}: {body}");
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{url}: {e}: {body}"))
}

/// Polls `url` until `accept` says yes; the last body either way.
async fn get_json_when(
    url: &str,
    within: Duration,
    accept: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let v = get_json(url).await;
        if accept(&v) || tokio::time::Instant::now() >= deadline {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// An event posted to `/ingest` reaches every events server and every
/// region (`docs/DEPLOYMENT_GAPS.md` §1.6): with Redis as the bus the
/// ingesting server writes it to its region's stream under the posting
/// node's id, sees it again through its own consumer — once — and its
/// bridge forwards it to the other region, whose events server delivers
/// it too. Two regions, a Redis database each, one events server in each.
#[tokio::test]
async fn an_ingested_event_reaches_the_ingesting_server_and_the_other_region() {
    let (Some(redis_us), Some(redis_eu)) =
        (cluster::private_redis_url(), cluster::private_redis_url())
    else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping fcp-events-server ingest test");
        return;
    };
    let http_us = free_port();
    let http_eu = free_port();
    let us = Service::start(
        "events-us",
        BIN,
        &events_server_config("ev-us", "us-east", http_us, &redis_us, "eu-west", &redis_eu),
        &smoke::loopback(http_us, "/health"),
    )
    .await;
    let eu = Service::start(
        "events-eu",
        BIN,
        &events_server_config("ev-eu", "eu-west", http_eu, &redis_eu, "us-east", &redis_us),
        &smoke::loopback(http_eu, "/health"),
    )
    .await;

    // Ids serialise as bare UUIDs.
    let call_id = uuid::Uuid::new_v4().to_string();
    let event = serde_json::json!({
        "event_type": "call",
        "type": "initiated",
        "call_id": call_id,
        "tenant_id": "00000000-0000-0000-0000-000000000001",
        "from": "sip:alice@example.com",
        "to": "sip:bob@example.com",
        "direction": "inbound",
        "timestamp": 1_700_000_000u64,
        "node_id": "conf-a"
    });
    let bearer = format!("Bearer {TOKEN}");
    let (code, body) = smoke::http_request(
        "POST",
        &smoke::loopback(http_us, "/ingest"),
        &[
            ("Authorization", &bearer),
            ("Content-Type", "application/json"),
            ("X-Node-ID", "conf-a"),
        ],
        Some(&event.to_string()),
    )
    .await
    .expect("ingest answers");
    assert_eq!(code, 202, "{body}");

    // The state renders ids with their prefix.
    let tracked_id = format!("call-{call_id}");
    let has_call = |v: &serde_json::Value| {
        v["calls"]
            .as_array()
            .is_some_and(|calls| calls.iter().any(|c| c["call_id"] == tracked_id))
    };
    // The ingesting server: through Redis and its own consumer.
    let calls_us = get_json_when(
        &smoke::loopback(http_us, "/calls"),
        Duration::from_secs(10),
        has_call,
    )
    .await;
    assert!(
        has_call(&calls_us),
        "the ingesting server tracks the call: {calls_us}"
    );
    assert_eq!(calls_us["count"], 1, "delivered once: {calls_us}");
    // The other region: through the bridge.
    let calls_eu = get_json_when(
        &smoke::loopback(http_eu, "/calls"),
        Duration::from_secs(15),
        has_call,
    )
    .await;
    assert!(
        has_call(&calls_eu),
        "the other region's server tracks the call: {calls_eu}"
    );
    assert_eq!(calls_eu["count"], 1, "{calls_eu}");

    // The bridge counted it out (the phase-1 counters).
    let status_us = get_json(&smoke::loopback(http_us, "/status")).await;
    let bridge = status_us["region"]["bridge"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let to_eu = bridge
        .iter()
        .find(|p| p["region"] == "eu-west")
        .unwrap_or_else(|| panic!("a bridge to eu-west: {status_us}"));
    assert!(to_eu["forwarded"].as_u64().unwrap_or(0) >= 1, "{status_us}");

    us.stop().await;
    eu.stop().await;
}

/// Every configuration this binary ships parses with no unknown key and
/// prints with its secrets masked under `--check-config`
/// (`docs/SHIP_AND_OPERATE.md` phase 1); the ones a node must accept
/// outright are accepted.
#[test]
fn shipped_configurations_check() {
    smoke::assert_configs_check(
        BIN,
        &[],
        &["fcp-events-server-local.toml"],
        &[
            "deploy/docker-compose/configs/fcp-events-server.toml",
            "crates/fcp-events-server/fcp-events-server.toml.example",
        ],
    );
}
