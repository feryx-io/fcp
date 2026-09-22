//! Smoke test for the `fcp-presence` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-presence");

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
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-presence-server smoke test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-presence"
shutdown_timeout_secs = 5

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:presence@127.0.0.1:{sip}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();

    db.drop_db().await;
}

/// Leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): a draining
/// presence server says `503` on `/ready` so nothing sends it watchers,
/// says so on `/health` — which keeps answering `200`, so a liveness probe
/// does not kill it mid-drain — and serves again when the drain is called
/// off.
#[tokio::test]
async fn a_draining_presence_server_says_so_and_serves_again() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-presence-server drain test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "drain-presence"
shutdown_timeout_secs = 5
drain_wait_secs = 30

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:presence@127.0.0.1:{sip}"

[admission]
drain_register_expiry_secs = 60

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .while_up(move || async move {
            let ready = smoke::loopback(http, "/ready");
            let drain = smoke::loopback(http, "/drain");
            let health = smoke::loopback(http, "/health");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 200, "serving: {body}");

            let (code, body) = smoke::http_post(&drain, "{}").await.expect("drain starts");
            assert_eq!(code, 200, "{body}");
            assert!(body.contains("draining"), "{body}");
            assert!(body.contains("\"wait_secs\":30"), "{body}");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 503, "draining: {body}");
            let (code, body) = smoke::http_get(&health).await.expect("health answers");
            assert_eq!(code, 200, "health stays 200 through a drain");
            assert!(body.contains("\"draining\":true"), "{body}");

            let (code, body) = smoke::http_request("DELETE", &drain, &[], None)
                .await
                .expect("drain is called off");
            assert_eq!(code, 200, "{body}");
            assert!(body.contains("serving"), "{body}");
            let (code, _) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 200, "serving again");
        })
        .run()
        .await;
    report.assert_clean();

    db.drop_db().await;
}

/// The API token is required and checked (`docs/EDGE_SECURITY.md` §2.6).
///
/// This server compared the token with `==`, which stops at the first byte
/// that differs — how long the answer takes measures how much of the token
/// the caller guessed. It now uses `fcp_core::secret::constant_time_eq`,
/// whose own tests cover the comparison; what this covers is that the
/// token is still *demanded* and that a wrong one is still refused.
#[tokio::test]
async fn a_wrong_api_token_is_refused() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-presence-server token test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "token-presence"
shutdown_timeout_secs = 5

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:presence@127.0.0.1:{sip}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"
api_token = "s3cret-token-value"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
    );

    let service =
        fcp_testkit::Service::start("presence", BIN, &config, &smoke::loopback(http, "/health"))
            .await;

    let protected = smoke::loopback(http, "/v1/metrics");
    let ask = |headers: Vec<(&'static str, &'static str)>| {
        let url = protected.clone();
        async move {
            smoke::http_request("GET", &url, &headers, None)
                .await
                .expect("request")
                .0
        }
    };

    assert_eq!(ask(vec![]).await, 401, "no token is refused");
    assert_eq!(
        ask(vec![("Authorization", "Bearer wrong-token-value")]).await,
        401,
        "a wrong token of the same length is refused"
    );
    assert_eq!(
        ask(vec![("Authorization", "Bearer s3cret-token-valu")]).await,
        401,
        "a token that is right but for its last byte is refused"
    );
    assert_eq!(
        ask(vec![("Authorization", "Bearer s3cret-token-value")]).await,
        200,
        "the right token is let through"
    );

    service.stop().await;
    db.drop_db().await;
}
