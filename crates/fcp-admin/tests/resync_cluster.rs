//! A resync reaches a phone on TLS behind NAT through the call manager it
//! registered at (`docs/PROVISIONING.md` §8): a phone's flow (RFC 5626) is
//! held by one node, and only that node can send down it. With two call
//! managers on one database and one Redis, the node without the phone's
//! flow sends nothing, the node holding it sends down it, and the admin,
//! asking both, tells the phone exactly once. Skipped without
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`.

use fcp_testkit::certs::ClusterCa;
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Phone, PhoneEvent, Seed, Service, TestDatabase};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "resync-cluster-static-token-0123456789";
const NODE_TOKEN: &str = "resync-cluster-node-token-0123456789";
const REALM: &str = "fcp.local";

struct Node {
    service: Service,
    http: u16,
    sips: u16,
}

async fn call_manager(
    db: &TestDatabase,
    dir: &Path,
    redis: &str,
    ca: &ClusterCa,
    node_id: &str,
) -> Node {
    let (sip, sips, http) = (free_port(), free_port(), free_port());
    let cert = ca.node(node_id);
    let config = format!(
        r#"
[service]
node_id = "{node_id}"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
sips_bind = "127.0.0.1:{sips}"
tls_cert_path = "{cert}"
tls_key_path = "{key}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[cluster]
enabled = true

[cdr]
spool_dir = "{dir}/{node_id}-cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"
api_token = "{NODE_TOKEN}"

[media]
prompts_dir = "{dir}/{node_id}-prompts"
tts_cache_dir = "{dir}/{node_id}-tts"
recording_dir = "{dir}/{node_id}-recordings"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir),
        cert = smoke::toml_path(&cert.cert_path),
        key = smoke::toml_path(&cert.key_path),
    );
    let service = Service::start(
        node_id,
        fcp_testkit::bins::path("fcp-call-manager", "fcp-call-manager"),
        &config,
        &loopback(http, "/health"),
    )
    .await;
    Node {
        service,
        http,
        sips,
    }
}

async fn node_resync(node: &Node, device: &str, flowless: bool) -> Value {
    reqwest::Client::new()
        .post(loopback(node.http, &format!("/devices/{device}/resync")))
        .bearer_auth(NODE_TOKEN)
        .json(&json!({ "flowless": flowless }))
        .send()
        .await
        .expect("the node answers")
        .json()
        .await
        .unwrap()
}

async fn notifies(phone: &Phone, within: Duration) -> usize {
    let mut n = 0;
    while phone
        .expect(
            within,
            |e| matches!(e, PhoneEvent::Notify { event, .. } if event.starts_with("check-sync")),
        )
        .await
        .is_some()
    {
        n += 1;
    }
    n
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resync_reaches_a_tls_phone_through_the_node_holding_its_flow() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let Some(redis) = private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping");
        return;
    };
    let dir = smoke::temp_dir();
    let seed = Seed::new(db.url(), REALM).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let certs = dir.path().join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    let ca = ClusterCa::new(&certs);
    let a = call_manager(&db, dir.path(), &redis, &ca, "resync-a").await;
    let b = call_manager(&db, dir.path(), &redis, &ca, "resync-b").await;

    let admin_http = free_port();
    let admin_config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "resync-admin"

[api]
bind_address = "127.0.0.1:{admin_http}"
auth_enabled = true
auth_token = "{TOKEN}"

[storage]
database_url = "{db}"

[cluster]
redis_url = "{redis}"

[call_manager]
auth_token = "{NODE_TOKEN}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let admin = Service::start(
        "admin",
        BIN,
        &admin_config,
        &loopback(admin_http, "/api/v1/health"),
    )
    .await;

    // Alice's softphone registers over TLS at node B: B holds its flow.
    let phone = Phone::new_tls(
        &alice.username,
        &alice.password,
        REALM,
        SocketAddr::from(([127, 0, 0, 1], b.sips)),
        &ca.ca_path,
    )
    .await
    .expect("the TLS phone binds");
    assert_eq!(phone.register().await.unwrap(), 200);
    let devices: Value = reqwest::Client::new()
        .get(loopback(admin_http, "/api/v1/devices"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let device = devices["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["sip_uri"].as_str().is_some_and(|u| u.contains("alice@")))
        .and_then(|d| d["id"].as_str())
        .expect("alice's softphone")
        .to_string();

    // Node A cannot reach the phone's flow, so sends nothing; B sends down it.
    let from_a = node_resync(&a, &device, false).await;
    assert_eq!(from_a["sent"], 0, "{from_a}");
    assert_eq!(
        notifies(&phone, Duration::from_secs(2)).await,
        0,
        "nothing from A"
    );
    let from_b = node_resync(&b, &device, false).await;
    assert_eq!(from_b["sent"], 1, "{from_b}");
    assert_eq!(
        notifies(&phone, Duration::from_secs(3)).await,
        1,
        "B, down the flow"
    );

    // The admin asks both nodes (once each heartbeats): the phone is told once.
    let mut seen = 0;
    for _ in 0..40 {
        let nodes: Value = reqwest::Client::new()
            .get(loopback(admin_http, "/api/v1/cluster/nodes"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap_or(Value::Null);
        seen = nodes["nodes"]
            .as_array()
            .map(|n| {
                n.iter()
                    .filter(|n| {
                        n["node_id"]
                            .as_str()
                            .is_some_and(|id| id.starts_with("resync-"))
                            && n["status"] != "offline"
                    })
                    .count()
            })
            .unwrap_or(0);
        if seen >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    if seen < 2 {
        let nodes = reqwest::Client::new()
            .get(loopback(admin_http, "/api/v1/cluster/nodes"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        panic!("both call managers heartbeat: {nodes}");
    }
    let resync: Value = reqwest::Client::new()
        .post(loopback(
            admin_http,
            &format!("/api/v1/devices/{device}/resync"),
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resync["sent"], 1, "{resync}");
    assert_eq!(
        notifies(&phone, Duration::from_secs(3)).await,
        1,
        "told once"
    );

    admin.stop().await;
    a.service.stop().await;
    b.service.stop().await;
    db.drop_db().await;
}
