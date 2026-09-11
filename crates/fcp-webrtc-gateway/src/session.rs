//! Browser sessions: a room join or a call. Each owns one WebRTC peer
//! connection (the gateway offers, the browser answers through forge-api's
//! `/v1/webrtc/connections/{id}` routes) and, once the browser is
//! connected, one SIP leg — an INVITE to the conference server for a room,
//! or to the destination for a call — with the media relay between them.
//!
//! ```text
//! POST /v1/rooms/{room}/join ─► PeerConnection (offer) ─► browser answers, trickles
//!        │                                                        │
//!        └─ drive: wait Connected ─► ports ─► SDP offer ─► INVITE ─► 200 OK
//!                                          ─► SipLeg ─► run_relay ─► BYE / far-end BYE
//! ```

use crate::placement::ConferencePlacement;
use crate::relay::{
    log_relay_end, run_relay, BrowserLeg, RelayEnd, RelayStats, RelayStatsSnapshot, SipAudioLeg,
    SipLeg, SipVideoLeg,
};
use dashmap::DashMap;
use fcp_media::{
    construct_offer, extract_telephone_event_pt, parse_content_sdes_crypto, parse_sdes_crypto,
    parse_sdp_codecs, parse_video_sdes_crypto, parse_video_sections, remote_audio_endpoint,
    AudioOfferSection, OfferOptions, SdesCryptoLine, SdpCodecInfo, SrtpOffer, VideoOffer,
    VideoOfferSection,
};
use fcp_sip::uac::SipUserAgentClient;
use forge_api::routes::webrtc::WebRtcManager;
use forge_core::AudioCodec;
use forge_rtp::{
    LatchPolicy, PortPair, PortPool, RemoteEndpoint, RtpSocketConfig, RtpSocketPair, SrtpContext,
};
use forge_webrtc::TurnServer;
use forge_webrtc::{
    ConnectionState, Direction, NegotiatedVideo, PeerConfig, PeerConnection, PeerEvent,
    TransportConfig, VideoConfig,
};
use serde::{Deserialize, Serialize};
use sip_core::Headers;
use sip_uac::integrated::CallHandle;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, Notify, RwLock};
use tracing::{debug, info, warn};

/// The header a room join carries its token in (verified by the conference
/// server; see `docs/WEBRTC.md`).
pub use fcp_core::JOIN_TOKEN_HEADER;

/// Audio codecs offered to browsers, in preference order: what the
/// conference mixer speaks, so nothing is transcoded. Every browser ships
/// all three (G.711 is mandatory in WebRTC, RFC 7874 §3).
pub fn browser_audio_codecs() -> Vec<(AudioCodec, u8)> {
    vec![
        (AudioCodec::G722, 9),
        (AudioCodec::PCMU, 0),
        (AudioCodec::PCMA, 8),
    ]
}

/// The SDP name of an audio codec the relay carries.
pub fn audio_codec_name(codec: AudioCodec) -> &'static str {
    match codec {
        AudioCodec::G722 => "G722",
        AudioCodec::PCMU => "PCMU",
        AudioCodec::PCMA => "PCMA",
        AudioCodec::Opus => "opus",
        _ => "unknown",
    }
}

/// What the session manager needs from the configuration.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// The IP on the SIP leg's SDP.
    pub public_ip: String,
    /// The gateway's own SIP URI, for P-Asserted-Identity.
    pub local_uri: String,
    pub stun_servers: Vec<String>,
    pub turn_servers: Vec<TurnServer>,
    pub gather_timeout: Duration,
    pub ice_timeout: Duration,
    pub invite_timeout: Duration,
    /// See [`BrowserLeg::inactivity_timeout`].
    pub inactivity_timeout: Duration,
    /// Offer SDES SRTP on the SIP leg.
    pub srtp: bool,
    /// ICE servers handed to browsers with the offer.
    pub browser_ice_servers: Vec<IceServer>,
}

/// What a session is for.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionKind {
    /// A call to a SIP destination.
    Call { destination: String },
    /// A join of a conference room.
    Room {
        room_number: String,
        display_name: String,
        /// Whether the browser was offered video.
        video: bool,
        /// Whether the browser was offered a second video section for a
        /// shared screen (§15.6).
        content: bool,
        #[serde(skip)]
        join_token: Option<String>,
    },
}

impl SessionKind {
    /// Whether this is a room join.
    pub fn is_room(&self) -> bool {
        matches!(self, SessionKind::Room { .. })
    }
}

/// Where a session is in its life.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Waiting for the browser to answer and connect.
    Connecting,
    /// INVITE sent, waiting for the far end.
    Inviting,
    /// The far end is ringing.
    Ringing,
    /// Media is relayed.
    Connected,
    /// Over; the record lingers briefly for status.
    Terminated,
    /// Failed, with why.
    Failed(String),
}

/// What was negotiated for video on the SIP leg, for status.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VideoSummary {
    pub codec: String,
    pub payload_type: u8,
    pub srtp: bool,
}

/// The floor of the room's shared screen as this session knows it,
/// from the conference server's `INFO` notices (§15.6, 7c).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FloorState {
    /// `free` (nobody shares), `held` (someone else does), `granted`
    /// (this browser does), `refused` (this browser's share was turned
    /// down) or `released` (this browser's share ended).
    pub state: String,
    /// Who shares, while `held` or `granted`; on `refused`, who holds
    /// the floor when that is why.
    pub presenter: Option<String>,
    /// Why, on `refused` and `released`.
    pub reason: Option<String>,
    /// How many notices have been taken in: a page that starts a share
    /// acts only on a verdict newer than the one it saw before.
    pub notices: u64,
}

impl Default for FloorState {
    fn default() -> Self {
        FloorState {
            state: "free".into(),
            presenter: None,
            reason: None,
            notices: 0,
        }
    }
}

