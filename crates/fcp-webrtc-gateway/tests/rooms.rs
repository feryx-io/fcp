//! A browser in a room through the WebRTC gateway (`docs/TEST_FLOOR.md`,
//! T5): a forge-webrtc peer stands in for the browser — it takes the
//! gateway's offer from the HTTP API, answers, trickles candidates and
//! carries PCMU over DTLS-SRTP — and a phone in the same room hears it
//! and is heard. Then the phase 5 migration under the browser: the
//! conference node it is on drains, and the gateway swaps its SIP side
//! to the other node without the browser noticing. Skipped without
//! `FCP_TEST_DATABASE_URL` (and `FCP_TEST_REDIS_URL` for the cluster).

use bytes::Bytes;
use fcp_core::storage::{CdrKind, HangupParty};
use fcp_core::{JoinRole, JoinToken};
use fcp_testkit::cluster::{ClusterOptions, ConferenceCluster};
use fcp_testkit::media::{allocate_port_pair, tone_frame, Codec, ToneDetector};
use fcp_testkit::smoke::{http_request, loopback};
use fcp_testkit::{bins, free_port, Phone, Seed, Service, TestDatabase};
use forge_core::AudioCodec;
use forge_webrtc::{ConnectionState, PeerConfig, PeerConnection, PeerEvent};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-webrtc-gw");
const REALM: &str = "e2e.local";
const ROOM: &str = "8000";
const JOIN_SECRET: &str = "e2e-join-secret";
const OPERATOR: &str = "e2e-operator";
const SOON: Duration = Duration::from_secs(5);
const A_WHILE: Duration = Duration::from_secs(20);

/// The gateway, its HTTP address and the operator token.
struct Gateway {
    service: Option<Service>,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Gateway {
    /// Start a gateway that joins rooms on `conference` (a static
    /// conference server) or, with `cluster_redis`, through cluster
    /// placement.
    async fn start(conference: SocketAddr, cluster_redis: Option<&str>) -> Self {
        Self::start_as("e2e-webrtc-gw", None, 0, conference, cluster_redis).await
    }

    /// The same, under a node id of its own and advertising where a
    /// browser reaches it — what a draining peer hands its browsers
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    async fn start_as(
        node_id: &str,
        public_url: Option<&str>,
        drain_wait_secs: u64,
        conference: SocketAddr,
        cluster_redis: Option<&str>,
    ) -> Self {
        let sip = free_port();
        let http = free_port();
        // The gateway wants 400 ports: a band below the phones' 30000+.
        let rtp_start = 20000 + (uuid::Uuid::new_v4().as_u128() % 20) as u16 * 400;
        let dir = tempfile::tempdir().expect("tempdir");
        // With Redis, the gateway's session records ship to the CDR stream
        // (the only sink a node without a database has).
        let cluster = match cluster_redis {
            Some(redis) => format!(
                "[storage]\nredis_url = \"{redis}\"\n\n[cluster]\nenabled = true\nheartbeat_interval_secs = 1\n\n[cdr]\nspool_dir = \"{}/cdr-spool\"\nsink = \"stream\"\n",
                dir.path().display()
            ),
            None => String::new(),
        };
        let config = format!(
            r#"
[service]
node_id = "{node_id}"
shutdown_timeout_secs = 5
drain_wait_secs = {drain_wait_secs}

[http]
bind_address = "127.0.0.1:{http}"
{public_url}

[auth]
tokens = ["{OPERATOR}"]

[sip]
udp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}

[webrtc]
stun_servers = []

[conference]
server_uri = "sip:conference@{conference}"
node_capacity = 1

[tls]
enabled = false

{cluster}
[logging]
level = "info"
format = "compact"
"#,
            rtp_end = rtp_start + 400,
            public_url = match public_url {
                Some(url) => format!("public_url = \"{url}\"\n"),
                None => String::new(),
            },
        );
        let service = Service::start("webrtc-gw", BIN, &config, &loopback(http, "/health")).await;
        Self {
            service: Some(service),
            http,
            _dir: dir,
        }
    }

