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
use fcp_testkit::media::{allocate_rtp_range, tone_frame, Codec, ToneDetector};
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
        Self::start_as("e2e-webrtc-gw", None, 0, conference, cluster_redis, false).await
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
        public_join: bool,
    ) -> Self {
        let sip = free_port();
        let http = free_port();
        // The gateway wants 400 ports, reserved so that two gateways in a
        // test (or tests side by side) never share a band.
        let rtp_start = allocate_rtp_range(400);
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
# Every browser in a test comes from one address.
rate_limit_requests = 600
public_join = {public_join}
serve_join_page = {public_join}
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
    /// Packets that arrived on each of a forwarding room's tracks, by
    /// section index.
    tracks: Arc<std::sync::Mutex<std::collections::BTreeMap<u8, u64>>>,
}

impl Browser {
    /// `POST /v1/rooms/{room}/join`, answer the offer, trickle candidates,
    /// and wait until the gateway reports the session connected.
    async fn join(gw: &Gateway, join_token: &str, name: &str) -> Result<Self, String> {
        let body = serde_json::json!({
            "display_name": name,
            "video": false,
            "join_token": join_token,
        });
        Self::join_with(
            gw,
            body,
            PeerConfig {
                codecs: vec![(AudioCodec::PCMU, 0)],
                video: None,
                ..PeerConfig::default()
            },
        )
        .await
    }

    /// Join asking for a forwarding room's streams on `tracks` tracks, a
    /// browser taking every one of them (VP8).
    async fn join_forwarding(
        gw: &Gateway,
        join_token: &str,
        name: &str,
        tracks: u8,
    ) -> Result<Self, String> {
        let body = serde_json::json!({
            "display_name": name,
            "video": true,
            "content": false,
            "streams": tracks,
            "join_token": join_token,
        });
        Self::join_with(
            gw,
            body,
            PeerConfig {
                codecs: vec![(AudioCodec::PCMU, 0)],
                video: Some(forge_webrtc::VideoConfig {
                    codecs: vec![(forge_webrtc::VideoCodec::VP8, 96)],
                    streams: tracks,
                    ..forge_webrtc::VideoConfig::default()
                }),
                ..PeerConfig::default()
            },
        )
        .await
    }

