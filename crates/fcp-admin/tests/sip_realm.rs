//! The admin computes digest hashes in the deployment's SIP realm
//! (`[sip] realm`, else the call managers' from the operational
//! configuration): users it makes register at a call manager whose realm
//! is not `fcp.local`; when the realm changes, the hashes whose passwords
//! are kept are made again in it and still register, and one made from a
//! password that is not kept cannot be and is refused. Against the admin
//! and call manager binaries; skipped without `FCP_TEST_DATABASE_URL`.

use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, Seed, Service, TestDatabase};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "sip-realm-static-token-0123456789abcdef";
/// The tenant's SIP domain, which names it to the registrar.
const DOMAIN: &str = "acme.example";

fn call_manager_config(db: &TestDatabase, dir: &Path, sip: u16, http: u16) -> String {
    format!(
        r#"
[service]
node_id = "realm-node"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{DOMAIN}"

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[media]
prompts_dir = "{dir}/node-prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir),
    )
}

fn admin_config(db: &TestDatabase, dir: &Path, http: u16) -> String {
    format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "realm-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

[storage]
database_url = "{db}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir),
    )
}

async fn api(http: u16, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
    let mut request = reqwest::Client::new()
        .request(method, loopback(http, path))
        .bearer_auth(TOKEN);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("the admin answers");
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn registers(username: &str, password: &str, sip: u16) -> u16 {
    let phone = Phone::new(
        username,
        password,
        DOMAIN,
        SocketAddr::from(([127, 0, 0, 1], sip)),
    )
    .await
    .expect("the phone binds");
    phone.register().await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_admin_hashes_in_the_registrars_realm_and_follows_it_when_it_changes() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    Seed::new(db.url(), DOMAIN).await;
    let (sip, node_http, admin_http) = (free_port(), free_port(), free_port());
    let node_config = call_manager_config(&db, dir.path(), sip, node_http);
    let node = Service::start(
        "call-manager",
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &node_config,
        &loopback(node_http, "/health"),
    )
    .await;
    let admin_toml = admin_config(&db, dir.path(), admin_http);
    let admin = Service::start(
        "admin",
        BIN,
        &admin_toml,
        &loopback(admin_http, "/api/v1/health"),
    )
    .await;

    // No [sip] realm: the call managers' is the admin's.
    let (_, status) = api(admin_http, reqwest::Method::GET, "/api/v1/status", None).await;
    assert_eq!(status["sip_realm"], DOMAIN, "{status}");

    // A user whose SIP password is kept, and one whose hash is made from
    // the login password, which is not.
    for body in [
        json!({"username": "kept", "password": "Kept-Passw0rd!", "sip_provisioning_password": "Kept-Sip-Secret-1"}),
        json!({"username": "lost", "password": "Lost-Passw0rd!"}),
    ] {
        let (code, made) = api(
            admin_http,
            reqwest::Method::POST,
            "/api/v1/users",
            Some(body),
        )
        .await;
        assert_eq!(code, 201, "{made}");
    }
    assert_eq!(
        registers("kept", "Kept-Sip-Secret-1", sip).await,
        200,
        "in the call managers' realm"
    );
    assert_eq!(registers("lost", "Lost-Passw0rd!", sip).await, 200);

    // The realm changes, in the operational configuration the call
    // managers read; both come back up in it.
    let (code, section) = api(
        admin_http,
        reqwest::Method::GET,
        "/api/v1/config/call-manager/registrar",
        None,
    )
    .await;
    assert_eq!(code, 200, "{section}");
    let mut value = section["value"].clone();
    value["realm"] = json!("beta.example");
    let (code, put) = api(
        admin_http,
        reqwest::Method::PUT,
        "/api/v1/config/call-manager/registrar",
        Some(json!({"value": value, "tenant_id": "_system"})),
    )
    .await;
    assert!(code == 200 || code == 201, "{code} {put}");
    node.stop().await;
    admin.stop().await;
    let node = Service::start(
        "call-manager",
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &node_config,
        &loopback(node_http, "/health"),
    )
    .await;
    let admin = Service::start(
        "admin",
        BIN,
        &admin_toml,
        &loopback(admin_http, "/api/v1/health"),
    )
    .await;
    let (_, status) = api(admin_http, reqwest::Method::GET, "/api/v1/status", None).await;
    assert_eq!(status["sip_realm"], "beta.example", "{status}");

    // The kept password's hash was made again in the new realm; the other
    // could not be.
    assert_eq!(
        registers("kept", "Kept-Sip-Secret-1", sip).await,
        200,
        "re-hashed"
    );
    assert_eq!(
        registers("lost", "Lost-Passw0rd!", sip).await,
        401,
        "no password to re-hash"
    );

    admin.stop().await;
    node.stop().await;
    db.drop_db().await;
}
