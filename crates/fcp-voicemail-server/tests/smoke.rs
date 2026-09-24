//! Smoke test for the `fcp-voicemail` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};
use fcp_testkit::TestDatabase;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-voicemail");

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
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-voicemail-server smoke test");
        return;
    };
    let dir = smoke::temp_dir();
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-voicemail"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:voicemail@127.0.0.1:{sip}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[voicemail]
recordings_dir = "{dir}/recordings"
greetings_dir = "{dir}/greetings"

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

/// Leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): a draining
/// voicemail server says `503` on `/ready` so nothing sends it callers,
/// says so on `/status`, and serves again when the drain is called off —
/// all while `/health` keeps answering `200`, so a liveness probe does not
/// kill it in the middle of a message.
#[tokio::test]
async fn a_draining_voicemail_server_says_so_and_serves_again() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping fcp-voicemail-server drain test");
        return;
    };
    let dir = smoke::temp_dir();
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "drain-voicemail"
drain_wait_secs = 30

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:voicemail@127.0.0.1:{sip}"

[admission]
max_concurrent_calls = 100

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[voicemail]
recordings_dir = "{dir}/recordings"
greetings_dir = "{dir}/greetings"

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

/// Every configuration this binary ships parses with no unknown key and
/// prints with its secrets masked under `--check-config`
/// (`docs/SHIP_AND_OPERATE.md` phase 1); the ones a node must accept
/// outright are accepted.
#[test]
fn shipped_configurations_check() {
    smoke::assert_configs_check(
        BIN,
        &[],
        &["fcp-voicemail-local.toml"],
        &["crates/fcp-voicemail-server/fcp-voicemail-server.toml.example"],
    );
}

/// A secret given in the environment wins over the file and is never
/// printed (`docs/SHIP_AND_OPERATE.md` phase 2).
#[test]
fn a_secret_from_the_environment_wins_over_the_file() {
    smoke::assert_env_override(
        BIN,
        r#"[sip]
udp_bind = "127.0.0.1:5072"
tcp_bind = "127.0.0.1:5072"

[http]
bind_address = "0.0.0.0:8087"
"#,
        ("FCP_VOICEMAIL_API_TOKEN", "vm-env-token-7f3a"),
        r#"api_token = "***""#,
    );
}