impl FloorState {
    /// Take in a notice from the conference server.
    pub fn apply(&mut self, notice: &ContentNotice) {
        let (state, presenter, reason) = match (notice.floor.as_str(), notice.mine) {
            ("granted", true) => ("granted", notice.presenter.clone(), None),
            ("granted", false) => ("held", notice.presenter.clone(), None),
            ("released", true) => ("released", None, notice.reason.clone()),
            ("released", false) => ("free", None, None),
            ("refused", true) => ("refused", notice.presenter.clone(), notice.reason.clone()),
            _ => return,
        };
        self.state = state.into();
        self.presenter = presenter;
        self.reason = reason;
        self.notices += 1;
    }
}

/// What the conference server tells a session about the floor: the body
/// of an `INFO` of type `application/vnd.fcp.content+json`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ContentNotice {
    /// `granted`, `released` or `refused`.
    pub floor: String,
    /// Whether the notice is about this session's participant.
    #[serde(default)]
    pub mine: bool,
    #[serde(default)]
    pub presenter: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// The shared screen's section on the SIP leg, and the share as it
/// stands, for status.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ContentSummary {
    #[serde(flatten)]
    pub video: VideoSummary,
    /// Whether the browser has sent screen packets in the last moments.
    pub sending: bool,
    #[serde(flatten)]
    pub floor: FloorState,
}

/// How recently the browser must have sent a screen packet to count as
/// sending.
const SENDING_WINDOW: Duration = Duration::from_secs(3);

/// A join request body.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct JoinRequest {
    /// Shown to the room as the participant's name.
    #[serde(default)]
    pub display_name: String,
    /// Offer the browser video.
    #[serde(default)]
    pub video: bool,
    /// Offer the browser a second video section to share a screen on;
    /// `null` means yes whenever `video` is.
    #[serde(default)]
    pub content: Option<bool>,
    /// The join token the conference server verifies (4c); optional until
    /// then, in which case the meeting must need no PIN.
    #[serde(default)]
    pub join_token: Option<String>,
}

/// A session's public state.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    #[serde(flatten)]
    pub kind: SessionKind,
    pub connection_id: String,
    pub state: SessionState,
    pub duration_secs: f64,
    /// The audio codec both legs speak, once negotiated.
    pub audio_codec: Option<String>,
    /// The video negotiated on the SIP leg, if any.
    pub video: Option<VideoSummary>,
    /// The shared screen's section on the SIP leg and the floor, if the
    /// section was negotiated.
    pub content: Option<ContentSummary>,
    /// Whether the SIP leg's audio is SRTP.
    pub srtp: bool,
    /// The conference host or SIP destination the INVITE went to.
    pub sip_target: Option<String>,
    pub stats: RelayStatsSnapshot,
}

/// An ICE server a browser should use, as `RTCPeerConnection` takes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// What creating a session returns: the offer the browser must answer,
/// and the token that lets it finish this session's signalling
/// (`/v1/rooms/{id}/answer`, `/ice-candidate`, status and leave) without
/// an operator credential.
#[derive(Debug, Clone, Serialize)]
pub struct CreatedSession {
    #[serde(flatten)]
    pub info: SessionInfo,
    pub sdp_offer: String,
    pub dtls_fingerprint: String,
    pub session_token: String,
    pub ice_servers: Vec<IceServer>,
}

/// One session.
pub struct Session {
    pub id: String,
    pub connection_id: String,
    pub kind: SessionKind,
    pub state: SessionState,
    pub created_at: Instant,
    pub audio_codec: Option<String>,
    pub video: Option<VideoSummary>,
    pub content: Option<VideoSummary>,
    pub floor: FloorState,
    pub srtp: bool,
    pub sip_target: Option<String>,
    pub sip_call_id: Option<String>,
    /// Bearer the browser presents on this session's routes.
    session_token: String,
    dialog: Option<sip_dialog::Dialog>,
    call_handle: Option<Arc<CallHandle>>,
    ports: Vec<PortPair>,
    stop: Arc<Notify>,
    stats: Arc<RelayStats>,
}

impl Session {
    fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            kind: self.kind.clone(),
            connection_id: self.connection_id.clone(),
            state: self.state.clone(),
            duration_secs: self.created_at.elapsed().as_secs_f64(),
            audio_codec: self.audio_codec.clone(),
            video: self.video.clone(),
            content: self.content.clone().map(|video| ContentSummary {
                video,
                sending: self.stats.content_sent_within(SENDING_WINDOW),
                floor: self.floor.clone(),
            }),
            srtp: self.srtp,
            sip_target: self.sip_target.clone(),
            stats: self.stats.snapshot(),
        }
    }

    fn is_over(&self) -> bool {
        matches!(
            self.state,
            SessionState::Terminated | SessionState::Failed(_)
        )
    }
}

/// How long a finished session stays listed.
const LINGER: Duration = Duration::from_secs(60);

/// A request the API turns into a status.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Internal(String),
}

/// Room numbers as the conference server accepts them.
pub fn valid_room_number(room: &str) -> bool {
    !room.is_empty()
        && room.len() <= 64
        && room
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+*#".contains(c))
}

/// A destination as `IntegratedUAC` wants it: a `sip:`/`sips:`/`tel:` URI.
pub fn normalize_destination(destination: &str) -> String {
    let d = destination.trim();
    if d.starts_with("sip:") || d.starts_with("sips:") || d.starts_with("tel:") {
        d.to_string()
    } else {
        format!("sip:{d}")
    }
}

/// A browser connection ready to be answered.
struct BrowserConnection {
    peer: Arc<Mutex<PeerConnection>>,
    events: mpsc::Receiver<PeerEvent>,
    offer: String,
    fingerprint: String,
    ice_ports: Option<PortPair>,
}

/// The SIP leg's negotiated shape, from the answer.
struct SipLegPlan {
    leg: SipLeg,
    audio_codec: String,
    srtp: bool,
    video: Option<VideoSummary>,
    content: Option<VideoSummary>,
}

/// One video section the gateway offered the SIP side: the browser's
/// negotiation for it, our ports and key.
struct OfferedVideo {
    negotiated: NegotiatedVideo,
    ports: PortPair,
    srtp: Option<SrtpOffer>,
}

