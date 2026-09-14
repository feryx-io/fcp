//! Two conference nodes as a cluster (`docs/TEST_FLOOR.md`, T4): a room
//! spread across both nodes is one room over the inter-node trunk, and a
//! node that drains hands its callers to the other — the call manager's
//! leg by executing the REFER (distributed conferencing phase 5b), a
//! phone's leg by following it. Needs a database and Redis:
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`; skipped otherwise.

use fcp_testkit::certs::ClusterCa;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{bins, free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-conference");
const REALM: &str = "e2e.local";
const ROOM: &str = "8000";
const SECRET: &str = "e2e-cluster-secret";
const SOON: Duration = Duration::from_secs(5);
const A_WHILE: Duration = Duration::from_secs(20);

/// One conference node of the cluster.
struct Node {
    id: String,
    service: Option<Service>,
    sip: SocketAddr,
    http: u16,
}

impl Node {
    async fn room(&self) -> Option<serde_json::Value> {
        let (code, body) = http_get(&loopback(self.http, &format!("/conferences/{ROOM}")))
            .await
            .ok()?;
        (code == 200)
            .then(|| serde_json::from_str(&body).ok())
            .flatten()
    }

    /// The room's participant ids here (none when the room is not here).
    async fn participants(&self) -> Vec<String> {
        self.room()
            .await
            .and_then(|r| {
                r["participants"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    async fn participant(&self, pid: &str) -> serde_json::Value {
        let (code, body) = http_get(&loopback(
            self.http,
            &format!("/conferences/{ROOM}/participants/{pid}"),
        ))
        .await
        .unwrap_or_else(|e| panic!("participant {pid}: {e}"));
        assert_eq!(code, 200, "participant {pid} on {}: {body}", self.id);
        serde_json::from_str(&body).unwrap()
    }

    /// The id of the participant here whose caller id contains `name`.
    async fn find(&self, name: &str) -> Option<String> {
        for pid in self.participants().await {
            let p = self.participant(&pid).await;
            if p["caller_id"].as_str().unwrap_or("").contains(name) {
                return Some(pid);
            }
        }
        None
    }
}

/// The cluster: two conference nodes and a call manager on one database
/// and one Redis logical database of their own.
struct Cluster {
    nodes: Vec<Node>,
    call_manager: Option<Service>,
    cm_sip: SocketAddr,
    db: Option<TestDatabase>,
    seed: Seed,
    _dir: tempfile::TempDir,
}

impl Cluster {
    async fn start() -> Option<Self> {
        let redis = std::env::var("FCP_TEST_REDIS_URL").ok()?;
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        // A Redis logical database of this test's own (1–15; a random
        // base per process, then one per test), so membership and the
        // registry never see another test's nodes.
        static BASE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let base = *BASE.get_or_init(|| (uuid::Uuid::new_v4().as_u128() % 15) as u32);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let redis = format!("{}/{}", redis.trim_end_matches('/'), 1 + (base + n) % 15);
        let dir = tempfile::tempdir().expect("tempdir");
        let ca = ClusterCa::new(dir.path());
        let suffix = uuid::Uuid::new_v4().simple().to_string()[..6].to_string();

        let mut nodes = Vec::new();
        for name in ["a", "b"] {
            let id = format!("conf-{name}-{suffix}");
            let cert = ca.node(&id);
            let sip = free_port();
            let tls = free_port();
            let http = free_port();
            let rtp_start = allocate_port_pair().rtp_port;
            let config = format!(
                r#"
[service]
node_id = "{id}"
drain_wait_secs = 0

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
tls_bind = "127.0.0.1:{tls}"
tls_cert_path = "{cert}"
tls_key_path = "{key}"
tls_client_ca_path = "{ca}"
local_ip = "127.0.0.1"
local_uri = "sip:conference@127.0.0.1:{sip}"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[http]
bind_address = "127.0.0.1:{http}"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}

[conference]
recordings_dir = "{dir}/recordings-{name}"
announcements_dir = "{dir}/announcements"

[cluster]
enabled = true
secret = "{SECRET}"
heartbeat_interval_secs = 1
trunk_reconcile_interval_secs = 1
migration_timeout_secs = 10

[logging]
level = "info"
format = "compact"
"#,
                cert = cert.cert_path.display(),
                key = cert.key_path.display(),
                ca = ca.ca_path.display(),
                db = db.url(),
                rtp_end = rtp_start + 120,
                dir = dir.path().display(),
            );
            let service = Service::start(&id, BIN, &config, &loopback(http, "/health")).await;
            nodes.push(Node {
                id,
                service: Some(service),
                sip: SocketAddr::from(([127, 0, 0, 1], sip)),
                http,
            });
        }

        // The call manager: cluster-aware placement, a room spread past
        // one participant per node.
        let cm_sip_port = free_port();
        let cm_http = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let config = format!(
            r#"
[service]
node_id = "cm-{suffix}"

[sip]
udp_bind = "127.0.0.1:{cm_sip_port}"
tcp_bind = "127.0.0.1:{cm_sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"
redis_url = "{redis}"

[http]
bind_address = "127.0.0.1:{cm_http}"

[b2bua]
conference_server_uri = "sip:conference@{fallback}"
conference_node_capacity = 1

[cluster]
enabled = true
heartbeat_interval_secs = 1

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"

[logging]
level = "info"
format = "compact"
"#,
            db = db.url(),
            fallback = nodes[0].sip,
            rtp_end = rtp_start + 40,
            dir = dir.path().display(),
        );
        let bin = bins::path("fcp-call-manager", "fcp-call-manager");
        let call_manager =
            Service::start("call-manager", bin, &config, &loopback(cm_http, "/health")).await;
        // Let the heartbeats find each other.
        tokio::time::sleep(Duration::from_millis(2500)).await;

        Some(Self {
            nodes,
            call_manager: Some(call_manager),
            cm_sip: SocketAddr::from(([127, 0, 0, 1], cm_sip_port)),
            db: Some(db),
            seed,
            _dir: dir,
        })
    }

    /// A phone registered at the call manager.
    async fn registered_phone(&self, name: &str, extension: &str) -> Phone {
        let user = self.seed.user(name, "pw", extension).await;
        let phone = Phone::new(&user.username, &user.password, REALM, self.cm_sip)
            .await
            .unwrap();
        assert_eq!(phone.register().await.unwrap(), 200, "{name} registers");
        phone
    }

    /// The node that lists a participant whose caller id contains `name`,
    /// with the participant id; waits for the registry to settle.
    async fn where_is(&self, name: &str) -> Option<(usize, String)> {
        let deadline = tokio::time::Instant::now() + A_WHILE;
        loop {
            for (i, node) in self.nodes.iter().enumerate() {
                if node.service.is_none() {
                    continue;
                }
                if let Some(pid) = node.find(name).await {
                    return Some((i, pid));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn fail(&mut self, why: &str, phone: &Phone) -> ! {
        let mut seen = Vec::new();
        while let Some(ev) = phone.next_event(Duration::from_millis(50)).await {
            seen.push(ev);
        }
        let mut logs = String::new();
        if let Some(s) = self.call_manager.take() {
            logs.push_str(&s.kill().await);
        }
        for node in &mut self.nodes {
            if let Some(s) = node.service.take() {
                logs.push_str(&s.kill().await);
            }
        }
        panic!("{why}\nphone {} saw: {seen:#?}\n{logs}", phone.username);
    }

    async fn finish(mut self) {
        if let Some(s) = self.call_manager.take() {
            s.stop().await;
        }
        for node in &mut self.nodes {
            if let Some(s) = node.service.take() {
                s.stop().await;
            }
        }
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
    }
}

macro_rules! cluster_or_skip {
    () => {
        match Cluster::start().await {
            Some(c) => c,
            None => {
                eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
                return;
            }
        }
    };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_room_spread_over_two_nodes_is_one_room() {
    let mut cluster = cluster_or_skip!();
    cluster.seed.meeting(ROOM, None, None).await;
    let alice = cluster.registered_phone("alice", "1001").await;
    let bob = cluster.registered_phone("bob", "1002").await;

    // Alice lands on one node; with the room capped at one participant
    // per node, Bob lands on the other.
    let alice_call = alice.call(ROOM).await.unwrap();
    if let Err(e) = alice_call.answered(A_WHILE).await {
        cluster
            .fail(&format!("Alice was not admitted: {e}"), &alice)
            .await;
    }
    let (a_node, _) = match cluster.where_is("alice").await {
        Some(x) => x,
        None => cluster.fail("Alice is on no node", &alice).await,
    };
    let bob_call = bob.call(ROOM).await.unwrap();
    if let Err(e) = bob_call.answered(A_WHILE).await {
        cluster
            .fail(&format!("Bob was not admitted: {e}"), &bob)
            .await;
    }
    let (b_node, _) = match cluster.where_is("bob").await {
        Some(x) => x,
        None => cluster.fail("Bob is on no node", &bob).await,
    };
    if a_node == b_node {
        cluster
            .fail(
                &format!("both callers on node {}", cluster.nodes[a_node].id),
                &bob,
            )
            .await;
    }

    // The nodes trunk the room between them — each lists the other as
    // its trunk peer once its side of the trunk carries media — and the
    // callers hear each other across it.
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let mut trunked = 0;
        for node in &cluster.nodes {
            if let Some(r) = node.room().await {
                if r["trunks"]
                    .as_array()
                    .map(|t| !t.is_empty())
                    .unwrap_or(false)
                {
                    trunked += 1;
                }
            }
        }
        if trunked == 2 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            cluster
                .fail("no trunk formed between the nodes", &alice)
                .await;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    alice_call.media().play_tone(440.0, 8000);
    if !bob_call.media().heard(440, Duration::from_secs(10)).await {
        cluster
            .fail("Bob does not hear Alice across the trunk", &bob)
            .await;
    }
    bob_call.media().play_tone(880.0, 8000);
    if !alice_call.media().heard(880, Duration::from_secs(10)).await {
        cluster
            .fail("Alice does not hear Bob across the trunk", &alice)
            .await;
    }

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    cluster.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn callers_survive_their_node_draining() {
    let mut cluster = cluster_or_skip!();
    cluster.seed.meeting(ROOM, None, None).await;
    let alice = cluster.registered_phone("alice", "1001").await;
    let bob = cluster.registered_phone("bob", "1002").await;

    // Alice through the call manager on one node, Bob on the other, and
    // Carol dialing Alice's node directly as a phone that follows REFER.
    let alice_call = alice.call(ROOM).await.unwrap();
    if let Err(e) = alice_call.answered(A_WHILE).await {
        cluster
            .fail(&format!("Alice was not admitted: {e}"), &alice)
            .await;
    }
    let (a_node, alice_id) = match cluster.where_is("alice").await {
        Some(x) => x,
        None => cluster.fail("Alice is on no node", &alice).await,
    };
    let bob_call = bob.call(ROOM).await.unwrap();
    if let Err(e) = bob_call.answered(A_WHILE).await {
        cluster
            .fail(&format!("Bob was not admitted: {e}"), &bob)
            .await;
    }
    let (b_node, _) = match cluster.where_is("bob").await {
        Some(x) => x,
        None => cluster.fail("Bob is on no node", &bob).await,
    };
    assert_ne!(a_node, b_node, "the room is spread");
    let carol = Phone::new("carol", "x", REALM, cluster.nodes[a_node].sip)
        .await
        .unwrap();
    carol.follow_refers(true);
    let carol_call = carol.call(ROOM).await.unwrap();
    if let Err(e) = carol_call.answered(A_WHILE).await {
        cluster
            .fail(&format!("Carol was not admitted: {e}"), &carol)
            .await;
    }

    // Carol counts once her node lists her (the 200 OK comes before the
    // room does).
    let deadline = tokio::time::Instant::now() + A_WHILE;
    while cluster.nodes[a_node].find("carol").await.is_none() {
        if tokio::time::Instant::now() >= deadline {
            cluster
                .fail("Carol is not listed on her node", &carol)
                .await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Alice's node drains: she is moved to Bob's node as the same
    // participant, Carol follows the REFER herself, nobody redials.
    let draining = cluster.nodes[a_node].service.take().expect("node running");
    let logs = draining.stop().await;
    assert!(
        logs.contains("Participant referred to another node"),
        "the draining node referred its callers:\n{logs}"
    );
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let there = cluster.nodes[b_node].participants().await;
        if there.contains(&alice_id) && cluster.nodes[b_node].find("carol").await.is_some() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let mut carol_saw = Vec::new();
            while let Some(ev) = carol.next_event(Duration::from_millis(50)).await {
                carol_saw.push(ev);
            }
            let why = format!(
                "after the drain, node {} has {there:?}
carol saw: {carol_saw:#?}
--- drained node ---
{logs}",
                cluster.nodes[b_node].id
            );
            cluster.fail(&why, &alice).await;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        alice_call.phase(),
        fcp_testkit::phone::CallPhase::Answered,
        "Alice's call is still up"
    );
    assert!(
        alice
            .expect(SOON, |e| matches!(e, PhoneEvent::ReInvite { .. }))
            .await
            .is_some(),
        "the call manager re-INVITEd Alice's phone with the new node's answer"
    );
    bob_call.media().play_tone(880.0, 8000);
    if !alice_call.media().heard(880, Duration::from_secs(10)).await {
        cluster
            .fail("Alice does not hear Bob after the move", &alice)
            .await;
    }

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    cluster.finish().await;
}
