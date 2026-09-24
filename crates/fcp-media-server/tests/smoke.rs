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
/// A `${NAME}` in the file is resolved from the environment when the
/// file is loaded and masked like any secret, and a name the environment
/// lacks refuses the file (`docs/PRODUCTION_STACK.md` phase 2).
#[test]
fn a_reference_in_the_file_is_resolved_from_the_environment() {
    const CONFIG: &str = r#"[http]
bind_address = "127.0.0.1:8083"

[auth]
tokens = ["${FCP_SMOKE_REF_MEDIA}", "media-other-token"]
"#;
    smoke::assert_env_override(
        BIN,
        CONFIG,
        ("FCP_SMOKE_REF_MEDIA", "media-ref-token-7f3a"),
        r#""***""#,
    );
    smoke::assert_reference_unset_refuses(BIN, CONFIG, "FCP_SMOKE_REF_MEDIA");
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
            "fcp-media-local.toml",
            "deploy/docker-compose/configs/fcp-media-server.toml",
            "crates/fcp-media-server/fcp-media-server.toml.example",
        ],
        &[],
    );
}

/// A secret given in the environment wins over the file and is never
/// printed (`docs/SHIP_AND_OPERATE.md` phase 2).
#[test]
fn a_secret_from_the_environment_wins_over_the_file() {
    smoke::assert_env_override(
        BIN,
        r#"[http]
bind_address = "127.0.0.1:8083"
"#,
        (
            "FCP_MEDIA_AUTH_TOKENS",
            "media-env-token-7f3a,media-env-token-8e4b",
        ),
        r#""***""#,
    );
}
