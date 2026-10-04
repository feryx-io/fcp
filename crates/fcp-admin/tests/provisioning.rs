//! A phone provisioned end to end (`docs/PROVISIONING.md`,
//! `docs/IDENTITY_AND_DEVICES.md` phase 7c): it fetches its file from the
//! tenant's URL as a Yealink names it, registers at the call manager with
//! what the file told it, its registration lands on its device with the
//! model and firmware its User-Agent gave, and the admin tells it to fetch
//! again — by the device and by its template — with the vendor's NOTIFY,
//! sent by the call manager. Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, PhoneEvent, Seed, Service, TestDatabase};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "prov-test-static-token-0123456789abcdef";
const NODE_TOKEN: &str = "prov-test-node-token-0123456789abcdef";
/// The admin computes digest hashes over its realm, `fcp.local`, so the
/// call manager authenticates in it too.
const REALM: &str = "fcp.local";
const SOON: Duration = Duration::from_secs(5);
const MAC: &str = "80:5E:C0:00:00:01";
/// What a Yealink says fetching its file: its MAC included.
const FETCH_AGENT: &str = "Yealink SIP-T54W 96.86.0.70 80:5e:c0:00:00:01";
/// What it says registering: no MAC, so its device is found by the
/// address it fetched from.
const USER_AGENT: &str = "Yealink SIP-T54W 96.86.0.70";

/// A call manager on the admin's database.
async fn call_manager(db: &TestDatabase, dir: &Path) -> (Service, u16, u16) {
    let sip_port = free_port();
    let http = free_port();
    let config = format!(
        r#"
[service]
node_id = "prov-node"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"
api_token = "{NODE_TOKEN}"

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
    );
    let service = Service::start(
        "call-manager",
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &config,
        &loopback(http, "/health"),
    )
    .await;
    (service, http, sip_port)
}

/// A line `key = value` of a Yealink file.
fn setting<'a>(file: &'a str, key: &str) -> &'a str {
    file.lines()
        .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix(" = ")))
        .unwrap_or_else(|| panic!("no `{key}` in\n{file}"))
        .trim()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_fetches_its_file_registers_with_it_and_is_told_to_fetch_again() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    // The default tenant, whose SIP domain names it to the registrar.
    Seed::new(db.url(), REALM).await;
    let (node, node_http, sip_port) = call_manager(&db, dir.path()).await;
    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "prov-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

# What phones are told: the call manager above.
[api.provisioning]
sip_server = "127.0.0.1"
sip_port = {sip_port}
sip_transport = "udp"

[storage]
database_url = "{db}"

