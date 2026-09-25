//! A site's link against a live cloud (`docs/HYBRID_CONTROL_PLANE.md`
//! phase 1): the admin runs with its link listeners under the testkit's
//! CA; a tenant and a site are made through the API; `fcp-link` enrolls
//! with the one-time token and heartbeats under mutual TLS; the cloud
//! shows the site enrolled with its link up; the spent token enrolls
//! nobody again; a heartbeat without a certificate never reaches a
//! handler; a disabled site's heartbeats are refused and an enabled one's
//! taken again; and a link that stops is marked down by the sweep. Gated
//! on `FCP_TEST_DATABASE_URL`; skipped with a notice when unset.

use fcp_testkit::certs::ClusterCa;
use fcp_testkit::smoke::{free_port, http_request, loopback, toml_path, trust_ca};
use fcp_testkit::{bins, Service, TestDatabase};
use std::time::Duration;

const ADMIN_TOKEN: &str = "link-test-admin-token-change-me-32";
const A_WHILE: Duration = Duration::from_secs(20);

async fn admin(
    port: u16,
    method: &str,
    path: &str,
    tenant: Option<&str>,
    body: Option<&str>,
) -> (u16, serde_json::Value) {
    let bearer = format!("Bearer {ADMIN_TOKEN}");
    let mut headers = vec![
        ("Authorization", bearer.as_str()),
        ("Content-Type", "application/json"),
    ];
    if let Some(tenant) = tenant {
        headers.push(("x-fcp-tenant-id", tenant));
    }
    let url = loopback(port, path);
    let (code, text) = http_request(method, &url, &headers, body)
        .await
        .unwrap_or_else(|e| panic!("{method} {url}: {e}"));
    (
        code,
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
    )
}

