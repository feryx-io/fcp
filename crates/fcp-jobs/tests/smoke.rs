//! Smoke test for the `fcp-jobs` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-jobs");

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
    let Ok(redis) = std::env::var("FCP_TEST_REDIS_URL") else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping fcp-jobs smoke test (needs Redis)");
        return;
    };
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-jobs"

[redis]
url = "{redis}"

[workers]
count = 1
shutdown_timeout = 5

[health]
enabled = true
bind_address = "127.0.0.1:{http}"

[logging]
level = "info"
format = "compact"
"#,
        redis = redis,
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();
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
        &[
            "fcp-jobs-local.toml",
            "deploy/docker-compose/configs/fcp-jobs.toml",
            "crates/fcp-jobs/fcp-jobs.toml.example",
        ],
        &[],
    );
}
