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

use crate::forwarding::{Forwarding, ForwardingSummary};
use crate::placement::ConferencePlacement;
use crate::relay::{
    log_relay_end, run_relay, BrowserLeg, RelayEnd, RelayStats, RelayStatsSnapshot, SipAudioLeg,
    SipLeg, SipVideoLeg,
};
use dashmap::DashMap;
use fcp_core::MIGRATION_TICKET_HEADER;
use fcp_media::{
    construct_offer, extract_telephone_event_pt, parse_content_sdes_crypto, parse_sdes_crypto,
    parse_sdp_codecs, parse_video_sdes_crypto, parse_video_sections, remote_audio_endpoint,
    AudioOfferSection, OfferOptions, SdesCryptoLine, SdpCodecInfo, SrtpOffer, VideoOffer,
    VideoOfferSection,
};
use fcp_sip::conference_migration::{report_refer_outcome, MigrationRefer};
use fcp_sip::uac::SipUserAgentClient;
use forge_api::routes::webrtc::WebRtcManager;
use forge_core::AudioCodec;
use forge_rtp::{
    LatchPolicy, PortPair, PortPool, RemoteEndpoint, RtpSocketConfig, RtpSocketPair, SrtpContext,
};
use forge_webrtc::TurnServer;
use forge_webrtc::{
    ConnectionState, Direction, NegotiatedVideo, PeerConfig, PeerConnection, PeerEvent,
    TransportConfig, VideoConfig, VideoSender,
};
use serde::{Deserialize, Serialize};
use sip_core::{Headers, SipUri};
use sip_uac::integrated::CallHandle;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
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
    /// The call manager calls go through (`[sip] outbound_proxy`).
    pub outbound_proxy: Option<String>,
    /// Which destinations are emergency numbers (`[e911]`).
    pub emergency: fcp_core::e911::EmergencyConfig,
}

/// The tenant user a call is placed for, named by the operator that asks
/// for it; the call manager believes it from this gateway when the gateway
/// is one of its trusted edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerIdentity {
    /// `tenant-<uuid>` or the bare UUID.
    #[serde(deserialize_with = "tenant_id_either_form")]
    pub tenant_id: fcp_core::TenantId,
    pub username: String,
}

fn tenant_id_either_form<'de, D>(deserializer: D) -> Result<fcp_core::TenantId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    raw.parse().map_err(serde::de::Error::custom)
}

/// What a session is for.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionKind {
    /// A call to a SIP destination.
    Call {
        destination: String,
        /// Who the call is placed for, when the operator named them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller: Option<CallerIdentity>,
        /// Where the browser is, which the call manager uses to place the
        /// caller by network (`docs/E911_BLOCK_2.md` §6.2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_address: Option<std::net::IpAddr>,
        /// Set when the destination is an emergency number.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emergency: Option<fcp_core::e911::EmergencyKind>,
    },
    /// A join of a conference room.
    Room {
        room_number: String,
        display_name: String,
        /// Whether the browser was offered video.
        video: bool,
        /// Whether the browser was offered a second video section for a
        /// shared screen (§15.6).
        content: bool,
        /// How many tracks the browser was offered for a forwarding room's
        /// streams; 0, the room's one picture.
        #[serde(default, skip_serializing_if = "is_zero")]
        streams: u8,
        #[serde(skip)]
        join_token: Option<String>,
    },
}

fn is_zero(n: &u8) -> bool {
    *n == 0
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

/// The audience's voice as this session knows it (`docs/WEBINARS.md`
/// §2.5): its hand, whether a host has brought it to the stage, and the
/// questions it may see, from what the browser asked and the room's
/// `fcp-qa` INFOs.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct QaState {
    pub hand: bool,
    pub on_stage: bool,
    /// Whether the join token makes this participant a host or panelist,
    /// who moderates questions. The room checks for itself; this is for
    /// the page.
    pub moderates: bool,
    /// The room's questions as this participant sees them, in the order
    /// asked; a dismissed one is dropped.
    pub questions: Vec<fcp_core::qa::QuestionView>,
}

/// The most questions a session keeps: a room's most.
const MAX_SESSION_QUESTIONS: usize = 2000;

impl QaState {
    /// Take in a message from the room.
    pub fn apply(&mut self, message: &fcp_core::qa::QaMessage) {
        use fcp_core::qa::{QaMessage, QuestionStatus};
        match message {
            QaMessage::Hand { raised } => self.hand = *raised,
            QaMessage::Stage { on } => {
                self.on_stage = *on;
                if *on {
                    self.hand = false;
                }
            }
            QaMessage::Questions { questions } => {
                for q in questions {
                    let at = self.questions.iter().position(|h| h.id == q.id);
                    match (at, q.status == QuestionStatus::Dismissed) {
                        (Some(i), true) => {
                            self.questions.remove(i);
                        }
                        (Some(i), false) => self.questions[i] = q.clone(),
                        (None, false) if self.questions.len() < MAX_SESSION_QUESTIONS => {
                            self.questions.push(q.clone())
                        }
                        _ => {}
                    }
                }
            }
            // A participant's own requests; the room does not send them.
            QaMessage::Ask { .. }
            | QaMessage::Upvote { .. }
            | QaMessage::Publish { .. }
            | QaMessage::Answer { .. }
            | QaMessage::Dismiss { .. } => {}
        }
    }
}

/// A room's chat as this session knows it (`docs/WEBINARS.md` §2.5): the
/// messages it may see, oldest first, from the room's `fcp-chat` INFOs,
/// and whether its own chat is muted.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ChatState {
    pub muted: bool,
    /// The last [`MAX_SESSION_MESSAGES`]; a deleted one is dropped.
    pub messages: Vec<fcp_core::chat::ChatView>,
}

/// The most messages a session keeps.
const MAX_SESSION_MESSAGES: usize = 500;

