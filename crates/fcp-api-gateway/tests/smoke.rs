//! Smoke test for the `fcp-api-gateway` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-api-gateway");

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
node_id = "smoke-gateway"

[gateway]
bind_address = "127.0.0.1:{http}"

[rate_limit]
enabled = false

[cache]
enabled = false

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