    async fn join_with(
        gw: &Gateway,
        body: serde_json::Value,
        peer_cfg: PeerConfig,
    ) -> Result<Self, String> {
        let body = body.to_string();
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

        let mut peer = PeerConnection::with_config(peer_cfg)
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
            tracks: Arc::default(),
        };
        browser.spawn_listener();
        Ok(browser)
    }

    /// Decode what arrives and feed the tone detectors.
    fn spawn_listener(&self) {
        let events = Arc::clone(&self.events);
        let heard = Arc::clone(&self.heard);
        let tracks = Arc::clone(&self.tracks);
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
                    Some(PeerEvent::StreamRtp(i, _)) => {
                        *tracks.lock().unwrap().entry(i).or_default() += 1;
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
    let (service, sip, _) = conference_server_http(db_url, dir).await;
    (service, sip)
}

/// [`conference_server`], and the port of its HTTP API.
async fn conference_server_http(db_url: &str, dir: &std::path::Path) -> (Service, SocketAddr, u16) {
    conference_server_with(db_url, dir, "").await
}

/// [`conference_server_http`] with more of its `[conference]` section.
async fn conference_server_with(
    db_url: &str,
    dir: &std::path::Path,
    conference: &str,
) -> (Service, SocketAddr, u16) {
    let sip = free_port();
    let http = free_port();
    let rtp_start = allocate_rtp_range(120);
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
# Video priced by the fixed table, not a start-up benchmark a busy
# runner makes small.
video_benchmark_seconds = 0
{conference}

[logging]
level = "info"
format = "compact"
"#,
        rtp_end = rtp_start + 120,
        dir = dir.display(),
    );
    let bin = bins::path("fcp-conference", "fcp-conference-server");
    let service = Service::start("conference", bin, &config, &loopback(http, "/health")).await;
    (service, SocketAddr::from(([127, 0, 0, 1], sip)), http)
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
            ..ClusterOptions::default()
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
            ..ClusterOptions::default()
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
        false,
    )
    .await;
    let mut staying = Gateway::start_as(
        "e2e-gw-staying",
        Some(GW2_URL),
        0,
        cluster.nodes[0].sip,
        Some(&cluster.redis_url),
        false,
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

/// Drives the gateway's join page in Chromium through Playwright, with a
/// fake camera: `PW` is the Playwright package, `URL` the page. It prints
/// the page's log as it goes and leaves when its stdin closes.
const JOIN_IN_CHROMIUM: &str = r#"
const { chromium } = require(process.env.PW);
(async () => {
  const browser = await chromium.launch({
    args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'],
  });
  const page = await browser.newPage();
  page.on('console', (m) => console.log('page:', m.text()));
  await page.goto(process.env.URL);
  const seen = new Set();
  const tick = setInterval(async () => {
    try {
      const lines = (await page.textContent('#log') || '').split('\n').filter((l) => l.trim());
      for (const l of lines) if (!seen.has(l)) { seen.add(l); console.log('log:', l); }
    } catch (_) {}
  }, 500);
  process.stdin.resume();
  process.stdin.on('end', async () => { clearInterval(tick); await browser.close(); process.exit(0); });
})().catch((e) => { console.error(e); process.exit(1); });
"#;

/// A real browser's camera reaches the room as simulcast
/// (`docs/FORWARDING_ROOMS.md` §2.7, R1b): the gateway asks Chromium for
/// three encodings, the join page sizes them, the gateway passes them on
/// under the browser's rid extension, and the room binds each to its SSRC
/// and reads its size from its VP8 keyframes. Runs only where
/// `FCP_TEST_PLAYWRIGHT` names a Playwright package (and `FCP_TEST_NODE`
/// a node, else `node`), the conference server is built with
/// `--features video-vpx,video-openh264`, and the database is set.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_s_camera_reaches_the_room_as_simulcast() {
    let Ok(playwright) = std::env::var("FCP_TEST_PLAYWRIGHT") else {
        eprintln!("skipped: FCP_TEST_PLAYWRIGHT is not set");
        return;
    };
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting_with(
        ROOM,
        serde_json::json!({
            "video_enabled": true,
            "video_codecs": ["H264", "VP8"],
            "video_layout": "grid",
            "video_passthrough": "off",
        }),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (conference, conf_sip, conf_http) = conference_server_http(db.url(), dir.path()).await;
    let mut gw = Gateway::start_as("e2e-webrtc-gw", None, 0, conf_sip, None, true).await;

    let script = dir.path().join("join.js");
    std::fs::write(&script, JOIN_IN_CHROMIUM).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/join?room={ROOM}&name=web&video=true&autojoin=true&token={}",
        gw.http,
        join_token(&seed, "web"),
    );
    let mut chromium = tokio::process::Command::new(
        std::env::var("FCP_TEST_NODE").unwrap_or_else(|_| "node".into()),
    )
    .arg(&script)
    .env("PW", &playwright)
    .env("URL", &url)
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true)
    .spawn()
    .expect("node runs");

    // The room's own report of the browser's camera.
    let scenario = async {
        let deadline = tokio::time::Instant::now() + A_WHILE;
        let mut last = serde_json::Value::Null;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let tenant = seed.tenant_id.to_string();
            let Ok((200, body)) = http_request(
                "GET",
                &loopback(conf_http, &format!("/conferences/{ROOM}/stats")),
                &[("x-fcp-tenant-id", &tenant)],
                None,
            )
            .await
            else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
                continue;
            };
            let layers = json["participants"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|p| p["video"]["layers"].as_array().cloned());
            last = json;
            let Some(layers) = layers else { continue };
            let bound: Vec<&serde_json::Value> = layers
                .iter()
                .filter(|l| l["ssrc"].as_u64().unwrap_or(0) != 0 && l["live"] == true)
                .collect();
            // Read from VP8 keyframes, not guessed.
            let sized = bound
                .iter()
                .filter(|l| l["resolution"].as_str().is_some_and(|r| r != "0x0"))
                .count();
            if layers.len() == 3 && bound.len() >= 2 && sized >= 2 {
                let rids: Vec<&str> = layers.iter().filter_map(|l| l["rid"].as_str()).collect();
                if rids != ["h", "m", "l"] {
                    return Err(format!("the room's layers are {rids:?}"));
                }
                // The tallest live layer is the one decoded: the room's
                // single tile is taller than any of them.
                let height = |l: &serde_json::Value| -> u32 {
                    l["resolution"]
                        .as_str()
                        .and_then(|r| r.split('x').nth(1))
                        .and_then(|h| h.parse().ok())
                        .unwrap_or(0)
                };
                let tallest = bound.iter().max_by_key(|l| height(l)).unwrap();
                if tallest["decoding"] != true {
                    return Err(format!("the tallest live layer is not decoded: {layers:?}"));
                }
                return Ok(layers);
            }
        }
        Err(format!("no simulcast reached the room: {last}"))
    }
    .await;
    drop(chromium.stdin.take());
    let out = tokio::time::timeout(SOON, chromium.wait_with_output()).await;
    match scenario {
        Ok(layers) => eprintln!("the room's layers: {layers:#?}"),
        Err(why) => {
            let page = match out {
                Ok(Ok(o)) => format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                ),
                _ => "chromium did not stop".to_string(),
            };
            let logs = gw.logs().await;
            let conf = conference.kill().await;
            panic!("{why}\n--- page\n{page}\n--- gateway\n{logs}\n--- conference\n{conf}");
        }
    }
    gw.service.take().unwrap().stop().await;
    conference.stop().await;
    db.drop_db().await;
}