/// Owns every session.
pub struct SessionManager {
    sessions: DashMap<String, Arc<RwLock<Session>>>,
    by_sip_call_id: DashMap<String, String>,
    uac: Arc<SipUserAgentClient>,
    webrtc: Arc<WebRtcManager>,
    ports: PortPool,
    placement: Arc<ConferencePlacement>,
    cfg: SessionConfig,
}

impl SessionManager {
    pub fn new(
        uac: Arc<SipUserAgentClient>,
        webrtc: Arc<WebRtcManager>,
        ports: PortPool,
        placement: Arc<ConferencePlacement>,
        cfg: SessionConfig,
    ) -> Self {
        Self {
            sessions: DashMap::new(),
            by_sip_call_id: DashMap::new(),
            uac,
            webrtc,
            ports,
            placement,
            cfg,
        }
    }

    /// Join a room: a new peer connection with an audio (and, if asked,
    /// video) offer for the browser; the INVITE follows once it connects.
    pub async fn join_room(
        self: &Arc<Self>,
        room_number: &str,
        req: JoinRequest,
    ) -> Result<CreatedSession, SessionError> {
        if !valid_room_number(room_number) {
            return Err(SessionError::BadRequest(
                "room number must be 1–64 characters of letters, digits or -_.+*#".into(),
            ));
        }
        if !self.placement.is_configured() {
            return Err(SessionError::Internal(
                "no conference server configured: set [conference] server_uri or enable the cluster"
                    .into(),
            ));
        }
        let display_name = if req.display_name.trim().is_empty() {
            "Browser".to_string()
        } else {
            req.display_name
                .trim()
                .chars()
                .filter(|c| *c != '"' && !c.is_control())
                .take(64)
                .collect()
        };
        let content = req.video && req.content.unwrap_or(true);
        let kind = SessionKind::Room {
            room_number: room_number.to_string(),
            display_name,
            video: req.video,
            content,
            join_token: req.join_token.filter(|t| !t.is_empty()),
        };
        let conn = self.new_browser_connection(req.video, content).await?;
        Ok(self.start(kind, conn).await)
    }

    /// Call a SIP destination. With `connection_id` the browser's existing
    /// connection (from `POST /v1/webrtc/connections`) is used, and its
    /// negotiated codec must be one the SIP side carries; without it a new
    /// connection is created and its offer returned.
    pub async fn call(
        self: &Arc<Self>,
        destination: &str,
        connection_id: Option<&str>,
    ) -> Result<CreatedSession, SessionError> {
        if destination.trim().is_empty() {
            return Err(SessionError::BadRequest("destination is required".into()));
        }
        let kind = SessionKind::Call {
            destination: normalize_destination(destination),
        };
        let conn = match connection_id.filter(|id| !id.is_empty()) {
            Some(id) => {
                let peer = self.webrtc.get_connection(id).ok_or_else(|| {
                    SessionError::NotFound(format!("WebRTC connection '{id}' not found"))
                })?;
                let (events, offer, fingerprint) = {
                    let mut guard = peer.lock().await;
                    let events = guard.take_events().ok_or_else(|| {
                        SessionError::Conflict(format!(
                            "WebRTC connection '{id}' is already in use by a session"
                        ))
                    })?;
                    (
                        events,
                        guard.local_sdp().unwrap_or_default().to_string(),
                        guard.dtls_fingerprint().to_string(),
                    )
                };
                BrowserConnection {
                    peer,
                    events,
                    offer,
                    fingerprint,
                    ice_ports: None,
                }
            }
            None => self.new_browser_connection(false, false).await?,
        };
        Ok(self.start(kind, conn).await)
    }

    /// A peer connection offering the mixer's audio codecs and, if asked,
    /// video — and a second video section for a shared screen, so that
    /// sharing later needs no renegotiation — registered with forge-api
    /// so the browser answers and trickles through the existing routes.
    async fn new_browser_connection(
        &self,
        video: bool,
        content: bool,
    ) -> Result<BrowserConnection, SessionError> {
        // The ICE socket comes out of the media port range so a firewall
        // opened for RTP covers it.
        let ice_ports = self
            .ports
            .allocate()
            .await
            .map_err(|e| SessionError::Internal(format!("no media port for ICE: {e}")))?;
        let cfg = PeerConfig {
            stun_servers: self.cfg.stun_servers.clone(),
            direction: Direction::SendRecv,
            codecs: browser_audio_codecs(),
            dtmf: true,
            video: video.then(|| VideoConfig {
                direction: Direction::SendRecv,
                content,
                ..VideoConfig::default()
            }),
            transport: TransportConfig {
                turn_servers: self.cfg.turn_servers.clone(),
                local_port: ice_ports.rtp_port,
                ..TransportConfig::default()
            },
        };
        let built = async {
            let mut peer = PeerConnection::with_config(cfg)
                .await
                .map_err(|e| SessionError::Internal(format!("peer connection: {e}")))?;
            let events = peer.take_events().expect("events taken once");
            let offer = peer
                .create_offer()
                .await
                .map_err(|e| SessionError::Internal(format!("offer: {e}")))?;
            let (offer, events) = self.gather_then_reoffer(&mut peer, events, offer).await;
            Ok::<_, SessionError>((peer, events, offer))
        }
        .await;
        let (peer, events, offer) = match built {
            Ok(v) => v,
            Err(e) => {
                self.ports.deallocate(ice_ports).await;
                return Err(e);
            }
        };
        let fingerprint = peer.dtls_fingerprint().to_string();
        let id = peer.connection_id().to_string();
        let peer = Arc::new(Mutex::new(peer));
        self.webrtc.add_connection(id, peer.clone());
        Ok(BrowserConnection {
            peer,
            events,
            offer,
            fingerprint,
            ice_ports: Some(ice_ports),
        })
    }