impl ChatState {
    /// Take in a message from the room.
    pub fn apply(&mut self, message: &fcp_core::chat::ChatMessage) {
        use fcp_core::chat::ChatMessage;
        match message {
            ChatMessage::Muted { on } => self.muted = *on,
            ChatMessage::Messages { messages } => {
                for m in messages {
                    let at = self.messages.iter().position(|h| h.id == m.id);
                    match (at, m.deleted) {
                        (Some(i), true) => {
                            self.messages.remove(i);
                        }
                        (Some(i), false) => self.messages[i] = m.clone(),
                        (None, false) => self.messages.push(m.clone()),
                        (None, true) => {}
                    }
                }
                let over = self.messages.len().saturating_sub(MAX_SESSION_MESSAGES);
                self.messages.drain(..over);
            }
            // A participant's own requests; the room does not send them.
            ChatMessage::Post { .. } | ChatMessage::Delete { .. } | ChatMessage::Mute { .. } => {}
        }
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

/// The simulcast streams asked of a browser's camera, largest first; the
/// join page scales them 1, 2 and 4 times down.
pub const BROWSER_SIMULCAST: [&str; 3] = ["h", "m", "l"];

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
    /// Ask a room that forwards for each person's own stream, on this many
    /// receive-only tracks the page lays out itself
    /// (`docs/FORWARDING_ROOMS.md` §2.7); none, the room's one picture.
    /// At most [`forge_webrtc::MAX_STREAMS`]; needs `video`.
    #[serde(default)]
    pub streams: Option<u8>,
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
    /// Set while this node is leaving service: the browser should join
    /// again — here, if a serving gateway is named
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handover: Option<Handover>,
    /// The room forwards each person's stream to this browser: who is in
    /// it, and whose stream is on each of the browser's tracks (by mid).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarding: Option<ForwardingSummary>,
    /// In a room: the browser's hand and whether it is on the stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qa: Option<QaState>,
    /// In a room: its chat as this browser sees it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat: Option<ChatState>,
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
    pub qa: QaState,
    pub chat: ChatState,
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
    /// What the browser negotiated, kept so the SIP side can be offered
    /// the same shape again to another node (phase 5c).
    shape: Option<SipShape>,
    /// The current SIP leg's port pairs, a subset of `ports`.
    sip_ports: Vec<PortPair>,
    /// Hands the running relay a new SIP leg.
    swap_tx: Option<mpsc::Sender<SipLeg>>,
    /// A move to another node is in flight.
    migrating: bool,
    /// The session's record: its call id (sent to the conference node, so
    /// the room's record of the leg shares it), when it began and connected,
    /// and who ended it (`docs/CDR_PIPELINE.md` §5).
    record_call_id: fcp_core::CallId,
    started_at: SystemTime,
    connected_at: Option<SystemTime>,
    ended_by: Option<fcp_core::storage::HangupParty>,
    last_sip_call_id: Option<String>,
    /// The room took `Recv-Info: fcp-view`: it forwards each person's
    /// stream, mapped onto the browser's tracks here.
    forwarding: Option<Arc<std::sync::Mutex<Forwarding>>>,
    /// Woken when the roster missed a document: subscribe again for the
    /// whole of it.
    roster_gap: Arc<Notify>,
}

/// The browser's side of the negotiation, as the SIP offer mirrors it.
#[derive(Clone)]
struct SipShape {
    audio_codec: AudioCodec,
    audio_pt: u8,
    video: Option<NegotiatedVideo>,
    content: Option<NegotiatedVideo>,
    /// The browser agreed transport-cc: the SIP side offers it too, so the
    /// gateway can translate between them.
    transport_cc: bool,
}

/// A SIP offer built for a shape: the SDP and what it was built from.
struct SipOffer {
    offer: String,
    codec_name: &'static str,
    audio_ports: PortPair,
    audio_srtp: Option<SrtpOffer>,
    video: Option<OfferedVideo>,
    content: Option<OfferedVideo>,
}

impl SipOffer {
    /// Every port pair the offer took from the pool.
    fn ports(&self) -> Vec<PortPair> {
        std::iter::once(self.audio_ports)
            .chain(self.video.as_ref().map(|v| v.ports))
            .chain(self.content.as_ref().map(|c| c.ports))
            .collect()
    }
}

/// How a REFER asking for a move was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStart {
    /// The move is under way; 202.
    Accepted,
    /// No session on that Call-ID.
    Unknown,
    /// A move is already in flight.
    Busy,
    /// The session is not relaying yet, or is over.
    NotReady,
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
            // Filled in by the manager, which knows whether this node is
            // leaving service and where a browser should go instead.
            handover: None,
            forwarding: self
                .forwarding
                .as_ref()
                .map(|f| f.lock().unwrap_or_else(|e| e.into_inner()).summary()),
            qa: self.kind.is_room().then(|| self.qa.clone()),
            chat: self.kind.is_room().then(|| self.chat.clone()),
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

/// A new session's id. Made before the session so that this node's place
/// for it can be taken — or refused — before any work is done for it
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
fn new_session_id() -> String {
    format!("sess-{}", uuid::Uuid::new_v4())
}

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
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Unavailable(String),
    /// This node will not take the session: it is leaving service, or it
    /// is at a limit (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). `503`
    /// with `Retry-After`, the answer every FCP node gives.
    #[error("{detail}")]
    Refused {
        detail: String,
        retry_after_secs: u64,
    },
}

/// Room numbers as the conference server accepts them.
pub fn valid_room_number(room: &str) -> bool {
    !room.is_empty()
        && room.len() <= 64
        && room
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+*#".contains(c))
}

/// Where a call to `destination` goes: a URI as given, `user@host` as a SIP
/// URI, and a bare number through the call manager when there is one.
pub fn route_destination(destination: &str, outbound_proxy: Option<&str>) -> String {
    let d = destination.trim();
    match outbound_proxy {
        Some(proxy)
            if !d.contains('@')
                && !d.starts_with("sip:")
                && !d.starts_with("sips:")
                && !d.starts_with("tel:") =>
        {
            format!("sip:{d}@{proxy}")
        }
        _ => normalize_destination(d),
    }
}

/// A username fit for a SIP header value.
fn valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= 128
        && username
            .chars()
            .all(|c| c.is_ascii_graphic() && !"<>\"\\;,".contains(c))
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
    cdr: Option<CdrSettings>,
    /// What this node accepts, and whether it is leaving service
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5, §6).
    admission: Option<Arc<dyn fcp_sip::admission::NodeAdmission>>,
    /// Where a browser should go while this node drains.
    handover: Option<Arc<dyn HandoverAdvice>>,
}

/// Where a browser on a draining node is told to go
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). The gateway's own
/// implementation reads the cluster; a node without one gives `None` for
/// the URL, which tells the browser to come back where it is.
pub trait HandoverAdvice: Send + Sync {
    /// The browser-facing URL of a serving peer, or `None` when there is
    /// no peer to name.
    fn peer_url(&self) -> Option<String>;
}

