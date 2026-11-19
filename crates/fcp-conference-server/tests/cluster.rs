//! Two conference nodes as a cluster (`docs/TEST_FLOOR.md`, T4): a room
//! spread across both nodes is one room over the inter-node trunk, and a
//! node that drains hands its callers to the other — the call manager's
//! leg by executing the REFER (distributed conferencing phase 5b), a
//! phone's leg by following it. Needs a database and Redis:
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`; skipped otherwise.

use fcp_testkit::cluster::{ClusterOptions, ConferenceCluster};
use fcp_testkit::media::allocate_rtp_range;
use fcp_testkit::smoke::{http_get, http_request, loopback};
use fcp_testkit::{bins, free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const REALM: &str = "e2e.local";
const ROOM: &str = "8000";
const SOON: Duration = Duration::from_secs(5);
const A_WHILE: Duration = Duration::from_secs(20);

/// The cluster: the testkit's two conference nodes and a call manager
/// on one database.
struct Cluster {
    nodes: ConferenceCluster,
    call_manager: Option<Service>,
    cm_sip: SocketAddr,
    db: Option<TestDatabase>,
    seed: Seed,
    _dir: tempfile::TempDir,
}

impl Cluster {
    async fn start() -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let nodes =
            ConferenceCluster::start(db.url(), dir.path(), ClusterOptions::default()).await?;
        let suffix = uuid::Uuid::new_v4().simple().to_string()[..6].to_string();

        // The call manager: cluster-aware placement, a room spread past
        // one participant per node.
        let cm_sip_port = free_port();
        let cm_http = free_port();
        let rtp_start = allocate_rtp_range(40);
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
            redis = nodes.redis_url,
            fallback = nodes.nodes[0].sip,
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

    async fn where_is(&self, name: &str) -> Option<(usize, String)> {
        self.nodes.where_is(ROOM, name, A_WHILE).await
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
        logs.push_str(&self.nodes.kill_all().await);
        panic!("{why}\nphone {} saw: {seen:#?}\n{logs}", phone.username);
    }

    async fn finish(mut self) {
        if let Some(s) = self.call_manager.take() {
            s.stop().await;
        }
        self.nodes.stop_all().await;
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
                &format!("both callers on node {}", cluster.nodes.nodes[a_node].id),
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
        for node in &cluster.nodes.nodes {
            if let Some(r) = node.room(ROOM).await {
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
    let carol = Phone::new("carol", "x", REALM, cluster.nodes.nodes[a_node].sip)
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
    while cluster.nodes.nodes[a_node]
        .find(ROOM, "carol")
        .await
        .is_none()
    {
        if tokio::time::Instant::now() >= deadline {
            cluster
                .fail("Carol is not listed on her node", &carol)
                .await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Alice's node drains: she is moved to Bob's node as the same
    // participant, Carol follows the REFER herself, nobody redials.
    let logs = cluster.nodes.nodes[a_node].stop().await;
    assert!(
        logs.contains("Participant referred to another node"),
        "the draining node referred its callers:\n{logs}"
    );
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let there = cluster.nodes.nodes[b_node].participants(ROOM).await;
        if there.contains(&alice_id)
            && cluster.nodes.nodes[b_node]
                .find(ROOM, "carol")
                .await
                .is_some()
        {
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
                cluster.nodes.nodes[b_node].id
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

/// A drain started over HTTP is the same drain SIGTERM runs, and it can
/// be called off (`docs/CAPACITY_AND_SURVIVABILITY.md` §6, phase 2b-4).
///
/// Alice's node is told to leave service while it keeps running: it
/// answers `503` on `/ready`, says `draining` on `/status`, refuses a new
/// caller with `503`, and moves Alice to the other node exactly as
/// stopping it would. Then the drain is called off and the node takes
/// callers again — with its rooms, and the drain never having touched
/// them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drain_over_http_moves_the_callers_and_can_be_called_off() {
    let mut cluster = cluster_or_skip!();
    cluster.seed.meeting(ROOM, None, None).await;
    let alice = cluster.registered_phone("alice", "1001").await;
    let bob = cluster.registered_phone("bob", "1002").await;

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
    let draining_http = cluster.nodes.nodes[a_node].http;

    // Serving, then leaving service — the node keeps running throughout.
    let (code, _) = http_get(&loopback(draining_http, "/ready")).await.unwrap();
    assert_eq!(code, 200, "the node is serving");
    let (code, body) = http_request("POST", &loopback(draining_http, "/drain"), &[], Some("{}"))
        .await
        .unwrap();
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("draining"), "{body}");
    let (code, _) = http_get(&loopback(draining_http, "/ready")).await.unwrap();
    assert_eq!(code, 503, "a draining node is not ready for callers");
    let (code, body) = http_get(&loopback(draining_http, "/status")).await.unwrap();
    assert_eq!(code, 200);
    assert!(body.contains("\"draining\":true"), "{body}");
    let (code, body) = http_get(&loopback(draining_http, "/health")).await.unwrap();
    assert_eq!(code, 200, "health stays 200 through a drain: {body}");

    // A caller dialling the draining node is refused, not left ringing.
    let carol = Phone::new("carol", "x", REALM, cluster.nodes.nodes[a_node].sip)
        .await
        .unwrap();
    let carol_call = carol.call(ROOM).await.unwrap();
    match carol_call.answered(SOON).await {
        Ok(()) => {
            cluster
                .fail("a draining node admitted a new caller", &carol)
                .await
        }
        Err(_) => assert!(
            matches!(
                carol_call.phase(),
                fcp_testkit::phone::CallPhase::Failed(503)
            ),
            "a draining node refuses with 503, not {:?}",
            carol_call.phase()
        ),
    }

    // Alice is moved to the other node, without redialling.
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        if cluster.nodes.nodes[b_node]
            .participants(ROOM)
            .await
            .contains(&alice_id)
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            cluster
                .fail("Alice was never moved off the draining node", &alice)
                .await;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        alice_call.phase(),
        fcp_testkit::phone::CallPhase::Answered,
        "Alice's call is still up"
    );
    bob_call.media().play_tone(880.0, 8000);
    if !alice_call.media().heard(880, Duration::from_secs(10)).await {
        cluster
            .fail("Alice does not hear Bob after the move", &alice)
            .await;
    }

    // Called off: the node serves again.
    let (code, body) = http_request("DELETE", &loopback(draining_http, "/drain"), &[], None)
        .await
        .unwrap();
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("serving"), "{body}");
    let (code, _) = http_get(&loopback(draining_http, "/ready")).await.unwrap();
    assert_eq!(code, 200, "serving again");

    // And takes callers again: the same phone that was refused gets in.
    let carol_again = carol.call(ROOM).await.unwrap();
    if let Err(e) = carol_again.answered(A_WHILE).await {
        cluster
            .fail(
                &format!("the node did not take a caller after the drain was called off: {e}"),
                &carol,
            )
            .await;
    }

    // Through `fail`, so a hang-up refused shows the nodes' logs (#318).
    for (name, call, phone) in [
        ("carol", &carol_again, &carol),
        ("alice", &alice_call, &alice),
        ("bob", &bob_call, &bob),
    ] {
        let code = call.hangup().await;
        if !matches!(code, Ok(200)) {
            cluster
                .fail(&format!("{name}'s hang-up: {code:?}"), phone)
                .await;
        }
    }
    cluster.finish().await;
}

/// Deep testing D5b (`docs/DEEP_TESTING.md`): a node holding half a room
/// is killed with `SIGKILL`, so nothing is drained or referred. Its caller
/// is moved by the call manager to the room on the other node, and hears
/// the room again; the other node's callers talk on, and a new caller
/// joins them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn callers_on_a_killed_node_are_moved_to_another() {
    let mut cluster = cluster_or_skip!();
    cluster.seed.meeting(ROOM, None, None).await;
    let alice = cluster.registered_phone("alice", "1001").await;
    let bob = cluster.registered_phone("bob", "1002").await;
    let carol = cluster.registered_phone("carol", "1003").await;

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
    assert_ne!(a_node, b_node, "the room is spread");

    // Alice's node dies.
    let killed = cluster.nodes.nodes[a_node]
        .service
        .take()
        .expect("running")
        .kill()
        .await;

    // The call manager moves her: her phone is re-INVITEd with the other
    // node's answer, and that node lists her.
    if alice
        .expect(A_WHILE, |e| matches!(e, PhoneEvent::ReInvite { .. }))
        .await
        .is_none()
    {
        let why = format!("Alice was never moved off the dead node\n--- killed node ---\n{killed}");
        cluster.fail(&why, &alice).await;
    }
    let deadline = tokio::time::Instant::now() + A_WHILE;
    while cluster.nodes.nodes[b_node]
        .find(ROOM, "alice")
        .await
        .is_none()
    {
        if tokio::time::Instant::now() >= deadline {
            cluster
                .fail("Alice is not in the room on the live node", &alice)
                .await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        alice_call.phase(),
        fcp_testkit::phone::CallPhase::Answered,
        "Alice's call is still up"
    );
    alice_call.media().play_tone(440.0, 8000);
    if !bob_call.media().heard(440, Duration::from_secs(10)).await {
        cluster
            .fail("Bob does not hear Alice after the move", &bob)
            .await;
    }
    bob_call.media().play_tone(880.0, 8000);
    if !alice_call.media().heard(880, Duration::from_secs(10)).await {
        cluster
            .fail("Alice does not hear Bob after the move", &alice)
            .await;
    }

    // A new caller is placed on the node that is left.
    let carol_call = carol.call(ROOM).await.unwrap();
    if let Err(e) = carol_call.answered(A_WHILE).await {
        cluster
            .fail(&format!("Carol was not admitted: {e}"), &carol)
            .await;
    }
    match cluster.where_is("carol").await {
        Some((n, _)) if n == b_node => {}
        other => {
            let why = format!("Carol is not on the live node: {other:?}");
            cluster.fail(&why, &carol).await;
        }
    }
    bob_call.media().play_tone(660.0, 8000);
    if !carol_call.media().heard(660, Duration::from_secs(10)).await {
        cluster.fail("Carol does not hear Bob", &carol).await;
    }

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    assert_eq!(carol_call.hangup().await.unwrap(), 200);
    cluster.finish().await;
}

/// Dial `node`'s room straight, with VP8 video, asking for each person's
/// own stream (`docs/FORWARDING_ROOMS.md` §2.3).
async fn join_forwarding_at(node: SocketAddr, name: &str) -> (Phone, fcp_testkit::Call) {
    let phone = Phone::new(name, "x", REALM, node)
        .await
        .expect("phone binds");
    phone.enable_video(fcp_testkit::video::VideoMode::Plain);
    phone.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
    let call = phone
        .call_with_headers(ROOM, &[("Recv-Info", "fcp-content, fcp-view")])
        .await
        .expect("INVITE sent");
    call.answered(A_WHILE)
        .await
        .unwrap_or_else(|e| panic!("{name} was not admitted: {e}"));
    (phone, call)
}

/// `GET /conferences/{room}/video` on `node`: the room's video, with its
/// trunks in detail.
async fn room_video(node: &fcp_testkit::cluster::Node) -> Option<serde_json::Value> {
    let (code, body) = http_get(&loopback(node.http, &format!("/conferences/{ROOM}/video")))
        .await
        .ok()?;
    (code == 200)
        .then(|| serde_json::from_str(&body).ok())
        .flatten()
}

/// The SSRC a roster says its subscriber receives `pid` on.
fn src_id_of(roster: &str, pid: &str) -> Option<u32> {
    let user = &roster[roster.find(&format!("sip:{pid}@"))?..];
    let user = &user[..user.find("</user>")?];
    let video = &user[user.find("<media id=\"video\">")?..];
    let video = &video[..video.find("</media>")?];
    let at = video.find("<src-id>")? + "<src-id>".len();
    video[at..at + video[at..].find('<')?].parse().ok()
}

/// The latest roster `phone` has been sent in subscription `call_id`,
/// once one passes `check`.
async fn roster_when(
    phone: &Phone,
    call_id: &str,
    within: Duration,
    check: impl Fn(&str) -> bool,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        if let Some(PhoneEvent::Notify { body, .. }) = phone
            .expect(left, |e| {
                matches!(e, PhoneEvent::Notify { call_id: c, event, .. }
                    if c == call_id && event == "conference")
            })
            .await
        {
            if check(&body) {
                return Some(body);
            }
        }
    }
}

/// A forwarding room on two nodes (`docs/FORWARDING_ROOMS.md` §2.8, R7):
/// the trunk between them forwards each person's own stream, so a caller
/// on one node is sent each caller on the other under an SSRC of their
/// own, named in their roster as everyone else is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forwarding_room_on_two_nodes_forwards_each_person_across_the_trunk() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting_with(
        ROOM,
        serde_json::json!({
            "video_enabled": true,
            "video_codecs": ["VP8"],
            "video_mode": "auto",
        }),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let Some(mut nodes) = ConferenceCluster::start(
        db.url(),
        dir.path(),
        ClusterOptions {
            conference_extra: "video_native_codecs_only = false\n".into(),
            ..ClusterOptions::default()
        },
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        db.drop_db().await;
        return;
    };
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (a, b) = (nodes.nodes[0].sip, nodes.nodes[1].sip);

    // Alice and Carol on one node, Bob on the other.
    let (_alice, alice_call) = join_forwarding_at(a, "alice").await;
    let (_carol, carol_call) = join_forwarding_at(a, "carol").await;
    let (bob, bob_call) = join_forwarding_at(b, "bob").await;
    let alice_id = nodes.nodes[0]
        .find(ROOM, "alice")
        .await
        .expect("Alice on A");
    let carol_id = nodes.nodes[0]
        .find(ROOM, "carol")
        .await
        .expect("Carol on A");
    let bob_id = nodes.nodes[1].find(ROOM, "bob").await.expect("Bob on B");

    let senders = [&alice_call, &carol_call, &bob_call];
    let pump = || async {
        for s in senders {
            let _ = s.video().expect("video").send_frames(1, 10).await;
        }
        tokio::time::sleep(Duration::from_millis(33)).await;
    };

    // Both ends of the trunk forward, and each lists the other's people.
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let mut ready = 0;
        for (node, members) in [(&nodes.nodes[0], 1), (&nodes.nodes[1], 2)] {
            if let Some(r) = room_video(node).await {
                if r["trunks"].as_array().is_some_and(|t| {
                    t.iter()
                        .any(|t| t["forwarding"] == true && t["peer_members"] == members)
                }) {
                    ready += 1;
                }
            }
        }
        if ready == 2 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let rooms = (
                room_video(&nodes.nodes[0]).await,
                room_video(&nodes.nodes[1]).await,
            );
            let logs = nodes.kill_all().await;
            panic!("the trunk never forwarded: {rooms:#?}\n{logs}");
        }
        pump().await;
    }

    // Bob's roster names Alice and Carol, who are on the other node, with
    // the SSRCs he receives them on; and he receives them.
    assert_eq!(
        bob_call.subscribe_in_call("conference", 600).await.unwrap(),
        200
    );
    let mut got = std::collections::HashMap::new();
    let mut named = None;
    let deadline = tokio::time::Instant::now() + A_WHILE;
    while tokio::time::Instant::now() < deadline {
        pump().await;
        if named.is_none() {
            named = roster_when(&bob, bob_call.call_id(), Duration::from_millis(50), |r| {
                src_id_of(r, &alice_id).is_some() && src_id_of(r, &carol_id).is_some()
            })
            .await;
        }
        got = bob_call.video().unwrap().received_by_ssrc();
        if let Some(r) = &named {
            let (sa, sc) = (
                src_id_of(r, &alice_id).unwrap(),
                src_id_of(r, &carol_id).unwrap(),
            );
            if got.get(&sa).copied().unwrap_or(0) >= 10 && got.get(&sc).copied().unwrap_or(0) >= 10
            {
                break;
            }
        }
    }
    let Some(roster) = named else {
        let r = room_video(&nodes.nodes[1]).await;
        let logs = nodes.kill_all().await;
        panic!("Bob's roster never named Alice and Carol; received {got:?}\n{r:#?}\n{logs}");
    };
    assert!(
        roster.contains("alice") && roster.contains("carol"),
        "{roster}"
    );
    let (sa, sc) = (
        src_id_of(&roster, &alice_id).unwrap(),
        src_id_of(&roster, &carol_id).unwrap(),
    );
    if got.get(&sa).copied().unwrap_or(0) < 10 || got.get(&sc).copied().unwrap_or(0) < 10 {
        let r = (
            room_video(&nodes.nodes[0]).await,
            room_video(&nodes.nodes[1]).await,
        );
        let logs = nodes.kill_all().await;
        panic!(
            "Bob never received Alice ({sa}) and Carol ({sc}): {got:?}\n{roster}\n{r:#?}\n{logs}"
        );
    }

    // Alice receives Bob, from the other node, the same way.
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        pump().await;
        let p = nodes.nodes[0].participant(ROOM, &alice_id).await;
        let streams = p["video"]["forwarded"]["streams"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let bob_ssrc = streams
            .iter()
            .find(|s| s["participant"] == bob_id.as_str())
            .and_then(|s| s["ssrc"].as_u64());
        let got = alice_call.video().unwrap().received_by_ssrc();
        if bob_ssrc.is_some_and(|s| got.get(&(s as u32)).copied().unwrap_or(0) >= 10) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let logs = nodes.kill_all().await;
            panic!("Alice never received Bob: {streams:#?} {got:?}\n{logs}");
        }
    }

    for call in senders {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    nodes.stop_all().await;
    db.drop_db().await;
}
