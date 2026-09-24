//! Smoke test for the `fcp-admin` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");

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
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-admin smoke test");
        return;
    };
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "smoke-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = false

[storage]
database_url = "{db}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/api/v1/health"))
        .run()
        .await;
    report.assert_clean();

    db.drop_db().await;
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
        &["fcp-admin-local.toml"],
        &[
            "deploy/docker-compose/configs/fcp-admin.toml",
            "crates/fcp-admin/fcp-admin.toml.example",
        ],
    );
}

/// A secret given in the environment wins over the file and is never
/// printed (`docs/SHIP_AND_OPERATE.md` phase 2).
#[test]
fn a_secret_from_the_environment_wins_over_the_file() {
    smoke::assert_env_override(
        BIN,
        r#"[api]
bind_address = "127.0.0.1:8080"
auth_enabled = false
"#,
        (
            "FCP_ADMIN_DATABASE_URL",
            "postgresql://fcp:env-db-pass-7f3a@db.internal:5432/fcp",
        ),
        r#"database_url = "postgresql://fcp:***@db.internal:5432/fcp""#,
    );
}