/// What a browser is told while this node is leaving service: come back,
/// here if a peer was found, where you are otherwise.
#[derive(Debug, Clone, Serialize)]
pub struct Handover {
    /// Why the browser is being moved. One value today: `node_draining`.
    pub reason: &'static str,
    /// The gateway to join instead, when a serving one is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Where the gateway's session records go and what they name
/// (`docs/CDR_PIPELINE.md` §5).
#[derive(Clone)]
pub struct CdrSettings {
    pub writer: fcp_sip::CdrWriter,
    pub node_id: String,
    pub region: Option<String>,
    /// The tenant for a session whose join token names none (`[conference]
    /// tenant_id`).
    pub default_tenant: Option<fcp_core::TenantId>,
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
            cdr: None,
            admission: None,
            handover: None,
        }
    }

    /// The limits this node keeps, and its drain state.
    pub fn with_admission(mut self, admission: Arc<dyn fcp_sip::admission::NodeAdmission>) -> Self {
        self.admission = Some(admission);
        self
    }

    /// Where browsers go when this node is leaving service.
    pub fn with_handover(mut self, handover: Arc<dyn HandoverAdvice>) -> Self {
        self.handover = Some(handover);
        self
    }

    /// Whether this node is leaving service.
    pub fn is_draining(&self) -> bool {
        self.admission.as_ref().is_some_and(|a| a.draining())
    }

    /// Sessions this node is still carrying, of either kind: a browser
    /// connected or being connected. A session that is over lingers in
    /// the list for a minute so a browser can read why it ended; a drain
    /// must not wait for those (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    pub async fn live_count(&self) -> usize {
        let mut n = 0;
        for entry in self.sessions.iter() {
            if !entry.value().read().await.is_over() {
                n += 1;
            }
        }
        n
    }

    /// Take this node's place for a session that is starting, or say why
    /// not (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). An emergency call is
    /// never refused, on a draining node least of all.
    async fn admit(&self, id: &str, emergency: bool) -> Result<(), SessionError> {
        let Some(admission) = &self.admission else {
            return Ok(());
        };
        if emergency {
            return Ok(());
        }
        let tenant = self
            .cdr
            .as_ref()
            .and_then(|c| c.default_tenant)
            .unwrap_or_else(fcp_core::TenantId::nil);
        admission.admit(id, &tenant).await.map_err(|refusal| {
            warn!(session = %id, refusal = refusal.as_str(), "session refused");
            SessionError::Refused {
                detail: refusal.detail().to_string(),
                retry_after_secs: admission.retry_after_secs(),
            }
        })
    }

    /// Give back the place a session held, however it ended.
    fn release(&self, id: &str) {
        if let Some(admission) = &self.admission {
            admission.ended(id);
        }
    }

    /// What a browser in a room is told while this node is draining: move,
    /// and where to. A call to a SIP phone is not told anything — there is
    /// no second node that holds the far end, so it is waited for instead
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    fn handover_for(&self, kind: &SessionKind) -> Option<Handover> {
        if !kind.is_room() || !self.is_draining() {
            return None;
        }
        Some(Handover {
            reason: "node_draining",
            url: self.handover.as_ref().and_then(|h| h.peer_url()),
        })
    }

    /// Where session records go, when they go anywhere.
    pub fn cdr_writer(&self) -> Option<&fcp_sip::CdrWriter> {
        self.cdr.as_ref().map(|c| &c.writer)
    }

    /// Write a record of every session that ends.
    pub fn with_cdr(mut self, settings: CdrSettings) -> Self {
        self.cdr = Some(settings);
        self
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
        let streams = if req.video {
            req.streams.unwrap_or(0).min(forge_webrtc::MAX_STREAMS)
        } else {
            0
        };
        let kind = SessionKind::Room {
            room_number: room_number.to_string(),
            display_name,
            video: req.video,
            content,
            streams,
            join_token: req.join_token.filter(|t| !t.is_empty()),
        };
        // What this node accepts, before it opens a peer connection and
        // takes ICE ports for it (§5). A draining node refuses here, and
        // the browser is placed elsewhere.
        let id = new_session_id();
        self.admit(&id, false).await?;
        let conn = match self
            .new_browser_connection(req.video, content, streams)
            .await
        {
            Ok(conn) => conn,
            Err(e) => {
                self.release(&id);
                return Err(e);
            }
        };
        Ok(self.start(id, kind, conn).await)
    }

    /// Call a SIP destination. With `connection_id` the browser's existing
    /// connection (from `POST /v1/webrtc/connections`) is used, and its
    /// negotiated codec must be one the SIP side carries; without it a new
    /// connection is created and its offer returned.
    ///
    /// An emergency number is placed only for a named caller and only
    /// through the call manager, which locates the caller and routes the
    /// call; otherwise it is refused with the reason, never sent unlocated
    /// (`docs/E911_BLOCK_2.md` §4).
    pub async fn call(
        self: &Arc<Self>,
        destination: &str,
        connection_id: Option<&str>,
        caller: Option<CallerIdentity>,
        client_address: std::net::IpAddr,
    ) -> Result<CreatedSession, SessionError> {
        if destination.trim().is_empty() {
            return Err(SessionError::BadRequest("destination is required".into()));
        }
        if let Some(caller) = &caller {
            if !valid_username(&caller.username) {
                return Err(SessionError::BadRequest(
                    "caller.username is not a valid username".into(),
                ));
            }
        }
        let emergency = self.cfg.emergency.classify(destination);
        let target = match emergency {
            Some(kind) => {
                if caller.is_none() {
                    warn!(destination = %destination, "refusing an emergency call with no caller");
                    return Err(SessionError::Forbidden(
                        "emergency calls need the caller's tenant and user, so the call can be \
                         located; this session has neither"
                            .into(),
                    ));
                }
                let Some(proxy) = self.cfg.outbound_proxy.as_deref() else {
                    warn!(destination = %destination, "refusing an emergency call: no outbound_proxy");
                    return Err(SessionError::Unavailable(
                        "emergency calls are not available here: the gateway has no call manager \
                         ([sip] outbound_proxy) to locate and route them"
                            .into(),
                    ));
                };
                let dialed = fcp_core::e911::dialed_user_part(destination);
                info!(dialed = %dialed, kind = %kind, "emergency call from a browser");
                format!("sip:{dialed}@{proxy}")
            }
            None => route_destination(destination, self.cfg.outbound_proxy.as_deref()),
        };
        let kind = SessionKind::Call {
            destination: target,
            caller,
            emergency,
            client_address: Some(client_address),
        };
        // What this node accepts (§5). An emergency call is admitted
        // whatever the limits say and even while draining, as it is
        // everywhere else on the platform.
        let session_id = new_session_id();
        self.admit(&session_id, emergency.is_some()).await?;
        let conn = match self.browser_connection_for(connection_id).await {
            Ok(conn) => conn,
            Err(e) => {
                self.release(&session_id);
                return Err(e);
            }
        };
        Ok(self.start(session_id, kind, conn).await)
    }

    /// The browser's connection for a call: the one it already has, or a
    /// new one whose offer the call returns.
    async fn browser_connection_for(
        &self,
        connection_id: Option<&str>,
    ) -> Result<BrowserConnection, SessionError> {
        match connection_id.filter(|id| !id.is_empty()) {
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
                Ok(BrowserConnection {
                    peer,
                    events,
                    offer,
                    fingerprint,
                    ice_ports: None,
                })
            }
            None => self.new_browser_connection(false, false, 0).await,
        }
    }

    /// A peer connection offering the mixer's audio codecs and, if asked,
    /// video — and a second video section for a shared screen, so that
    /// sharing later needs no renegotiation — registered with forge-api
    /// so the browser answers and trickles through the existing routes.
    async fn new_browser_connection(
        &self,
        video: bool,
        content: bool,
        streams: u8,
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
                // The camera's simulcast, passed on to the room, which
                // decodes the layer its tile needs
                // (`docs/FORWARDING_ROOMS.md` §2.7). The join page sizes
                // the encodings: `h` full, `m` half, `l` a quarter.
                simulcast: BROWSER_SIMULCAST.iter().map(|r| r.to_string()).collect(),
                // A forwarding room's people, each on a track of the
                // browser's own, fixed for the session.
                streams,
                // Translated to the room's numbers and back
                // (`docs/TRANSPORT_CC.md` §3.4).
                transport_cc: true,
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
            // The offer brings the transport up, and the event channel
            // with it; there is nothing to take before then.
            let offer = peer
                .create_offer()
                .await
                .map_err(|e| SessionError::Internal(format!("offer: {e}")))?;
            let events = peer
                .take_events()
                .ok_or_else(|| SessionError::Internal("peer connection has no events".into()))?;
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

    async fn start(
        self: &Arc<Self>,
        id: String,
        kind: SessionKind,
        conn: BrowserConnection,
    ) -> CreatedSession {
        let connection_id = conn.peer.lock().await.connection_id().to_string();
        // A host's or panelist's token: the page offers them moderation.
        let moderates = matches!(
            &kind,
            SessionKind::Room { join_token: Some(t), .. }
                if fcp_core::JoinToken::peek_unverified(t).is_some_and(|t| matches!(
                    t.role,
                    fcp_core::JoinRole::Host | fcp_core::JoinRole::Panelist
                ))
        );
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
            qa: QaState {
                moderates,
                ..QaState::default()
            },
            chat: ChatState::default(),
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
            shape: None,
            sip_ports: Vec::new(),
            swap_tx: None,
            migrating: false,
            record_call_id: fcp_core::CallId::new(),
            started_at: SystemTime::now(),
            connected_at: None,
            ended_by: None,
            last_sip_call_id: None,
            forwarding: None,
            roster_gap: Arc::new(Notify::new()),
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
            browser_tcc,
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
                guard.transport_cc_extension(),
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

        // A forwarding room's tracks: a sender per section the browser
        // kept, by index, and each section's mid.
        let (stream_senders, stream_mids): (Vec<Option<(VideoSender, u8)>>, Vec<String>) = {
            let guard = peer.lock().await;
            let senders = guard
                .negotiated_streams()
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let v = v.as_ref()?;
                    let sender = guard.stream_sender(i as u8).ok()?;
                    Some((sender, v.payload_type))
                })
                .collect();
            (senders, guard.stream_mids().to_vec())
        };

        // 3. The SIP leg's ports, keys and offer.
        let shape = SipShape {
            audio_codec,
            audio_pt,
            video: video.clone(),
            content: content.clone(),
            transport_cc: browser_tcc.is_some(),
        };
        let sip_offer = match self.build_sip_offer(&id, &shape).await {
            Ok(o) => o,
            Err(why) => {
                self.finish(&session, &peer, Some(why)).await;
                return;
            }
        };
        {
            let mut s = session.write().await;
            s.ports.extend(sip_offer.ports());
            s.sip_ports = sip_offer.ports();
            s.shape = Some(shape);
        }
        let codec_name = sip_offer.codec_name;
        let offer = sip_offer.offer.clone();

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
            video = sip_offer.video.is_some(),
            content = sip_offer.content.is_some(),
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
        // The room forwards each person's stream when it answered our
        // `Recv-Info: fcp-view` in kind, and the browser kept a track.
        let forwards = stream_senders.iter().any(Option::is_some)
            && final_response.headers().get_all("Recv-Info").any(|v| {
                v.split(',')
                    .any(|p| p.trim().eq_ignore_ascii_case(fcp_core::VIEW_INFO_PACKAGE))
            });
        let forwarding = forwards.then(|| {
            let usable = stream_senders.iter().map(Option::is_some).collect();
            Arc::new(std::sync::Mutex::new(Forwarding::new(
                stream_mids.clone(),
                usable,
            )))
        });

        // 6. The SIP leg from the answer.
        let plan = match self
            .open_sip_leg(
                &answer,
                codec_name,
                sip_offer.audio_ports,
                sip_offer.audio_srtp,
                sip_offer.video,
                sip_offer.content,
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
            s.connected_at.get_or_insert_with(SystemTime::now);
            s.dialog = Some(dialog.clone());
            s.sip_call_id = Some(sip_call_id.clone());
            s.audio_codec = Some(plan.audio_codec.clone());
            s.srtp = plan.srtp;
            s.video = plan.video.clone();
            s.content = plan.content.clone();
            s.forwarding = forwarding.clone();
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
            streams: if forwarding.is_some() {
                stream_senders
            } else {
                Vec::new()
            },
            forwarding: forwarding.clone(),
            transport_cc: browser_tcc,
        };
        // Whose stream is whose: the room's roster, in the call.
        if forwarding.is_some() {
            let mgr = Arc::clone(&self);
            let session = Arc::clone(&session);
            tokio::spawn(async move { mgr.follow_roster(session).await });
        }
        let (swap_tx, swap_rx) = mpsc::channel(1);
        session.write().await.swap_tx = Some(swap_tx);
        let end = run_relay(browser, plan.leg, stats, stop, swap_rx).await;
        session.write().await.swap_tx = None;
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

    /// The SIP side's ports, keys and offer for a shape. Audio must get
    /// its ports; a video section that cannot is left out. On `Err`
    /// nothing stays allocated.
    async fn build_sip_offer(&self, id: &str, shape: &SipShape) -> Result<SipOffer, String> {
        let audio_ports = self
            .ports
            .allocate()
            .await
            .map_err(|e| format!("no media ports: {e}"))?;
        // A port pair and, under SRTP, a key per video section offered.
        let mut offered_video = None;
        let mut offered_content = None;
        if let Some(v) = shape.video.clone() {
            match self.ports.allocate().await {
                Ok(ports) => {
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
        if let (Some(_), Some(c)) = (&offered_video, shape.content.clone()) {
            match self.ports.allocate().await {
                Ok(ports) => {
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
        let codec_name = audio_codec_name(shape.audio_codec);
        let sip_codecs = [SdpCodecInfo {
            payload_type: shape.audio_pt,
            encoding_name: codec_name.to_string(),
            clock_rate: 8000,
        }];
        let tcc = shape
            .transport_cc
            .then_some(crate::transport_cc::SIP_EXTENSION_ID);
        fn section_of(o: &OfferedVideo, tcc: Option<u8>) -> VideoOfferSection<'_> {
            VideoOfferSection {
                rtp_port: o.ports.rtp_port,
                payload_type: o.negotiated.payload_type,
                codec: o.negotiated.codec,
                fmtp: o.negotiated.fmtp.as_deref(),
                feedback: true,
                srtp: o.srtp.as_ref().map(|s| &s.sdp),
                // The browser's layers, sent on as they are (§2.7).
                simulcast: o.negotiated.simulcast.as_ref(),
                transport_cc: tcc,
            }
        }
        let video_section = offered_video.as_ref().map(|o| section_of(o, tcc));
        let content_section = offered_content.as_ref().map(|o| section_of(o, tcc));
        let built = construct_offer(&OfferOptions {
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
        });
        let offer = match built {
            Ok(o) => o,
            Err(e) => {
                self.ports.deallocate(audio_ports).await;
                for p in offered_video
                    .iter()
                    .chain(offered_content.iter())
                    .map(|o| o.ports)
                {
                    self.ports.deallocate(p).await;
                }
                return Err(format!("offer: {e}"));
            }
        };
        Ok(SipOffer {
            offer,
            codec_name,
            audio_ports,
            audio_srtp,
            video: offered_video,
            content: offered_content,
        })
    }

    // ------------------------------------------------------------ migration

    /// The conference node this session is on is draining and sent a
    /// REFER naming another node, with a ticket
    /// (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5c). Start the move —
    /// it runs on its own task, the REFER being answered at once — and say
    /// how the REFER is to be answered.
    pub async fn begin_migration(
        self: &Arc<Self>,
        sip_call_id: &str,
        refer: MigrationRefer,
    ) -> MigrationStart {
        let Some(id) = self
            .by_sip_call_id
            .get(sip_call_id)
            .map(|e| e.value().clone())
        else {
            return MigrationStart::Unknown;
        };
        let Some(session) = self.sessions.get(&id).map(|e| e.value().clone()) else {
            return MigrationStart::Unknown;
        };
        let old_dialog = {
            let mut s = session.write().await;
            if s.state != SessionState::Connected
                || s.swap_tx.is_none()
                || s.shape.is_none()
                || s.is_over()
            {
                return MigrationStart::NotReady;
            }
            let Some(dialog) = s.dialog.clone() else {
                return MigrationStart::NotReady;
            };
            if s.migrating {
                return MigrationStart::Busy;
            }
            s.migrating = true;
            dialog
        };
        info!(session = %id, to = %refer.target.as_str(), "moving the SIP leg to another node");
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            mgr.migrate(session, old_dialog, refer).await;
        });
        MigrationStart::Accepted
    }

    /// The move, then its report to the old node: a sipfrag NOTIFY on the
    /// old leg, and on success a BYE for it. The browser sees none of it.
    async fn migrate(
        self: Arc<Self>,
        session: Arc<RwLock<Session>>,
        old_dialog: sip_dialog::Dialog,
        refer: MigrationRefer,
    ) {
        let id = session.read().await.id.clone();
        let outcome = self.try_migrate(&session, &refer).await;
        session.write().await.migrating = false;
        let (code, reason) = match &outcome {
            Ok(()) => {
                info!(session = %id, "SIP leg moved to the new node");
                (200, "OK")
            }
            Err((code, reason, why)) => {
                warn!(session = %id, code, %why, "SIP leg not moved; staying on the old node");
                (*code, *reason)
            }
        };
        let mut dialog = old_dialog.clone();
        if let Ok(local) = SipUri::parse(&self.cfg.local_uri) {
            if let Err(e) = report_refer_outcome(&self.uac, &local, &mut dialog, code, reason).await
            {
                debug!(session = %id, error = %e, "refer NOTIFY failed");
            }
        }
        if outcome.is_ok() {
            // The old node's own BYE may cross this one; each side answers
            // 481 to a dialog it no longer holds.
            if let Err(e) = tokio::time::timeout(Duration::from_secs(10), self.uac.bye(&old_dialog))
                .await
                .map_err(|_| "unanswered".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()))
            {
                debug!(session = %id, error = %e, "BYE to the old node failed");
            }
        }
    }

    /// Dial the new node with the same shape the browser has, open the
    /// leg from its answer, hand it to the relay, and take over the
    /// session's SIP side. `Err` carries what the old node is told.
    async fn try_migrate(
        &self,
        session: &Arc<RwLock<Session>>,
        refer: &MigrationRefer,
    ) -> Result<(), (u16, &'static str, String)> {
        let (id, shape, display_name, old_sip_call_id) = {
            let s = session.read().await;
            let display_name = match &s.kind {
                SessionKind::Room { display_name, .. } => Some(display_name.clone()),
                SessionKind::Call { .. } => None,
            };
            (
                s.id.clone(),
                s.shape.clone(),
                display_name,
                s.sip_call_id.clone(),
            )
        };
        let shape = shape.ok_or_else(|| (500, "Server Internal Error", "no shape".to_string()))?;
        let sip_offer = self
            .build_sip_offer(&id, &shape)
            .await
            .map_err(|why| (500, "Server Internal Error", why))?;
        let new_ports = sip_offer.ports();
        session
            .write()
            .await
            .ports
            .extend(new_ports.iter().copied());

        let mut headers = Headers::new();
        if let Some(name) = display_name {
            if let Err(e) = headers.push(
                "P-Asserted-Identity",
                format!("\"{name}\" <{}>", self.cfg.local_uri),
            ) {
                {
                    self.drop_new_leg_ports(session, &new_ports).await;
                    return Err((500, "Server Internal Error", format!("header: {e}")));
                }
            }
        }
        if let Err(e) = headers.push(MIGRATION_TICKET_HEADER, refer.ticket.clone()) {
            {
                self.drop_new_leg_ports(session, &new_ports).await;
                return Err((500, "Server Internal Error", format!("header: {e}")));
            }
        }
        // The new node's record of the leg keeps the session's call id.
        let record_call_id = session.read().await.record_call_id;
        let _ = headers.push(fcp_core::CALL_ID_HEADER, record_call_id.to_string());
        let target = refer.target.as_str().to_string();
        let handle = match self
            .uac
            .invite_with_headers(&target, Some(&sip_offer.offer), headers)
            .await
        {
            Ok(h) => Arc::new(h),
            Err(e) => {
                self.drop_new_leg_ports(session, &new_ports).await;
                return Err((503, "Service Unavailable", format!("INVITE: {e}")));
            }
        };
        while handle.await_provisional().await.is_some() {}
        let final_response =
            match tokio::time::timeout(self.cfg.invite_timeout, handle.await_final()).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    self.drop_new_leg_ports(session, &new_ports).await;
                    return Err((503, "Service Unavailable", format!("INVITE failed: {e}")));
                }
                Err(_) => {
                    let _ = handle.cancel().await;
                    {
                        self.drop_new_leg_ports(session, &new_ports).await;
                        return Err((408, "Request Timeout", "no answer from the new node".into()));
                    }
                }
            };
        let code = final_response.start_line().code();
        if !(200..300).contains(&code) {
            let reason = match code {
                403 => "Forbidden",
                503 => "Service Unavailable",
                _ => "Not Acceptable Here",
            };
            {
                self.drop_new_leg_ports(session, &new_ports).await;
                return Err((code, reason, format!("the new node answered {code}")));
            }
        }
        let dialog = handle.dialog.read().await.clone();
        let new_sip_call_id = dialog.id().call_id().to_string();
        let answer = String::from_utf8_lossy(final_response.body()).to_string();
        let plan = match self
            .open_sip_leg(
                &answer,
                sip_offer.codec_name,
                sip_offer.audio_ports,
                sip_offer.audio_srtp,
                sip_offer.video,
                sip_offer.content,
            )
            .await
        {
            Ok(p) => p,
            Err(why) => {
                let _ = self.uac.bye(&dialog).await;
                {
                    self.drop_new_leg_ports(session, &new_ports).await;
                    return Err((488, "Not Acceptable Here", why));
                }
            }
        };

        // Hand the relay the new leg, if it is still running.
        let swap_tx = {
            let s = session.read().await;
            if s.is_over() {
                None
            } else {
                s.swap_tx.clone()
            }
        };
        let Some(swap_tx) = swap_tx else {
            let _ = self.uac.bye(&dialog).await;
            {
                self.drop_new_leg_ports(session, &new_ports).await;
                return Err((481, "Call Gone", "the session ended during the move".into()));
            }
        };
        if swap_tx.send(plan.leg).await.is_err() {
            let _ = self.uac.bye(&dialog).await;
            {
                self.drop_new_leg_ports(session, &new_ports).await;
                return Err((481, "Call Gone", "the relay ended during the move".into()));
            }
        }

        // The session's SIP side is the new leg now.
        let old_ports = {
            let mut s = session.write().await;
            s.dialog = Some(dialog);
            s.call_handle = Some(handle);
            s.sip_call_id = Some(new_sip_call_id.clone());
            s.sip_target = Some(target);
            s.audio_codec = Some(plan.audio_codec);
            s.srtp = plan.srtp;
            s.video = plan.video;
            s.content = plan.content;
            let old = std::mem::replace(&mut s.sip_ports, new_ports);
            s.ports.retain(|p| !old.contains(p));
            old
        };
        self.by_sip_call_id.insert(new_sip_call_id, id.clone());
        if let Some(old) = old_sip_call_id {
            self.by_sip_call_id.remove(&old);
        }
        // The relay drops the old leg's sockets as it swaps; give that a
        // moment before the ports go back to the pool.
        let pool = self.ports.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            for p in old_ports {
                pool.deallocate(p).await;
            }
        });
        Ok(())
    }

    /// A move that failed after its ports were taken: give them back and
    /// forget them.
    async fn drop_new_leg_ports(&self, session: &Arc<RwLock<Session>>, ports: &[PortPair]) {
        session.write().await.ports.retain(|p| !ports.contains(p));
        for p in ports {
            self.ports.deallocate(*p).await;
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
            SessionKind::Call {
                destination,
                caller,
                client_address,
                ..
            } => {
                if let Some(caller) = caller {
                    headers
                        .push(fcp_core::CALLER_TENANT_HEADER, caller.tenant_id.to_string())
                        .map_err(|e| format!("header: {e}"))?;
                    headers
                        .push(fcp_core::CALLER_USER_HEADER, caller.username)
                        .map_err(|e| format!("header: {e}"))?;
                    // The gateway is the browser's edge: it says where the
                    // browser is, and the call manager believes it only
                    // from a trusted edge.
                    if let Some(address) = client_address {
                        headers
                            .push(fcp_core::CLIENT_ADDRESS_HEADER, address.to_string())
                            .map_err(|e| format!("header: {e}"))?;
                    }
                }
                Ok((destination, headers))
            }
            SessionKind::Room {
                room_number,
                display_name,
                join_token,
                streams,
                ..
            } => {
                // Told of its hand and the stage (`docs/WEBINARS.md` §2.5).
                headers
                    .push("Recv-Info", fcp_core::QA_INFO_PACKAGE)
                    .map_err(|e| format!("header: {e}"))?;
                // Sent the room's chat.
                headers
                    .push("Recv-Info", fcp_core::CHAT_INFO_PACKAGE)
                    .map_err(|e| format!("header: {e}"))?;
                // Each person's own stream, where the room forwards.
                if streams > 0 && video.is_some() {
                    headers
                        .push("Recv-Info", fcp_core::VIEW_INFO_PACKAGE)
                        .map_err(|e| format!("header: {e}"))?;
                }
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
                let record_call_id = session.read().await.record_call_id;
                headers
                    .push(fcp_core::CALL_ID_HEADER, record_call_id.to_string())
                    .map_err(|e| format!("header: {e}"))?;
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
            if s.sip_call_id.is_some() {
                s.last_sip_call_id = s.sip_call_id.clone();
            }
            self.write_record(&s);
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
        self.release(&id);
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            tokio::time::sleep(LINGER).await;
            sessions.remove(&id);
        });
    }

    /// The record of a session that is over: `kind = gateway`, the call id
    /// it gave the conference node, the browser's time, the relay's
    /// packet counts.
    fn write_record(&self, s: &Session) {
        let Some(cdr) = &self.cdr else {
            return;
        };
        let (to_uri, display_name, tenant) = match &s.kind {
            SessionKind::Call {
                destination,
                caller,
                ..
            } => (
                destination.clone(),
                None,
                caller.as_ref().map(|c| c.tenant_id),
            ),
            SessionKind::Room {
                room_number,
                display_name,
                join_token,
                ..
            } => (
                format!("conference:{room_number}"),
                Some(display_name.clone()),
                join_token
                    .as_deref()
                    .and_then(fcp_core::JoinToken::peek_unverified)
                    .map(|t| t.tenant_id),
            ),
        };
        let Some(tenant_id) = tenant.or(cdr.default_tenant) else {
            warn!(session = %s.id, "no tenant for the session's record; not written");
            return;
        };
        let reason = match &s.state {
            SessionState::Failed(why) => fcp_sip::TerminationReason::failed(why.clone()),
            _ if s.connected_at.is_none() => fcp_sip::TerminationReason::Cancelled,
            _ => fcp_sip::TerminationReason::Hangup { by: s.ended_by },
        };
        let stats = s.stats.snapshot();
        let mut builder = fcp_cdr::RecordBuilder::new(
            fcp_core::storage::CdrKind::Gateway,
            s.record_call_id,
            tenant_id,
        )
        .from(format!("webrtc:{}", s.id), display_name)
        .to(to_uri, None)
        .started_at(s.started_at)
        .answered_at(s.connected_at)
        .ended_at(Some(SystemTime::now()))
        .reason(reason)
        .media_anchored(true)
        .codecs(s.audio_codec.clone(), None)
        .rtp_packets(
            stats.audio_to_sip + stats.video_to_sip + stats.content_to_sip,
            stats.audio_to_browser + stats.video_to_browser + stats.content_to_browser,
        )
        .node(Some(cdr.node_id.clone()), cdr.region.clone())
        .custom_data(Some(serde_json::json!({
            "session_id": s.id,
            "sip_target": s.sip_target,
            "srtp": s.srtp,
            "video": s.video.is_some(),
            "content": s.content.is_some(),
            "dropped": stats.dropped,
            "srtp_errors": stats.srtp_errors,
        })));
        if let Some(sip_call_id) = &s.last_sip_call_id {
            builder = builder.sip_call_id(sip_call_id.clone());
        }
        cdr.writer.write(builder.build());
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
        fcp_core::secret::constant_time_eq(s.session_token.as_bytes(), token.as_bytes())
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
            s.ended_by = Some(fcp_core::storage::HangupParty::Caller);
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
                s.ended_by = Some(fcp_core::storage::HangupParty::Callee);
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

    /// The conference server told `sip_call_id` of its hand or the stage.
    /// Returns whether it was ours.
    pub async fn qa_notice(&self, sip_call_id: &str, message: &fcp_core::qa::QaMessage) -> bool {
        let Some(session) = self.session_by_sip_call_id(sip_call_id) else {
            return false;
        };
        let mut s = session.write().await;
        s.qa.apply(message);
        info!(
            session = %s.id,
            hand = s.qa.hand,
            on_stage = s.qa.on_stage,
            questions = s.qa.questions.len(),
            "fcp-qa notice"
        );
        true
    }

    /// The conference server sent `sip_call_id` chat. Returns whether it
    /// was ours.
    pub async fn chat_notice(
        &self,
        sip_call_id: &str,
        message: &fcp_core::chat::ChatMessage,
    ) -> bool {
        let Some(session) = self.session_by_sip_call_id(sip_call_id) else {
            return false;
        };
        let mut s = session.write().await;
        s.chat.apply(message);
        debug!(
            session = %s.id,
            muted = s.chat.muted,
            messages = s.chat.messages.len(),
            "fcp-chat notice"
        );
        true
    }

    /// Subscribe to the room's roster inside the call, and keep the
    /// subscription up while the session lasts: its NOTIFYs say whose
    /// stream is on each of the browser's tracks ([`Self::roster_notice`]).
    async fn follow_roster(self: Arc<Self>, session: Arc<RwLock<Session>>) {
        loop {
            let (dialog, stop) = {
                let s = session.read().await;
                if s.is_over() {
                    return;
                }
                (s.dialog.clone(), s.stop.clone())
            };
            let Some(mut dialog) = dialog else { return };
            let id = session.read().await.id.clone();
            let request = in_dialog_request(
                &dialog,
                sip_core::Method::Subscribe,
                &[
                    ("Event", "conference".to_string()),
                    (
                        "Accept",
                        fcp_core::conference_info::CONTENT_TYPE.to_string(),
                    ),
                    ("Expires", ROSTER_EXPIRES.as_secs().to_string()),
                ],
                None,
            );
            let next = match request {
                Ok(request) => match tokio::time::timeout(
                    SOON,
                    self.uac
                        .inner()
                        .send_in_dialog_non_invite(&mut dialog, request),
                )
                .await
                {
                    Ok(Ok(r)) if (200..300).contains(&r.start_line().code()) => {
                        info!(session = %id, "roster subscribed");
                        ROSTER_EXPIRES / 2
                    }
                    Ok(Ok(r)) => {
                        warn!(session = %id, code = r.start_line().code(), "roster SUBSCRIBE refused; names stay unknown");
                        return;
                    }
                    Ok(Err(e)) => {
                        info!(session = %id, error = %e, "roster SUBSCRIBE failed; again shortly");
                        RETRY
                    }
                    Err(_) => RETRY,
                },
                Err(e) => {
                    warn!(session = %id, error = %e, "roster SUBSCRIBE not built");
                    return;
                }
            };
            // The dialog's CSeq moved on.
            {
                let mut s = session.write().await;
                if s.dialog.as_ref().is_some_and(|d| d.id() == dialog.id()) {
                    s.dialog = Some(dialog);
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(next) => {}
                _ = stop.notified() => return,
                _ = self.roster_gap(&session) => {}
            }
        }
    }

    /// Resolves when the session's roster missed a document and must be
    /// asked for whole again.
    async fn roster_gap(&self, session: &Arc<RwLock<Session>>) {
        let gap = session.read().await.roster_gap.clone();
        gap.notified().await;
    }

    /// The room sent a roster NOTIFY in the call `sip_call_id`; false when
    /// no session has it.
    pub async fn roster_notice(&self, sip_call_id: &str, body: &[u8]) -> bool {
        let Some(session) = self.session_by_sip_call_id(sip_call_id) else {
            return false;
        };
        let s = session.read().await;
        let (Some(f), Some(doc)) = (
            s.forwarding.as_ref(),
            fcp_core::conference_info::parse(&String::from_utf8_lossy(body)),
        ) else {
            return true;
        };
        let applied = f
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .roster_mut()
            .apply(doc);
        if applied == fcp_core::conference_info::Applied::Gap {
            s.roster_gap.notify_one();
        }
        true
    }

    /// Send the room the browser's view (`docs/FORWARDING_ROOMS.md` §2.3):
    /// an INFO in the `fcp-view` package. The room's answer is returned.
    pub async fn set_view(&self, id: &str, body: &[u8]) -> Result<u16, SessionError> {
        let view = fcp_core::video_view::View::parse(body).map_err(SessionError::BadRequest)?;
        let session = self
            .sessions
            .get(id)
            .map(|e| e.value().clone())
            .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
        let mut dialog = {
            let s = session.read().await;
            if s.forwarding.is_none() {
                return Err(SessionError::BadRequest(
                    "the room does not forward each person's stream to this session".into(),
                ));
            }
            s.dialog
                .clone()
                .ok_or_else(|| SessionError::NotFound(id.to_string()))?
        };
        let json =
            serde_json::to_vec(&view).map_err(|e| SessionError::Internal(format!("view: {e}")))?;
        let request = in_dialog_request(
            &dialog,
            sip_core::Method::Info,
            &[
                ("Info-Package", fcp_core::VIEW_INFO_PACKAGE.to_string()),
                ("Content-Disposition", "Info-Package".to_string()),
                ("Content-Type", fcp_core::VIEW_INFO_TYPE.to_string()),
            ],
            Some(json.into()),
        )
        .map_err(|e| SessionError::Internal(format!("view INFO: {e}")))?;
        let response = tokio::time::timeout(
            SOON,
            self.uac
                .inner()
                .send_in_dialog_non_invite(&mut dialog, request),
        )
        .await
        .map_err(|_| SessionError::Internal("the room did not answer the view".into()))?
        .map_err(|e| SessionError::Internal(format!("view INFO: {e}")))?;
        let mut s = session.write().await;
        if s.dialog.as_ref().is_some_and(|d| d.id() == dialog.id()) {
            s.dialog = Some(dialog);
        }
        Ok(response.start_line().code())
    }

    /// Raise or lower the browser's hand (`docs/WEBINARS.md` §2.5): an
    /// INFO in the `fcp-qa` package. The room's answer is returned.
    pub async fn set_hand(&self, id: &str, raised: bool) -> Result<u16, SessionError> {
        let code = self
            .send_qa(id, &fcp_core::qa::QaMessage::Hand { raised })
            .await?;
        if (200..300).contains(&code) {
            if let Some(session) = self.sessions.get(id).map(|e| e.value().clone()) {
                session.write().await.qa.hand = raised;
            }
        }
        Ok(code)
    }

    /// Send the browser's `fcp-qa` message to the room: a hand or a
    /// question (`docs/WEBINARS.md` §2.5). The room's answer is returned.
    pub async fn send_qa(
        &self,
        id: &str,
        message: &fcp_core::qa::QaMessage,
    ) -> Result<u16, SessionError> {
        self.send_info(
            id,
            fcp_core::QA_INFO_PACKAGE,
            fcp_core::QA_INFO_TYPE,
            message.to_body(),
        )
        .await
    }

    /// Send the browser's `fcp-chat` message to the room: a post, or (a
    /// host or panelist) a delete or a mute (`docs/WEBINARS.md` §2.5). The
    /// room's answer is returned.
    pub async fn send_chat(
        &self,
        id: &str,
        message: &fcp_core::chat::ChatMessage,
    ) -> Result<u16, SessionError> {
        self.send_info(
            id,
            fcp_core::CHAT_INFO_PACKAGE,
            fcp_core::CHAT_INFO_TYPE,
            message.to_body(),
        )
        .await
    }

    /// Send `body` to the room as an INFO of `package` and `content_type`.
    async fn send_info(
        &self,
        id: &str,
        package: &str,
        content_type: &str,
        body: String,
    ) -> Result<u16, SessionError> {
        let session = self
            .sessions
            .get(id)
            .map(|e| e.value().clone())
            .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
        let mut dialog = {
            let s = session.read().await;
            s.dialog
                .clone()
                .ok_or_else(|| SessionError::NotFound(id.to_string()))?
        };
        let request = in_dialog_request(
            &dialog,
            sip_core::Method::Info,
            &[
                ("Info-Package", package.to_string()),
                ("Content-Disposition", "Info-Package".to_string()),
                ("Content-Type", content_type.to_string()),
            ],
            Some(body.into_bytes().into()),
        )
        .map_err(|e| SessionError::Internal(format!("{package} INFO: {e}")))?;
        let response = tokio::time::timeout(
            SOON,
            self.uac
                .inner()
                .send_in_dialog_non_invite(&mut dialog, request),
        )
        .await
        .map_err(|_| SessionError::Internal("the room did not answer".into()))?
        .map_err(|e| SessionError::Internal(format!("{package} INFO: {e}")))?;
        let code = response.start_line().code();
        let mut s = session.write().await;
        if s.dialog.as_ref().is_some_and(|d| d.id() == dialog.id()) {
            s.dialog = Some(dialog);
        }
        Ok(code)
    }

    fn session_by_sip_call_id(&self, sip_call_id: &str) -> Option<Arc<RwLock<Session>>> {
        let id = self.by_sip_call_id.get(sip_call_id)?.value().clone();
        self.sessions.get(&id).map(|e| e.value().clone())
    }

    // ------------------------------------------------------------ queries

    /// Every session of one kind.
    pub async fn list(&self, rooms: bool) -> Vec<SessionInfo> {
        let mut out = Vec::new();
        for entry in self.sessions.iter() {
            let s = entry.value().read().await;
            if s.kind.is_room() == rooms {
                let mut info = s.info();
                info.handover = self.handover_for(&s.kind);
                out.push(info);
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
        (s.kind.is_room() == rooms).then(|| {
            let mut info = s.info();
            // A browser polling its session is how it learns this node is
            // leaving, and where to go (§6).
            info.handover = self.handover_for(&s.kind);
            info
        })
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
            transport_cc: section.transport_cc,
        },
        VideoSummary {
            codec: offered.negotiated.codec.sdp_name().to_string(),
            payload_type: pt,
            srtp,
        },
    )))
}

