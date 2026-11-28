//! Smoke test for the `fcp-backup` binary: its version and help, a
//! malformed file refused, the shipped configuration accepted, a database
//! URL from the environment winning and never printed, and a start that
//! answers `/health` and stops cleanly (with a database).

use fcp_testkit::smoke::{self, free_port, Smoke};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-backup");

#[test]
fn version_and_help() {
    smoke::assert_version_and_help(BIN, env!("CARGO_PKG_VERSION"));
}

#[test]
fn rejects_malformed_config() {
    smoke::assert_rejects_malformed_config(BIN);
}

#[test]
fn shipped_configurations_check() {
    smoke::assert_configs_check(
        BIN,
        &[],
        &["deploy/docker-compose/configs/fcp-backup.toml"],
        &[],
    );
}

#[test]
fn a_secret_from_the_environment_wins_over_the_file() {
    smoke::assert_env_override(
        BIN,
        r#"[storage]
database_url = "postgresql://fcp:change-me@postgres:5432/fcp"
"#,
        (
            "FCP_BACKUP_DATABASE_URL",
            "postgresql://fcp:from-the-environment@db.internal:5432/fcp",
        ),
        "db.internal",
    );
}

#[tokio::test]
async fn starts_serves_health_and_stops() {
    let Some(db) = fcp_testkit::db::TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL is not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-backup"

[http]
bind_address = "127.0.0.1:{http}"

[storage]
database_url = "{db}"

[backup]
work_dir = "{dir}/work"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();
    db.drop_db().await;
}
