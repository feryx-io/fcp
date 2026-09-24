//! Smoke test for the `fcp-sbc` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::cluster;
use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-sbc");

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
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-sbc smoke test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[core]
external_domain = "smoke.example.com"

[core.storage]
enabled = true
database_url = "{db}"
"#,
        db = db.url(),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .arg("--bind")
        .arg(format!("127.0.0.1:{sip}"))
        .arg("--health-bind")
        .arg(format!("127.0.0.1:{http}"))
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();
    // The config must actually reach the engine: storage was enabled under [core].
    report.assert_logged("CDR storage initialized");

    db.drop_db().await;
}

/// Leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): the SBC
/// answers `/drain` the way the call manager does, `/ready` turns `503`
/// while it drains so a load balancer stops choosing it, and `/health`
/// keeps answering `200` so a liveness probe does not kill it mid-drain.
#[tokio::test]
async fn a_draining_sbc_says_so_and_serves_again() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-sbc drain test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "drain-sbc"
drain_wait_secs = 0

[core]
external_domain = "smoke.example.com"

[core.storage]
enabled = true
database_url = "{db}"
"#,
        db = db.url(),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .arg("--bind")
        .arg(format!("127.0.0.1:{sip}"))
        .arg("--health-bind")
        .arg(format!("127.0.0.1:{http}"))
        .health_url(smoke::loopback(http, "/health"))
        .while_up(move || async move {
            let ready = smoke::loopback(http, "/ready");
            let drain = smoke::loopback(http, "/drain");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 200, "serving: {body}");

            let (code, body) = smoke::http_post(&drain, "{}").await.expect("drain starts");
            assert_eq!(code, 200, "{body}");
            assert!(body.contains("draining"), "{body}");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 503, "draining: {body}");
            assert!(body.contains("\"draining\":true"), "{body}");
            let (code, _) = smoke::http_get(&smoke::loopback(http, "/health"))
                .await
                .expect("health answers");
            assert_eq!(code, 200, "health stays 200 through a drain");

            let (code, body) = smoke::http_request("DELETE", &drain, &[], None)
                .await
                .expect("drain is called off");
            assert_eq!(code, 200, "{body}");
            assert!(body.contains("serving"), "{body}");
            let (code, _) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 200, "serving again");
        })
        .run()
        .await;
    report.assert_clean();

    db.drop_db().await;
}

/// One node id (`docs/DEPLOYMENT_GAPS.md` §1.2): the id an SBC is
/// configured with is the one its heartbeat is filed under, and a drain
/// marks that record — not a second, random one — so an operator who
/// drains `sbc-east-1` sees `sbc-east-1` draining. Also: bound to
/// loopback, it advertises the loopback address as it is.
#[tokio::test]
async fn a_configured_node_id_names_the_record_the_drain_marks() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-sbc membership test");
        return;
    };
    let Some(redis_url) = cluster::private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping fcp-sbc membership test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let node_id = format!("sbc-smoke-{}", uuid::Uuid::new_v4().simple());
    let config = format!(
        r#"redis_url = "{redis}"

[service]
node_id = "{node_id}"
drain_wait_secs = 0

[cluster]
enabled = true
heartbeat_interval_secs = 1

[core]
external_domain = "smoke.example.com"

[core.storage]
enabled = true
database_url = "{db}"
"#,
        db = db.url(),
        redis = redis_url,
    );

    let redis_for_probe = redis_url.clone();
    let id_for_probe = node_id.clone();
    let report = Smoke::new(BIN)
        .config_toml(config)
        .arg("--bind")
        .arg(format!("127.0.0.1:{sip}"))
        .arg("--health-bind")
        .arg(format!("127.0.0.1:{http}"))
        .health_url(smoke::loopback(http, "/health"))
        .while_up(move || async move {
            let record = cluster::heartbeat_when(
                &redis_for_probe,
                &id_for_probe,
                Duration::from_secs(10),
                |_| true,
            )
            .await
            .expect("the SBC heartbeats under its configured id");
            assert_eq!(record["node_id"], id_for_probe.as_str());
            assert_eq!(record["node_type"], "sbc");
            assert_eq!(record["draining"], false);
            assert_eq!(record["sip_address"], format!("127.0.0.1:{sip}"));
            assert_eq!(record["http_address"], format!("127.0.0.1:{http}"));

            let (code, body) = smoke::http_post(&smoke::loopback(http, "/drain"), "{}")
                .await
                .expect("drain starts");
            assert_eq!(code, 200, "{body}");

            let record = cluster::heartbeat_when(
                &redis_for_probe,
                &id_for_probe,
                Duration::from_secs(10),
                |r| r["draining"] == true,
            )
            .await
            .expect("the record is still there");
            assert_eq!(
                record["draining"], true,
                "the drain marks the record the configured id names: {record}"
            );
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
            "fcp-sbc-local.toml",
            "deploy/docker-compose/configs/fcp-sbc.toml",
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
        r#"[core.storage]
enabled = true
"#,
        (
            "FCP_SBC_DATABASE_URL",
            "postgresql://fcp:env-db-pass-7f3a@db.internal:5432/fcp",
        ),
        r#"database_url = "postgresql://fcp:***@db.internal:5432/fcp""#,
    );
}