/// A browser that asks is forwarded each person's stream on a track of its
/// own (`docs/FORWARDING_ROOMS.md` §2.7, R4b): the gateway joins with
/// `Recv-Info: fcp-view`, puts each of the room's SSRCs on one of the
/// browser's send-only sections, follows the roster in the call to say
/// whose stream is on which mid, and passes the browser's view on to the
/// room, which then sends only what it names.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_that_asks_gets_each_person_on_a_track_and_its_view_is_heard() {
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
    let (conference, conf_sip, _) =
        conference_server_with(db.url(), dir.path(), "video_native_codecs_only = false").await;
    let mut gw = Gateway::start(conf_sip, None).await;

    let scenario = async {
        // Two people send their cameras.
        let mut calls = Vec::new();
        let mut phones = Vec::new();
        for name in ["alice", "carol"] {
            let phone = Phone::new(name, "x", REALM, conf_sip)
                .await
                .map_err(|e| e.to_string())?;
            phone.enable_video(fcp_testkit::video::VideoMode::Plain);
            phone.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
            let call = phone.call(ROOM).await.map_err(|e| e.to_string())?;
            call.answered(SOON).await.map_err(|e| e.to_string())?;
            calls.push(call);
            phones.push(phone);
        }
        let browser = Browser::join_forwarding(&gw, &join_token(&seed, "web"), "web", 4).await?;
        browser.wait_state(&gw, "connected", A_WHILE).await?;
        let send = || async {
            for c in &calls {
                c.video()
                    .expect("a sender has video")
                    .send_frames(1, 10)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tokio::time::sleep(Duration::from_millis(33)).await;
            Ok::<_, String>(())
        };

        // Each person on a track of their own, and the session says whose.
        let deadline = tokio::time::Instant::now() + A_WHILE;
        let (alice, carol, tracks) = loop {
            // Frames all along; the session read now and then (its
            // routes are rate limited).
            for _ in 0..10 {
                send().await?;
            }
            let info = browser.info(&gw).await?;
            let f = &info["forwarding"];
            let id_of = |who: &str| {
                f["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|m| m["name"].as_str().is_some_and(|n| n.contains(who)))
                    .and_then(|m| m["id"].as_str())
                    .map(str::to_string)
            };
            let on = |id: &Option<String>| {
                f["streams"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|s| s["participant"].as_str() == id.as_deref() && s["speaker"] == false)
                    .and_then(|s| s["mid"].as_str())
                    .map(str::to_string)
            };
            let (a, c) = (id_of("alice"), id_of("carol"));
            let tracks = browser.tracks.lock().unwrap().clone();
            if let (Some(a), Some(c)) = (&a, &c) {
                let named = on(&Some(a.clone())).is_some() && on(&Some(c.clone())).is_some();
                if named && tracks.len() >= 2 && tracks.values().all(|n| *n >= 3) {
                    break (a.clone(), c.clone(), tracks);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("never each person on a track: {info}\n{tracks:?}"));
            }
        };
        eprintln!("tracks: {tracks:?}");
        let _ = alice;

        // The browser asks for Carol alone: only her track goes on.
        let view = serde_json::json!({ "streams": [{ "participant": carol, "max_height": 360 }] });
        let (code, body) = gw
            .api(
                "POST",
                &format!("/v1/rooms/{}/view", browser.id),
                &browser.token,
                Some(&view.to_string()),
            )
            .await?;
        if code != 204 {
            return Err(format!("the view answered {code}: {body}"));
        }
        let (code, _) = gw
            .api(
                "POST",
                &format!("/v1/rooms/{}/view", browser.id),
                &browser.token,
                Some("{\"streams\":[{}]}"),
            )
            .await?;
        if code != 400 {
            return Err(format!("a view that means nothing answered {code}"));
        }
        for _ in 0..15 {
            send().await?;
        }
        let before = browser.tracks.lock().unwrap().clone();
        let deadline = tokio::time::Instant::now() + A_WHILE;
        loop {
            send().await?;
            let now = browser.tracks.lock().unwrap().clone();
            let grew: Vec<u8> = now
                .iter()
                .filter(|(i, n)| **n > before.get(i).copied().unwrap_or(0))
                .map(|(i, _)| *i)
                .collect();
            if grew.len() == 1 && now[&grew[0]] >= before.get(&grew[0]).copied().unwrap_or(0) + 10 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("not Carol's track alone: {before:?} then {now:?}"));
            }
        }
        browser.leave(&gw).await?;
        for c in &calls {
            c.hangup().await.map_err(|e| e.to_string())?;
        }
        Ok::<_, String>(())
    }
    .await;
    if let Err(why) = scenario {
        let logs = gw.logs().await;
        let conf = conference.kill().await;
        panic!("{why}\n--- gateway\n{logs}\n--- conference\n{conf}");
    }
    gw.service.take().unwrap().stop().await;
    conference.stop().await;
    db.drop_db().await;
}

