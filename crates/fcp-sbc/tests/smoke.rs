//! Smoke test for the `fcp-sbc` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-sbc");

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
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-sbc smoke test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[core]
external_domain = "smoke.example.com"

[core.storage]
enabled = true
database_url = "{db}"
"#,
        db = db.url(),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .arg("--bind")
        .arg(format!("127.0.0.1:{sip}"))
        .arg("--health-bind")
        .arg(format!("127.0.0.1:{http}"))
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();
    // The config must actually reach the engine: storage was enabled under [core].
    report.assert_logged("CDR storage initialized");

    db.drop_db().await;
}
