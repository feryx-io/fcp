//! Smoke test for the `fcp-tts` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-tts");

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
node_id = "smoke-tts"

[http]
bind_address = "127.0.0.1:{http}"

[auth]
tokens = ["smoke-token"]

[tts]
default_provider = "stub"
cache_dir = "{dir}/tts-cache"

# Provider construction is lazy, so a placeholder key is enough to start.
[[tts.providers]]
name = "stub"
type = "openai"
api_key = "sk-smoke-test"

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

/// Every configuration this binary ships parses with no unknown key and
/// prints with its secrets masked under `--check-config`
/// (`docs/SHIP_AND_OPERATE.md` phase 1); the ones a node must accept
/// outright are accepted.
#[test]
fn shipped_configurations_check() {
    smoke::assert_configs_check(BIN, &[], &["fcp-tts-local.toml"], &[]);
}

/// A secret given in the environment wins over the file and is never
/// printed (`docs/SHIP_AND_OPERATE.md` phase 2).
#[test]
fn a_secret_from_the_environment_wins_over_the_file() {
    smoke::assert_env_override(
        BIN,
        r#"[http]
bind_address = "127.0.0.1:8093"
"#,
        ("FCP_TTS_AUTH_TOKENS", "tts-env-token-7f3a"),
        r#""***""#,
    );
}