/// How long a roster subscription is asked for; renewed at half.
const ROSTER_EXPIRES: Duration = Duration::from_secs(600);
/// A request in the call is answered within this, or taken as lost.
const SOON: Duration = Duration::from_secs(5);
/// A roster SUBSCRIBE that failed is tried again after this.
const RETRY: Duration = Duration::from_secs(5);

/// A request inside `dialog`: its From, To, Call-ID and a Via to be
/// filled, the given headers and body. The UAC sets the CSeq, Route and
/// the Via's address.
fn in_dialog_request(
    dialog: &sip_dialog::Dialog,
    method: sip_core::Method,
    extra: &[(&str, String)],
    body: Option<bytes::Bytes>,
) -> Result<sip_core::Request, String> {
    let id = dialog.id();
    let mut headers = Headers::new();
    let mut push = |name: &str, value: String| {
        headers
            .push(name, value)
            .map_err(|e| format!("header {name}: {e}"))
    };
    push(
        "From",
        format!("<{}>;tag={}", dialog.local_uri().as_str(), id.local_tag()),
    )?;
    push(
        "To",
        format!("<{}>;tag={}", dialog.remote_uri().as_str(), id.remote_tag()),
    )?;
    push("Call-ID", id.call_id().to_string())?;
    // A placeholder the UAC fills with our address; the branch names the
    // transaction.
    push(
        "Via",
        format!(
            "SIP/2.0/UDP 0.0.0.0;branch=z9hG4bK{}",
            uuid::Uuid::new_v4().simple()
        ),
    )?;
    push("Max-Forwards", "70".to_string())?;
    for (name, value) in extra {
        push(name, value.clone())?;
    }
    let body = body.unwrap_or_default();
    push("Content-Length", body.len().to_string())?;
    sip_core::Request::new(
        sip_core::RequestLine::new(method, dialog.remote_target().clone()),
        headers,
        body,
    )
    .map_err(|e| format!("request: {e}"))
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
    fn a_session_s_chat_follows_the_room() {
        use fcp_core::chat::{ChatAudience, ChatMessage, ChatView};
        let view = |id: &str, deleted: bool| ChatView {
            id: id.into(),
            text: if deleted { String::new() } else { "hi".into() },
            sender: None,
            sender_id: "p".into(),
            sender_moderates: false,
            mine: false,
            to: ChatAudience::Room,
            deleted,
            sent_at: "2026-11-18T10:00:00.000Z".into(),
        };
        let mut chat = ChatState::default();
        chat.apply(&ChatMessage::Messages {
            messages: vec![view("m-1", false), view("m-2", false), view("m-3", true)],
        });
        assert_eq!(
            chat.messages.len(),
            2,
            "a deleted one never seen is not kept"
        );
        chat.apply(&ChatMessage::Messages {
            messages: vec![view("m-1", true)],
        });
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].id, "m-2");
        chat.apply(&ChatMessage::Muted { on: true });
        assert!(chat.muted);
        chat.apply(&ChatMessage::Messages {
            messages: (0..MAX_SESSION_MESSAGES + 5)
                .map(|i| view(&format!("n-{i}"), false))
                .collect(),
        });
        assert_eq!(chat.messages.len(), MAX_SESSION_MESSAGES);
        assert_eq!(
            chat.messages.last().unwrap().id,
            format!("n-{}", MAX_SESSION_MESSAGES + 4)
        );
    }

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
    fn bare_numbers_go_through_the_call_manager() {
        let proxy = Some("10.0.0.2:5060");
        assert_eq!(route_destination("1001", proxy), "sip:1001@10.0.0.2:5060");
        assert_eq!(
            route_destination(" 1001@10.0.0.7", proxy),
            "sip:1001@10.0.0.7"
        );
        assert_eq!(
            route_destination("sip:alice@example.com", proxy),
            "sip:alice@example.com"
        );
        assert_eq!(route_destination("1001", None), "sip:1001");
    }

    #[test]
    fn caller_usernames_are_header_safe() {
        assert!(valid_username("alice"));
        assert!(valid_username("alice.smith+desk"));
        assert!(!valid_username(""));
        assert!(!valid_username("alice smith"));
        assert!(!valid_username("alice\r\nVia: x"));
        assert!(!valid_username("<alice>"));
    }

    #[test]
    fn session_kind_serializes_flat_without_the_token() {
        let kind = SessionKind::Room {
            room_number: "8001".into(),
            display_name: "Ann".into(),
            video: true,
            content: true,
            streams: 0,
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
