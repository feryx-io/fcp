//! Smoke test for the `fcp-media` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-media");

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
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-media"
shutdown_timeout_secs = 5

[http]
bind_address = "127.0.0.1:{http}"

[recording]
base_dir = "{dir}/recordings"
root_jail = "{dir}"
prompts_dir = "{dir}/prompts"

[auth]
tokens = ["smoke-token"]

[logging]
level = "info"
format = "compact"
"#,
        dir = smoke::toml_path(dir.path()),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();
}
