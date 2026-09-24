//! Smoke test for the `fcp-api-gateway` binary: it must start on free loopback
//! ports, answer its health endpoint, and exit cleanly on SIGTERM.
//! See `fcp_testkit::smoke` for the harness.

use fcp_testkit::smoke::{self, free_port, Smoke};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-api-gateway");

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
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-gateway"

[gateway]
bind_address = "127.0.0.1:{http}"

[rate_limit]
enabled = false

[cache]
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

/// A backend that answers `200` on one path only and remembers what it
/// was asked, so a test can see which path the gateway probed.
async fn fake_backend(healthy_path: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_server = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let seen = Arc::clone(&seen_by_server);
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("")
                    .to_string();
                let status = if path == healthy_path {
                    "200 OK"
                } else {
                    "404 Not Found"
                };
                seen.lock().unwrap().push(path);
                let body = "ok";
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    (port, seen)
}

/// The gateway probes each backend at the path that backend actually
/// serves — `/api/v1/health` on the admin, `/health` on the events
/// server — in the background, and `/gateway/status` reports the last
/// probe rather than probing on request (`docs/DEPLOYMENT_GAPS.md` §1.4).
#[tokio::test]
async fn backends_are_probed_at_their_own_health_routes_in_the_background() {
    let (admin_port, admin_seen) = fake_backend("/api/v1/health").await;
    let (events_port, events_seen) = fake_backend("/health").await;
    let http = free_port();
    let config = format!(
        r#"[service]
node_id = "smoke-gateway-probes"

[gateway]
bind_address = "127.0.0.1:{http}"

[backends.admin]
url = "http://127.0.0.1:{admin_port}"
health_check_interval = 1

[backends.events]
url = "http://127.0.0.1:{events_port}"
health_check_interval = 1

[rate_limit]
enabled = false

[cache]
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
            let status_url = smoke::loopback(http, "/gateway/status");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            let mut last = String::new();
            loop {
                let (code, body) = smoke::http_get(&status_url).await.expect("status answers");
                assert_eq!(code, 200, "{body}");
                let v: serde_json::Value = serde_json::from_str(&body).expect("json");
                if v["status"] == "operational" {
                    assert_eq!(v["backends"]["admin"]["status"], "healthy", "{body}");
                    assert_eq!(v["backends"]["admin"]["path"], "/api/v1/health", "{body}");
                    assert_eq!(v["backends"]["events"]["status"], "healthy", "{body}");
                    assert_eq!(v["backends"]["events"]["path"], "/health", "{body}");
                    assert!(
                        v["backends"]["admin"]["checked_secs_ago"].is_u64(),
                        "{body}"
                    );
                    assert!(v["backends"]["jobs"].is_null(), "{body}");
                    break;
                }
                last = body;
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the probes never reported both backends healthy: {last}"
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let _ = last;

            // The probe loop, not the status request, is what asked.
            let before = admin_seen.lock().unwrap().len();
            for _ in 0..3 {
                let _ = smoke::http_get(&status_url).await;
            }
            let after = admin_seen.lock().unwrap().len();
            assert!(
                after <= before + 1,
                "status requests must not probe: {before} -> {after}"
            );
            assert!(
                admin_seen
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|p| p == "/api/v1/health"),
                "{:?}",
                admin_seen.lock().unwrap()
            );
            assert!(
                events_seen.lock().unwrap().iter().all(|p| p == "/health"),
                "{:?}",
                events_seen.lock().unwrap()
            );
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
            "fcp-api-gateway-local.toml",
            "deploy/docker-compose/configs/fcp-api-gateway.toml",
            "crates/fcp-api-gateway/fcp-api-gateway.toml.example",
        ],
        &[],
    );
}