    /// Server-reflexive candidates arrive a STUN round trip after the host
    /// ones. Wait (briefly) for gathering to finish and build the offer
    /// again so they ride inside it: the browser never fetches the
    /// gateway's candidates after the offer.
    async fn gather_then_reoffer(
        &self,
        peer: &mut PeerConnection,
        mut events: mpsc::Receiver<PeerEvent>,
        offer: String,
    ) -> (String, mpsc::Receiver<PeerEvent>) {
        let deadline = tokio::time::Instant::now() + self.cfg.gather_timeout;
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Some(PeerEvent::GatheringComplete)) => break,
                Ok(Some(_)) => continue,
                Ok(None) => return (offer, events),
                Err(_) => {
                    debug!(
                        connection = %peer.connection_id(),
                        "candidate gathering still running; offering what we have"
                    );
                    return (offer, events);
                }
            }
        }
        if peer.rollback_local_offer().is_err() {
            return (offer, events);
        }
        match peer.create_offer().await {
            Ok(fresh) => (fresh, events),
            Err(e) => {
                warn!(error = %e, "re-offer after gathering failed; keeping the first offer");
                (offer, events)
            }
        }
    }

    async fn start(self: &Arc<Self>, kind: SessionKind, conn: BrowserConnection) -> CreatedSession {
        let id = format!("sess-{}", uuid::Uuid::new_v4());
        let connection_id = conn.peer.lock().await.connection_id().to_string();
        let session = Session {
            id: id.clone(),
            connection_id,
            kind,
            state: SessionState::Connecting,
            created_at: Instant::now(),
            audio_codec: None,
            video: None,
            content: None,
            floor: FloorState::default(),
            srtp: false,
            sip_target: None,
            sip_call_id: None,
            session_token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            dialog: None,
            call_handle: None,
            ports: conn.ice_ports.into_iter().collect(),
            stop: Arc::new(Notify::new()),
            stats: Arc::new(RelayStats::default()),
        };
        let info = session.info();
        let session_token = session.session_token.clone();
        let session = Arc::new(RwLock::new(session));
        self.sessions.insert(id.clone(), session.clone());
        info!(session = %id, kind = ?info.kind, connection = %info.connection_id, "session created");
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            mgr.drive(session, conn.peer, conn.events).await;
        });
        CreatedSession {
            info,
            sdp_offer: conn.offer,
            dtls_fingerprint: conn.fingerprint,
            session_token,
            ice_servers: self.cfg.browser_ice_servers.clone(),
        }
    }

    // ------------------------------------------------------------ lifecycle

    /// The whole life of a session after creation.
    async fn drive(
        self: Arc<Self>,
        session: Arc<RwLock<Session>>,
        peer: Arc<Mutex<PeerConnection>>,
        events: mpsc::Receiver<PeerEvent>,
    ) {
        let (id, stop) = {
            let s = session.read().await;
            (s.id.clone(), s.stop.clone())
        };

        // 1. The browser answers and connects.
        let events = match self.wait_connected(&peer, events, &stop).await {
            Ok(events) => events,
            Err(why) => {
                self.finish(&session, &peer, Some(why)).await;
                return;
            }
        };
        if session.read().await.is_over() {
            self.finish(&session, &peer, None).await;
            return;
        }

        // 2. What the browser negotiated.
        let (
            audio_codec,
            audio_pt,
            video,
            content,
            audio_sender,
            video_sender,
            content_sender,
            browser_te,
        ) = {
            let guard = peer.lock().await;
            let (codec, pt) = guard.negotiated_codec();
            let video = guard.negotiated_video().cloned();
            // A shared screen rides only beside a camera section.
            let content = video
                .as_ref()
                .and_then(|_| guard.negotiated_content().cloned());
            let audio_sender = match guard.sender() {
                Ok(s) => s,
                Err(e) => {
                    drop(guard);
                    self.finish(&session, &peer, Some(format!("audio sender: {e}")))
                        .await;
                    return;
                }
            };
            let video_sender = video.as_ref().and_then(|_| guard.video_sender().ok());
            let content_sender = content.as_ref().and_then(|_| guard.content_sender().ok());
            let te = guard.remote_sdp().and_then(extract_telephone_event_pt);
            (
                codec,
                pt,
                video,
                content,
                audio_sender,
                video_sender,
                content_sender,
                te,
            )
        };
        if !matches!(
            audio_codec,
            AudioCodec::G722 | AudioCodec::PCMU | AudioCodec::PCMA
        ) {
            self.finish(
                &session,
                &peer,
                Some(format!(
                    "the browser negotiated {}, which the SIP side cannot carry without transcoding; offer G.722 or G.711",
                    audio_codec_name(audio_codec)
                )),
            )
            .await;
            return;
        }

        // 3. The SIP leg's ports, keys and offer.
        let audio_ports = match self.ports.allocate().await {
            Ok(p) => p,
            Err(e) => {
                self.finish(&session, &peer, Some(format!("no media ports: {e}")))
                    .await;
                return;
            }
        };
        session.write().await.ports.push(audio_ports);
        // A port pair and, under SRTP, a key per video section offered.
        let mut offered_video = None;
        let mut offered_content = None;
        if let Some(v) = video.clone() {
            match self.ports.allocate().await {
                Ok(ports) => {
                    session.write().await.ports.push(ports);
                    offered_video = Some(OfferedVideo {
                        negotiated: v,
                        ports,
                        srtp: self.cfg.srtp.then(|| SrtpOffer::generate().ok()).flatten(),
                    });
                }
                Err(e) => {
                    warn!(session = %id, error = %e, "no ports for video; joining audio-only")
                }
            }
        }
        if let (Some(_), Some(c)) = (&offered_video, content.clone()) {
            match self.ports.allocate().await {
                Ok(ports) => {
                    session.write().await.ports.push(ports);
                    offered_content = Some(OfferedVideo {
                        negotiated: c,
                        ports,
                        srtp: self.cfg.srtp.then(|| SrtpOffer::generate().ok()).flatten(),
                    });
                }
                Err(e) => {
                    warn!(session = %id, error = %e, "no ports for a shared screen; joining without one")
                }
            }
        }
        let audio_srtp = if self.cfg.srtp {
            SrtpOffer::generate().ok()
        } else {
            None
        };
        let codec_name = audio_codec_name(audio_codec);
        let sip_codecs = [SdpCodecInfo {
            payload_type: audio_pt,
            encoding_name: codec_name.to_string(),
            clock_rate: 8000,
        }];
        fn section_of(o: &OfferedVideo) -> VideoOfferSection<'_> {
            VideoOfferSection {
                rtp_port: o.ports.rtp_port,
                payload_type: o.negotiated.payload_type,
                codec: o.negotiated.codec,
                fmtp: o.negotiated.fmtp.as_deref(),
                feedback: true,
                srtp: o.srtp.as_ref().map(|s| &s.sdp),
            }
        }
        let video_section = offered_video.as_ref().map(section_of);
        let content_section = offered_content.as_ref().map(section_of);
        let offer = match construct_offer(&OfferOptions {
            server_ip: &self.cfg.public_ip,
            origin_id: &id[5..13.min(id.len())],
            session_name: "fcp-webrtc-gw",
            audio: AudioOfferSection {
                rtp_port: audio_ports.rtp_port,
                codecs: &sip_codecs,
                telephone_event: Some((101, 8000)),
                srtp: audio_srtp.as_ref().map(|s| &s.sdp),
                ptime_ms: Some(20),
            },
            video: video_section,
            content: content_section,
        }) {
            Ok(o) => o,
            Err(e) => {
                self.finish(&session, &peer, Some(format!("offer: {e}")))
                    .await;
                return;
            }
        };

        // 4. Where to, and with what headers.
        let (target, headers) = match self.target_for(&session, video.as_ref()).await {
            Ok(t) => t,
            Err(why) => {
                self.finish(&session, &peer, Some(why)).await;
                return;
            }
        };
        {
            let mut s = session.write().await;
            s.state = SessionState::Inviting;
            s.sip_target = Some(target.clone());
        }
        info!(
            session = %id,
            target = %target,
            codec = codec_name,
            video = offered_video.is_some(),
            content = offered_content.is_some(),
            "sending INVITE"
        );
        let handle = match self
            .uac
            .invite_with_headers(&target, Some(&offer), headers)
            .await
        {
            Ok(h) => Arc::new(h),
            Err(e) => {
                self.finish(&session, &peer, Some(format!("INVITE: {e}")))
                    .await;
                return;
            }
        };
        session.write().await.call_handle = Some(handle.clone());

        // 5. Ringing, then the final answer.
        while let Some(resp) = handle.await_provisional().await {
            let code = resp.start_line().code();
            debug!(session = %id, code, "provisional response");
            if code == 180 || code == 183 {
                let mut s = session.write().await;
                if s.state == SessionState::Inviting {
                    s.state = SessionState::Ringing;
                }
            }
        }
        let final_response =
            match tokio::time::timeout(self.cfg.invite_timeout, handle.await_final()).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    self.finish(&session, &peer, Some(format!("INVITE failed: {e}")))
                        .await;
                    return;
                }
                Err(_) => {
                    let _ = handle.cancel().await;
                    self.finish(&session, &peer, Some("no answer from the far end".into()))
                        .await;
                    return;
                }
            };
        let code = final_response.start_line().code();
        if !(200..300).contains(&code) {
            let why = if session.read().await.is_over() {
                None
            } else {
                Some(format!("{code} {}", final_response.start_line().reason()))
            };
            self.finish(&session, &peer, why).await;
            return;
        }
        let dialog = handle.dialog.read().await.clone();
        let sip_call_id = dialog.id().call_id().to_string();
        let answer = String::from_utf8_lossy(final_response.body()).to_string();

        // 6. The SIP leg from the answer.
        let plan = match self
            .open_sip_leg(
                &answer,
                codec_name,
                audio_ports,
                audio_srtp,
                offered_video,
                offered_content,
            )
            .await
        {
            Ok(p) => p,
            Err(why) => {
                if let Err(e) = self.uac.bye(&dialog).await {
                    debug!(session = %id, error = %e, "BYE after a bad answer failed");
                }
                self.finish(&session, &peer, Some(why)).await;
                return;
            }
        };
        let stats = {
            let mut s = session.write().await;
            if s.is_over() {
                // Hung up while the INVITE was out.
                drop(s);
                let _ = self.uac.bye(&dialog).await;
                self.finish(&session, &peer, None).await;
                return;
            }
            s.state = SessionState::Connected;
            s.dialog = Some(dialog.clone());
            s.sip_call_id = Some(sip_call_id.clone());
            s.audio_codec = Some(plan.audio_codec.clone());
            s.srtp = plan.srtp;
            s.video = plan.video.clone();
            s.content = plan.content.clone();
            s.stats.clone()
        };
        self.by_sip_call_id.insert(sip_call_id.clone(), id.clone());
        info!(
            session = %id,
            codec = %plan.audio_codec,
            srtp = plan.srtp,
            video = ?plan.video.as_ref().map(|v| &v.codec),
            content = plan.content.is_some(),
            "connected; relaying media"
        );

        // 7. Relay until someone hangs up.
        let browser = BrowserLeg {
            events,
            audio: audio_sender,
            audio_payload_type: audio_pt,
            telephone_event_pt: browser_te,
            video: match (video_sender, &plan.video, &video) {
                (Some(sender), Some(_), Some(v)) => Some((sender, v.payload_type)),
                _ => None,
            },
            content: match (content_sender, &plan.content, &content) {
                (Some(sender), Some(_), Some(c)) => Some((sender, c.payload_type)),
                _ => None,
            },
            inactivity_timeout: self.cfg.inactivity_timeout,
        };
        let end = run_relay(browser, plan.leg, stats, stop).await;
        log_relay_end(&id, &end);
        self.by_sip_call_id.remove(&sip_call_id);
        let hung_up_here = session.read().await.is_over();
        if !hung_up_here && end != RelayEnd::Stopped {
            // The browser went away or a socket died: tell the far end.
            if let Err(e) = self.uac.bye(&dialog).await {
                debug!(session = %id, error = %e, "BYE failed");
            }
        }
        let why = match end {
            RelayEnd::BrowserFailed(w) => Some(format!("browser transport failed: {w}")),
            RelayEnd::SipSocketError(w) => Some(format!("media socket failed: {w}")),
            RelayEnd::Stopped | RelayEnd::BrowserClosed => None,
        };
        self.finish(&session, &peer, why).await;
    }

    /// Wait for the browser to finish ICE and DTLS, or for a hangup.
    async fn wait_connected(
        &self,
        peer: &Arc<Mutex<PeerConnection>>,
        mut events: mpsc::Receiver<PeerEvent>,
        stop: &Arc<Notify>,
    ) -> Result<mpsc::Receiver<PeerEvent>, String> {
        if peer.lock().await.get_state() == ConnectionState::Connected {
            return Ok(events);
        }
        let deadline = tokio::time::Instant::now() + self.cfg.ice_timeout;
        loop {
            tokio::select! {
                _ = stop.notified() => return Ok(events),
                ev = tokio::time::timeout_at(deadline, events.recv()) => match ev {
                    Ok(Some(PeerEvent::Connected)) => return Ok(events),
                    Ok(Some(PeerEvent::Failed(why))) => return Err(format!("browser transport failed: {why}")),
                    Ok(Some(PeerEvent::Closed)) => return Err("browser connection closed".into()),
                    Ok(Some(_)) => continue,
                    Ok(None) => return Err("browser connection went away".into()),
                    Err(_) => return Err(format!(
                        "the browser did not connect within {} s",
                        self.cfg.ice_timeout.as_secs()
                    )),
                }
            }
        }
    }

    /// The INVITE target and headers for a session.
    async fn target_for(
        &self,
        session: &Arc<RwLock<Session>>,
        video: Option<&NegotiatedVideo>,
    ) -> Result<(String, Headers), String> {
        let kind = session.read().await.kind.clone();
        let mut headers = Headers::new();
        match kind {
            SessionKind::Call { destination } => Ok((destination, headers)),
            SessionKind::Room {
                room_number,
                display_name,
                join_token,
                ..
            } => {
                let codecs: Vec<String> = video
                    .map(|v| vec![v.codec.sdp_name().to_string()])
                    .unwrap_or_default();
                let host = self
                    .placement
                    .host_for(&room_number, &codecs)
                    .await
                    .ok_or_else(|| "no conference node available".to_string())?;
                headers
                    .push(
                        "P-Asserted-Identity",
                        format!("\"{display_name}\" <{}>", self.cfg.local_uri),
                    )
                    .map_err(|e| format!("header: {e}"))?;
                if let Some(token) = join_token {
                    headers
                        .push(JOIN_TOKEN_HEADER, token)
                        .map_err(|e| format!("header: {e}"))?;
                }
                Ok((format!("sip:{room_number}@{host}"), headers))
            }
        }
    }

    /// The SIP leg from the far end's answer: sockets pointed at it, SRTP
    /// when it took our keys, and the payload types it mirrored.
    async fn open_sip_leg(
        &self,
        answer: &str,
        codec_name: &str,
        audio_ports: PortPair,
        audio_srtp: Option<SrtpOffer>,
        video: Option<OfferedVideo>,
        content: Option<OfferedVideo>,
    ) -> Result<SipLegPlan, String> {
        let codecs = parse_sdp_codecs(answer);
        let audio_pt = codecs
            .iter()
            .find(|c| c.encoding_name.eq_ignore_ascii_case(codec_name))
            .map(|c| c.payload_type)
            .ok_or_else(|| format!("the answer did not keep {codec_name}"))?;
        let te_pt = extract_telephone_event_pt(answer);
        let remote_audio = remote_audio_endpoint(answer)
            .ok_or_else(|| "the answer has no audio address".to_string())?;
        let (audio_ctx, srtp) = match (audio_srtp, parse_sdes_crypto(answer).into_iter().next()) {
            (Some(ours), Some(theirs)) => (
                Some(
                    ours.accept(&theirs)
                        .map_err(|e| format!("audio SRTP: {e}"))?,
                ),
                true,
            ),
            (Some(_), None) => {
                debug!("the far end answered audio without a=crypto; plain RTP");
                (None, false)
            }
            (None, _) => (None, false),
        };
        let audio_socket = RtpSocketPair::new(audio_ports, RtpSocketConfig::default())
            .await
            .map_err(|e| format!("audio socket: {e}"))?;
        point_at(&audio_socket, remote_audio).await;

        // Each video section on its own socket and key, as answered.
        let sections = parse_video_sections(answer);
        let (video_leg, video_summary) = match video {
            Some(offered) => {
                let crypto = parse_video_sdes_crypto(answer);
                open_video_section("video", offered, sections.camera.as_ref(), crypto)
                    .await?
                    .map_or((None, None), |(leg, summary)| (Some(leg), Some(summary)))
            }
            None => (None, None),
        };
        let (content_leg, content_summary) = match content {
            Some(offered) if video_leg.is_some() => {
                let crypto = parse_content_sdes_crypto(answer);
                open_video_section("content", offered, sections.content.as_ref(), crypto)
                    .await?
                    .map_or((None, None), |(leg, summary)| (Some(leg), Some(summary)))
            }
            _ => (None, None),
        };

        Ok(SipLegPlan {
            leg: SipLeg {
                audio: SipAudioLeg {
                    socket: Arc::new(audio_socket),
                    srtp: audio_ctx.map(|c| Arc::new(Mutex::new(c))),
                    payload_type: audio_pt,
                    telephone_event_pt: te_pt,
                },
                video: video_leg,
                content: content_leg,
            },
            audio_codec: codec_name.to_string(),
            srtp,
            video: video_summary,
            content: content_summary,
        })
    }

    /// Close the browser side, release ports, record the outcome, and
    /// forget the session after a while.
    async fn finish(
        &self,
        session: &Arc<RwLock<Session>>,
        peer: &Arc<Mutex<PeerConnection>>,
        failure: Option<String>,
    ) {
        let (id, connection_id, ports, call_id) = {
            let mut s = session.write().await;
            match failure {
                Some(why) if !s.is_over() => {
                    warn!(session = %s.id, %why, "session failed");
                    s.state = SessionState::Failed(why);
                }
                _ if !s.is_over() => s.state = SessionState::Terminated,
                _ => {}
            }
            s.dialog = None;
            s.call_handle = None;
            (
                s.id.clone(),
                s.connection_id.clone(),
                std::mem::take(&mut s.ports),
                s.sip_call_id.take(),
            )
        };
        if let Some(cid) = call_id {
            self.by_sip_call_id.remove(&cid);
        }
        self.webrtc.remove_connection(&connection_id);
        peer.lock().await.close();
        for pair in ports {
            self.ports.deallocate(pair).await;
        }
        info!(session = %id, "session over");
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            tokio::time::sleep(LINGER).await;
            sessions.remove(&id);
        });
    }

    // ------------------------------------------------------------ signalling

    /// The browser's answer to this session's offer.
    pub async fn answer(&self, id: &str, sdp_answer: &str) -> Result<(), SessionError> {
        let peer = self.peer_of(id).await?;
        let mut guard = peer.lock().await;
        guard
            .set_remote_answer(sdp_answer)
            .await
            .map_err(|e| SessionError::BadRequest(format!("answer: {e}")))
    }

    /// A trickled candidate from the browser.
    pub async fn add_ice_candidate(&self, id: &str, candidate: &str) -> Result<(), SessionError> {
        let peer = self.peer_of(id).await?;
        let mut guard = peer.lock().await;
        guard
            .add_ice_candidate_str(candidate)
            .await
            .map_err(|e| SessionError::BadRequest(format!("candidate: {e}")))
    }

    async fn peer_of(&self, id: &str) -> Result<Arc<Mutex<PeerConnection>>, SessionError> {
        let session = self
            .sessions
            .get(id)
            .map(|e| e.value().clone())
            .ok_or_else(|| SessionError::NotFound(format!("session '{id}' not found")))?;
        let connection_id = {
            let s = session.read().await;
            if s.is_over() {
                return Err(SessionError::Conflict(format!("session '{id}' is over")));
            }
            s.connection_id.clone()
        };
        self.webrtc
            .get_connection(&connection_id)
            .ok_or_else(|| SessionError::Conflict(format!("session '{id}' has no peer connection")))
    }

    /// Whether `token` is the session token of `id`.
    pub async fn session_token_matches(&self, id: &str, token: &str) -> bool {
        let Some(session) = self.sessions.get(id).map(|e| e.value().clone()) else {
            return false;
        };
        let s = session.read().await;
        constant_time_eq(s.session_token.as_bytes(), token.as_bytes())
    }

    // ------------------------------------------------------------ control

    /// End a session from the API: BYE (or CANCEL) to the far end, close the
    /// browser side.
    pub async fn hangup(&self, id: &str) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get(id)
            .map(|e| e.value().clone())
            .ok_or_else(|| SessionError::NotFound(format!("session '{id}' not found")))?;
        let (dialog, handle, stop, state) = {
            let mut s = session.write().await;
            if s.is_over() {
                return Err(SessionError::Conflict(format!(
                    "session '{id}' is already {}",
                    match &s.state {
                        SessionState::Failed(_) => "failed",
                        _ => "over",
                    }
                )));
            }
            let state = s.state.clone();
            s.state = SessionState::Terminated;
            (s.dialog.take(), s.call_handle.take(), s.stop.clone(), state)
        };
        info!(session = %id, from = ?state, "hangup");
        match state {
            SessionState::Connected => {
                if let Some(dialog) = dialog {
                    if let Err(e) = self.uac.bye(&dialog).await {
                        warn!(session = %id, error = %e, "BYE failed");
                    }
                }
            }
            SessionState::Inviting | SessionState::Ringing => {
                if let Some(handle) = handle {
                    let _ = handle.cancel().await;
                }
            }
            _ => {}
        }
        stop.notify_one();
        Ok(())
    }

    /// The far end sent BYE for `sip_call_id`. Returns whether it was ours.
    pub async fn remote_bye(&self, sip_call_id: &str) -> bool {
        let Some(id) = self
            .by_sip_call_id
            .get(sip_call_id)
            .map(|e| e.value().clone())
        else {
            return false;
        };
        let Some(session) = self.sessions.get(&id).map(|e| e.value().clone()) else {
            return false;
        };
        let stop = {
            let mut s = session.write().await;
            if !s.is_over() {
                s.state = SessionState::Terminated;
            }
            s.dialog = None;
            s.stop.clone()
        };
        info!(session = %id, "far end hung up");
        stop.notify_one();
        true
    }

    /// The conference server sent a floor notice for `sip_call_id`.
    /// Returns whether it was ours.
    pub async fn content_notice(&self, sip_call_id: &str, notice: &ContentNotice) -> bool {
        let Some(id) = self
            .by_sip_call_id
            .get(sip_call_id)
            .map(|e| e.value().clone())
        else {
            return false;
        };
        let Some(session) = self.sessions.get(&id).map(|e| e.value().clone()) else {
            return false;
        };
        let mut s = session.write().await;
        s.floor.apply(notice);
        info!(session = %id, floor = %s.floor.state, presenter = ?s.floor.presenter, reason = ?s.floor.reason, "floor notice");
        true
    }

    // ------------------------------------------------------------ queries

    /// Every session of one kind.
    pub async fn list(&self, rooms: bool) -> Vec<SessionInfo> {
        let mut out = Vec::new();
        for entry in self.sessions.iter() {
            let s = entry.value().read().await;
            if s.kind.is_room() == rooms {
                out.push(s.info());
            }
        }
        out.sort_by(|a, b| {
            b.duration_secs
                .partial_cmp(&a.duration_secs)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// One session of one kind.
    pub async fn get(&self, id: &str, rooms: bool) -> Option<SessionInfo> {
        let session = self.sessions.get(id)?.value().clone();
        let s = session.read().await;
        (s.kind.is_room() == rooms).then(|| s.info())
    }

    /// Whether a session of one kind exists.
    pub async fn exists(&self, id: &str, rooms: bool) -> bool {
        self.get(id, rooms).await.is_some()
    }

    /// Sessions currently relaying media.
    pub async fn connected_count(&self) -> usize {
        let mut n = 0;
        for entry in self.sessions.iter() {
            if entry.value().read().await.state == SessionState::Connected {
                n += 1;
            }
        }
        n
    }
}

/// One video section of the SIP leg from what the far end answered for
/// it: `None` when it rejected the section or answered it unusably (the
/// codec dropped, no address), so the join goes on without it.
async fn open_video_section(
    what: &str,
    offered: OfferedVideo,
    answered: Option<&VideoOffer>,
    crypto: Vec<SdesCryptoLine>,
) -> Result<Option<(SipVideoLeg, VideoSummary)>, String> {
    let Some(section) = answered.filter(|s| s.port != 0) else {
        debug!("the far end rejected {what}; joining without it");
        return Ok(None);
    };
    let pt = section
        .codecs
        .iter()
        .find(|c| c.codec == offered.negotiated.codec)
        .map(|c| c.payload_type);
    let (Some(pt), Some(remote)) = (pt, section.remote) else {
        debug!("the answer's {what} section has no usable codec or address; joining without it");
        return Ok(None);
    };
    let (ctx, srtp) = match (offered.srtp, crypto.into_iter().next()) {
        (Some(ours), Some(theirs)) => match ours.accept(&theirs) {
            Ok(ctx) => (Some(ctx), true),
            Err(e) => {
                warn!(error = %e, "{what} SRTP; leaving it out");
                return Ok(None);
            }
        },
        _ => (None, false),
    };
    let socket = RtpSocketPair::new(
        offered.ports,
        RtpSocketConfig {
            tos: 0xA0,
            ..RtpSocketConfig::default()
        },
    )
    .await
    .map_err(|e| format!("{what} socket: {e}"))?;
    point_at(&socket, remote).await;
    Ok(Some((
        SipVideoLeg {
            socket: Arc::new(socket),
            srtp: ctx.map(|c| Arc::new(Mutex::new(c))),
            payload_type: pt,
        },
        VideoSummary {
            codec: offered.negotiated.codec.sdp_name().to_string(),
            payload_type: pt,
            srtp,
        },
    )))
}

/// Equal without leaking where they differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Point a socket pair at the far end and accept media only from there.
async fn point_at(socket: &RtpSocketPair, remote: std::net::SocketAddr) {
    socket
        .set_remote_endpoint(RemoteEndpoint {
            rtp_addr: remote,
            rtcp_addr: Some(std::net::SocketAddr::new(
                remote.ip(),
                remote.port().wrapping_add(1),
            )),
        })
        .await;
    let mut allowed = HashSet::new();
    allowed.insert(remote.ip());
    socket
        .set_latch_policy(LatchPolicy::AllowedIps(allowed))
        .await;
}

#[allow(dead_code)]
fn _srtp_context_is_send(_: SrtpContext) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_numbers_follow_the_conference_servers_rules() {
        assert!(valid_room_number("8001"));
        assert!(valid_room_number("team-room_1.2+3*#"));
        assert!(!valid_room_number(""));
        assert!(!valid_room_number("room 1"));
        assert!(!valid_room_number(&"9".repeat(65)));
    }

    #[test]
    fn destinations_get_a_scheme() {
        assert_eq!(
            normalize_destination("sip:alice@example.com"),
            "sip:alice@example.com"
        );
        assert_eq!(
            normalize_destination(" 1001@10.0.0.7 "),
            "sip:1001@10.0.0.7"
        );
        assert_eq!(
            normalize_destination("tel:+15551234567"),
            "tel:+15551234567"
        );
    }

    #[test]
    fn session_kind_serializes_flat_without_the_token() {
        let kind = SessionKind::Room {
            room_number: "8001".into(),
            display_name: "Ann".into(),
            video: true,
            content: true,
            join_token: Some("secret".into()),
        };
        let json = serde_json::to_string(&kind).unwrap();
        assert!(json.contains("\"kind\":\"room\""));
        assert!(json.contains("\"room_number\":\"8001\""));
        assert!(!json.contains("secret"));
    }

    #[test]
    fn a_join_with_video_offers_a_screen_unless_told_not_to() {
        let req: JoinRequest = serde_json::from_str(r#"{"video": true}"#).unwrap();
        assert_eq!(req.content, None);
        assert!(req.video && req.content.unwrap_or(true));
        let req: JoinRequest =
            serde_json::from_str(r#"{"video": true, "content": false}"#).unwrap();
        assert!(!(req.video && req.content.unwrap_or(true)));
        let req: JoinRequest = serde_json::from_str(r#"{"content": true}"#).unwrap();
        assert!(
            !(req.video && req.content.unwrap_or(true)),
            "no screen without a camera section"
        );
    }

    #[test]
    fn floor_notices_move_the_floor_state() {
        let notice = |json: &str| serde_json::from_str::<ContentNotice>(json).unwrap();
        let mut floor = FloorState::default();
        assert_eq!(floor.state, "free");
        floor.apply(&notice(
            r#"{"floor":"granted","mine":false,"presenter":"Ann"}"#,
        ));
        assert_eq!(
            (floor.state.as_str(), floor.presenter.as_deref()),
            ("held", Some("Ann"))
        );
        floor.apply(&notice(
            r#"{"floor":"refused","mine":true,"presenter":"Ann","reason":"held"}"#,
        ));
        assert_eq!(floor.state, "refused");
        assert_eq!(floor.reason.as_deref(), Some("held"));
        floor.apply(&notice(
            r#"{"floor":"released","mine":false,"reason":"ended"}"#,
        ));
        assert_eq!(
            floor,
            FloorState {
                notices: 3,
                ..FloorState::default()
            }
        );
        floor.apply(&notice(
            r#"{"floor":"granted","mine":true,"presenter":"Bob"}"#,
        ));
        assert_eq!(floor.state, "granted");
        floor.apply(&notice(
            r#"{"floor":"released","mine":true,"reason":"host"}"#,
        ));
        assert_eq!(
            (floor.state.as_str(), floor.reason.as_deref()),
            ("released", Some("host"))
        );
        // Something newer than this gateway knows is ignored, not fatal.
        floor.apply(&notice(r#"{"floor":"queued","mine":true}"#));
        assert_eq!(floor.state, "released");
        assert_eq!(floor.notices, 5);
    }
}