    /// One request to the gateway's API as `token`: the status and the
    /// JSON body (or the text, when it is not JSON).
    async fn api(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&str>,
    ) -> Result<(u16, serde_json::Value), String> {
        let auth = format!("Bearer {token}");
        let (code, body) = http_request(
            method,
            &loopback(self.http, path),
            &[("Authorization", &auth)],
            body,
        )
        .await
        .map_err(|e| format!("{method} {path}: {e}"))?;
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or_else(|_| serde_json::Value::String(body.clone()));
        Ok((code, json))
    }

    /// The gateway's logs, for a failure.
    async fn logs(&mut self) -> String {
        match self.service.take() {
            Some(s) => s.kill().await,
            None => String::new(),
        }
    }
}

/// A browser: a forge-webrtc peer speaking PCMU, joined through the
/// gateway's HTTP API.
struct Browser {
    id: String,
    token: String,
    peer: Arc<Mutex<PeerConnection>>,
    events: Arc<Mutex<mpsc::Receiver<PeerEvent>>>,
    sender: forge_webrtc::AudioSender,
    heard: Arc<Mutex<Vec<ToneDetector>>>,
}

impl Browser {
    /// `POST /v1/rooms/{room}/join`, answer the offer, trickle candidates,
    /// and wait until the gateway reports the session connected.
    async fn join(gw: &Gateway, join_token: &str, name: &str) -> Result<Self, String> {
        let body = serde_json::json!({
            "display_name": name,
            "video": false,
            "join_token": join_token,
        })
        .to_string();
        let (code, created) = gw
            .api(
                "POST",
                &format!("/v1/rooms/{ROOM}/join"),
                OPERATOR,
                Some(&body),
            )
            .await?;
        if code != 201 {
            return Err(format!("join answered {code}: {created}"));
        }
        let id = created["id"].as_str().ok_or("no session id")?.to_string();
        let token = created["session_token"]
            .as_str()
            .ok_or("no session token")?
            .to_string();
        let offer = created["sdp_offer"].as_str().ok_or("no offer")?.to_string();

        let mut peer = PeerConnection::with_config(PeerConfig {
            codecs: vec![(AudioCodec::PCMU, 0)],
            video: None,
            ..PeerConfig::default()
        })
        .await
        .map_err(|e| format!("peer: {e}"))?;
        peer.set_remote_offer(&offer)
            .await
            .map_err(|e| format!("offer: {e}"))?;
        let answer = peer
            .create_answer()
            .await
            .map_err(|e| format!("answer: {e}"))?;
        let mut events = peer.take_events().ok_or("no events")?;
        let (code, resp) = gw
            .api(
                "POST",
                &format!("/v1/rooms/{id}/answer"),
                &token,
                Some(&serde_json::json!({ "sdp_answer": answer }).to_string()),
            )
            .await?;
        if !(200..300).contains(&code) {
            return Err(format!("answer answered {code}: {resp}"));
        }
        // Trickle our candidates until connected.
        let deadline = tokio::time::Instant::now() + A_WHILE;
        loop {
            tokio::select! {
                Some(ev) = events.recv() => match ev {
                    PeerEvent::LocalCandidate(c) => {
                        let body = serde_json::json!({ "candidate": c.to_sdp_attribute() }).to_string();
                        let (code, resp) = gw
                            .api("POST", &format!("/v1/rooms/{id}/ice-candidate"), &token, Some(&body))
                            .await?;
                        if !(200..300).contains(&code) {
                            return Err(format!("candidate answered {code}: {resp}"));
                        }
                    }
                    PeerEvent::Failed(why) => return Err(format!("browser transport failed: {why}")),
                    PeerEvent::Closed => return Err("browser connection closed".into()),
                    _ => {}
                },
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
            if peer.get_state() == ConnectionState::Connected {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("browser did not connect".into());
            }
        }
        let sender = peer.sender().map_err(|e| format!("sender: {e}"))?;
        let browser = Self {
            id,
            token,
            peer: Arc::new(Mutex::new(peer)),
            events: Arc::new(Mutex::new(events)),
            sender,
            heard: Arc::new(Mutex::new(Vec::new())),
        };
        browser.spawn_listener();
        Ok(browser)
    }

    /// Decode what arrives and feed the tone detectors.
    fn spawn_listener(&self) {
        let events = Arc::clone(&self.events);
        let heard = Arc::clone(&self.heard);
        tokio::spawn(async move {
            loop {
                let ev = events.lock().await.recv().await;
                match ev {
                    Some(PeerEvent::Rtp(pkt)) => {
                        let Some(codec) = Codec::from_payload_type(pkt.header.payload_type())
                        else {
                            continue;
                        };
                        let samples: Vec<i16> =
                            pkt.payload.iter().map(|&b| codec.decode(b)).collect();
                        for d in heard.lock().await.iter_mut() {
                            d.feed(&samples);
                        }
                    }
                    Some(PeerEvent::Closed) | None => return,
                    Some(_) => {}
                }
            }
        });
    }

    /// Play a PCMU sine at `hz` for `ms`, in the background.
    fn play_tone(&self, hz: f32, ms: u64) {
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let mut phase = 0.0f32;
            let mut ticker = tokio::time::interval(Duration::from_millis(20));
            for _ in 0..ms / 20 {
                ticker.tick().await;
                let pcm = tone_frame(hz, &mut phase);
                let payload: Vec<u8> = pcm.iter().map(|&s| Codec::Pcmu.encode(s)).collect();
                if sender.send_audio(Bytes::from(payload), 160).await.is_err() {
                    return;
                }
            }
        });
    }

    /// Wait until `hz` has been heard; `false` at the deadline.
    async fn heard(&self, hz: u32, within: Duration) -> bool {
        self.heard.lock().await.push(ToneDetector::new(hz));
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if self.heard.lock().await.iter().any(|d| d.heard()) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// `GET /v1/rooms/{id}` as the session.
    async fn info(&self, gw: &Gateway) -> Result<serde_json::Value, String> {
        let (code, info) = gw
            .api("GET", &format!("/v1/rooms/{}", self.id), &self.token, None)
            .await?;
        if code != 200 {
            return Err(format!("session info answered {code}: {info}"));
        }
        Ok(info)
    }

    /// Wait until the gateway reports the session in `state`.
    async fn wait_state(&self, gw: &Gateway, state: &str, within: Duration) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let info = self.info(gw).await?;
            if info["state"] == state {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("session never {state}: {info}"));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn leave(&self, gw: &Gateway) -> Result<(), String> {
        let (code, body) = gw
            .api(
                "DELETE",
                &format!("/v1/rooms/{}", self.id),
                &self.token,
                None,
            )
            .await?;
        if !(200..300).contains(&code) {
            return Err(format!("leave answered {code}: {body}"));
        }
        self.peer.lock().await.close();
        Ok(())
    }
}

