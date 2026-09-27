//! Smoke test for the `fcp-link` binary: its version and help, a
//! malformed configuration refused, the site stack's shipped
//! configuration accepted by `--check-config` with the site's `.env`,
//! and the process starting on a free loopback port, answering its health
//! endpoint with nothing to enroll to, and exiting cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-link");

#[test]
fn version_and_help() {
    smoke::assert_version_and_help(BIN, env!("CARGO_PKG_VERSION"));
}

#[test]
fn rejects_malformed_config() {
    smoke::assert_rejects_malformed_config(BIN);
}

/// The site stack's `fcp-link.toml` is accepted with its `.env` in the
/// environment — the cloud's addresses and the enrollment token are
/// `FCP_LINK_*` overrides — and the token is masked in what
/// `--check-config` prints (`docs/SITE_SETUP.md`).
#[test]
fn the_shipped_configuration_checks() {
    smoke::assert_configs_check(
        BIN,
        &[],
        &["deploy/docker-compose/site/configs/fcp-link.toml"],
        &[],
    );
}

/// A cloud address the link must name: without one `--check-config`
/// refuses with the key and the override that would set it.
#[test]
fn refuses_a_configuration_without_a_cloud() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fcp-link.toml");
    std::fs::write(&path, "[service]\nnode_id = \"x\"\n").unwrap();
    let out = smoke::run_blocking_with_env(
        BIN,
        &["--check-config", "--config", path.to_str().unwrap()],
        &[],
        std::time::Duration::from_secs(30),
    );
    assert_eq!(
        out.status.code(),
        Some(fcp_core::config::CHECK_CONFIG_REFUSED),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("link.cloud_url") && out.stderr.contains("FCP_LINK_CLOUD_URL"),
        "{}",
        out.stderr
    );
}

/// The link starts with nowhere to enroll: its health answers, saying
/// so, while it keeps trying; it stops cleanly.
#[tokio::test]
async fn starts_serves_health_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let health = free_port();
    let nowhere = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-link"

[link]
cloud_url = "https://127.0.0.1:{nowhere}"
link_url = "https://127.0.0.1:{nowhere}"
enrollment_token = "not-a-real-token"
state_dir = "{state}"
heartbeat_interval_secs = 1

[health]
bind_address = "127.0.0.1:{health}"

[logging]
level = "info"
format = "compact"
"#,
        state = dir.path().join("state").to_string_lossy(),
    );
    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(health, "/health"))
        .run()
        .await;
    report.assert_clean();
}