/// Three pages of the join page in one Chromium, each with the fake camera
/// and microphone: `PW` is the Playwright package, `URLS` the three pages'
/// addresses, a line each, the last the one watched. The watched page must
/// draw a tile for each of the others (and none for itself) with ann's and
/// bob's pictures playing and carol, a phone without video, as her
/// initials; pin bob large and send the room that view; and light carol's
/// speaking ring while she plays a tone. It prints `RESULT <json>` or
/// `FAIL <why>`, then every page's status and log, and leaves.
const FORWARDING_IN_CHROMIUM: &str = r#"
const { chromium } = require(process.env.PW);
(async () => {
  const browser = await chromium.launch({
    args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream', '--autoplay-policy=no-user-gesture-required'],
  });
  const urls = process.env.URLS.split('\n');
  const pages = [];
  for (const url of urls) {
    const page = await (await browser.newContext()).newPage();
    await page.goto(url);
    pages.push(page);
  }
  const watched = pages[pages.length - 1];
  const tiles = () => watched.$$eval('#grid .tile', (els) => els.map((el) => {
    const v = el.querySelector('video');
    return {
      name: el.dataset.name, mid: el.dataset.mid, width: v.videoWidth, height: v.videoHeight,
      label: el.querySelector('.label').textContent, initials: el.querySelector('.initials').textContent,
      large: el.classList.contains('large'), speaking: el.classList.contains('speaking'),
      novideo: el.classList.contains('novideo'),
    };
  }));
  const until = async (what, test, ms) => {
    const end = Date.now() + ms;
    for (;;) {
      const t = await tiles();
      if (test(t)) return t;
      if (Date.now() > end) throw new Error(`${what}: ${JSON.stringify(t)}`);
      await new Promise((r) => setTimeout(r, 200));
    }
  };
  const playing = (t, who) => t.some((x) => x.name.includes(who) && x.width > 0 && !x.novideo);
  let result = {};
  try {
    result.grid = await until('never a picture from each of the others',
      (t) => playing(t, 'ann') && playing(t, 'bob') && t.some((x) => x.name.includes('carol') && x.novideo), 60000);
    if (result.grid.some((x) => x.name === 'web')) throw new Error('the page drew a tile for itself');
    await watched.click('#grid .tile[data-name*="bob"] .pin');
    result.pinned = await until('bob never large', (t) => t.some((x) => x.name.includes('bob') && x.large), 5000);
    const end = Date.now() + 10000;
    for (;;) {
      const log = await watched.textContent('#log');
      const sent = log.split('\n').filter((l) => l.includes('view sent') && l.includes('"priority":1'));
      if (sent.length) { result.view = sent[sent.length - 1]; break; }
      if (log.includes('view refused')) throw new Error('the room refused the view');
      if (Date.now() > end) throw new Error('the pinned view was never sent');
      await new Promise((r) => setTimeout(r, 200));
    }
    // Still both pictures, Bob's now on the large tile.
    result.after = await until('a picture stopped after the pin', (t) => playing(t, 'ann') && playing(t, 'bob'), 10000);
    result.speaking = (await until('carol never lit a speaking ring',
      (t) => t.some((x) => x.name.includes('carol') && x.speaking), 30000))
      .filter((x) => x.speaking).map((x) => x.name);
    console.log('RESULT ' + JSON.stringify(result));
  } catch (e) {
    console.log('FAIL ' + e.message);
  }
  for (const [i, page] of pages.entries()) {
    console.log(`--- page ${i}: ${await page.textContent('#call-status')}`);
    console.log(await page.textContent('#log'));
  }
  await browser.close();
})().catch((e) => { console.error(e); process.exit(1); });
"#;

