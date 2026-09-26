//! Smoke test for the `fcp-call-manager` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");

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
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-call-manager smoke test");
        return;
    };
    let dir = smoke::temp_dir();
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-call-manager"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "smoke.example.com"

[storage]
database_url = "{db}"
# The primary standing in for a replica: probed, reachable, "healthy".
read_replica_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[media]
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"

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
        .while_up(move || async move {
            // The replica's health is said on /status and /metrics
            // (`docs/DEPLOYMENT_GAPS.md` §1.7).
            let (code, body) = smoke::http_get(&smoke::loopback(http, "/status"))
                .await
                .expect("status answers");
            assert_eq!(code, 200, "{body}");
            let v: serde_json::Value = serde_json::from_str(&body).expect("json");
            let replica = &v["replica"];
            assert_eq!(replica["reachable"], true, "{body}");
            assert_eq!(replica["healthy"], true, "{body}");
            assert!(
                replica["probe_age_secs"].as_u64().is_some_and(|a| a <= 10),
                "{body}"
            );
            assert!(
                replica["host"].as_str().is_some_and(|h| !h.is_empty()),
                "{body}"
            );

            let (code, metrics) = smoke::http_get(&smoke::loopback(http, "/metrics"))
                .await
                .expect("metrics answer");
            assert_eq!(code, 200);
            assert!(metrics.contains("fcp_replica_healthy{"), "{metrics}");
            assert!(
                metrics.contains("fcp_replica_probe_age_seconds{"),
                "{metrics}"
            );
        })
        .run()
        .await;
    report.assert_clean();

    db.drop_db().await;
}
/// A `${NAME}` in the file is resolved from the environment when the
/// file is loaded and masked like any secret, and a name the environment
/// lacks refuses the file (`docs/PRODUCTION_STACK.md` phase 2).
#[test]
fn a_reference_in_the_file_is_resolved_from_the_environment() {
    const CONFIG: &str = r#"[sip]
udp_bind = "127.0.0.1:5060"
tcp_bind = "127.0.0.1:5060"

[http]
bind_address = "127.0.0.1:8084"
api_token = "${FCP_SMOKE_REF_CALL_MANAGER}"
"#;
    smoke::assert_env_override(
        BIN,
        CONFIG,
        ("FCP_SMOKE_REF_CALL_MANAGER", "cm-ref-token-7f3a"),
        r#"api_token = "***""#,
    );
    smoke::assert_reference_unset_refuses(BIN, CONFIG, "FCP_SMOKE_REF_CALL_MANAGER");
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
            "fcp-call-manager-local.toml",
            "deploy/docker-compose/configs/fcp-call-manager.toml",
            "deploy/docker-compose/site/configs/fcp-call-manager.toml",
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
udp_bind = "127.0.0.1:5060"
tcp_bind = "127.0.0.1:5060"

[http]
bind_address = "0.0.0.0:8084"
"#,
        ("FCP_CALL_MANAGER_API_TOKEN", "cm-env-token-7f3a"),
        r#"api_token = "***""#,
    );
}
