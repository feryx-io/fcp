//! Smoke test for the `fcp-webrtc-gw` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-webrtc-gw");

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
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-webrtc"
shutdown_timeout_secs = 5

[http]
bind_address = "127.0.0.1:{http}"

[sip]
udp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[auth]
tokens = ["smoke-token"]

[tls]
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