/// The join page in a forwarding room (`docs/FORWARDING_ROOMS.md` §2.7,
/// R4c), in Chromium: three browsers join a room that forwards, each
/// taking tracks for the others, and a phone joins without video. The
/// watched page draws a tile per person with their own picture playing
/// (the phone as initials), names on them and none for itself; pinning one
/// makes it large and sends the room a view naming it first; and the
/// phone, heard, lights a speaking ring. Runs where
/// [`a_browser_s_camera_reaches_the_room_as_simulcast`] does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_join_page_draws_a_forwarding_room_and_pins_one_large() {
    let Ok(playwright) = std::env::var("FCP_TEST_PLAYWRIGHT") else {
        eprintln!("skipped: FCP_TEST_PLAYWRIGHT is not set");
        return;
    };
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting_with(
        ROOM,
        serde_json::json!({
            "video_enabled": true,
            // The gateway offers browsers H.264 first.
            "video_codecs": ["H264", "VP8"],
            "video_mode": "auto",
            "video_passthrough": "off",
        }),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (conference, conf_sip, conf_http) = conference_server_http(db.url(), dir.path()).await;
    let mut gw = Gateway::start_as("e2e-webrtc-gw", None, 0, conf_sip, None, true).await;

    // Carol calls in from a phone, audio only, and is heard throughout.
    let carol = Phone::new("carol", "x", REALM, conf_sip)
        .await
        .expect("a phone");
    let call = carol.call(ROOM).await.expect("carol calls the room");
    call.answered(SOON).await.expect("the room answers carol");
    call.media().play_tone(440.0, 180_000);

    let script = dir.path().join("forwarding.js");
    std::fs::write(&script, FORWARDING_IN_CHROMIUM).unwrap();
    let urls: Vec<String> = ["ann", "bob", "web"]
        .iter()
        .map(|name| {
            format!(
                "http://127.0.0.1:{}/join?room={ROOM}&name={name}&video=true&streams=4&autojoin=true&token={}",
                gw.http,
                join_token(&seed, name),
            )
        })
        .collect();
    let run = tokio::process::Command::new(
        std::env::var("FCP_TEST_NODE").unwrap_or_else(|_| "node".into()),
    )
    .arg(&script)
    .env("PW", &playwright)
    .env("URLS", urls.join("\n"))
    .kill_on_drop(true)
    .output();
    let out = match tokio::time::timeout(Duration::from_secs(180), run).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => panic!("node did not run: {e}"),
        Err(_) => panic!("chromium did not finish"),
    };
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let result = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RESULT "))
        .and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok());
    call.media().stop_tone();
    match result {
        Some(result) => eprintln!("the page: {result:#}"),
        None => {
            let tenant = seed.tenant_id.to_string();
            let stats = http_request(
                "GET",
                &loopback(conf_http, &format!("/conferences/{ROOM}/stats")),
                &[("x-fcp-tenant-id", &tenant)],
                None,
            )
            .await
            .map(|(_, body)| body)
            .unwrap_or_default();
            let logs = gw.logs().await;
            let conf = conference.kill().await;
            panic!(
                "{stdout}\n{}\n--- the room\n{stats}\n--- gateway\n{logs}\n--- conference\n{conf}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    gw.service.take().unwrap().stop().await;
    conference.stop().await;
    db.drop_db().await;
}

/// Transport-cc to a browser (`docs/TRANSPORT_CC.md` §3.4, R5c2), in
/// Chromium: a browser that joins a forwarding room negotiates transport-cc
/// with the gateway, the gateway with the room, and the room's estimate
/// for the browser comes from the browser's own feedback, renumbered by
/// the gateway — not from a REMB. A phone sends video for it to be
/// forwarded. Runs where [`a_browser_s_camera_reaches_the_room_as_simulcast`]
/// does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_s_feedback_is_the_room_s_estimate_through_the_gateway() {
    let Ok(playwright) = std::env::var("FCP_TEST_PLAYWRIGHT") else {
        eprintln!("skipped: FCP_TEST_PLAYWRIGHT is not set");
        return;
    };
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting_with(
        ROOM,
        serde_json::json!({
            "video_enabled": true,
            "video_codecs": ["H264", "VP8"],
            "video_mode": "auto",
            "video_passthrough": "off",
        }),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (conference, conf_sip, conf_http) = conference_server_http(db.url(), dir.path()).await;
    let mut gw = Gateway::start_as("e2e-webrtc-gw", None, 0, conf_sip, None, true).await;

    // Carol's phone sends video for the room to forward to the browser.
    let carol = Phone::new("carol", "x", REALM, conf_sip)
        .await
        .expect("a phone");
    carol.enable_video(fcp_testkit::video::VideoMode::Plain);
    let call = carol.call(ROOM).await.expect("carol calls the room");
    call.answered(SOON).await.expect("the room answers carol");
    let carol_video = call.video().expect("carol has video");
    carol_video.set_packet_size(1000);

    let script = dir.path().join("join.js");
    std::fs::write(&script, JOIN_IN_CHROMIUM).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/join?room={ROOM}&name=web&video=true&streams=4&autojoin=true&token={}",
        gw.http,
        join_token(&seed, "web"),
    );
    let mut chromium = tokio::process::Command::new(
        std::env::var("FCP_TEST_NODE").unwrap_or_else(|_| "node".into()),
    )
    .arg(&script)
    .env("PW", &playwright)
    .env("URL", &url)
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true)
    .spawn()
    .expect("node runs");

    let scenario = async {
        let deadline = tokio::time::Instant::now() + A_WHILE * 2;
        let mut last = serde_json::Value::Null;
        while tokio::time::Instant::now() < deadline {
            // Half a second of Carol's camera between looks.
            for _ in 0..15 {
                let _ = carol_video.send_frames(1, 30).await;
                tokio::time::sleep(Duration::from_millis(33)).await;
            }
            let tenant = seed.tenant_id.to_string();
            let Ok((200, body)) = http_request(
                "GET",
                &loopback(conf_http, &format!("/conferences/{ROOM}/stats")),
                &[("x-fcp-tenant-id", &tenant)],
                None,
            )
            .await
            else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
                continue;
            };
            let forwarded = json["participants"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|p| {
                    let f = &p["video"]["forwarded"];
                    f.is_object().then(|| f.clone())
                });
            last = json;
            let Some(f) = forwarded else { continue };
            if f["estimate"] == "transport_cc"
                && f["transport_cc_kbps"].as_u64().is_some_and(|k| k > 0)
            {
                if f["remb_kbps"] != 0 {
                    // Chromium sends no REMB once transport-cc is agreed.
                    return Err(format!("a REMB as well: {f}"));
                }
                return Ok(f);
            }
        }
        Err(format!(
            "the room never estimated from the browser's feedback: {last}"
        ))
    }
    .await;
    drop(chromium.stdin.take());
    let out = tokio::time::timeout(SOON, chromium.wait_with_output()).await;
    match scenario {
        Ok(f) => eprintln!("the browser's viewer: {f:#}"),
        Err(why) => {
            let page = match out {
                Ok(Ok(o)) => format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                ),
                _ => "chromium did not stop".to_string(),
            };
            let logs = gw.logs().await;
            let conf = conference.kill().await;
            panic!("{why}\n--- page\n{page}\n--- gateway\n{logs}\n--- conference\n{conf}");
        }
    }
    gw.service.take().unwrap().stop().await;
    conference.stop().await;
    db.drop_db().await;
}

