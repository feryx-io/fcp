//! Smoke test for the `fcp-registrar` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-registrar");

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
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-registrar smoke test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-registrar"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"

[registrar]
auth_enabled = true
realm = "smoke.example.com"

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
/// registrar says `503` on `/ready` so nothing sends it devices, says so
/// on `/status`, and serves again when the drain is called off — all while
/// `/health` keeps answering `200`, so a liveness probe does not kill it
/// mid-drain.
#[tokio::test]
async fn a_draining_registrar_says_so_and_serves_again() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-registrar drain test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "drain-registrar"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"

[registrar]
auth_enabled = true
realm = "smoke.example.com"

[admission]
max_concurrent_registrations = 100
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
            let status = smoke::loopback(http, "/status");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 200, "serving: {body}");

            let (code, body) = smoke::http_post(&drain, "{}").await.expect("drain starts");
            assert_eq!(code, 200, "{body}");
            assert!(body.contains("draining"), "{body}");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 503, "draining: {body}");
            let (code, body) = smoke::http_get(&status).await.expect("status answers");
            assert_eq!(code, 200);
            assert!(body.contains("\"draining\":true"), "{body}");
            // A liveness probe must still see a live process.
            let (code, _) = smoke::http_get(&smoke::loopback(http, "/health"))
                .await
                .expect("health answers");
            assert_eq!(code, 200, "health stays 200 through a drain");

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
