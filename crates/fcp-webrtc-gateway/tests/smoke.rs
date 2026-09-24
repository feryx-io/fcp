//! Smoke test for the `fcp-webrtc-gw` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-webrtc-gw");

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
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-webrtc"
shutdown_timeout_secs = 5

[http]
bind_address = "127.0.0.1:{http}"

[sip]
udp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[auth]
tokens = ["smoke-token"]

[tls]
enabled = false

[logging]
level = "info"
format = "compact"
"#,
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .run()
        .await;
    report.assert_clean();
}

/// Leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): a draining
/// gateway says `503` on `/ready` so nothing sends it browsers, says so on
/// `/status`, refuses a new room join with a `Retry-After`, and serves
/// again when the drain is called off — all while `/health` keeps
/// answering `200`, so a liveness probe does not kill it mid-drain.
///
/// `/ready` is open, as a load balancer reads it without a credential;
/// `/drain` and `/status` are operator routes.
#[tokio::test]
async fn a_draining_gateway_says_so_and_serves_again() {
    let sip = free_port();
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "drain-webrtc"
shutdown_timeout_secs = 5
drain_wait_secs = 30

[http]
bind_address = "127.0.0.1:{http}"
public_url = "https://gw1.example.com"

[sip]
udp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[admission]
max_concurrent_calls = 100

[conference]
server_uri = "sip:conf@127.0.0.1:5080"

[auth]
tokens = ["smoke-token"]

[tls]
enabled = false

[logging]
level = "info"
format = "compact"
"#,
    );

    let report = Smoke::new(BIN)
        .config_toml(config)
        .health_url(smoke::loopback(http, "/health"))
        .while_up(move || async move {
            let operator: &[(&str, &str)] = &[("authorization", "Bearer smoke-token")];
            let ready = smoke::loopback(http, "/ready");
            let drain = smoke::loopback(http, "/drain");
            let status = smoke::loopback(http, "/status");
            let join = smoke::loopback(http, "/v1/rooms/8001/join");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 200, "serving: {body}");

            let (code, body) = smoke::http_request("POST", &drain, operator, Some("{}"))
                .await
                .expect("drain starts");
            assert_eq!(code, 200, "{body}");
            assert!(body.contains("draining"), "{body}");
            assert!(body.contains("\"wait_secs\":30"), "{body}");

            let (code, body) = smoke::http_get(&ready).await.expect("ready answers");
            assert_eq!(code, 503, "draining: {body}");
            let (code, body) = smoke::http_request("GET", &status, operator, None)
                .await
                .expect("status answers");
            assert_eq!(code, 200, "{body}");
            assert!(body.contains("\"draining\":true"), "{body}");
            // A liveness probe must still see a live process.
            let (code, _) = smoke::http_get(&smoke::loopback(http, "/health"))
                .await
                .expect("health answers");
            assert_eq!(code, 200, "health stays 200 through a drain");

            // A browser sent here anyway is refused, and told when to
            // come back rather than left waiting.
            let (code, body) = smoke::http_request("POST", &join, operator, Some("{}"))
                .await
                .expect("the join is answered");
            assert_eq!(
                code, 503,
                "a draining gateway takes no new browsers: {body}"
            );
            assert!(body.contains("draining"), "{body}");

            let (code, body) = smoke::http_request("DELETE", &drain, operator, None)
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
            "fcp-webrtc-gw-local.toml",
            "deploy/docker-compose/configs/fcp-webrtc-gw.toml",
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
bind_address = "127.0.0.1:8095"
"#,
        ("FCP_WEBRTC_GW_AUTH_TOKENS", "gw-env-token-7f3a"),
        r#""***""#,
    );
}