[call_manager]
url = "http://127.0.0.1:{node_http}"
auth_token = "{NODE_TOKEN}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let admin = Service::start("admin", BIN, &config, &loopback(http, "/api/v1/health")).await;
    let client = reqwest::Client::new();
    let api = |method: reqwest::Method, path: &str, body: Option<Value>| {
        let mut request = client
            .request(method, loopback(http, path))
            .bearer_auth(TOKEN);
        if let Some(body) = body {
            request = request.json(&body);
        }
        async move {
            let response = request.send().await.expect("the admin answers");
            let status = response.status().as_u16();
            let text = response.text().await.unwrap();
            let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            (status, json, text)
        }
    };
    use reqwest::Method;

    // The tenant: Alice, her desk phone by MAC alone, the Yealink starter,
    // and the provisioning URL.
    let (status, alice, text) = api(
        Method::POST,
        "/api/v1/users",
        Some(json!({"username": "alice", "password": "Str0ng-Passw0rd!",
                    "sip_provisioning_password": "Alice-Sip-Secret-1",
                    "display_name": "Alice Example"})),
    )
    .await;
    assert_eq!(status, 201, "{text}");
    let (status, device, text) = api(
        Method::POST,
        "/api/v1/devices",
        Some(json!({"name": "Alice's desk", "device_type": "desk_phone",
                    "mac_address": MAC, "owner_user_id": alice["id"]})),
    )
    .await;
    assert_eq!(status, 201, "{text}");
    let device_id = device["id"].as_str().unwrap().to_string();
    assert_eq!(device["vendor"], "yealink", "from the MAC's prefix");
    // Alice's softphone is named as she is: the desk phone's registration
    // must still land on the desk phone.
    let (status, softphone, text) = api(
        Method::POST,
        "/api/v1/devices",
        Some(
            json!({"name": "Alice's softphone", "device_type": "softphone",
                    "sip_uri": format!("sip:alice@{REALM}"), "owner_user_id": alice["id"]}),
        ),
    )
    .await;
    assert_eq!(status, 201, "{text}");
    let softphone_id = softphone["id"].as_str().unwrap().to_string();
    let (status, template, text) = api(
        Method::POST,
        "/api/v1/config-templates/library/yealink",
        None,
    )
    .await;
    assert_eq!(status, 201, "{text}");
    let template_id = template["id"].as_str().unwrap().to_string();
    let (status, key, text) = api(Method::POST, "/api/v1/provisioning/key", None).await;
    assert_eq!(status, 201, "{text}");
    let key = key["key"].as_str().unwrap().to_string();

    // The phone asks for its file as a Yealink does, through the proxy.
    let fetched = client
        .get(loopback(
            http,
            &format!("/provision/{key}/805ec0000001.cfg"),
        ))
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-for", "127.0.0.1")
        .header("user-agent", FETCH_AGENT)
        .send()
        .await
        .expect("the admin answers");
    let fetched_status = fetched.status().as_u16();
    let file = fetched.text().await.unwrap();
    if fetched_status != 200 {
        let (_, log, _) = api(
            Method::GET,
            &format!("/api/v1/devices/{device_id}/fetches"),
            None,
        )
        .await;
        panic!("the file was not served ({fetched_status}): {file}; the device's log: {log}");
    }
    let user_name = setting(&file, "account.1.user_name");
    let auth_name = setting(&file, "account.1.auth_name");
    let password = setting(&file, "account.1.password");
    let domain = setting(&file, "account.1.sip_server.1.address");
    let server = setting(&file, "account.1.outbound_proxy.1.address");
    let port: u16 = setting(&file, "account.1.outbound_proxy.1.port")
        .parse()
        .unwrap();
    assert_eq!(
        (user_name, domain, server, port),
        ("alice", REALM, "127.0.0.1", sip_port),
        "{file}"
    );
    assert_eq!(setting(&file, "account.1.sip_server.1.transport_type"), "0");
    assert!(
        file.contains(&format!("/provision/{key}/")),
        "the phone is told where to keep fetching:\n{file}"
    );

    // It registers with what the file said, and says what it is.
    let phone = Phone::with_user_agent(
        user_name,
        auth_name,
        password,
        domain,
        SocketAddr::new(server.parse().unwrap(), port),
        USER_AGENT,
    )
    .await
    .expect("the phone binds");
    let code = phone.register().await.unwrap();
    if code != 200 {
        let logs = node.stop().await;
        panic!("REGISTER answered {code}; the node's log:\n{logs}");
    }

    // Its registration lands on its device, with its model and firmware.
    let mut shown = Value::Null;
    for _ in 0..50 {
        let (_, d, _) = api(Method::GET, &format!("/api/v1/devices/{device_id}"), None).await;
        if d["registration_state"] == "registered" && d["firmware_version"].is_string() {
            shown = d;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if shown["registration_state"] != "registered" {
        let (_, d, _) = api(Method::GET, &format!("/api/v1/devices/{device_id}"), None).await;
        let (_, soft, _) = api(
            Method::GET,
            &format!("/api/v1/devices/{softphone_id}"),
            None,
        )
        .await;
        let (_, log, _) = api(
            Method::GET,
            &format!("/api/v1/devices/{device_id}/fetches"),
            None,
        )
        .await;
        panic!("not written on the desk phone: {d}\nsoftphone: {soft}\nfetches: {log}");
    }
    assert_eq!(shown["model"], "T54W", "{shown}");
    assert_eq!(shown["firmware_version"], "96.86.0.70", "{shown}");
    assert_eq!(shown["user_agent"], USER_AGENT, "{shown}");
    assert!(
        shown["last_ip_address"]
            .as_str()
            .is_some_and(|a| a.starts_with("127.0.0.1:")),
        "{shown}"
    );
    assert!(shown["last_registration"].is_string(), "{shown}");
    let (_, soft, _) = api(
        Method::GET,
        &format!("/api/v1/devices/{softphone_id}"),
        None,
    )
    .await;
    assert_eq!(soft["registration_state"], "unregistered", "{soft}");

    // Told to fetch again: by the device, then by its template.
    let told = || async {
        phone
            .expect(SOON, |e| {
                matches!(e, PhoneEvent::Notify { event, .. } if event.starts_with("check-sync;reboot=false"))
            })
            .await
            .is_some()
    };
    let (status, resync, text) = api(
        Method::POST,
        &format!("/api/v1/devices/{device_id}/resync"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(resync["sent"], 1, "{text}");
    assert!(told().await, "no check-sync for the device");
    let (status, resync, text) = api(
        Method::POST,
        &format!("/api/v1/config-templates/{template_id}/resync"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(
        (resync["devices"].as_u64(), resync["sent"].as_u64()),
        (Some(1), Some(1)),
        "{text}"
    );
    assert!(told().await, "no check-sync for the template");

    // Unregistered, the device says so.
    assert_eq!(phone.register_for(0).await.unwrap(), 200);
    let mut state = Value::Null;
    for _ in 0..50 {
        let (_, d, _) = api(Method::GET, &format!("/api/v1/devices/{device_id}"), None).await;
        state = d["registration_state"].clone();
        if state == "unregistered" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(state, "unregistered");
    // Not registered, a resync reaches nobody.
    let (status, resync, text) = api(
        Method::POST,
        &format!("/api/v1/devices/{device_id}/resync"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(resync["sent"], 0, "{text}");

    admin.stop().await;
    node.stop().await;
    db.drop_db().await;
}
