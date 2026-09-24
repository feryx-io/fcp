//! Smoke test for the `fcp-ivr` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-ivr");

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
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-ivr smoke test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-ivr"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:ivr@127.0.0.1:{sip}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();

    db.drop_db().await;
}

/// Leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): a draining
/// IVR says `503` on `/ready` so nothing sends it callers, says so on
/// `/status`, and serves again when the drain is called off — all while
/// `/health` keeps answering `200`, so a liveness probe does not kill it
/// mid-flow.
#[tokio::test]
async fn a_draining_ivr_says_so_and_serves_again() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-ivr drain test");
        return;
    };
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "drain-ivr"
drain_wait_secs = 30

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:ivr@127.0.0.1:{sip}"

[admission]
max_concurrent_calls = 100

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .while_up(move || async move {
            let ready = smoke::loopback(http, "/ready");
            let drain = smoke::loopback(http, "/drain");
            let status = smoke::loopback(http, "/status");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 200, "serving: {body}");

            let (code, body) = smoke::http_post(&drain, "{}").await.expect("drain starts");
            assert_eq!(code, 200, "{body}");
            assert!(body.contains("draining"), "{body}");
            assert!(body.contains("\"wait_secs\":30"), "{body}");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 503, "draining: {body}");
            let (code, body) = smoke::http_get(&status).await.expect("status answers");
            assert_eq!(code, 200);
            assert!(body.contains("\"draining\":true"), "{body}");
            // A liveness probe must still see a live process.
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
/// A `${NAME}` in the file is resolved from the environment when the
/// file is loaded and masked like any secret, and a name the environment
/// lacks refuses the file (`docs/PRODUCTION_STACK.md` phase 2).
#[test]
fn a_reference_in_the_file_is_resolved_from_the_environment() {
    const CONFIG: &str = r#"[sip]
udp_bind = "127.0.0.1:5070"
tcp_bind = "127.0.0.1:5070"

[http]
bind_address = "127.0.0.1:8086"
api_token = "${FCP_SMOKE_REF_IVR}"
"#;
    smoke::assert_env_override(
        BIN,
        CONFIG,
        ("FCP_SMOKE_REF_IVR", "ivr-ref-token-7f3a"),
        r#"api_token = "***""#,
    );
    smoke::assert_reference_unset_refuses(BIN, CONFIG, "FCP_SMOKE_REF_IVR");
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
            "fcp-ivr-local.toml",
            "deploy/docker-compose/configs/fcp-ivr.toml",
        ],
        &["crates/fcp-ivr/fcp-ivr.toml.example"],
    );
}

/// A secret given in the environment wins over the file and is never
/// printed (`docs/SHIP_AND_OPERATE.md` phase 2).
#[test]
fn a_secret_from_the_environment_wins_over_the_file() {
    smoke::assert_env_override(
        BIN,
        r#"[sip]
udp_bind = "127.0.0.1:5070"
tcp_bind = "127.0.0.1:5070"

[http]
bind_address = "0.0.0.0:8086"
"#,
        ("FCP_IVR_API_TOKEN", "ivr-env-token-7f3a"),
        r#"api_token = "***""#,
    );
}