/// The audience's voice through the gateway (`docs/WEBINARS.md` §2.5): a
/// browser in a webinar raises its hand with `POST /v1/rooms/{id}/hand`,
/// the room has it, and when a host brings it to the stage the room's
/// `fcp-qa` INFO shows on the session as `qa`. It asks a question with
/// `POST /v1/rooms/{id}/questions`, and the room's word of it shows as
/// `qa.questions`. It chats with `POST /v1/rooms/{id}/chat` (W3b): to the
/// hosts, as a webinar's attendee may, and the room's word of it shows as
/// `chat.messages`; a host's mute shows as `chat.muted`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_raises_its_hand_is_brought_to_the_stage_and_asks() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.meeting_with(ROOM, serde_json::json!({ "kind": "webinar" }))
        .await;
    let dir = tempfile::tempdir().unwrap();
    let (conference, conf_sip, conf_http) = conference_server_http(db.url(), dir.path()).await;
    let mut gw = Gateway::start(conf_sip, None).await;
    let conf = |method: &'static str, path: String| async move {
        let (code, body) = http_request(method, &loopback(conf_http, &path), &[], Some("{}"))
            .await
            .map_err(|e| format!("{method} {path}: {e}"))?;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok::<_, String>((code, json))
    };

    let scenario = async {
        // A guest token in a webinar admits an attendee.
        let browser = Browser::join(&gw, &join_token(&seed, "web"), "web").await?;
        browser.wait_state(&gw, "connected", A_WHILE).await?;
        let session = format!("/v1/rooms/{}", browser.id);
        // It waits in the lobby, and is told so, until the webinar goes
        // live (W4).
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            let (_, info) = gw.api("GET", &session, &browser.token, None).await?;
            if info["qa"]["webinar"] == "practice" && info["qa"]["lobby"] == true {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("never told it waits in the lobby: {info}"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let (code, out) = conf("POST", format!("/conferences/{ROOM}/webinar/live")).await?;
        if code != 200 {
            return Err(format!("going live answered {code}: {out}"));
        }
        let deadline = tokio::time::Instant::now() + SOON;
        let info = loop {
            let (_, info) = gw.api("GET", &session, &browser.token, None).await?;
            if info["qa"]["webinar"] == "live" && info["qa"]["lobby"] == false {
                break info;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("never let in from the lobby: {info}"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        if info["qa"]
            != serde_json::json!({ "hand": false, "on_stage": false, "moderates": false, "questions": [], "webinar": "live", "lobby": false })
        {
            return Err(format!("a new session's qa: {info}"));
        }
        if info["chat"] != serde_json::json!({ "muted": false, "messages": [] }) {
            return Err(format!("a new session's chat: {info}"));
        }

        let (code, body) = gw
            .api(
                "POST",
                &format!("{session}/hand"),
                &browser.token,
                Some(r#"{"raised":true}"#),
            )
            .await?;
        if code != 204 {
            return Err(format!("the hand answered {code}: {body}"));
        }
        let (_, info) = gw.api("GET", &session, &browser.token, None).await?;
        if info["qa"]["hand"] != true {
            return Err(format!("the hand is not up on the session: {info}"));
        }
        let (_, room) = conf("GET", format!("/conferences/{ROOM}")).await?;
        let pid = room["participants"][0]
            .as_str()
            .ok_or(format!("nobody in the room: {room}"))?
            .to_string();
        let (_, p) = conf("GET", format!("/conferences/{ROOM}/participants/{pid}")).await?;
        if !p["hand"].is_string() || p["role"] != "attendee" {
            return Err(format!("the room has no hand up: {p}"));
        }

        let (code, out) = conf(
            "POST",
            format!("/conferences/{ROOM}/participants/{pid}/stage"),
        )
        .await?;
        if code != 200 {
            return Err(format!("the stage answered {code}: {out}"));
        }
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            let (_, info) = gw.api("GET", &session, &browser.token, None).await?;
            if info["qa"]["hand"] == false && info["qa"]["on_stage"] == true {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("never told it is on the stage: {info}"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // A question (W3.2): asked through the gateway, and the room's word
        // of it kept on the session. What is not a question's is refused.
        let (code, body) = gw
            .api(
                "POST",
                &format!("{session}/questions"),
                &browser.token,
                Some(r#"{"op":"ask","text":"Is this on?"}"#),
            )
            .await?;
        if code != 204 {
            return Err(format!("the question answered {code}: {body}"));
        }
        let (code, _) = gw
            .api(
                "POST",
                &format!("{session}/questions"),
                &browser.token,
                Some(r#"{"op":"hand","raised":true}"#),
            )
            .await?;
        if code != 400 {
            return Err(format!("a hand by the questions route answered {code}"));
        }
        let (code, _) = gw
            .api(
                "POST",
                &format!("{session}/questions"),
                &browser.token,
                Some(r#"{"op":"dismiss","id":"q-none"}"#),
            )
            .await?;
        if code != 403 {
            return Err(format!("an attendee's dismiss answered {code}"));
        }
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            let (_, info) = gw.api("GET", &session, &browser.token, None).await?;
            let q = &info["qa"]["questions"][0];
            if q["text"] == "Is this on?" && q["mine"] == true {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("never told of its question: {info}"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Chat (W3b): a webinar's attendee writes to the hosts, not the
        // room, and the room's word of it is kept on the session.
        let chat = format!("{session}/chat");
        let (code, _) = gw
            .api(
                "POST",
                &chat,
                &browser.token,
                Some(r#"{"op":"post","text":"Hi","to":"room"}"#),
            )
            .await?;
        if code != 403 {
            return Err(format!("an attendee's post to the room answered {code}"));
        }
        let (code, body) = gw
            .api(
                "POST",
                &chat,
                &browser.token,
                Some(r#"{"op":"post","text":"Hi","to":"hosts"}"#),
            )
            .await?;
        if code != 204 {
            return Err(format!("the post answered {code}: {body}"));
        }
        let (code, _) = gw
            .api(
                "POST",
                &chat,
                &browser.token,
                Some(r#"{"op":"muted","on":true}"#),
            )
            .await?;
        if code != 400 {
            return Err(format!("the room's own message answered {code}"));
        }
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            let (_, info) = gw.api("GET", &session, &browser.token, None).await?;
            let m = &info["chat"]["messages"][0];
            if m["text"] == "Hi" && m["mine"] == true && m["to"] == "hosts" {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("never told of its message: {info}"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let (code, out) = conf(
            "POST",
            format!("/conferences/{ROOM}/participants/{pid}/chat-mute"),
        )
        .await?;
        // The helper sends `{}`: the room wants `on`.
        if code != 400 {
            return Err(format!("a mute without on answered {code}: {out}"));
        }
        let (code, body) = http_request(
            "POST",
            &loopback(
                conf_http,
                &format!("/conferences/{ROOM}/participants/{pid}/chat-mute"),
            ),
            &[],
            Some(r#"{"on":true}"#),
        )
        .await
        .map_err(|e| format!("mute: {e}"))?;
        if code != 200 {
            return Err(format!("the mute answered {code}: {body}"));
        }
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            let (_, info) = gw.api("GET", &session, &browser.token, None).await?;
            if info["chat"]["muted"] == true {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("never told it is muted: {info}"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
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
    gw.service.take().unwrap().stop().await;
    conference.stop().await;
    db.drop_db().await;
}