async fn until<F: Fn(&serde_json::Value) -> bool>(
    what: &str,
    port: u16,
    path: &str,
    tenant: Option<&str>,
    check: F,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let (code, body) = admin(port, "GET", path, tenant, None).await;
        if code == 200 && check(&body) {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: last {code} {body}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_enrolls_once_heartbeats_under_mutual_tls_and_is_refused_when_disabled() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the link test");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let certs_dir = dir.path().join("certs");
    std::fs::create_dir_all(&certs_dir).unwrap();
    let ca = ClusterCa::new(&certs_dir);
    let admin_cert = ca.node("fcp-admin");
    trust_ca(&ca.ca_path);
    let (http, enroll_port, link_port, link_health) =
        (free_port(), free_port(), free_port(), free_port());

    // The cloud: an admin with its link listeners, heartbeats every
    // second, a link stale after three.
    let admin_config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "link-test-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{ADMIN_TOKEN}"

[storage]
database_url = "{db}"

[link]
enabled = true
enroll_bind = "127.0.0.1:{enroll_port}"
bind = "127.0.0.1:{link_port}"
cert = "{cert}"
key = "{key}"
ca_dir = "{dir}/link-ca"
heartbeat_interval_secs = 1
heartbeat_stale_secs = 3

[logging]
level = "info"
format = "compact"
"#,
        dir = toml_path(dir.path()),
        db = db.url(),
        cert = toml_path(&admin_cert.cert_path),
        key = toml_path(&admin_cert.key_path),
    );
    let admin_service = Service::start_with(
        "fcp-admin",
        bins::path("fcp-admin", "fcp-admin"),
        &admin_config,
        &loopback(http, "/api/v1/health"),
        &[],
    )
    .await;
    assert!(
        dir.path().join("link-ca").join("link-ca.crt").is_file(),
        "the admin made its link CA on first start"
    );

    // A tenant, and a site in it with its one-time token.
    let (code, tenant) = admin(
        http,
        "POST",
        "/api/v1/tenants",
        None,
        Some(&serde_json::json!({ "name": "acme" }).to_string()),
    )
    .await;
    assert_eq!(code, 201, "{tenant}");
    let tenant_id = tenant["id"].as_str().unwrap().to_string();
    let (code, site) = admin(
        http,
        "POST",
        "/api/v1/sites",
        Some(&tenant_id),
        Some(&serde_json::json!({ "name": "acme-hq" }).to_string()),
    )
    .await;
    assert_eq!(code, 201, "{site}");
    let site_id = site["id"].as_str().unwrap().to_string();
    let token = site["enrollment_token"].as_str().unwrap().to_string();
    let site_path = format!("/api/v1/sites/{site_id}");

    // The site: fcp-link with the token, the cloud's CA, its own state.
    let link_config = format!(
        r#"[service]
node_id = "acme-hq-link"

[link]
cloud_url = "https://127.0.0.1:{enroll_port}"
link_url = "https://127.0.0.1:{link_port}"
enrollment_token = "{token}"
state_dir = "{dir}/link-state"
cloud_ca = "{ca}"
heartbeat_interval_secs = 1

[health]
bind_address = "127.0.0.1:{link_health}"

[logging]
level = "info"
format = "compact"
"#,
        dir = toml_path(dir.path()),
        ca = toml_path(&ca.ca_path),
    );
    let link = Service::start_with(
        "fcp-link",
        env!("CARGO_BIN_EXE_fcp-link"),
        &link_config,
        &loopback(link_health, "/health"),
        &[],
    )
    .await;

    // Enrolled, and its link up, as the cloud shows the tenant.
    let shown = until(
        "the site never enrolled with its link up",
        http,
        &site_path,
        Some(&tenant_id),
        |s| s["status"] == "enrolled" && s["link_up"] == true,
    )
    .await;
    assert!(shown["last_seen_at"].as_u64().is_some(), "{shown}");
    assert!(
        shown["certificate_expires_at"].as_u64().is_some(),
        "{shown}"
    );
    assert_eq!(shown["link_version"], env!("CARGO_PKG_VERSION"), "{shown}");
    let state_dir = dir.path().join("link-state");
    for file in ["link.key", "link.crt", "link-ca.crt", "site.json"] {
        assert!(
            state_dir.join(file).is_file(),
            "{file} in the state directory"
        );
    }
    let (code, text) = http_request("GET", &loopback(link_health, "/status"), &[], None)
        .await
        .unwrap();
    assert_eq!(code, 200);
    let status: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(status["enrolled"], true, "{status}");
    assert_eq!(status["site_id"], site_id, "{status}");
    assert!(
        status["heartbeats_sent"].as_u64().unwrap_or(0) >= 1,
        "{status}"
    );
    assert!(status["last_heartbeat_error"].is_null(), "{status}");

    // The cluster page sees every tenant's sites.
    let (code, cluster) = admin(http, "GET", "/api/v1/cluster/sites", None, None).await;
    assert_eq!(code, 200, "{cluster}");
    assert_eq!(cluster["available"], true, "{cluster}");
    assert!(
        cluster["sites"].as_array().is_some_and(|list| list
            .iter()
            .any(|s| s["id"] == site_id && s["link_up"] == true)),
        "{cluster}"
    );

    // The spent token enrolls nobody again.
    let ca_pem = std::fs::read(&ca.ca_path).unwrap();
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .add_root_certificate(reqwest::Certificate::from_pem(&ca_pem).unwrap())
        .build()
        .unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let csr = rcgen::CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .serialize_request(&key)
        .unwrap()
        .pem()
        .unwrap();
    let again = client
        .post(format!("https://127.0.0.1:{enroll_port}/link/v1/enroll"))
        .json(&serde_json::json!({ "token": token, "csr_pem": csr, "link_version": "test" }))
        .send()
        .await
        .expect("the enroll listener answers");
    assert_eq!(
        again.status().as_u16(),
        403,
        "{}",
        again.text().await.unwrap_or_default()
    );

    // A heartbeat without a certificate from the link CA never reaches a
    // handler: the link listener's TLS refuses the connection.
    let bare = client
        .post(format!("https://127.0.0.1:{link_port}/link/v1/heartbeat"))
        .json(&serde_json::json!({ "link_version": "x", "link_protocol": 1, "nodes": [] }))
        .send()
        .await;
    assert!(
        bare.is_err(),
        "a connection without a client certificate was accepted: {:?}",
        bare.map(|r| r.status())
    );

    // Disabled: the cloud shows the link down and refuses the heartbeats.
    let (code, disabled) = admin(
        http,
        "POST",
        &format!("{site_path}/disable"),
        Some(&tenant_id),
        None,
    )
    .await;
    assert_eq!(code, 200, "{disabled}");
    assert_eq!(disabled["link_up"], false, "{disabled}");
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let (_, text) = http_request("GET", &loopback(link_health, "/status"), &[], None)
            .await
            .unwrap();
        let status: serde_json::Value = serde_json::from_str(&text).unwrap();
        if status["last_heartbeat_error"]
            .as_str()
            .is_some_and(|e| e.contains("disabled"))
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the link never learned it was disabled: {status}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // Enabled again: the same certificate and secret serve, and the link
    // comes back up.
    let (code, enabled) = admin(
        http,
        "POST",
        &format!("{site_path}/enable"),
        Some(&tenant_id),
        None,
    )
    .await;
    assert_eq!(code, 200, "{enabled}");
    assert_eq!(enabled["status"], "enrolled", "{enabled}");
    until(
        "the link never came back after enabling",
        http,
        &site_path,
        Some(&tenant_id),
        |s| s["link_up"] == true,
    )
    .await;

    // The link stops; the sweep marks it down once it is stale.
    let log = link.stop().await;
    assert!(!log.contains("panicked"), "the link panicked:\n{log}");
    until(
        "the sweep never marked the stopped link down",
        http,
        &site_path,
        Some(&tenant_id),
        |s| s["link_up"] == false,
    )
    .await;

    let log = admin_service.stop().await;
    assert!(!log.contains("panicked"), "the admin panicked:\n{log}");
    assert!(
        log.contains("A site enrolled"),
        "the admin logged the enrollment"
    );
    assert!(
        log.contains("A site's link is down"),
        "the admin logged the link going down:\n{}",
        log.lines().rev().take(20).collect::<Vec<_>>().join("\n")
    );
    db.drop_db().await;
}
