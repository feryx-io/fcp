//! Smoke test for the `fcp-conference` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::cluster;
use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-conference");

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
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-conference-server smoke test");
        return;
    };
    let dir = smoke::temp_dir();
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-conference"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:conference@127.0.0.1:{sip}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[conference]
recordings_dir = "{dir}/recordings"
announcements_dir = "{dir}/announcements"

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

/// A node bound to `0.0.0.0` advertises its `local_ip`
/// (`docs/DEPLOYMENT_GAPS.md` §1.1): the call manager's and the gateway's
/// placement, and a migration's `Refer-To`, send callers to the
/// heartbeat's `sip_address`, so it must be somewhere a caller can reach.
#[tokio::test]
async fn a_node_bound_anywhere_advertises_its_local_ip() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-conference-server heartbeat test");
        return;
    };
    let Some(redis_url) = cluster::private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping fcp-conference-server heartbeat test");
        return;
    };
    let dir = smoke::temp_dir();
    let sip = free_port();
    let http = free_port();
    let node_id = format!("conf-smoke-{}", uuid::Uuid::new_v4().simple());
    let config = format!(
        r#"[service]
node_id = "{node_id}"

[sip]
udp_bind = "0.0.0.0:{sip}"
tcp_bind = "0.0.0.0:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:conference@127.0.0.1:{sip}"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[http]
bind_address = "0.0.0.0:{http}"
# A public HTTP bind must be authenticated; /health stays open.
auth_enabled = true
auth_token = "smoke-heartbeat-token"

[conference]
recordings_dir = "{dir}/recordings"
announcements_dir = "{dir}/announcements"

[cluster]
enabled = true
heartbeat_interval_secs = 1

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        redis = redis_url,
        dir = smoke::toml_path(dir.path()),
    );

    let redis_for_probe = redis_url.clone();
    let id_for_probe = node_id.clone();
    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .while_up(move || async move {
            let record = cluster::heartbeat_when(
                &redis_for_probe,
                &id_for_probe,
                Duration::from_secs(10),
                |_| true,
            )
            .await
            .expect("the node heartbeats");
            assert_eq!(
                record["sip_address"],
                format!("127.0.0.1:{sip}"),
                "bound to 0.0.0.0, the node advertises local_ip: {record}"
            );
            assert_eq!(record["http_address"], format!("127.0.0.1:{http}"));
        })
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
        &[
            "fcp-conference-local.toml",
            "deploy/docker-compose/configs/fcp-conference-server.toml",
            "deploy/docker-compose/configs/fcp-conference-server.gpu.toml",
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
        r#"[sip]
udp_bind = "127.0.0.1:5074"
tcp_bind = "127.0.0.1:5074"

[http]
bind_address = "0.0.0.0:8090"
auth_enabled = true
"#,
        ("FCP_CONFERENCE_AUTH_TOKEN", "conf-env-token-7f3a"),
        r#"auth_token = "***""#,
    );
}