fn join_token(seed: &Seed, name: &str) -> String {
    JoinToken::new(
        seed.tenant_id,
        ROOM,
        name,
        JoinRole::Guest,
        Duration::from_secs(300),
    )
    .mint(JOIN_SECRET.as_bytes())
}

/// A single conference server with the join secret, for the gateway.
async fn conference_server(db_url: &str, dir: &std::path::Path) -> (Service, SocketAddr) {
    let sip = free_port();
    let http = free_port();
    let rtp_start = allocate_port_pair().rtp_port;
    let config = format!(
        r#"
[service]
node_id = "e2e-conference"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
local_uri = "sip:conference@127.0.0.1:{sip}"
# The gateway names the record each browser's leg belongs to
# (`docs/EDGE_SECURITY.md` §2.3), so the room believes what it says.
trusted_edges = ["127.0.0.1"]

[storage]
database_url = "{db_url}"

[http]
bind_address = "127.0.0.1:{http}"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}

[conference]
recordings_dir = "{dir}/recordings"
announcements_dir = "{dir}/announcements"
join_secret = "{JOIN_SECRET}"

[logging]
level = "info"
format = "compact"
"#,
        rtp_end = rtp_start + 120,
        dir = dir.display(),
    );
    let bin = bins::path("fcp-conference", "fcp-conference-server");
    let service = Service::start("conference", bin, &config, &loopback(http, "/health")).await;
    (service, SocketAddr::from(([127, 0, 0, 1], sip)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_joins_a_room_and_hears_a_phone() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting(ROOM, Some("1234"), None).await; // a PIN the token bypasses
    let dir = tempfile::tempdir().unwrap();
    let (conference, conf_sip) = conference_server(db.url(), dir.path()).await;
    let mut gw = Gateway::start(conf_sip, None).await;

    // Bob dials the room on the phone and keys the PIN; the browser
    // joins through the gateway with a token, past it.
    let bob = Phone::new("bob", "x", REALM, conf_sip).await.unwrap();
    let bob_call = bob.call(ROOM).await.unwrap();
    bob_call.answered(SOON).await.unwrap();
    for d in ['1', '2', '3', '4', '#'] {
        bob_call.media().send_dtmf(d);
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let scenario = async {
        let browser = Browser::join(&gw, &join_token(&seed, "web"), "web").await?;
        browser.wait_state(&gw, "connected", A_WHILE).await?;

        // Each hears the other.
        bob_call.media().play_tone(880.0, 4000);
        if !browser.heard(880, Duration::from_secs(8)).await {
            return Err("the browser never heard Bob".to_string());
        }
        browser.play_tone(440.0, 4000);
        if !bob_call.media().heard(440, Duration::from_secs(8)).await {
            return Err("Bob never heard the browser".to_string());
        }
        browser.leave(&gw).await?;
        Ok::<_, String>(())
    }
    .await;
    if let Err(why) = scenario {
        let logs = gw.logs().await;
        let conf = conference.kill().await;
        panic!("{why}\n{logs}\n{conf}");
    }
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    gw.service.take().unwrap().stop().await;
    conference.stop().await;
    db.drop_db().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_survives_its_node_draining() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting(ROOM, None, None).await;
    let dir = tempfile::tempdir().unwrap();
    let Some(mut cluster) = ConferenceCluster::start(
        db.url(),
        dir.path(),
        ClusterOptions {
            nodes: 2,
            join_secret: Some(JOIN_SECRET.to_string()),
        },
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    let mut gw = Gateway::start(cluster.nodes[0].sip, Some(&cluster.redis_url)).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;

    let scenario = async {
        // The browser is placed on one node; Bob dials the other directly.
        let browser = Browser::join(&gw, &join_token(&seed, "web"), "web").await?;
        browser.wait_state(&gw, "connected", A_WHILE).await?;
        let (web_node, web_id) = cluster
            .where_is(ROOM, "web", A_WHILE)
            .await
            .ok_or("the browser is on no node")?;
        let other = 1 - web_node;
        let bob = Phone::new("bob", "x", REALM, cluster.nodes[other].sip)
            .await
            .map_err(|e| e.to_string())?;
        let bob_call = bob.call(ROOM).await.map_err(|e| e.to_string())?;
        bob_call.answered(SOON).await.map_err(|e| e.to_string())?;
        let before = browser.info(&gw).await?;
        let target_before = before["sip_target"].as_str().unwrap_or("").to_string();

        // The browser's node drains: the gateway swaps its SIP side to
        // the other node, the browser's peer connection untouched.
        let logs = cluster.nodes[web_node].stop().await;
        if !logs.contains("Participant referred to another node") {
            return Err(format!(
                "the draining node never referred the gateway's leg:\n{logs}"
            ));
        }
        let deadline = tokio::time::Instant::now() + A_WHILE;
        loop {
            let there = cluster.nodes[other].participants(ROOM).await;
            let info = browser.info(&gw).await?;
            let target_now = info["sip_target"].as_str().unwrap_or("").to_string();
            if there.contains(&web_id)
                && target_now != target_before
                && info["state"] == "connected"
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "after the drain: the other node has {there:?}, the session is {info}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if browser.peer.lock().await.get_state() != ConnectionState::Connected {
            return Err("the browser's peer connection did not survive the move".into());
        }
        bob_call.media().play_tone(880.0, 4000);
        if !browser.heard(880, Duration::from_secs(8)).await {
            return Err("the browser never heard Bob after the move".into());
        }
        browser.leave(&gw).await?;
        if bob_call.hangup().await.map_err(|e| e.to_string())? != 200 {
            return Err("Bob's BYE was refused".into());
        }
        Ok::<_, String>(())
    }
    .await;
    if let Err(why) = scenario {
        let logs = gw.logs().await;
        let nodes = cluster.kill_all().await;
        panic!("{why}\n{logs}\n{nodes}");
    }
    gw.service.take().unwrap().stop().await;
    cluster.stop_all().await;

    // The records (docs/CDR_PIPELINE.md §5): the gateway's session, from the
    // stream, stored by a processor as fcp-jobs would; and the browser's leg
    // on each node, the drained one and the one it moved to, all under the
    // session's call id.
    let consumer = store_the_stream(db.url(), &cluster.redis_url).await;
    let sessions = seed.cdrs(Some(CdrKind::Gateway), 1, A_WHILE).await;
    let session = &sessions[0];
    assert_eq!(session.tenant_id, seed.tenant_id, "{session:#?}");
    assert_eq!(session.to_uri, format!("conference:{ROOM}"), "{session:#?}");
    assert!(session.answer_time.is_some(), "{session:#?}");
    assert_eq!(
        session.hangup_party,
        Some(HangupParty::Caller),
        "{session:#?}"
    );
    assert_eq!(session.node_id.as_deref(), Some("e2e-webrtc-gw"));
    assert!(
        session.rtp_packets_received.unwrap_or(0) > 0,
        "{session:#?}"
    );
    let legs: Vec<_> = seed
        .cdrs(Some(CdrKind::Conference), 3, A_WHILE)
        .await
        .into_iter()
        .filter(|r| r.call_id == session.call_id)
        .collect();
    assert_eq!(legs.len(), 2, "the browser's leg on each node: {legs:#?}");
    let moved = legs
        .iter()
        .find(|r| {
            r.custom_data
                .as_ref()
                .is_some_and(|d| !d["migrated_to"].is_null())
        })
        .unwrap_or_else(|| panic!("no record of the leg that moved: {legs:#?}"));
    assert_eq!(moved.hangup_party, Some(HangupParty::System), "{moved:#?}");
    let arrived = legs
        .iter()
        .find(|r| {
            r.custom_data
                .as_ref()
                .is_some_and(|d| !d["migrated_from"].is_null())
        })
        .unwrap_or_else(|| panic!("no record of the leg that arrived: {legs:#?}"));
    assert_ne!(moved.node_id, arrived.node_id);
    consumer.stop().await;
    db.drop_db().await;
}

/// The gateway's own drain (`docs/CAPACITY_AND_SURVIVABILITY.md` §6,
/// phase 2b-3): a browser's session is anchored in the gateway process,
/// so a node leaving service cannot move it — it tells the browser where
/// to go instead, and the browser joins the room again there.
///
/// Two gateways in one cluster: the browser is on the first, which then
/// drains. Its session says to move, and names the second's browser-facing
/// address. The browser joins there — the same room, with the same join
/// token — hears the phone that is already in the room, and leaves the
/// first, which is then carrying nothing and stops.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draining_gateway_hands_its_browsers_to_another() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting(ROOM, None, None).await;
    let dir = tempfile::tempdir().unwrap();
    let Some(mut cluster) = ConferenceCluster::start(
        db.url(),
        dir.path(),
        ClusterOptions {
            nodes: 1,
            join_secret: Some(JOIN_SECRET.to_string()),
        },
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    const GW2_URL: &str = "https://gw2.example.com";
    let mut leaving = Gateway::start_as(
        "e2e-gw-leaving",
        Some("https://gw1.example.com"),
        30,
        cluster.nodes[0].sip,
        Some(&cluster.redis_url),
    )
    .await;
    let mut staying = Gateway::start_as(
        "e2e-gw-staying",
        Some(GW2_URL),
        0,
        cluster.nodes[0].sip,
        Some(&cluster.redis_url),
    )
    .await;
    // Long enough for each gateway's heartbeat to reach the other.
    tokio::time::sleep(Duration::from_millis(2500)).await;

    let scenario = async {
        let token = join_token(&seed, "web");
        let browser = Browser::join(&leaving, &token, "web").await?;
        browser.wait_state(&leaving, "connected", A_WHILE).await?;
        let bob = Phone::new("bob", "x", REALM, cluster.nodes[0].sip)
            .await
            .map_err(|e| e.to_string())?;
        let bob_call = bob.call(ROOM).await.map_err(|e| e.to_string())?;
        bob_call.answered(SOON).await.map_err(|e| e.to_string())?;

        // The gateway leaves service.
        let (code, body) = leaving.api("POST", "/drain", OPERATOR, Some("{}")).await?;
        if code != 200 || body["status"] != "draining" {
            return Err(format!("the drain was not started: {code} {body}"));
        }
        let (code, _) = leaving.api("GET", "/ready", OPERATOR, None).await?;
        if code != 503 {
            return Err(format!("a draining gateway answered /ready with {code}"));
        }

        // The browser is told to move, and where.
        let deadline = tokio::time::Instant::now() + A_WHILE;
        loop {
            let info = browser.info(&leaving).await?;
            if info["handover"]["url"] == GW2_URL {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "the session never named a gateway to move to: {info}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        // What the join page does with that: join the room on the named
        // gateway with the same token, then leave the old session.
        let moved = Browser::join(&staying, &token, "web").await?;
        moved.wait_state(&staying, "connected", A_WHILE).await?;
        bob_call.media().play_tone(880.0, 6000);
        if !moved.heard(880, Duration::from_secs(10)).await {
            return Err("the browser never heard the room after moving".into());
        }
        browser.leave(&leaving).await?;

        // With nothing left to carry, the drained gateway is empty.
        let deadline = tokio::time::Instant::now() + A_WHILE;
        loop {
            let (_, rooms) = leaving.api("GET", "/v1/rooms", OPERATOR, None).await?;
            let live = rooms["sessions"]
                .as_array()
                .map(|s| s.iter().filter(|s| s["state"] == "connected").count())
                .unwrap_or(0);
            if live == 0 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("the drained gateway still has {live} sessions"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        moved.leave(&staying).await?;
        if bob_call.hangup().await.map_err(|e| e.to_string())? != 200 {
            return Err("Bob's BYE was refused".into());
        }
        Ok::<_, String>(())
    }
    .await;
    if let Err(why) = scenario {
        let one = leaving.logs().await;
        let two = staying.logs().await;
        let nodes = cluster.kill_all().await;
        panic!("{why}\n{one}\n{two}\n{nodes}");
    }
    leaving.service.take().unwrap().stop().await;
    staying.service.take().unwrap().stop().await;
    cluster.stop_all().await;
    db.drop_db().await;
}

/// A processor consuming the CDR stream into the test database, as the
/// one in fcp-jobs does.
async fn store_the_stream(db_url: &str, redis_url: &str) -> fcp_cdr::Consumer {
    let pg = fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig {
        connection_string: db_url.to_string(),
        max_connections: 2,
        connection_timeout: Duration::from_secs(10),
        encryption: None,
        read_replica_url: None,
        max_replica_lag: Duration::from_secs(5),
    })
    .await
    .expect("connect to the test database");
    let redis = fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
        url: redis_url.to_string(),
        ..Default::default()
    })
    .await
    .expect("connect to redis");
    let source = Arc::new(fcp_storage_redis::cdr_stream::RedisCdrStream::new(
        redis,
        "e2e-processor",
        Duration::from_secs(3600),
    ));
    let store: Arc<dyn fcp_core::storage::CdrStore> = Arc::new(pg);
    fcp_cdr::Consumer::spawn(
        source,
        Arc::new(fcp_cdr::Processor::new(store, None)),
        fcp_cdr::ConsumerConfig::default(),
    )
}
