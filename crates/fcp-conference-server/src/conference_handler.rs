use crate::video_bridge::{
    self, claim_beats, FloorNotice, FloorNotifier, VideoEventSink, VideoLeg, VideoLegStats,
    CONTENT_INFO_TYPE,
};
use anyhow::{bail, Context, Result};
use dashmap::DashMap;
use fcp_core::storage::TenantStore;
use fcp_core::{DynStorage, Meeting, MigrationTicket, TenantId};
use fcp_events::{ConferenceEvent, EventPublisher, PlatformEvent};
use fcp_media::{
    construct_answer, construct_answer_for_a_leg, decode_sdes_key_params,
    generate_srtp_key_material, parse_content_sdes_crypto, parse_sdes_crypto, parse_sdp_codecs,
    parse_video_sdes_crypto, AnswerOptions, SrtpSdpContext, VideoAnswer,
};
use fcp_sip::prelude::SipUserAgentClient;
use fcp_sip::PeerIdentity;
use forge_codecs::{g711, g722, AudioCodec};
use forge_conference::video::{StreamKind, VideoBackend};
use forge_conference::{
    ConferenceAIConfig, ConferenceAIManager, ConferenceBridge, ConferenceConfig,
    ConferenceDtmfConfig, ConferenceError, DtmfCommandHandler, PinAuthResult, RoomConfig,
};
use forge_dtmf::rfc2833::Rfc2833Detector;
use forge_dtmf::{
    DtmfDeduplicator, DtmfDigit, DtmfEvent, DtmfEventType, DtmfMethod, GoertzelDetector,
};
use forge_engine::{AISessionConfig, AISessionManager};
use forge_resampler::Resampler;
use forge_rtp::{
    PortPair, PortPool, RemoteEndpoint, RtpSocketConfig, RtpSocketPair, SrtpContext,
    SrtpKeyMaterial,
};
use parking_lot::RwLock;
use sip_core::{Headers, SipUri, Uri};
use sip_dialog::Dialog;
use sip_uas::UserAgentServer;
use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::audio_bridge::{self, AudioStats};
use crate::bfcp::{BfcpAssignment, Floors};
use crate::config::{AiSection, ConferenceSection};
use crate::migration::{self, Notice};
use crate::trunks::{self, TRUNK_NODE_HEADER, TRUNK_TENANT_HEADER};
use futures::StreamExt;

/// Fallback remote address used when SDP parsing fails to extract the caller's RTP endpoint.
const UNKNOWN_REMOTE: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0);

/// Aggregated audio quality metrics across all active sessions.
#[derive(Default)]
pub struct AggregatedAudioStats {
    pub packets_received: u64,
    pub packets_decoded: u64,
    pub decode_errors: u64,
    pub packets_sent: u64,
    pub encode_errors: u64,
    pub dtmf_events: u64,
}

/// Latency telemetry for ConferenceHandler trait methods.
pub struct HandlerLatency {
    pub invite_count: AtomicU64,
    pub invite_total_us: AtomicU64,
    pub reinvite_count: AtomicU64,
    pub reinvite_total_us: AtomicU64,
    pub bye_count: AtomicU64,
    pub bye_total_us: AtomicU64,
    pub subscribe_count: AtomicU64,
    pub subscribe_total_us: AtomicU64,
    pub refer_count: AtomicU64,
    pub refer_total_us: AtomicU64,
}

impl HandlerLatency {
    pub fn new() -> Self {
        Self {
            invite_count: AtomicU64::new(0),
            invite_total_us: AtomicU64::new(0),
            reinvite_count: AtomicU64::new(0),
            reinvite_total_us: AtomicU64::new(0),
            bye_count: AtomicU64::new(0),
            bye_total_us: AtomicU64::new(0),
            subscribe_count: AtomicU64::new(0),
            subscribe_total_us: AtomicU64::new(0),
            refer_count: AtomicU64::new(0),
            refer_total_us: AtomicU64::new(0),
        }
    }

    fn record(&self, count: &AtomicU64, total_us: &AtomicU64, elapsed: Duration) {
        count.fetch_add(1, Ordering::Relaxed);
        total_us.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
    }
}

/// Tracks a single participant's connection to a conference room.
pub struct ParticipantSession {
    pub call_id: String,
    pub room_number: String,
    pub participant_id: String,
    pub is_host: bool,
    /// True if this participant was invited via dial-out
    pub is_dial_out: bool,
    /// For an inter-node conference trunk: the peer node's id. The
    /// participant is `__trunk__<peer>` and carries that node's mix.
    pub trunk_peer: Option<String>,
    pub audio_bridge_handle: JoinHandle<()>,
    pub port_pair: PortPair,
    pub rtp_socket: Arc<RtpSocketPair>,
    pub remote_addr: SocketAddr,
    pub codec_name: String,
    pub tenant_id: TenantId,
    pub dialog: Dialog,
    pub joined_at: SystemTime,
    pub te_pt: Option<u8>,
    pub stats: Arc<AudioStats>,
    /// Display name from SIP From header or P-Asserted-Identity
    pub display_name: Option<String>,
    /// Caller ID (SIP URI user part or tel number)
    pub caller_id: Option<String>,
    /// SRTP context for encrypting/decrypting RTP packets (None = plain RTP)
    pub srtp_context: Option<Arc<tokio::sync::RwLock<SrtpContext>>>,
    /// Last session refresh time (for RFC 4028 session timer reaping)
    pub last_refreshed: Instant,
    /// The video leg (second socket pair, its own SRTP, the room's
    /// source and subscription), when the caller negotiated video.
    pub video: Option<VideoLeg>,
    /// The shared screen's leg (a third socket pair, its own SRTP, the
    /// room's content source and content channel), when the caller's
    /// offer carried an `a=content:slides` section (§15.6).
    pub content: Option<VideoLeg>,
    /// Admitted by a join token (`X-FCP-Join`): a browser through the
    /// WebRTC gateway, which understands the floor `INFO` (§15.6, 7c).
    pub joined_by_token: bool,
    /// The BFCP ids the answer gave a room system that offered floor
    /// control (§15.6, 7e).
    pub bfcp: Option<BfcpAssignment>,
    /// What the leg's record needs that nothing else here keeps
    /// (`docs/CDR_PIPELINE.md` §5).
    pub record: LegRecord,
}

/// The parts of a conference leg's record the session does not otherwise
/// hold.
#[derive(Debug, Clone)]
pub struct LegRecord {
    /// The call the leg belongs to: the call manager's or the gateway's,
    /// named in `X-FCP-Call-Id`, or one of the leg's own.
    pub call_id: fcp_core::CallId,
    /// Who the leg is from, and to.
    pub from_uri: String,
    pub to_uri: Option<String>,
    /// The node a migrated participant came from.
    pub migrated_from: Option<String>,
}

impl LegRecord {
    /// A leg that arrived with `request`.
    pub fn from_request(request: &sip_core::Request, migrated_from: Option<String>) -> Self {
        let call_id = sip_parse::header(request.headers(), fcp_core::CALL_ID_HEADER)
            .and_then(|v| v.as_str().trim().parse::<fcp_core::CallId>().ok())
            .unwrap_or_default();
        let from_uri = request
            .headers()
            .get_smol("From")
            .and_then(sip_parse::parse_from_header)
            .and_then(|from| from.inner().sip_uri().map(|u| u.as_str().to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        Self {
            call_id,
            from_uri,
            to_uri: None,
            migrated_from,
        }
    }

    /// A leg this node placed or accepted for itself.
    pub fn own(from_uri: String, to_uri: Option<String>) -> Self {
        Self {
            call_id: fcp_core::CallId::new(),
            from_uri,
            to_uri,
            migrated_from: None,
        }
    }
}

/// Sink for "this node now has N local participants in room R" updates,
/// and the source of room-wide state a node inherits when it starts
/// hosting a room other nodes already host. The cluster-backed
/// implementation reads and writes the conference registry; without
/// clustering nothing is attached.
#[async_trait::async_trait]
pub trait RoomRegistry: Send + Sync {
    fn room_participants_changed(
        &self,
        tenant_id: String,
        room_number: String,
        participants: usize,
    );

    /// Whether the room is locked cluster-wide. Consulted once, when this
    /// node creates its local copy of a room, so a locked room stays
    /// locked when placement spreads it to a fresh node.
    async fn room_locked(&self, _tenant_id: &str, _room_number: &str) -> bool {
        false
    }

    /// How many callers this node has across every room: what it
    /// advertises as its load in cluster membership, so placement's
    /// "least-loaded conference node" can tell the nodes apart.
    fn node_callers_changed(&self, _total: usize) {}
}

/// What a room hears when the tenant asks for a recording announcement
/// and names no prompt of its own.
const RECORDING_CONSENT_TEXT: &str = "This conference is being recorded.";

/// Handles SIP INVITEs for conference rooms.
///
/// When a SIP INVITE arrives targeting a conference room number:
/// 1. Looks up the meeting config from storage
/// 2. Answers the call and sets up RTP
/// 3. Optionally collects a PIN for authentication
/// 4. Creates/joins the forge-conference room (with limits enforced)
/// 5. Spawns a per-participant audio bridge task
/// 6. Publishes conference events (join/leave)
pub struct ConferenceCallHandler {
    pub bridge: Arc<ConferenceBridge>,
    pub storage: Arc<DynStorage>,
    pub default_tenant_id: TenantId,
    pub conference_config: RwLock<ConferenceSection>,
    sessions: DashMap<String, ParticipantSession>,
    #[allow(dead_code)]
    pub local_uri: String,
    pub local_addr: String,
    port_pool: Arc<PortPool>,
    event_bus: Arc<dyn EventPublisher>,
    uac: Arc<SipUserAgentClient>,
    dtmf_handlers: DashMap<String, Arc<DtmfCommandHandler>>,
    /// In-memory meeting cache: (TenantId, room_number) → (Meeting, cached_at)
    meeting_cache: DashMap<(TenantId, String), (Meeting, Instant)>,
    /// Shared AI session manager (created once, reused for all rooms)
    ai_session_manager: Arc<AISessionManager>,
    pub ai_config: RwLock<AiSection>,
    /// Per-source-IP INVITE rate limiter: IP → (count, window_start)
    invite_rate: DashMap<std::net::IpAddr, (u32, Instant)>,
    /// RFC 4575 conference event subscribers: call_id → ConferenceSubscription
    conference_subscriptions: DashMap<String, ConferenceSubscription>,
    /// Cached parsed SipUri from local_uri (parsed once at construction, never panics at runtime)
    local_sip_uri: SipUri,
    /// Set to true during graceful shutdown to reject new INVITEs while draining existing sessions
    draining: std::sync::atomic::AtomicBool,
    /// Offer SDES-SRTP on the trunks this node dials (`cluster.trunk_srtp`).
    trunk_srtp: std::sync::atomic::AtomicBool,
    /// Secondary index: room_number → Vec<call_id> for O(1) room-to-session lookups.
    room_sessions: DashMap<String, Vec<String>>,
    /// This node's cluster id: what peers see in our trunk INVITEs and in
    /// our certificate.
    node_id: String,

    /// Cluster conference registry hook (see [`RoomRegistry`]).
    room_registry: std::sync::RwLock<Option<Arc<dyn RoomRegistry>>>,
    /// Latency telemetry for trait methods (INVITE, re-INVITE, BYE, etc.)
    pub handler_latency: HandlerLatency,
    /// Optional TTS service for synthesizing prompts from text
    tts_service: Option<Arc<fcp_media::TtsService>>,
    /// The codec registry, thread pool and device video rooms run on;
    /// `None` leaves every meeting audio-only.
    video_backend: Option<Arc<VideoBackend>>,
    /// The node's video budget, when accounting is on.
    video_capacity: Option<Arc<crate::video_capacity::VideoCapacity>>,
    /// Rooms whose video events are being forwarded to the bus.
    video_event_pumps: Arc<DashMap<String, ()>>,
    /// BFCP floor control for the room systems that speak it (§15.6, 7e).
    bfcp: Arc<Floors>,
    /// When each peer's speaker claim last arrived, by `room|node`. A
    /// claim that stops being refreshed is dropped rather than left
    /// holding the floor for a node that has gone quiet (§15.4, 5c).
    speaker_claims: DashMap<String, Instant>,
    /// Each peer's claim to the room's content floor and when it last
    /// arrived, by `room|node` (§15.6, 7d). The presenter's name is what
    /// this node's browsers are told; the claim's age is what expires it.
    content_claims: DashMap<String, (ContentClaim, Instant)>,
    /// Calls that have been answered and are waiting for PIN input.
    pending_pin_auth: DashMap<String, PendingPinAuth>,
    /// Where leg records go (`docs/CDR_PIPELINE.md`), and the region they
    /// name.
    cdr: Option<fcp_sip::CdrWriter>,
    region: Option<String>,
    /// The REFERs sent to callers' legs as this node drains
    /// (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5).
    migrations: migration::Migrations,
    /// Migration tickets admitted here, so each admits once.
    used_tickets: migration::UsedTickets,
    /// The secret migration tickets are signed under (`cluster.secret`);
    /// `None` means this node neither issues nor accepts them.
    cluster_secret: std::sync::RwLock<Option<String>>,
    /// How long a caller's leg has to report after a REFER
    /// (`cluster.migration_timeout_secs`).
    migration_timeout: std::sync::RwLock<Duration>,
    /// Pre-loaded DTMF feedback audio: "Muted" / "Unmuted" PCM samples
    pub dtmf_muted_samples: Option<Arc<Vec<i16>>>,
    pub dtmf_unmuted_samples: Option<Arc<Vec<i16>>>,
    pub dtmf_locked_samples: Option<Arc<Vec<i16>>>,
    pub dtmf_unlocked_samples: Option<Arc<Vec<i16>>>,
    pub dtmf_ending_samples: Option<Arc<Vec<i16>>>,
    /// Recordings under way on this node, by room number (§11).
    recordings: DashMap<String, ActiveRecording>,
    /// Per-participant recordings under way: (room, participant) → row.
    participant_recordings: DashMap<(String, String), fcp_core::Recording>,
    /// The handler's own `Arc`, for the tasks it spawns that need the
    /// whole handler rather than a few clones — the moderator-release
    /// path, which may have to start a recording. Set once at startup by
    /// [`ConferenceCallHandler::set_self_ref`].
    self_ref: std::sync::RwLock<std::sync::Weak<Self>>,
}

/// A recording under way on this node: its registry row, the announcement
/// anyone joining while it runs must hear, and whatever is writing it.
pub struct ActiveRecording {
    /// The row the admin API lists and downloads by.
    pub record: fcp_core::Recording,
    /// The consent announcement, when the tenant's policy asks for one.
    consent_audio: Option<Arc<Vec<i16>>>,
    writer: RecordingWriter,
}

/// What is writing a recording.
enum RecordingWriter {
    /// forge-conference's own recorder on the room: mixed audio to WAV,
    /// as conference recordings have always been.
    Wav,
    /// This node's muxer: the composite and the mix to one WebM or MP4
    /// file.
    #[cfg(any(feature = "recording-webm", feature = "recording-mp4"))]
    Media(crate::recording::Recorder),
}

/// Tracks an active RFC 4575 conference event subscription.
struct ConferenceSubscription {
    /// The room number this subscription is for
    room_number: String,
    /// Contact URI of the subscriber (for sending NOTIFY)
    contact_uri: SipUri,
    /// When the subscription expires
    expires_at: Instant,
}

/// Tracks an answered conference call that is still collecting a room PIN.
struct PendingPinAuth {
    room_number: String,
    participant_id: String,
    info_digits_tx: tokio::sync::mpsc::UnboundedSender<char>,
}

/// Collects PIN digits from both RFC 2833 RTP and SIP INFO.
struct PinDtmfCollector {
    detector: Rfc2833Detector,
    dedup: DtmfDeduplicator,
    info_digits_rx: tokio::sync::mpsc::UnboundedReceiver<char>,
    decoder: Box<dyn AudioCodec>,
    inband_detector: GoertzelDetector,
    inband_resampler: Option<Resampler>,
    audio_payload_type: u8,
}

impl PinDtmfCollector {
    fn new(
        dtmf_clock_rate: u32,
        info_digits_rx: tokio::sync::mpsc::UnboundedReceiver<char>,
        decoder: Box<dyn AudioCodec>,
        audio_payload_type: u8,
    ) -> Self {
        let codec_rate = decoder.native_format().sample_rate;
        Self {
            detector: Rfc2833Detector::new(dtmf_clock_rate),
            dedup: DtmfDeduplicator::new(),
            info_digits_rx,
            inband_detector: GoertzelDetector::new(8000, 160),
            inband_resampler: if codec_rate != 8000 {
                Some(Resampler::new(codec_rate, 8000, 1).expect("valid 8kHz inband resampler"))
            } else {
                None
            },
            decoder,
            audio_payload_type,
        }
    }

    fn accept_info_digit(&mut self, ch: char) -> bool {
        let Ok(digit) = DtmfDigit::from_char(ch) else {
            return false;
        };
        let event = DtmfEvent::new(digit, DtmfEventType::End, DtmfMethod::SipInfo);
        self.dedup.should_publish(&event)
    }

    async fn drain_post_submit_dtmf(&mut self, rtp_socket: &RtpSocketPair, te_payload_type: u8) {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(250);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            tokio::select! {
                maybe_digit = self.info_digits_rx.recv() => {
                    let Some(ch) = maybe_digit else {
                        continue;
                    };
                    if self.accept_info_digit(ch) {
                        debug!(digit = %ch, "Draining INFO DTMF after PIN submit");
                    }
                }
                recv = tokio::time::timeout(remaining.min(Duration::from_millis(25)), rtp_socket.recv_rtp()) => {
                    match recv {
                        Ok(Ok((packet, _addr))) => {
                            if packet.header.payload_type() != te_payload_type {
                                continue;
                            }

                            if let Ok(events) = self.detector.process_with_timestamp(&packet.payload[..], packet.header.timestamp) {
                                for event in events {
                                    if event.event_type == DtmfEventType::End && self.dedup.should_publish(&event) {
                                        debug!(digit = %event.digit, "Draining RTP DTMF after PIN submit");
                                    }
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            debug!(error = %e, "RTP recv error while draining post-submit DTMF");
                            break;
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    }

    async fn collect_pin_dtmf(
        &mut self,
        rtp_socket: &RtpSocketPair,
        te_pt: Option<u8>,
        timeout: Duration,
    ) -> Option<String> {
        let te_payload_type = te_pt.unwrap_or(audio_bridge::DEFAULT_TE_PAYLOAD_TYPE);
        let mut digits = String::new();
        let max_digits = 20;
        let deadline = tokio::time::Instant::now() + timeout;
        let sleep = tokio::time::sleep_until(deadline);
        tokio::pin!(sleep);
        let mut rtp_packets_seen = 0u64;
        let mut te_packets_seen = 0u64;
        let mut audio_packets_seen = 0u64;
        let mut decode_errors = 0u64;
        let mut info_digits_seen = 0u64;
        let mut unexpected_payload_types = BTreeSet::new();
        let mut peak_sample: i16 = 0;
        let mut inband_events_seen = 0u64;

        loop {
            tokio::select! {
                _ = &mut sleep => {
                    debug!("PIN collection timed out after {} digits", digits.len());
                    log_pin_collector_summary(
                        "timeout",
                        &digits,
                        rtp_packets_seen,
                        te_packets_seen,
                        audio_packets_seen,
                        info_digits_seen,
                        decode_errors,
                        &unexpected_payload_types,
                        self.audio_payload_type,
                        te_payload_type,
                        peak_sample,
                        inband_events_seen,
                    );
                    return if digits.is_empty() { None } else { Some(digits) };
                }
                maybe_digit = self.info_digits_rx.recv() => {
                    let Some(ch) = maybe_digit else {
                        continue;
                    };
                    if !self.accept_info_digit(ch) {
                        continue;
                    }
                    info_digits_seen += 1;

                    if ch == '#' {
                        debug!(digits = %digits, "PIN entry completed with INFO #");
                        log_pin_collector_summary(
                            "info_submit",
                            &digits,
                            rtp_packets_seen,
                            te_packets_seen,
                            audio_packets_seen,
                            info_digits_seen,
                            decode_errors,
                            &unexpected_payload_types,
                            self.audio_payload_type,
                            te_payload_type,
                            peak_sample,
                            inband_events_seen,
                        );
                        self.drain_post_submit_dtmf(rtp_socket, te_payload_type).await;
                        return Some(digits);
                    }

                    digits.push(ch);
                    debug!(digit = %ch, collected = %digits, "INFO DTMF digit received");
                    if digits.len() >= max_digits {
                        self.drain_post_submit_dtmf(rtp_socket, te_payload_type).await;
                        return Some(digits);
                    }
                }
                recv = rtp_socket.recv_rtp() => {
                    match recv {
                        Ok((packet, _addr)) => {
                            rtp_packets_seen += 1;
                            let packet_pt = packet.header.payload_type();

                            if packet_pt == te_payload_type {
                                te_packets_seen += 1;
                                if let Ok(events) = self.detector.process_with_timestamp(&packet.payload[..], packet.header.timestamp) {
                                    for event in events {
                                        if event.event_type != DtmfEventType::End || !self.dedup.should_publish(&event) {
                                            continue;
                                        }

                                        if event.digit == DtmfDigit::Hash {
                                            debug!(digits = %digits, "PIN entry completed with RTP #");
                                            log_pin_collector_summary(
                                                "rtp_submit",
                                                &digits,
                                                rtp_packets_seen,
                                                te_packets_seen,
                                                audio_packets_seen,
                                                info_digits_seen,
                                                decode_errors,
                                                &unexpected_payload_types,
                                                self.audio_payload_type,
                                                te_payload_type,
                                                peak_sample,
                                                inband_events_seen,
                                            );
                                            self.drain_post_submit_dtmf(rtp_socket, te_payload_type).await;
                                            return Some(digits);
                                        }

                                        if let Some(ch) = dtmf_digit_to_char(&event.digit) {
                                            digits.push(ch);
                                            debug!(digit = %ch, collected = %digits, "DTMF digit received");
                                            if digits.len() >= max_digits {
                                                self.drain_post_submit_dtmf(rtp_socket, te_payload_type).await;
                                                return Some(digits);
                                            }
                                        }
                                    }
                                }
                                continue;
                            }

                            if packet_pt != self.audio_payload_type {
                                unexpected_payload_types.insert(packet_pt);
                                continue;
                            }
                            audio_packets_seen += 1;

                            match self.decoder.decode(&packet.payload[..]) {
                                Ok(pcm_samples) => {
                                    let detector_samples = if let Some(ref mut rs) = self.inband_resampler {
                                        match rs.resample(&pcm_samples) {
                                            Ok(samples) => samples,
                                            Err(e) => {
                                                debug!(error = %e, "Inband resample error during PIN collection");
                                                pcm_samples
                                            }
                                        }
                                    } else {
                                        pcm_samples
                                    };

                                    // Track peak sample for diagnostics
                                    if let Some(&max) = detector_samples.iter().max() {
                                        if max > peak_sample { peak_sample = max; }
                                    }
                                    if let Some(&min) = detector_samples.iter().min() {
                                        if (-min) > peak_sample { peak_sample = -min; }
                                    }

                                    if let Ok(events) = self.inband_detector.process_samples(&detector_samples) {
                                        inband_events_seen += events.len() as u64;
                                        for event in events {
                                            if event.event_type != DtmfEventType::End || !self.dedup.should_publish(&event) {
                                                continue;
                                            }

                                            if event.digit == DtmfDigit::Hash {
                                                debug!(digits = %digits, "PIN entry completed with inband #");
                                                log_pin_collector_summary(
                                                    "inband_submit",
                                                    &digits,
                                                    rtp_packets_seen,
                                                    te_packets_seen,
                                                    audio_packets_seen,
                                                    info_digits_seen,
                                                    decode_errors,
                                                    &unexpected_payload_types,
                                                    self.audio_payload_type,
                                                    te_payload_type,
                                                    peak_sample,
                                                    inband_events_seen,
                                                );
                                                self.drain_post_submit_dtmf(rtp_socket, te_payload_type).await;
                                                return Some(digits);
                                            }

                                            if let Some(ch) = dtmf_digit_to_char(&event.digit) {
                                                digits.push(ch);
                                                debug!(digit = %ch, collected = %digits, "Inband DTMF digit received");
                                                if digits.len() >= max_digits {
                                                    self.drain_post_submit_dtmf(rtp_socket, te_payload_type).await;
                                                    return Some(digits);
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    decode_errors += 1;
                                    debug!(error = %e, pt = packet_pt, "Decode error during PIN collection");
                                }
                            }
                        }
                        Err(e) => {
                            debug!(error = %e, "RTP recv error during PIN collection");
                            log_pin_collector_summary(
                                "rtp_error",
                                &digits,
                                rtp_packets_seen,
                                te_packets_seen,
                                audio_packets_seen,
                                info_digits_seen,
                                decode_errors,
                                &unexpected_payload_types,
                                self.audio_payload_type,
                                te_payload_type,
                                peak_sample,
                                inband_events_seen,
                            );
                            return None;
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn log_pin_collector_summary(
    result: &str,
    digits: &str,
    rtp_packets_seen: u64,
    te_packets_seen: u64,
    audio_packets_seen: u64,
    info_digits_seen: u64,
    decode_errors: u64,
    unexpected_payload_types: &BTreeSet<u8>,
    audio_payload_type: u8,
    te_payload_type: u8,
    peak_sample: i16,
    inband_events_seen: u64,
) {
    info!(
        result,
        digits = %digits,
        rtp_packets_seen,
        te_packets_seen,
        audio_packets_seen,
        info_digits_seen,
        decode_errors,
        inband_events_seen,
        peak_sample,
        unexpected_payload_types = ?unexpected_payload_types,
        audio_payload_type,
        te_payload_type,
        "PIN collector summary"
    );
}

impl ConferenceCallHandler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bridge: Arc<ConferenceBridge>,
        storage: Arc<DynStorage>,
        default_tenant_id: TenantId,
        conference_config: ConferenceSection,
        local_uri: String,
        local_addr: String,
        port_pool: Arc<PortPool>,
        event_bus: Arc<dyn EventPublisher>,
        uac: Arc<SipUserAgentClient>,
        ai_config: AiSection,
        node_id: String,
    ) -> Self {
        // Parse the local SIP URI once at construction time — avoids runtime .expect()
        let local_sip_uri = Uri::parse(&local_uri)
            .ok()
            .and_then(|u| u.as_sip().cloned())
            .unwrap_or_else(|| {
                warn!(uri = %local_uri, "Failed to parse local SIP URI at construction, using fallback");
                SipUri::parse("sip:conference@localhost")
                    .unwrap_or_else(|_| SipUri::parse("sip:x@127.0.0.1").unwrap())
            });

        Self {
            bridge,
            storage,
            default_tenant_id,
            conference_config: RwLock::new(conference_config),
            sessions: DashMap::new(),
            local_uri,
            local_addr,
            port_pool,
            event_bus,
            uac,
            dtmf_handlers: DashMap::new(),
            meeting_cache: DashMap::new(),
            ai_session_manager: Arc::new(AISessionManager::new()),
            ai_config: RwLock::new(ai_config),
            invite_rate: DashMap::new(),
            conference_subscriptions: DashMap::new(),
            local_sip_uri,
            draining: std::sync::atomic::AtomicBool::new(false),
            trunk_srtp: std::sync::atomic::AtomicBool::new(true),
            room_sessions: DashMap::new(),
            node_id,
            room_registry: std::sync::RwLock::new(None),
            handler_latency: HandlerLatency::new(),
            tts_service: None,
            video_backend: None,
            video_capacity: None,
            video_event_pumps: Arc::new(DashMap::new()),
            bfcp: Arc::new(Floors::new()),
            speaker_claims: DashMap::new(),
            content_claims: DashMap::new(),
            pending_pin_auth: DashMap::new(),
            cdr: None,
            region: None,
            migrations: migration::Migrations::default(),
            used_tickets: migration::UsedTickets::default(),
            cluster_secret: std::sync::RwLock::new(None),
            migration_timeout: std::sync::RwLock::new(Duration::from_secs(15)),
            dtmf_muted_samples: None,
            dtmf_unmuted_samples: None,
            dtmf_locked_samples: None,
            dtmf_unlocked_samples: None,
            dtmf_ending_samples: None,
            recordings: DashMap::new(),
            participant_recordings: DashMap::new(),
            self_ref: std::sync::RwLock::new(std::sync::Weak::new()),
        }
    }

    /// The node's video budget (design §9.3): joins that would not fit are
    /// refused, and the budget is re-priced from the rooms periodically.
    pub fn set_video_capacity(&mut self, capacity: Arc<crate::video_capacity::VideoCapacity>) {
        self.video_capacity = Some(capacity);
    }

    /// The floor control every room's BFCP endpoints share, for the
    /// sockets `main` serves it on.
    pub fn bfcp(&self) -> Arc<Floors> {
        Arc::clone(&self.bfcp)
    }

    pub fn video_capacity(&self) -> Option<&Arc<crate::video_capacity::VideoCapacity>> {
        self.video_capacity.as_ref()
    }

    /// Re-price the video budget from every active room; `None` without a
    /// budget. Returns the units committed, per device.
    pub fn recompute_video_capacity(&self) -> Option<crate::video_capacity::Cost> {
        let capacity = self.video_capacity.as_ref()?;
        let rooms: Vec<Arc<forge_conference::ConferenceRoom>> = self
            .active_rooms()
            .into_iter()
            .filter_map(|(room, _)| self.bridge.get_room(&room).ok())
            .collect();
        Some(capacity.recompute(rooms))
    }

    /// Give rooms a video backend; without one every meeting is audio-only.
    pub fn set_video_backend(&mut self, backend: VideoBackend) {
        self.video_backend = Some(Arc::new(backend));
    }

    /// Set the TTS service for synthesizing prompts from meeting text fields.
    pub fn set_tts_service(&mut self, tts: Arc<fcp_media::TtsService>) {
        self.tts_service = Some(tts);
    }

    /// Signal that this handler should reject new INVITEs (graceful shutdown draining).
    /// Attach the cluster conference registry hook.
    pub fn set_room_registry(&self, registry: Arc<dyn RoomRegistry>) {
        *self.room_registry.write().unwrap() = Some(registry);
    }

    /// Local participants of a room that count towards placement: real
    /// callers only, not the `__…` virtual participants (audio feedback,
    /// AI, inter-node trunks).
    pub fn countable_participants(&self, room_number: &str) -> usize {
        self.room_participants(room_number)
            .map(|ids| ids.iter().filter(|id| !id.starts_with("__")).count())
            .unwrap_or(0)
    }

    /// Push this node's current participant count for a room to the
    /// registry hook (no-op without clustering).
    pub fn sync_room_registry(&self, room_number: &str) {
        let Some(registry) = self.room_registry.read().unwrap().clone() else {
            return;
        };
        let tenant = self
            .tenant_for_room(room_number)
            .unwrap_or(self.default_tenant_id)
            .to_string();
        registry.room_participants_changed(
            tenant,
            room_number.to_string(),
            self.countable_participants(room_number),
        );
        registry.node_callers_changed(self.node_callers());
    }

    /// Callers on this node across every room (trunks and virtual
    /// participants excluded).
    pub fn node_callers(&self) -> usize {
        self.active_rooms()
            .iter()
            .map(|(room, _)| self.countable_participants(room))
            .sum()
    }

    pub fn start_draining(&self) {
        self.draining.store(true, Ordering::Relaxed);
        info!("Conference handler entering drain mode — rejecting new INVITEs");
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Relaxed)
    }

    // ---- participant migration (phase 5) ---------------------------------

    /// The secret migration tickets are minted and verified under, and
    /// how long a REFER may wait for its report. No secret, no migration.
    pub fn set_migration(&self, secret: Option<String>, timeout: Duration) {
        *self.cluster_secret.write().unwrap() = secret.filter(|s| !s.is_empty());
        *self.migration_timeout.write().unwrap() = timeout;
    }

    pub fn migration_enabled(&self) -> bool {
        self.cluster_secret.read().unwrap().is_some()
    }

    /// The callers this node could move: every session that is a caller
    /// (not a trunk), admitted (not in the waiting room) and not already
    /// being moved; with the meeting's video need, for placement.
    pub async fn migration_candidates(&self) -> Vec<migration::Candidate> {
        let sessions: Vec<(String, TenantId, String, String)> = self
            .sessions
            .iter()
            .filter(|e| e.trunk_peer.is_none())
            .map(|e| {
                (
                    e.key().clone(),
                    e.tenant_id,
                    e.room_number.clone(),
                    e.participant_id.clone(),
                )
            })
            .collect();
        let mut out = Vec::with_capacity(sessions.len());
        for (call_id, tenant_id, room_number, participant_id) in sessions {
            if self.migrations.is_migrating(&call_id) {
                continue;
            }
            if let Ok(room) = self.bridge.get_room(&room_number) {
                if room.waiting_participants().contains(&participant_id) {
                    continue;
                }
            }
            let video = match self.lookup_meeting(&tenant_id, &room_number).await {
                Ok(Some(m)) if m.video_enabled => Some(fcp_cluster::VideoNeed::of(
                    m.video_codecs
                        .iter()
                        .map(|c| c.sdp_name().to_string())
                        .collect(),
                )),
                _ => None,
            };
            out.push(migration::Candidate {
                call_id,
                tenant_id,
                room_number,
                video,
            });
        }
        out
    }

    /// Ask the holder of `call_id`'s leg to move the caller to `target`:
    /// a REFER with the room on the target and a ticket for the
    /// participant as they are here — role, mute, gain, video — in the
    /// `Refer-To`. Accepted (2xx) means the move is in flight and the
    /// ledger waits for its report; anything else leaves the caller here.
    pub async fn refer_to_node(
        &self,
        call_id: &str,
        target: &fcp_cluster::ClusterMember,
    ) -> Result<()> {
        let secret = self
            .cluster_secret
            .read()
            .unwrap()
            .clone()
            .context("cluster.secret is not set")?;
        let (mut dialog, room_number, ticket) = {
            let s = self
                .sessions
                .get(call_id)
                .with_context(|| format!("no session for {call_id}"))?;
            let room = self
                .bridge
                .get_room(&s.room_number)
                .map_err(|e| anyhow::anyhow!("room {}: {e}", s.room_number))?;
            let meta = room.get_participant_metadata(&s.participant_id).ok();
            let video_on = room
                .video()
                .and_then(|v| v.participant(&s.participant_id))
                .map(|i| i.state.name() != "disabled")
                .unwrap_or(true);
            let ticket = MigrationTicket {
                version: fcp_core::migration_ticket::MIGRATION_TICKET_VERSION,
                tenant_id: s.tenant_id,
                room_number: s.room_number.clone(),
                participant_id: s.participant_id.clone(),
                display_name: s.display_name.clone(),
                caller_id: s.caller_id.clone(),
                is_host: s.is_host,
                muted: meta
                    .as_ref()
                    .is_some_and(|m| m.state == forge_mixer::ParticipantState::Muted),
                gain: meta.as_ref().map(|m| m.gain).unwrap_or(1.0),
                video: video_on,
                token_join: s.joined_by_token,
                from_node: self.node_id.clone(),
                expires_at: MigrationTicket::expiry_from_now(),
                nonce: MigrationTicket::new_nonce(),
            };
            (
                s.dialog.clone(),
                s.room_number.clone(),
                ticket.mint(secret.as_bytes()),
            )
        };
        let refer_to = migration::refer_to(&room_number, target, &ticket);
        let refer_to =
            SipUri::parse(&refer_to).map_err(|e| anyhow::anyhow!("Refer-To {refer_to}: {e}"))?;
        let (response, _subscription) = tokio::time::timeout(
            migration::REFER_TIMEOUT,
            self.uac.send_refer(&mut dialog, &refer_to, None),
        )
        .await
        .context("REFER unanswered")??;
        let code = response.start_line().code();
        if !(200..300).contains(&code) {
            bail!("REFER refused with {code}");
        }
        if let Some(mut s) = self.sessions.get_mut(call_id) {
            s.dialog = dialog;
        }
        let timeout = *self.migration_timeout.read().unwrap();
        self.migrations.begin(call_id, &target.node_id, timeout);
        info!(
            call_id = %call_id,
            room = %room_number,
            to = %target.node_id,
            "Participant referred to another node"
        );
        Ok(())
    }

    /// REFERs still waiting for their report.
    pub fn migrations_pending(&self) -> usize {
        self.migrations.pending()
    }

    /// Give up on the REFERs whose holder never reported; those callers
    /// stay and are hung up with the rest.
    pub fn expire_migrations(&self) -> Vec<String> {
        let expired = self.migrations.expire(Instant::now());
        for call_id in &expired {
            warn!(call_id = %call_id, "Migration unreported in time; the caller stays");
        }
        expired
    }

    /// The caller is on the other node: drop this leg.
    async fn finish_migration(&self, call_id: &str) {
        if let Some(dialog) = self.sessions.get(call_id).map(|s| s.dialog.clone()) {
            match tokio::time::timeout(BYE_TIMEOUT, self.uac.bye(&dialog)).await {
                Ok(Ok(_)) | Ok(Err(_)) => {}
                Err(_) => debug!(call_id = %call_id, "BYE after migration unanswered"),
            }
        }
        self.remove_session_for(
            call_id,
            fcp_sip::TerminationReason::system(
                fcp_sip::SystemCause::Migration,
                "Moved to another node",
            ),
        );
    }

    /// Maximum number of entries in the meeting cache (from config).
    fn max_meeting_cache_size(&self) -> usize {
        self.conference_config.read().meeting_cache_max_size
    }

    /// Look up a meeting by room number, with in-memory cache and DB fallback.
    ///
    /// Cache strategy:
    /// - On cache hit within TTL: return cached meeting (no DB query)
    /// - On cache miss or expired: query DB, update cache on success
    /// - On DB failure: return stale cached entry if available (graceful degradation)
    /// - Cache is bounded to MAX_MEETING_CACHE_SIZE entries; expired entries evicted on overflow
    pub async fn lookup_meeting(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
    ) -> Result<Option<Meeting>> {
        use fcp_core::storage::ConferenceStore;

        let cache_key = (*tenant_id, room_number.to_string());
        let ttl = Duration::from_secs(self.conference_config.read().cache_ttl_secs);

        // Check cache first (if TTL > 0)
        if ttl > Duration::ZERO {
            if let Some(entry) = self.meeting_cache.get(&cache_key) {
                let (meeting, cached_at) = entry.value();
                if cached_at.elapsed() < ttl {
                    return Ok(Some(meeting.clone()));
                }
            }
        }

        // Query database
        match self
            .storage
            .get_meeting_by_room_number(tenant_id, room_number)
            .await
        {
            Ok(Some(meeting)) => {
                if ttl > Duration::ZERO {
                    // Evict expired entries if cache is at capacity
                    if self.meeting_cache.len() >= self.max_meeting_cache_size() {
                        self.meeting_cache
                            .retain(|_, (_, cached_at)| cached_at.elapsed() < ttl);
                        // If still over limit after TTL eviction, drop oldest half
                        if self.meeting_cache.len() >= self.max_meeting_cache_size() {
                            let mut count = 0;
                            let to_remove = self.meeting_cache.len() / 2;
                            self.meeting_cache.retain(|_, _| {
                                count += 1;
                                count > to_remove
                            });
                        }
                    }
                    self.meeting_cache
                        .insert(cache_key, (meeting.clone(), Instant::now()));
                }
                Ok(Some(meeting))
            }
            Ok(None) => {
                // Meeting doesn't exist — remove stale cache entry if any
                self.meeting_cache.remove(&cache_key);
                Ok(None)
            }
            Err(e) => {
                // DB failure — return stale cache if available
                if let Some(entry) = self.meeting_cache.get(&cache_key) {
                    let (meeting, cached_at) = entry.value();
                    warn!(
                        room = %room_number,
                        stale_secs = cached_at.elapsed().as_secs(),
                        error = %e,
                        "DB lookup failed, using stale cached meeting"
                    );
                    return Ok(Some(meeting.clone()));
                }
                Err(e)
            }
        }
    }

    /// Invalidate cached meeting entry (call after update/delete).
    pub fn invalidate_meeting_cache(&self, tenant_id: &TenantId, room_number: &str) {
        self.meeting_cache
            .remove(&(*tenant_id, room_number.to_string()));
    }

    fn session_tenant_for_room(&self, room_number: &str) -> Option<TenantId> {
        let call_ids = self.room_sessions.get(room_number)?.clone();
        for call_id in call_ids {
            if let Some(session) = self.sessions.get(&call_id) {
                return Some(session.tenant_id);
            }
        }
        None
    }

    pub fn tenant_for_room(&self, room_number: &str) -> Option<TenantId> {
        self.session_tenant_for_room(room_number).or_else(|| {
            self.meeting_cache.iter().find_map(|entry| {
                let key = entry.key();
                if key.1 == room_number {
                    Some(key.0)
                } else {
                    None
                }
            })
        })
    }

    fn extract_tenant_domain(request: &sip_core::Request) -> Option<String> {
        let request_uri = request.uri().to_string();
        let uri_str = request_uri
            .strip_prefix("sip:")
            .or_else(|| request_uri.strip_prefix("sips:"))
            .unwrap_or(&request_uri);

        if let Some(hostport) = uri_str.split('@').nth(1) {
            // Strip URI parameters first, then strip port.
            let no_params = hostport.split(';').next().unwrap_or(hostport).trim();
            // IPv6 brackets: [::1]:5074 → [::1]; IPv4/hostname: 1.2.3.4:5074 → 1.2.3.4
            let domain = if no_params.starts_with('[') {
                no_params
                    .find(']')
                    .map(|end| &no_params[..=end])
                    .unwrap_or(no_params)
            } else {
                no_params.split(':').next().unwrap_or(no_params)
            };
            if !domain.is_empty() && domain != "localhost" && domain != "127.0.0.1" {
                return Some(domain.to_string());
            }
        }

        let to_header = request.headers().get("To")?;
        let to_str = to_header.to_string();
        let uri_str = if let Some(start) = to_str.find('<') {
            if let Some(end) = to_str.find('>') {
                &to_str[start + 1..end]
            } else {
                &to_str
            }
        } else {
            &to_str
        };

        let parsed = sip_core::SipUri::parse(uri_str.trim()).ok()?;
        let domain = parsed.host().to_string();
        if domain.is_empty() || domain == "localhost" || domain == "127.0.0.1" {
            None
        } else {
            Some(domain)
        }
    }

    async fn resolve_tenant(&self, request: &sip_core::Request) -> Result<TenantId> {
        let resolved = if let Some(domain) = Self::extract_tenant_domain(request) {
            if fcp_core::is_host_literal(&domain) {
                // IP literal — use default tenant directly.  Internal
                // service-to-service INVITEs (call-manager → conference)
                // use the server IP in the Request-URI.
                self.default_tenant_id
            } else if let Some(tenant) = self.storage.get_tenant_by_sip_domain(&domain).await? {
                tenant.id
            } else if let Some(tenant) = self.storage.get_tenant_by_name(&domain).await? {
                warn!(
                    domain = %domain,
                    tenant_id = %tenant.id,
                    "Conference tenant resolved via legacy name match; add the domain to sip_domains"
                );
                tenant.id
            } else {
                bail!("unknown tenant domain '{}'", domain);
            }
        } else {
            self.default_tenant_id
        };

        Ok(resolved)
    }

    // Kept as part of the module's intended API; not currently called from this binary.
    #[allow(dead_code)]
    async fn default_tenant_fallback(&self) -> Result<Option<TenantId>> {
        let tenants = self
            .storage
            .list_tenants()
            .await
            .context("failed to list tenants for default tenant fallback")?;
        if tenants.is_empty() || (tenants.len() == 1 && tenants[0].id == self.default_tenant_id) {
            Ok(Some(self.default_tenant_id))
        } else {
            Ok(None)
        }
    }

    /// Check if a call ID belongs to an active conference session.
    #[allow(dead_code)]
    pub fn has_session(&self, call_id: &str) -> bool {
        self.sessions.contains_key(call_id)
    }

    /// Get call IDs for a specific room (O(1) via secondary index).
    #[allow(dead_code)]
    pub fn sessions_for_room(&self, room_number: &str) -> Vec<String> {
        self.room_sessions
            .get(room_number)
            .map(|ids| ids.clone())
            .unwrap_or_default()
    }

    /// Get the total number of active participant sessions.
    #[allow(dead_code)]
    pub fn active_sessions(&self) -> usize {
        self.sessions.len()
    }

    /// List all active rooms with participant counts.
    pub fn active_rooms(&self) -> Vec<(String, usize)> {
        self.bridge
            .list_rooms()
            .into_iter()
            .filter_map(|room_id| {
                self.bridge
                    .get_room(&room_id)
                    .ok()
                    .map(|room| (room_id, room.participant_count()))
            })
            .collect()
    }

    /// Get the number of active RFC 4575 conference event subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.conference_subscriptions.len()
    }

    /// Get participant list for a specific room.
    #[allow(dead_code)]
    pub fn room_participants(&self, room_number: &str) -> Option<Vec<String>> {
        self.bridge
            .get_room(room_number)
            .ok()
            .map(|room| room.participants())
    }

    /// Get detailed metadata for a specific participant in a room.
    pub fn get_participant_details(
        &self,
        room_number: &str,
        participant_id: &str,
    ) -> Result<serde_json::Value> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        let meta = room
            .get_participant_metadata(participant_id)
            .map_err(|e| anyhow::anyhow!("Participant not found: {}", e))?;

        // Enrich with session data (codec, remote addr, stats)
        let session_data = self
            .sessions
            .iter()
            .find(|entry| entry.value().participant_id == participant_id);

        let mut json = serde_json::json!({
            "participant_id": meta.id,
            "state": format!("{:?}", meta.state),
            "gain": meta.gain,
            "is_recording": meta.is_recording,
            "packets_received": meta.packets_received,
            "is_speaking": meta.is_speaking,
            "is_host": room.is_host(participant_id),
        });

        if let Some(entry) = session_data {
            let session = entry.value();
            let joined_secs = session
                .joined_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            json["codec"] = serde_json::json!(session.codec_name);
            json["remote_addr"] = serde_json::json!(session.remote_addr.to_string());
            json["joined_at"] = serde_json::json!(joined_secs);
            json["call_id"] = serde_json::json!(session.call_id);
            if let Some(ref dn) = session.display_name {
                json["display_name"] = serde_json::json!(dn);
            }
            if let Some(ref cid) = session.caller_id {
                json["caller_id"] = serde_json::json!(cid);
            }
            json["is_dial_out"] = serde_json::json!(session.is_dial_out);
            let silence = session.stats.silence_frames.load(Ordering::Relaxed);
            json["stats"] = serde_json::json!({
                "packets_received": session.stats.packets_received.load(Ordering::Relaxed),
                "packets_decoded": session.stats.packets_decoded.load(Ordering::Relaxed),
                "decode_errors": session.stats.decode_errors.load(Ordering::Relaxed),
                "packets_sent": session.stats.packets_sent.load(Ordering::Relaxed),
                "encode_errors": session.stats.encode_errors.load(Ordering::Relaxed),
                "dtmf_events": session.stats.dtmf_events.load(Ordering::Relaxed),
                "silence_frames": silence,
            });
            if let Some(mos) = session.stats.estimated_mos() {
                json["estimated_mos"] = serde_json::json!((mos * 100.0).round() / 100.0);
            }
            // VAD presence: inactive if >30s silence (1500 frames at 50fps)
            json["is_inactive"] = serde_json::json!(session.stats.is_inactive(1500));
            json["video"] = video_bridge::participant_video_json(
                &room,
                participant_id,
                session.video.as_ref(),
                session.content.as_ref(),
            );
        }

        Ok(json)
    }

    /// The room's video and each participant's, for `GET
    /// /conferences/{room}/video` (design §12): what the live view needs
    /// and nothing of the audio. Trunk participants are left out; they
    /// carry another node's mix. `Err` when the room is not active here.
    pub fn get_room_video(&self, room_number: &str) -> Result<serde_json::Value> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        let participants: Vec<serde_json::Value> = room
            .get_all_participant_metadata()
            .iter()
            .filter(|meta| !crate::trunks::is_trunk_participant(&meta.id))
            .map(|meta| {
                let session = self
                    .sessions
                    .iter()
                    .find(|e| e.value().participant_id == meta.id);
                let mut p = serde_json::json!({
                    "participant_id": meta.id,
                    "is_speaking": meta.is_speaking,
                    "muted": matches!(meta.state, forge_mixer::ParticipantState::Muted),
                });
                match session {
                    Some(entry) => {
                        let session = entry.value();
                        p["is_host"] = serde_json::json!(session.is_host);
                        p["is_dial_out"] = serde_json::json!(session.is_dial_out);
                        if let Some(ref dn) = session.display_name {
                            p["display_name"] = serde_json::json!(dn);
                        }
                        if let Some(ref cid) = session.caller_id {
                            p["caller_id"] = serde_json::json!(cid);
                        }
                        p["video"] = video_bridge::participant_video_json(
                            &room,
                            &meta.id,
                            session.video.as_ref(),
                            session.content.as_ref(),
                        );
                        if let Some(a) = &session.bfcp {
                            p["bfcp"] = self
                                .bfcp
                                .participant_json(room_number, a.user_id)
                                .unwrap_or(serde_json::Value::Null);
                        }
                    }
                    None => {
                        p["video"] =
                            video_bridge::participant_video_json(&room, &meta.id, None, None);
                    }
                }
                p
            })
            .collect();
        Ok(serde_json::json!({
            "room_number": room_number,
            "node_id": self.node_id(),
            "participant_count": room.participant_count(),
            "participants": participants,
            // Trunks are not participants, but whether one carries video
            // decides whether the peer's callers are on screen at all.
            "trunks": self.room_trunk_json(room_number),
            "video": video_bridge::room_video_json(&room),
        }))
    }

    /// Apply a peer's claim about who is speaking behind its remote tile
    /// (§15.4, 5c). `Ok(false)` when the room is here but has no video
    /// or no tile for that peer yet — a claim that arrives before the
    /// trunk's video is nothing to worry about.
    pub fn apply_speaker_claim(
        &self,
        room_number: &str,
        node: &str,
        participant_id: Option<&str>,
        display_name: Option<&str>,
        energy: f32,
        speaking: bool,
    ) -> Result<bool> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|_| anyhow::anyhow!("room {room_number} is not hosted on this node"))?;
        let Some(video) = room.video() else {
            return Ok(false);
        };
        let tile = trunks::trunk_participant_id(node);
        if !video.is_remote(&tile) {
            return Ok(false);
        }
        let claim = participant_id.map(|id| forge_conference::video::RemoteSpeaker {
            node: node.to_string(),
            participant_id: id.to_string(),
            display_name: display_name.unwrap_or(id).to_string(),
            energy,
            speaking,
        });
        video.set_remote_speaker(&tile, claim);
        self.speaker_claims
            .insert(format!("{room_number}|{node}"), Instant::now());
        Ok(true)
    }

    /// Drop the claims of peers that have stopped refreshing them. A
    /// node whose last word was "my caller is speaking" must not hold
    /// the floor of every other node for good if it goes quiet or its
    /// trunk is slow to be noticed (§15.4, 5c).
    pub fn expire_speaker_claims(&self, max_age: Duration) {
        let stale: Vec<String> = self
            .speaker_claims
            .iter()
            .filter(|e| e.value().elapsed() > max_age)
            .map(|e| e.key().clone())
            .collect();
        for key in stale {
            let Some((room_number, node)) = key.split_once('|') else {
                self.speaker_claims.remove(&key);
                continue;
            };
            if let Ok(room) = self.bridge.get_room(room_number) {
                if let Some(video) = room.video() {
                    video.set_remote_speaker(&trunks::trunk_participant_id(node), None);
                }
            }
            debug!(room = %room_number, peer = %node, "Speaker claim expired");
            self.speaker_claims.remove(&key);
        }
    }

    /// A peer's claim to a room's content floor (§15.6, 7d): who is
    /// sharing there and since when. Every node keeps the earliest claim
    /// — the peer's or its own — and a node whose own presenter loses
    /// takes the floor from them (`replaced`) so the peer's screen,
    /// arriving as a remote content source, can have it. A claim with no
    /// participant withdraws the peer's share; the floor it held here is
    /// released. `Ok(false)` when the room is here but has no video or
    /// no trunk to that peer yet.
    pub fn apply_content_claim(
        &self,
        room_number: &str,
        node: &str,
        participant_id: Option<&str>,
        display_name: Option<&str>,
        since_ms: u64,
    ) -> Result<bool> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|_| anyhow::anyhow!("room {room_number} is not hosted on this node"))?;
        let Some(video) = room.video() else {
            return Ok(false);
        };
        let tile = trunks::trunk_participant_id(node);
        if !video.is_remote(&tile) {
            return Ok(false);
        }
        let key = format!("{room_number}|{node}");
        let Some(participant_id) = participant_id else {
            self.content_claims.remove(&key);
            if video.release_content_with(&tile, forge_conference::video::ContentStop::Ended) {
                debug!(room = %room_number, peer = %node, "The peer's share ended; floor released");
            }
            return Ok(true);
        };
        let claim = ContentClaim {
            participant_id: participant_id.to_string(),
            display_name: display_name.unwrap_or(participant_id).to_string(),
            since_ms,
        };
        let changed = self
            .content_claims
            .insert(key, (claim.clone(), Instant::now()))
            .is_none_or(|(old, _)| old != claim);
        match video.content() {
            // Our own presenter against the peer's: the earlier share
            // keeps the floor room-wide; the loser is told `replaced`.
            Some(ours) if !ours.remote => {
                let ours_since = unix_ms().saturating_sub(ours.held_for.as_millis() as u64);
                if claim_beats(
                    (since_ms, node, participant_id),
                    (ours_since, &self.node_id, &ours.participant_id),
                ) {
                    info!(
                        room = %room_number,
                        ours = %ours.participant_id,
                        theirs = %participant_id,
                        peer = %node,
                        "The peer's share is older; ours is replaced"
                    );
                    video.release_content_with(
                        &ours.participant_id,
                        forge_conference::video::ContentStop::Replaced,
                    );
                }
            }
            // The peer's screen holds the floor here already and its
            // packets came before its claim: the browsers were told the
            // tile's name, so they are told the presenter's now.
            Some(theirs) if changed && theirs.participant_id == tile => {
                self.floor_changed(
                    room_number,
                    FloorNotice {
                        floor: "granted",
                        participant_id: tile,
                        presenter: Some(claim.display_name),
                        reason: None,
                    },
                );
            }
            _ => {}
        }
        Ok(true)
    }

    /// Drop the content claims of peers that have stopped refreshing
    /// them, releasing the floor a quiet peer's screen holds here
    /// (§15.6, 7d) — it will not be arriving.
    pub fn expire_content_claims(&self, max_age: Duration) {
        let stale: Vec<String> = self
            .content_claims
            .iter()
            .filter(|e| e.value().1.elapsed() > max_age)
            .map(|e| e.key().clone())
            .collect();
        for key in stale {
            self.content_claims.remove(&key);
            let Some((room_number, node)) = key.split_once('|') else {
                continue;
            };
            if let Ok(room) = self.bridge.get_room(room_number) {
                if let Some(video) = room.video() {
                    video.release_content_with(
                        &trunks::trunk_participant_id(node),
                        forge_conference::video::ContentStop::Ended,
                    );
                }
            }
            debug!(room = %room_number, peer = %node, "Content claim expired");
        }
    }

    /// This node's own share in a room, as a claim to send to the peers
    /// hosting it. `None` when the floor is free or a peer's (their
    /// node makes that claim).
    pub fn local_content_claim(&self, room_number: &str) -> Option<serde_json::Value> {
        let room = self.bridge.get_room(room_number).ok()?;
        let content = room.video()?.content().filter(|c| !c.remote)?;
        Some(serde_json::json!({
            "node": self.node_id,
            "participant_id": content.participant_id,
            "display_name": content.display_name,
            "since_ms": unix_ms().saturating_sub(content.held_for.as_millis() as u64),
        }))
    }

    /// This node's own speaker in a room, as a claim to send to the
    /// peers hosting it: the participant the room is showing, with the
    /// level the mixer has for them right now. `None` when the room's
    /// speaker is a peer's caller (their node makes that claim) or there
    /// is none.
    pub fn local_speaker_claim(&self, room_number: &str) -> Option<serde_json::Value> {
        let room = self.bridge.get_room(room_number).ok()?;
        let video = room.video()?;
        // A speaker reached through a remote tile is the peer's to claim.
        if video.active_speaker_via().is_some() {
            return None;
        }
        let participant_id = video.active_speaker()?;
        let meta = room
            .get_all_participant_metadata()
            .into_iter()
            .find(|m| m.id == participant_id);
        let display_name = self
            .sessions
            .iter()
            .find(|e| e.value().participant_id == participant_id)
            .and_then(|e| e.value().display_name.clone())
            .unwrap_or_else(|| participant_id.clone());
        Some(serde_json::json!({
            "node": self.node_id,
            "participant_id": participant_id,
            "display_name": display_name,
            "energy": meta.as_ref().map(|m| m.energy).unwrap_or(0.0),
            "speaking": meta.as_ref().map(|m| m.is_speaking).unwrap_or(false),
        }))
    }

    /// Rooms this node hosts that both have video and are trunked to a
    /// peer: the only ones whose speaker anybody else needs to hear about.
    pub fn rooms_with_trunked_video(&self) -> Vec<(String, TenantId)> {
        self.room_sessions
            .iter()
            .filter_map(|entry| {
                let room_number = entry.key().clone();
                let has_trunk = entry.value().iter().any(|call_id| {
                    self.sessions
                        .get(call_id)
                        .map(|s| s.trunk_peer.is_some())
                        .unwrap_or(false)
                });
                if !has_trunk {
                    return None;
                }
                let room = self.bridge.get_room(&room_number).ok()?;
                room.video()?;
                let tenant_id = self.tenant_for_room(&room_number)?;
                Some((room_number, tenant_id))
            })
            .collect()
    }

    /// The trunks of a room as the video API reports them: the peer, and
    /// whether the trunk carries that peer's composite as well as its mix.
    fn room_trunk_json(&self, room_number: &str) -> Vec<serde_json::Value> {
        let Some(call_ids) = self.room_sessions.get(room_number) else {
            return Vec::new();
        };
        let mut trunks: Vec<serde_json::Value> = call_ids
            .iter()
            .filter_map(|call_id| self.sessions.get(call_id))
            .filter_map(|session| {
                let peer = session.trunk_peer.clone()?;
                Some(serde_json::json!({
                    "peer": peer,
                    "participant_id": session.participant_id,
                    "video": session.video.is_some(),
                    "video_codec": session
                        .video
                        .as_ref()
                        .map(|v| v.negotiated.codec.sdp_name()),
                    // Whether a shared screen crosses this trunk (7d).
                    "content": session.content.is_some(),
                }))
            })
            .collect();
        trunks.sort_by(|a, b| a["peer"].as_str().cmp(&b["peer"].as_str()));
        trunks
    }

    /// List participants waiting for moderator in a room.
    pub fn list_waiting(&self, room_number: &str) -> Result<Vec<String>> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        Ok(room.waiting_participants())
    }

    /// Promote a participant to host (releases waiting participants).
    pub fn promote_participant(&self, room_number: &str, participant_id: &str) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        room.promote_to_host(participant_id)
            .map_err(|e| anyhow::anyhow!("Failed to promote: {}", e))
    }

    /// Admit a specific waiting participant into the conference (admin bypass).
    pub fn admit_waiting(&self, room_number: &str, participant_id: &str) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        room.admit_waiting_participant(participant_id)
            .map_err(|e| anyhow::anyhow!("Failed to admit: {}", e))
    }

    /// Admit all waiting participants into the conference (admin bypass).
    pub fn admit_all_waiting(&self, room_number: &str) -> Result<usize> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        Ok(room.admit_all_waiting())
    }

    /// Set participant audio gain.
    ///
    /// Gain must be a finite value between 0.0 and 10.0 (inclusive).
    pub fn set_gain(&self, room_number: &str, participant_id: &str, gain: f32) -> Result<()> {
        if !gain.is_finite() || gain < 0.0 || gain > audio_bridge::MAX_GAIN {
            anyhow::bail!(
                "gain must be finite and in range [0.0, {}], got {}",
                audio_bridge::MAX_GAIN,
                gain
            );
        }
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        room.set_participant_gain(participant_id, gain)
            .map_err(|e| anyhow::anyhow!("Failed to set gain: {}", e))
    }

    /// Get aggregated room stats.
    pub fn get_room_stats(&self, room_number: &str) -> Result<serde_json::Value> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        let all_meta = room.get_all_participant_metadata();
        let participants: Vec<serde_json::Value> = all_meta
            .iter()
            .map(|meta| {
                let mut p = serde_json::json!({
                    "participant_id": meta.id,
                    "state": format!("{:?}", meta.state),
                    "gain": meta.gain,
                    "is_recording": meta.is_recording,
                    "is_speaking": meta.is_speaking,
                    "packets_received": meta.packets_received,
                });

                // Enrich with session data
                if let Some(entry) = self
                    .sessions
                    .iter()
                    .find(|e| e.value().participant_id == meta.id)
                {
                    let session = entry.value();
                    if let Some(ref dn) = session.display_name {
                        p["display_name"] = serde_json::json!(dn);
                    }
                    if let Some(ref cid) = session.caller_id {
                        p["caller_id"] = serde_json::json!(cid);
                    }
                    let s = &session.stats;
                    p["rtp_stats"] = serde_json::json!({
                        "packets_received": s.packets_received.load(Ordering::Relaxed),
                        "packets_decoded": s.packets_decoded.load(Ordering::Relaxed),
                        "decode_errors": s.decode_errors.load(Ordering::Relaxed),
                        "packets_sent": s.packets_sent.load(Ordering::Relaxed),
                        "encode_errors": s.encode_errors.load(Ordering::Relaxed),
                        "dtmf_events": s.dtmf_events.load(Ordering::Relaxed),
                    });
                    if let Some(mos) = s.estimated_mos() {
                        p["estimated_mos"] = serde_json::json!((mos * 100.0).round() / 100.0);
                    }
                    p["video"] = video_bridge::participant_video_json(
                        &room,
                        &meta.id,
                        session.video.as_ref(),
                        session.content.as_ref(),
                    );
                }

                p
            })
            .collect();

        Ok(serde_json::json!({
            "room_number": room_number,
            "participant_count": room.participant_count(),
            "waiting_count": room.waiting_count(),
            "is_locked": room.is_locked(),
            "is_recording": room.is_recording(),
            "host_count": room.host_count(),
            "participants": participants,
            "video": video_bridge::room_video_json(&room),
        }))
    }

    /// The people in a room (or one of them) as a recording names them, so
    /// a subject's voice can be found (`docs/COMPLIANCE_FOLLOWUPS.md` §4).
    fn recording_participants(
        &self,
        room_number: &str,
        only: Option<&str>,
    ) -> Vec<fcp_core::RecordingParticipant> {
        self.sessions
            .iter()
            .filter(|e| {
                e.room_number == room_number
                    && e.trunk_peer.is_none()
                    && only.is_none_or(|p| p == e.participant_id)
            })
            .map(|e| fcp_core::RecordingParticipant {
                participant_id: e.participant_id.clone(),
                uri: Some(e.record.from_uri.clone()),
                display_name: e.display_name.clone(),
                joined_at: e.joined_at,
            })
            .collect()
    }

    /// Someone joined a room being recorded: their name goes on the row.
    async fn note_recording_participant(&self, room_number: &str, participant_id: &str) {
        let Some(id) = self.recordings.get(room_number).map(|r| r.record.id) else {
            return;
        };
        let joined = self.recording_participants(room_number, Some(participant_id));
        if let Some(store) = self.storage.recording_store() {
            if let Err(e) = store.add_recording_participants(&id, &joined).await {
                warn!(recording = %id, error = %e, "Could not add a participant to the recording's row");
            }
        }
    }

    /// Complete a per-participant recording's row, if one is under way.
    fn finish_participant_recording(&self, room_number: &str, participant_id: &str) {
        let Some((_, record)) = self
            .participant_recordings
            .remove(&(room_number.to_string(), participant_id.to_string()))
        else {
            return;
        };
        let Some(store) = self.storage.recording_store().cloned() else {
            return;
        };
        tokio::spawn(async move {
            let ended_at = SystemTime::now();
            let secs = record
                .started_at
                .elapsed()
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let size = std::fs::metadata(&record.path)
                .map(|m| m.len())
                .unwrap_or(0);
            if let Err(e) = store
                .finish_recording(&record.id, ended_at, secs, size)
                .await
            {
                warn!(recording = %record.id, error = %e, "Could not complete a participant recording's row");
            }
        });
    }

    /// Start per-participant recording.
    pub async fn start_participant_recording(
        &self,
        room_number: &str,
        participant_id: &str,
    ) -> Result<String> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        let timestamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Truncate identifiers to keep total filename under filesystem limits (255 bytes)
        let safe_room = truncate_for_filename(room_number, 40);
        let safe_pid = truncate_for_filename(participant_id, 40);
        let path = format!(
            "{}/{}-{}-{}.wav",
            self.conference_config.read().recordings_dir,
            safe_room,
            safe_pid,
            timestamp
        );

        room.start_participant_recording(participant_id, &path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start recording: {}", e))?;

        // A row like a room recording's, so retention, holds and erasure
        // can find it.
        let tenant_id = self
            .tenant_for_room(room_number)
            .unwrap_or(self.default_tenant_id);
        let mut record = fcp_core::Recording::starting(
            tenant_id,
            room_number,
            &self.node_id,
            &path,
            fcp_core::RecordingFormat::Wav,
            fcp_core::RecordingTrigger::Api,
        );
        record.kind = fcp_core::RecordingKind::Participant;
        record.participant_id = Some(participant_id.to_string());
        record.participants = 1;
        record.participant_list = self.recording_participants(room_number, Some(participant_id));
        if let Some(store) = self.storage.recording_store() {
            if let Err(e) = store.create_recording(&record).await {
                let _ = room.stop_participant_recording(participant_id);
                let _ = std::fs::remove_file(&path);
                anyhow::bail!("recording not registered, so not kept: {e}");
            }
        }
        self.participant_recordings.insert(
            (room_number.to_string(), participant_id.to_string()),
            record,
        );

        info!(
            room = %room_number,
            participant = %participant_id,
            path = %path,
            "Per-participant recording started"
        );
        Ok(path)
    }

    /// Stop per-participant recording.
    pub fn stop_participant_recording(
        &self,
        room_number: &str,
        participant_id: &str,
    ) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        room.stop_participant_recording(participant_id)
            .map_err(|e| anyhow::anyhow!("Failed to stop recording: {}", e))?;
        self.finish_participant_recording(room_number, participant_id);

        info!(
            room = %room_number,
            participant = %participant_id,
            "Per-participant recording stopped"
        );
        Ok(())
    }

    /// Play an audio announcement into a conference room.
    ///
    /// Validates file exists and does not exceed max_announcement_size_mb before playing.
    /// Give the handler its own `Arc`, once, at startup.
    pub fn set_self_ref(self: &Arc<Self>) {
        if let Ok(mut slot) = self.self_ref.write() {
            *slot = Arc::downgrade(self);
        }
    }

    /// The handler itself, for a spawned task; `None` before
    /// [`set_self_ref`](Self::set_self_ref) or once it is being dropped.
    fn me(&self) -> Option<Arc<Self>> {
        self.self_ref.read().ok().and_then(|w| w.upgrade())
    }

    /// Where a room's video events go: the bus, the video budget as the
    /// floor's gate, and this handler as the one that tells gateway
    /// sessions about the floor.
    fn video_event_sink(&self, tenant_id: TenantId, room_number: &str) -> VideoEventSink {
        VideoEventSink {
            event_bus: Arc::clone(&self.event_bus),
            tenant_id,
            room_number: room_number.to_string(),
            node_id: self.node_id.clone(),
            pumps: Arc::clone(&self.video_event_pumps),
            content_gate: self
                .video_capacity
                .clone()
                .map(|c| c as Arc<dyn forge_conference::video::ContentGate>),
            floor: self.me().map(|me| me as Arc<dyn FloorNotifier>),
        }
    }

    /// Tell one gateway session about the floor: a SIP `INFO` of
    /// [`CONTENT_INFO_TYPE`] with the notice and `mine`. The dialog it
    /// advances is written back so the next request continues its CSeq.
    async fn send_floor_info(self: Arc<Self>, call_id: String, notice: FloorNotice, mine: bool) {
        let Some(mut dialog) = self.sessions.get(&call_id).map(|s| s.dialog.clone()) else {
            return;
        };
        let mut body = serde_json::to_value(&notice).unwrap_or_default();
        body["mine"] = serde_json::json!(mine);
        match self
            .uac
            .send_info(&mut dialog, CONTENT_INFO_TYPE, &body.to_string())
            .await
        {
            Ok(resp) if (200..300).contains(&resp.start_line().code()) => {
                if let Some(mut s) = self.sessions.get_mut(&call_id) {
                    s.dialog = dialog;
                }
            }
            Ok(resp) => debug!(
                call_id = %call_id,
                code = resp.start_line().code(),
                floor = notice.floor,
                "floor INFO refused"
            ),
            Err(e) => debug!(call_id = %call_id, error = %e, "floor INFO failed"),
        }
    }

    /// A participant admitted while someone holds the floor is told so
    /// (§15.6, 7c), as one there from the start would have been.
    fn tell_floor_on_join(&self, call_id: &str, room_number: &str) {
        let (Some(me), Ok(room)) = (self.me(), self.bridge.get_room(room_number)) else {
            return;
        };
        let Some(content) = room.video().and_then(|v| v.content()) else {
            return;
        };
        let notice = video_bridge::floor_notice(
            &content.participant_id,
            &forge_conference::video::ContentEvent::Started,
            Some(&content),
        );
        let call_id = call_id.to_string();
        tokio::spawn(async move {
            // The session is stored right after this is called, and the
            // gateway maps the dialog only once the INVITE completes: a
            // moment's grace makes the INFO land instead of drawing 481.
            tokio::time::sleep(Duration::from_secs(1)).await;
            me.send_floor_info(call_id, notice, false).await
        });
    }

    // ---- recording (design §11) -----------------------------------------

    /// Start recording a room. The container follows
    /// `conference.recording_format`: a meeting with video records its
    /// composite and mix to MP4 or WebM (whichever the node's codecs and
    /// the setting allow, §15.9), one without records the mix to WAV as
    /// it always has. The registry row is written before the first
    /// sample, so a recording is addressable from the moment it starts.
    pub async fn start_room_recording(
        &self,
        room_number: &str,
        trigger: fcp_core::RecordingTrigger,
    ) -> Result<fcp_core::Recording> {
        if self.recordings.contains_key(room_number) {
            anyhow::bail!("room {room_number} is already being recorded");
        }
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|_| anyhow::anyhow!("Conference room not active"))?;
        let tenant_id = self
            .tenant_for_room(room_number)
            .unwrap_or(self.default_tenant_id);
        let meeting = self
            .lookup_meeting(&tenant_id, room_number)
            .await
            .ok()
            .flatten();
        let conf = self.conference_config.read().clone();

        // Whether the room has a composite to record, and what this node
        // could encode it with.
        let room_video = matches!((&meeting, room.video()), (Some(m), Some(_)) if m.video_enabled);
        let usable = if room_video {
            self.video_backend
                .as_ref()
                .map(|b| crate::video_bridge::usable_codecs(b, conf.video_native_codecs_only))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // Validated at startup; an unparsable name reads as `auto`.
        let configured = crate::recording::parse_video_codec(&conf.recording_video_codec)
            .ok()
            .flatten();
        let choice =
            crate::recording::FormatChoice::parse(&conf.recording_format).unwrap_or_default();
        let (format, video_codec) =
            crate::recording::plan_format(choice, configured, &usable, room_video);

        let path = format!(
            "{}/{}-{}-{}.{}",
            conf.recordings_dir,
            room_number,
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            uuid::Uuid::new_v4().as_simple(),
            format.extension()
        );

        let mut record = fcp_core::Recording::starting(
            tenant_id,
            room_number,
            &self.node_id,
            &path,
            format,
            trigger,
        );
        record.meeting_id = meeting.as_ref().map(|m| m.id);
        record.participants = room.participant_count() as u32;
        record.participant_list = self.recording_participants(room_number, None);

        // The room hears the tenant's announcement before the first
        // sample, and so does everyone who joins while this runs.
        let (consent, consent_audio) = self
            .announce_recording_consent(room_number, &tenant_id, conf.sample_rate)
            .await;
        record.consent = consent;

        let writer = match format {
            fcp_core::RecordingFormat::Wav => {
                room.start_recording(&path, None)
                    .await
                    .map_err(|e| anyhow::anyhow!("starting the recorder: {e}"))?;
                RecordingWriter::Wav
            }
            fcp_core::RecordingFormat::Webm | fcp_core::RecordingFormat::Mp4 => {
                #[cfg(any(feature = "recording-webm", feature = "recording-mp4"))]
                {
                    let settings = crate::recording::RecorderSettings {
                        path: std::path::PathBuf::from(&path),
                        container: format,
                        video: video_codec.map(|codec| crate::recording::VideoSettings {
                            codec,
                            resolution: conf
                                .recording_resolution
                                .as_deref()
                                .and_then(forge_video::frame::Resolution::parse),
                            max_kbps: None,
                        }),
                        audio_bitrate: conf.recording_audio_bitrate,
                    };
                    RecordingWriter::Media(crate::recording::Recorder::start(
                        &record.id.to_string(),
                        &room,
                        settings,
                    )?)
                }
                #[cfg(not(any(feature = "recording-webm", feature = "recording-mp4")))]
                {
                    let _ = video_codec;
                    anyhow::bail!(
                        "this build cannot write {} (feature `recording-{}`)",
                        format.as_str(),
                        format.as_str()
                    )
                }
            }
        };

        // The row before the event: a listener that reacts to the event
        // can look the recording up straight away. A recording without a
        // row is one no retention, hold or erasure can find, so it is not
        // kept (docs/COMPLIANCE_FOLLOWUPS.md §4).
        if let Some(store) = self.storage.recording_store() {
            if let Err(e) = store.create_recording(&record).await {
                match writer {
                    RecordingWriter::Wav => {
                        let _ = room.stop_recording();
                    }
                    #[cfg(any(feature = "recording-webm", feature = "recording-mp4"))]
                    RecordingWriter::Media(recorder) => {
                        recorder.stop().await;
                    }
                }
                let _ = std::fs::remove_file(&record.path);
                anyhow::bail!("recording not registered, so not kept: {e}");
            }
        }
        self.recordings.insert(
            room_number.to_string(),
            ActiveRecording {
                record: record.clone(),
                consent_audio,
                writer,
            },
        );
        info!(
            room = %room_number,
            recording = %record.id,
            format = record.format.as_str(),
            path = %path,
            "Recording started"
        );
        self.publish_event(ConferenceEvent::RecordingStarted {
            room_number: room_number.to_string(),
            recording_id: record.id.to_string(),
            format: record.format.as_str().to_string(),
            node_id: self.node_id.clone(),
            tenant_id,
            timestamp: SystemTime::now(),
        });
        Ok(record)
    }

    /// Stop a room's recording and complete its row.
    pub async fn stop_room_recording(&self, room_number: &str) -> Result<fcp_core::Recording> {
        let Some((_, active)) = self.recordings.remove(room_number) else {
            // A room recording started before this node knew about the
            // registry (or by forge-conference itself) still stops.
            if let Ok(room) = self.bridge.get_room(room_number) {
                if room.is_recording() {
                    room.stop_recording()
                        .map_err(|e| anyhow::anyhow!("stopping the recorder: {e}"))?;
                    return Err(anyhow::anyhow!(
                        "room {room_number} had an unregistered recording, now stopped"
                    ));
                }
            }
            anyhow::bail!("room {room_number} is not being recorded");
        };
        let mut record = active.record;
        let (duration_secs, size_bytes) = match active.writer {
            RecordingWriter::Wav => {
                if let Ok(room) = self.bridge.get_room(room_number) {
                    if let Err(e) = room.stop_recording() {
                        warn!(room = %room_number, error = %e, "Failed to stop the WAV recorder");
                    }
                }
                let size = std::fs::metadata(&record.path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                let secs = record
                    .started_at
                    .elapsed()
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                (secs, size)
            }
            #[cfg(any(feature = "recording-webm", feature = "recording-mp4"))]
            RecordingWriter::Media(recorder) => {
                let outcome = recorder.stop().await;
                (outcome.duration_secs, outcome.size_bytes)
            }
        };
        let ended_at = SystemTime::now();
        record.ended_at = Some(ended_at);
        record.duration_secs = Some(duration_secs);
        record.size_bytes = Some(size_bytes);

        if let Some(store) = self.storage.recording_store() {
            if let Err(e) = store
                .finish_recording(&record.id, ended_at, duration_secs, size_bytes)
                .await
            {
                warn!(recording = %record.id, error = %e, "Could not complete the recording's row");
            }
        }
        info!(
            room = %room_number,
            recording = %record.id,
            seconds = duration_secs,
            bytes = size_bytes,
            "Recording stopped"
        );
        self.publish_event(ConferenceEvent::RecordingStopped {
            room_number: room_number.to_string(),
            recording_id: record.id.to_string(),
            format: record.format.as_str().to_string(),
            node_id: self.node_id.clone(),
            duration_secs,
            size_bytes,
            tenant_id: record.tenant_id,
            timestamp: ended_at,
        });
        Ok(record)
    }

    /// Whether this node is recording the room.
    pub fn is_room_recording(&self, room_number: &str) -> bool {
        self.recordings.contains_key(room_number)
    }

    /// The room's recording, if this node is making one.
    pub fn room_recording(&self, room_number: &str) -> Option<fcp_core::Recording> {
        self.recordings.get(room_number).map(|r| r.record.clone())
    }

    /// Every recording under way on this node.
    pub fn active_recordings(&self) -> Vec<fcp_core::Recording> {
        self.recordings
            .iter()
            .map(|r| r.value().record.clone())
            .collect()
    }

    /// The announcement anyone joining a recorded room must hear, when
    /// the tenant's policy asks for one.
    pub fn recording_consent_audio(&self, room_number: &str) -> Option<Arc<Vec<i16>>> {
        self.recordings
            .get(room_number)
            .and_then(|r| r.consent_audio.clone())
    }

    /// Tell the room it is being recorded, if the tenant's policy says to
    /// (`docs/COMPLIANCE.md`). Returns what happened, for the row, and the
    /// audio to play to anyone who joins later.
    async fn announce_recording_consent(
        &self,
        room_number: &str,
        tenant_id: &TenantId,
        sample_rate: u32,
    ) -> (fcp_core::RecordingConsent, Option<Arc<Vec<i16>>>) {
        use fcp_core::compliance::RecordingConsentMode;
        use fcp_core::RecordingConsent;

        let Some(store) = self.storage.compliance_store() else {
            return (RecordingConsent::NotRequired, None);
        };
        let policy = match store.get_compliance_policy(tenant_id).await {
            Ok(Some(policy)) => policy,
            Ok(None) => return (RecordingConsent::NotRequired, None),
            Err(e) => {
                warn!(room = %room_number, error = %e, "Could not read the recording-consent policy");
                return (RecordingConsent::Failed, None);
            }
        };
        if policy.recording_consent != RecordingConsentMode::Announce {
            return (RecordingConsent::NotRequired, None);
        }

        // The tenant's prompt when it names one, else synthesised speech.
        let mut audio_path: Option<String> = None;
        if let (Some(prompt_id), Some(prompts)) = (
            policy.recording_consent_prompt_id.as_deref(),
            self.storage.prompt_store(),
        ) {
            match prompt_id.parse::<fcp_core::PromptId>() {
                Ok(id) => match prompts.get_prompt(&id).await {
                    Ok(Some(prompt)) => {
                        audio_path = prompt.audio_path.map(|p| p.to_string_lossy().into_owned())
                    }
                    Ok(None) => {
                        warn!(room = %room_number, prompt = %prompt_id, "Recording-consent prompt not found")
                    }
                    Err(e) => {
                        warn!(room = %room_number, error = %e, "Could not read the recording-consent prompt")
                    }
                },
                Err(e) => {
                    warn!(prompt = %prompt_id, error = %e, "Invalid recording-consent prompt id")
                }
            }
        }
        if audio_path.is_none() {
            if let Some(tts) = &self.tts_service {
                match tokio::time::timeout(
                    Duration::from_secs(10),
                    tts.synthesize(RECORDING_CONSENT_TEXT, None, None),
                )
                .await
                {
                    Ok(Ok(path)) => audio_path = Some(path.to_string_lossy().into_owned()),
                    Ok(Err(e)) => {
                        warn!(room = %room_number, error = %e, "Recording-consent TTS failed")
                    }
                    Err(_) => warn!(room = %room_number, "Recording-consent TTS timed out"),
                }
            }
        }
        let Some(audio_path) = audio_path else {
            warn!(
                room = %room_number,
                "The tenant asks for a recording announcement but neither a prompt nor TTS is \
                 available; recording without one"
            );
            return (RecordingConsent::Failed, None);
        };

        // Keep the samples so later joiners hear it too.
        let player = forge_conference::AudioFeedbackPlayer::new(sample_rate);
        let samples = player
            .load_audio_file(&audio_path)
            .map(|s| Arc::new(s.to_vec()));
        match self.play_announcement(room_number, &audio_path).await {
            Ok(()) => {
                info!(room = %room_number, "Recording announcement played");
                (RecordingConsent::Announced, samples)
            }
            Err(e) => {
                warn!(room = %room_number, error = %e, "Recording announcement failed");
                (RecordingConsent::Failed, samples)
            }
        }
    }

    pub async fn play_announcement(&self, room_number: &str, audio_path: &str) -> Result<()> {
        // Validate announcement file exists and is within size limits
        let meta = std::fs::metadata(audio_path)
            .map_err(|e| anyhow::anyhow!("Announcement file not found '{}': {}", audio_path, e))?;

        let max_mb = self.conference_config.read().max_announcement_size_mb;
        if max_mb > 0 && meta.len() > max_mb * 1024 * 1024 {
            anyhow::bail!(
                "Announcement file too large: {} MB (max: {} MB)",
                meta.len() / (1024 * 1024),
                max_mb
            );
        }

        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        room.play_announcement(audio_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to play announcement: {}", e))?;

        info!(room = %room_number, path = %audio_path, "Announcement played");
        Ok(())
    }

    /// Get or create DTMF command handler for a room.
    fn get_or_create_dtmf_handler(&self, room_number: &str) -> Arc<DtmfCommandHandler> {
        if let Some(handler) = self.dtmf_handlers.get(room_number) {
            return Arc::clone(handler.value());
        }
        let config = ConferenceDtmfConfig::default();
        let handler = Arc::new(DtmfCommandHandler::new(room_number.to_string(), config));
        self.dtmf_handlers
            .insert(room_number.to_string(), Arc::clone(&handler));
        handler
    }

    /// Attach an AI participant to a conference room.
    pub async fn attach_ai(&self, room_number: &str) -> Result<()> {
        let ai_conf = self.ai_config.read().clone();
        if !ai_conf.enabled {
            anyhow::bail!("AI participants are not enabled in configuration");
        }

        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        // Build AI session config from server config
        let mut session_config = AISessionConfig::default();
        if let Some(ref key) = ai_conf.api_key {
            session_config.api_key = forge_core::SecureString::new(key.clone());
        }
        session_config.model = ai_conf.model.clone();
        session_config.voice = Some(ai_conf.voice.clone());
        if let Some(ref instructions) = ai_conf.instructions {
            session_config.instructions = Some(instructions.clone());
        }
        if let Some(ref endpoint) = ai_conf.endpoint {
            session_config.endpoint = Some(endpoint.clone());
        }

        // Create ConferenceAIManager
        let call_id = forge_core::CallId::generate();
        let conference_ai_config = ConferenceAIConfig::new(forge_core::AudioFormat::pcm_mono());
        let ai_manager = ConferenceAIManager::new(
            room_number.to_string(),
            call_id.clone(),
            Arc::clone(&self.ai_session_manager),
            conference_ai_config,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create AI manager: {}", e))?;

        // Attach AI session to the session manager
        self.ai_session_manager
            .attach_ai(call_id, session_config, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to attach AI session: {}", e))?;

        // Attach AI participant to room
        room.attach_ai(ai_manager, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to attach AI to room: {}", e))?;

        info!(room = %room_number, "AI participant attached");
        Ok(())
    }

    /// Detach AI participant from a conference room.
    pub async fn detach_ai(&self, room_number: &str) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        room.detach_ai()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to detach AI: {}", e))?;

        info!(room = %room_number, "AI participant detached");
        Ok(())
    }

    /// Get AI participant status for a room.
    pub fn ai_status(&self, room_number: &str) -> Result<serde_json::Value> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        let has_ai = room.has_ai();
        let state = room.ai_state().map(|s| format!("{:?}", s));
        let ai_conf = self.ai_config.read();

        Ok(serde_json::json!({
            "room_number": room_number,
            "ai_enabled": ai_conf.enabled,
            "ai_attached": has_ai,
            "ai_state": state,
            "model": ai_conf.model,
        }))
    }

    /// Kick a participant by their participant_id: send SIP BYE and clean up session.
    pub async fn kick_participant(&self, participant_id: &str) -> Result<()> {
        // Reverse lookup: find call_id by participant_id
        let call_id = self
            .sessions
            .iter()
            .find(|entry| entry.value().participant_id == participant_id)
            .map(|entry| entry.key().clone());

        let call_id = call_id.ok_or_else(|| {
            anyhow::anyhow!("No session found for participant: {}", participant_id)
        })?;

        // Send BYE before cleaning up
        if let Some(session) = self.sessions.get(&call_id) {
            if let Err(e) = self.uac.bye(&session.dialog).await {
                warn!(call_id = %call_id, error = %e, "Failed to send BYE for kicked participant");
            }
        }

        self.remove_session_for(
            &call_id,
            fcp_sip::TerminationReason::system(fcp_sip::SystemCause::Admin, "Kicked"),
        );
        Ok(())
    }

    /// Dial out to a destination and bridge them into the conference.
    ///
    /// Sends an outbound SIP INVITE. When the callee answers, their audio
    /// is bridged into the conference mixer as a regular participant.
    pub async fn dial_out(
        &self,
        room_number: &str,
        destination: &str,
        tenant_id: TenantId,
        greeting_tts: Option<&str>,
        greeting_prompt: Option<&str>,
    ) -> Result<String> {
        // 0. Normalize destination
        let destination = if destination.starts_with("sip:") || destination.starts_with("tel:") {
            destination.to_string()
        } else if destination.contains('@') {
            format!("sip:{}", destination)
        } else {
            // Bare extension/number → sip:ext@local_domain
            let domain = self
                .local_addr
                .split(':')
                .next()
                .unwrap_or(&self.local_addr);
            format!("sip:{}@{}", destination, domain)
        };
        let destination = &destination;

        // 1. Look up meeting config and get or create the room
        let meeting = self
            .lookup_meeting(&tenant_id, room_number)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Meeting '{}' not found", room_number))?;

        if !meeting.enabled {
            anyhow::bail!("Conference is disabled");
        }

        let room = self.room_for_join(&tenant_id, room_number).await?;

        // Configure room from meeting settings (idempotent)
        let room_config = self.meeting_to_room_config(&meeting);
        let global_config = ConferenceConfig::default();
        let _ = room.configure(room_config, &global_config);

        if let Some(cfg) = room.get_effective_config() {
            if cfg.max_channels > 0 && room.participant_count() >= cfg.max_channels {
                anyhow::bail!("Conference room is full");
            }
        }

        if room.is_locked() {
            anyhow::bail!("Conference room is locked");
        }

        // 2. Allocate RTP port pair
        let port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port pair for dial-out")?;

        // 3. Build multi-codec SDP offer
        let ip = self
            .local_addr
            .split(':')
            .next()
            .unwrap_or(&self.local_addr);
        let sdp_offer = format!(
            "v=0\r\n\
             o=fcp-conference 0 0 IN IP4 {ip}\r\n\
             s=Conference Dial-Out\r\n\
             c=IN IP4 {ip}\r\n\
             t=0 0\r\n\
             m=audio {rtp_port} RTP/AVP 0 8 9 101\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=rtpmap:9 G722/8000\r\n\
             a=rtpmap:101 telephone-event/8000\r\n\
             a=fmtp:101 0-16\r\n\
             a=sendrecv\r\n\
             a=ptime:20\r\n",
            ip = ip,
            rtp_port = port_pair.rtp_port,
        );

        // 4. Send INVITE via UAC
        info!(room = %room_number, destination = %destination, "Initiating dial-out INVITE");
        let call_handle = match self.uac.invite(destination, Some(&sdp_offer)).await {
            Ok(h) => h,
            Err(e) => {
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                self.publish_event(ConferenceEvent::DialOutFailed {
                    room_number: room_number.to_string(),
                    destination: destination.to_string(),
                    reason: format!("INVITE failed: {}", e),
                    tenant_id,
                    timestamp: SystemTime::now(),
                });
                anyhow::bail!("Failed to send INVITE: {}", e);
            }
        };

        // 5. Wait for final response with timeout
        let timeout_secs = self.conference_config.read().dial_out_timeout_secs;
        let timeout = Duration::from_secs(timeout_secs);
        let result = tokio::time::timeout(timeout, call_handle.await_final()).await;

        let response = match result {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                self.publish_event(ConferenceEvent::DialOutFailed {
                    room_number: room_number.to_string(),
                    destination: destination.to_string(),
                    reason: format!("Call failed: {}", e),
                    tenant_id,
                    timestamp: SystemTime::now(),
                });
                anyhow::bail!("Dial-out call failed: {}", e);
            }
            Err(_) => {
                // Timeout — cancel the INVITE
                let _ = call_handle.cancel().await;
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                self.publish_event(ConferenceEvent::DialOutFailed {
                    room_number: room_number.to_string(),
                    destination: destination.to_string(),
                    reason: format!("No answer (timeout {}s)", timeout_secs),
                    tenant_id,
                    timestamp: SystemTime::now(),
                });
                anyhow::bail!("Dial-out timed out after {}s", timeout_secs);
            }
        };

        // 6. Check for 2xx success
        let status_code = response.code();
        if !(200..300).contains(&status_code) {
            let pool = Arc::clone(&self.port_pool);
            tokio::spawn(async move { pool.deallocate(port_pair).await });
            let reason_phrase = response.reason();
            self.publish_event(ConferenceEvent::DialOutFailed {
                room_number: room_number.to_string(),
                destination: destination.to_string(),
                reason: format!("{} {}", status_code, reason_phrase),
                tenant_id,
                timestamp: SystemTime::now(),
            });
            anyhow::bail!("Callee rejected: {} {}", status_code, reason_phrase);
        }

        // 7. Parse SDP answer for codec and remote address
        let answer_body = response.body();
        let answer_sdp = std::str::from_utf8(&answer_body[..]).unwrap_or("");

        let offered_codecs = parse_sdp_codecs(answer_sdp);
        let (codec_name, payload_type) = {
            let mut result = ("PCMU".to_string(), 0u8);
            for codec in &offered_codecs {
                let name = codec.encoding_name.to_uppercase();
                if name != "TELEPHONE-EVENT" {
                    result = (name, codec.payload_type);
                    break;
                }
            }
            result
        };
        let frame_size: usize = match codec_name.as_str() {
            "G722" => 320,
            _ => 160,
        };
        let te_pt = find_telephone_event_pt(&offered_codecs, answer_sdp);
        let dtmf_clock_rate = find_telephone_event_clock_rate(&offered_codecs, answer_sdp);

        // 8. Create RTP socket and set remote endpoint
        let rtp_socket = Arc::new(
            RtpSocketPair::new(port_pair, RtpSocketConfig::default())
                .await
                .context("Failed to create RTP socket for dial-out")?,
        );
        if let Some(remote_addr) = parse_remote_rtp_addr(answer_sdp) {
            rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;
        }

        // 9. Get dialog from call handle
        let dialog = call_handle.dialog.read().await.clone();
        let call_id = dialog.id().call_id().to_string();

        // 10. Play greeting to the dial-out participant BEFORE adding to mixer.
        //     This ensures they hear only the greeting, not conference audio.
        let conf = self.conference_config.read().clone();
        {
            let mut greeting_samples: Option<Vec<i16>> = None;
            let player = forge_conference::AudioFeedbackPlayer::new(conf.sample_rate);

            // Priority: greeting_prompt (WAV from prompt store) > greeting_tts > meeting default > none
            if let Some(prompt_name) = greeting_prompt {
                if let Some(prompt_store) = self.storage.prompt_store() {
                    let tid = tenant_id;
                    if let Ok(Some(prompt)) =
                        prompt_store.get_prompt_by_name(&tid, prompt_name).await
                    {
                        if let Some(ref path) = prompt.audio_path {
                            greeting_samples = player.load_audio_file(path).map(|s| s.to_vec());
                        }
                    }
                }
            }

            if greeting_samples.is_none() {
                if let Some(tts_text) = greeting_tts {
                    if let Some(ref tts) = self.tts_service {
                        match tokio::time::timeout(
                            Duration::from_secs(10),
                            tts.synthesize(tts_text, None, None),
                        )
                        .await
                        {
                            Ok(Ok(path)) => {
                                greeting_samples =
                                    player.load_audio_file(&path).map(|s| s.to_vec());
                            }
                            Ok(Err(e)) => {
                                warn!(error = %e, "Dial-out TTS greeting synthesis failed")
                            }
                            Err(_) => warn!("Dial-out TTS greeting synthesis timed out (3s)"),
                        }
                    }
                }
            }

            // Fall back to a default dial-out greeting from the prompt store
            if greeting_samples.is_none() && greeting_tts.is_none() && greeting_prompt.is_none() {
                if let Some(prompt_store) = self.storage.prompt_store() {
                    let tid = tenant_id;
                    if let Ok(Some(prompt)) = prompt_store
                        .get_prompt_by_name(&tid, "conf_dialout_greeting")
                        .await
                    {
                        if let Some(ref path) = prompt.audio_path {
                            greeting_samples = player.load_audio_file(path).map(|s| s.to_vec());
                        }
                    }
                }
            }

            if let Some(samples) = greeting_samples {
                let mut greeting_encoder = create_codec_instance(&codec_name)?;
                play_prompt_to_rtp(
                    &rtp_socket,
                    &samples,
                    greeting_encoder.as_mut(),
                    payload_type,
                    frame_size,
                    &None, // no SRTP for dial-out
                    conf.sample_rate,
                )
                .await;
                info!(room = %room_number, destination = %destination, samples = samples.len(), "Played dial-out greeting");
            }
        }

        // 11. Generate participant ID and add to room (AFTER greeting plays)
        let short_uuid = uuid::Uuid::new_v4().to_string();
        let participant_id = format!("dialout-{}", &short_uuid[..8]);

        // Add as host to bypass wait-for-moderator (dial-out is an admin/host action).
        // This also releases any waiting guests, which is the desired behavior —
        // if the admin is dialing people in, the conference should be active.
        if let Err(e) = room.add_participant(&participant_id, true) {
            let _ = self.uac.bye(&dialog).await;
            let pool = Arc::clone(&self.port_pool);
            tokio::spawn(async move { pool.deallocate(port_pair).await });
            anyhow::bail!("Failed to add dial-out participant: {}", e);
        }

        // 12. Spawn audio bridge
        let encoder = create_codec_instance(&codec_name)?;
        let decoder = create_codec_instance(&codec_name)?;
        let stats = Arc::new(AudioStats::new());
        let stats_clone = Arc::clone(&stats);
        let dtmf_handler = if conf.dtmf_enabled {
            Some(self.get_or_create_dtmf_handler(room_number))
        } else {
            None
        };
        let bridge_room = Arc::clone(&room);
        let bridge_pid = participant_id.clone();
        let bridge_socket = Arc::clone(&rtp_socket);
        let audio_bridge_handle = tokio::spawn(audio_bridge::run_audio_bridge(
            bridge_room,
            bridge_pid,
            bridge_socket,
            encoder,
            decoder,
            payload_type,
            frame_size,
            conf.jitter_buffer_ms,
            te_pt,
            dtmf_clock_rate,
            dtmf_handler,
            false, // dial-out participants are not hosts
            stats_clone,
            None, // no SRTP for dial-out (plain RTP)
            conf.sample_rate,
            self.dtmf_muted_samples.clone(),
            self.dtmf_unmuted_samples.clone(),
            self.dtmf_locked_samples.clone(),
            self.dtmf_unlocked_samples.clone(),
            self.dtmf_ending_samples.clone(),
            false,
        ));

        // 12. Store session
        let remote_addr = parse_remote_rtp_addr(answer_sdp).unwrap_or(UNKNOWN_REMOTE);
        let codec_name_log = codec_name.clone();
        self.room_sessions
            .entry(room_number.to_string())
            .or_default()
            .push(call_id.clone());
        self.sessions.insert(
            call_id.clone(),
            ParticipantSession {
                call_id,
                room_number: room_number.to_string(),
                participant_id: participant_id.clone(),
                is_host: false,
                is_dial_out: true,
                trunk_peer: None,
                audio_bridge_handle,
                port_pair,
                rtp_socket,
                remote_addr,
                codec_name,
                tenant_id,
                dialog,
                joined_at: SystemTime::now(),
                te_pt,
                stats,
                display_name: Some(format!("Dial-out: {}", destination)),
                caller_id: Some(destination.to_string()),
                srtp_context: None,
                last_refreshed: Instant::now(),
                video: None,
                content: None,
                joined_by_token: false,
                bfcp: None,
                record: LegRecord::own(
                    format!("conference:{room_number}"),
                    Some(destination.to_string()),
                ),
            },
        );

        // 13. Publish events
        self.publish_event(ConferenceEvent::DialOutInvited {
            room_number: room_number.to_string(),
            participant_id: participant_id.clone(),
            destination: destination.to_string(),
            tenant_id,
            timestamp: SystemTime::now(),
        });
        self.sync_room_registry(room_number);
        self.publish_event(ConferenceEvent::ParticipantJoined {
            room_number: room_number.to_string(),
            participant_id: participant_id.clone(),
            migrated_from: None,
            is_host: false,
            has_video: false,
            tenant_id,
            timestamp: SystemTime::now(),
        });

        info!(
            room = %room_number,
            destination = %destination,
            participant = %participant_id,
            codec = %codec_name_log,
            "Dial-out participant connected"
        );

        Ok(participant_id)
    }

    /// Play audio samples to all participants' RTP sockets directly.
    /// Used for conference-wide announcements (e.g. "conference ending").
    pub async fn play_to_all_participants(&self, room_number: &str, samples: &[i16]) {
        // Get all sessions for this room
        if let Some(call_ids) = self.room_sessions.get(room_number) {
            let call_ids: Vec<String> = call_ids.clone();
            drop(call_ids); // release DashMap ref

            if let Some(sessions_ref) = self.room_sessions.get(room_number) {
                let cids: Vec<String> = sessions_ref.clone();
                drop(sessions_ref);

                let mut handles = Vec::new();
                for call_id in cids {
                    if let Some(session) = self.sessions.get(&call_id) {
                        let rtp_socket = Arc::clone(&session.rtp_socket);
                        let codec = session.codec_name.clone();
                        let pt = match codec.as_str() {
                            "PCMU" => 0u8,
                            "PCMA" => 8,
                            "G722" => 9,
                            _ => 0,
                        };
                        let fs: usize = if codec == "G722" { 320 } else { 160 };
                        let srtp = session.srtp_context.clone();
                        let pcm = samples.to_vec();
                        let sr = self.conference_config.read().sample_rate;
                        handles.push(tokio::spawn(async move {
                            if let Ok(mut enc) = create_codec_instance(&codec) {
                                play_prompt_to_rtp(
                                    &rtp_socket,
                                    &pcm,
                                    enc.as_mut(),
                                    pt,
                                    fs,
                                    &srtp,
                                    sr,
                                )
                                .await;
                            }
                        }));
                    }
                }
                // Wait for all playback to complete
                for h in handles {
                    let _ = h.await;
                }
            }
        }
    }

    /// Force-destroy a specific conference room: BYE all participants, stop recording, delete room.
    pub async fn destroy_conference(&self, room_number: &str) -> Result<usize> {
        // Verify room exists
        self.bridge
            .get_room(room_number)
            .map_err(|_| anyhow::anyhow!("Conference room not active: {}", room_number))?;

        let room_tenant_id = self.tenant_for_room(room_number);

        // Collect call IDs using room index for O(1) lookup
        let call_ids: Vec<String> = self
            .room_sessions
            .get(room_number)
            .map(|ids| ids.clone())
            .unwrap_or_default();

        let kicked = call_ids.len();

        // Send BYE and remove each session
        for call_id in &call_ids {
            if let Some(session) = self.sessions.get(call_id) {
                if let Err(e) = self.uac.bye(&session.dialog).await {
                    warn!(call_id = %call_id, error = %e, "Failed to send BYE for destroy");
                }
                drop(session);
            }
            self.remove_session_for(
                call_id,
                fcp_sip::TerminationReason::system(fcp_sip::SystemCause::Other, "Room closed"),
            );
        }

        // Stop recording if active
        if self.is_room_recording(room_number) {
            let _ = self.stop_room_recording(room_number).await;
        } else if let Ok(room) = self.bridge.get_room(room_number) {
            if room.is_recording() {
                let _ = room.stop_recording();
            }
        }

        // Delete the room
        let _ = self.bridge.delete_room(room_number);

        // Clean up associated state
        self.dtmf_handlers.remove(room_number);

        if let Some(tenant_id) = room_tenant_id {
            self.publish_event(ConferenceEvent::RoomDestroyed {
                room_number: room_number.to_string(),
                tenant_id,
                timestamp: SystemTime::now(),
            });
        } else {
            warn!(
                room = %room_number,
                "Skipped RoomDestroyed event because tenant context could not be resolved"
            );
        }

        info!(room = %room_number, participants = kicked, "Conference room force-destroyed");
        // The room is gone on this node: drop it from the cluster registry.
        self.sync_room_registry(room_number);
        Ok(kicked)
    }

    /// Graceful shutdown: drain all participant sessions and clean up resources.
    pub async fn shutdown(&self) {
        let session_count = self.sessions.len();
        info!(
            sessions = session_count,
            "Shutting down conference sessions"
        );

        // Collect all call IDs
        let call_ids: Vec<String> = self
            .sessions
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        // BYE every session, a batch at a time, each BYE bounded so a
        // peer that is already gone (a dead trunk, a phone off the
        // network) cannot hold up the rest of the shutdown.
        let dialogs: Vec<(String, Dialog)> = call_ids
            .iter()
            .filter_map(|call_id| {
                self.sessions
                    .get(call_id)
                    .map(|s| (call_id.clone(), s.dialog.clone()))
            })
            .collect();
        futures::stream::iter(dialogs)
            .for_each_concurrent(SHUTDOWN_BYE_CONCURRENCY, |(call_id, dialog)| async move {
                match tokio::time::timeout(BYE_TIMEOUT, self.uac.bye(&dialog)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        warn!(call_id = %call_id, error = %e, "Failed to send BYE during shutdown")
                    }
                    Err(_) => {
                        warn!(call_id = %call_id, "BYE unanswered during shutdown; closing anyway")
                    }
                }
            })
            .await;
        for call_id in &call_ids {
            self.remove_session_for(
                call_id,
                fcp_sip::TerminationReason::system(fcp_sip::SystemCause::Other, "Node stopped"),
            );
        }

        // Stop recordings and delete all active rooms
        for room_id in self.bridge.list_rooms() {
            if self.is_room_recording(&room_id) {
                let _ = self.stop_room_recording(&room_id).await;
            } else if let Ok(room) = self.bridge.get_room(&room_id) {
                if room.is_recording() {
                    let _ = room.stop_recording();
                }
            }
            let _ = self.bridge.delete_room(&room_id);
        }

        // Clear all in-memory maps to release resources
        self.dtmf_handlers.clear();
        self.meeting_cache.clear();
        self.invite_rate.clear();
        self.conference_subscriptions.clear();
        self.room_sessions.clear();

        info!("Conference shutdown complete");
    }

    /// Hot-reload safe configuration settings (conference section, AI config).
    /// Called by the config watcher when the config file changes.
    ///
    /// Reloadable settings: conference (cache_ttl, dtmf, rate limits, recordings,
    /// jitter buffer, session timers, PIN settings, SRTP, max participants),
    /// AI (enabled, model, voice, instructions, endpoint).
    ///
    /// NOT reloadable at runtime: SIP bind addresses, storage backend, HTTP bind,
    /// media port range (require process restart).
    pub fn reload_config(&self, conference: ConferenceSection, ai: AiSection) {
        let old_conf = self.conference_config.read().clone();
        let old_ai = self.ai_config.read().clone();

        let mut changes = Vec::new();
        if old_conf.cache_ttl_secs != conference.cache_ttl_secs {
            changes.push("cache_ttl");
        }
        if old_conf.dtmf_enabled != conference.dtmf_enabled {
            changes.push("dtmf_enabled");
        }
        if old_conf.max_invite_rate_per_ip != conference.max_invite_rate_per_ip {
            changes.push("max_invite_rate");
        }
        if old_conf.session_expires_secs != conference.session_expires_secs {
            changes.push("session_expires");
        }
        if old_conf.max_participants_per_node != conference.max_participants_per_node {
            changes.push("max_participants_per_node");
        }
        if old_conf.enable_srtp != conference.enable_srtp {
            changes.push("enable_srtp");
        }
        if old_conf.jitter_buffer_ms != conference.jitter_buffer_ms {
            changes.push("jitter_buffer_ms");
        }
        if old_conf.join_secret.is_some() != conference.join_secret.is_some() {
            changes.push("join_secret");
        }
        if old_conf.max_announcement_size_mb != conference.max_announcement_size_mb {
            changes.push("max_announcement_size_mb");
        }
        if old_ai.enabled != ai.enabled {
            changes.push("ai.enabled");
        }
        if old_ai.model != ai.model {
            changes.push("ai.model");
        }

        info!(
            cache_ttl = conference.cache_ttl_secs,
            dtmf = conference.dtmf_enabled,
            ai_enabled = ai.enabled,
            changed_fields = ?changes,
            "Config hot-reload applied"
        );
        *self.conference_config.write() = conference;
        *self.ai_config.write() = ai;
    }

    /// Get the recordings directory path (convenience accessor).
    pub fn recordings_dir(&self) -> String {
        self.conference_config.read().recordings_dir.clone()
    }

    /// Get the number of active SIP sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Aggregate audio stats across all active sessions for Prometheus metrics.
    pub fn aggregate_audio_stats(&self) -> AggregatedAudioStats {
        let mut agg = AggregatedAudioStats::default();
        for entry in self.sessions.iter() {
            let stats = &entry.value().stats;
            agg.packets_received += stats.packets_received.load(Ordering::Relaxed);
            agg.packets_decoded += stats.packets_decoded.load(Ordering::Relaxed);
            agg.decode_errors += stats.decode_errors.load(Ordering::Relaxed);
            agg.packets_sent += stats.packets_sent.load(Ordering::Relaxed);
            agg.encode_errors += stats.encode_errors.load(Ordering::Relaxed);
            agg.dtmf_events += stats.dtmf_events.load(Ordering::Relaxed);
        }
        agg
    }

    /// Reap expired sessions (RFC 4028 session timer).
    ///
    /// Walks all sessions and removes any that haven't been refreshed
    /// (via re-INVITE or UPDATE) within `session_expires_secs`.
    /// Sends BYE to the expired participants.
    pub async fn reap_expired_sessions(&self) {
        let session_expires_secs = self.conference_config.read().session_expires_secs;
        if session_expires_secs == 0 {
            return; // Session timers disabled
        }
        let max_age = Duration::from_secs(session_expires_secs);

        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|entry| {
                // Trunks carry no session timer; the trunk reconciler
                // tears them down when the peer is gone.
                entry.value().trunk_peer.is_none()
                    && entry.value().last_refreshed.elapsed() > max_age
            })
            .map(|entry| entry.key().clone())
            .collect();

        for call_id in &expired {
            if let Some(session) = self.sessions.get(call_id) {
                warn!(
                    call_id = %call_id,
                    room = %session.room_number,
                    elapsed_secs = session.last_refreshed.elapsed().as_secs(),
                    "Session expired (no refresh), sending BYE"
                );
                if let Err(e) = self.uac.bye(&session.dialog).await {
                    warn!(call_id = %call_id, error = %e, "Failed to send BYE for expired session");
                }
                drop(session);
            }
            self.remove_session_for(
                call_id,
                fcp_sip::TerminationReason::system(
                    fcp_sip::SystemCause::SessionExpired,
                    "Session expired",
                ),
            );
        }

        if !expired.is_empty() {
            info!(count = expired.len(), "Reaped expired conference sessions");
        }
    }

    /// Check if a source IP has exceeded the INVITE rate limit.
    ///
    /// Uses a sliding window of 1 second. Returns `true` if the rate
    /// is within limits (OK to proceed), `false` if rate exceeded.
    /// Periodically evicts stale entries (older than 60s) to prevent unbounded growth.
    fn check_invite_rate(&self, peer_ip: std::net::IpAddr, max_rate: u32) -> bool {
        if max_rate == 0 {
            return true; // 0 = unlimited
        }
        let now = Instant::now();

        // Periodic cleanup: evict entries older than 60s (every ~100 INVITEs)
        // Also cap map size at 10,000 entries to bound memory
        let should_evict = self.invite_rate.len() > 100
            && self
                .invite_rate
                .iter()
                .next()
                .and_then(|e| now.checked_duration_since(e.value().1))
                .is_some_and(|elapsed| elapsed > Duration::from_secs(10));
        if should_evict {
            self.invite_rate.retain(|_, (_, window_start)| {
                now.duration_since(*window_start) < Duration::from_secs(60)
            });
            // Hard cap: if still over 10k, drop oldest half
            if self.invite_rate.len() > 10_000 {
                let mut count = 0;
                let to_remove = self.invite_rate.len() / 2;
                self.invite_rate.retain(|_, _| {
                    count += 1;
                    count > to_remove
                });
            }
        }

        let mut entry = self.invite_rate.entry(peer_ip).or_insert((0, now));
        let (count, window_start) = entry.value_mut();
        if now.duration_since(*window_start) > Duration::from_secs(1) {
            // New window
            *count = 1;
            *window_start = now;
            true
        } else if *count < max_rate {
            *count += 1;
            true
        } else {
            false // Rate exceeded
        }
    }

    /// Publish a conference event to the event bus and notify RFC 4575 subscribers.
    fn publish_event(&self, event: ConferenceEvent) {
        // Determine room number from the event for subscriber notification
        let room_number = match &event {
            ConferenceEvent::ParticipantJoined { room_number, .. }
            | ConferenceEvent::ParticipantLeft { room_number, .. }
            | ConferenceEvent::MeetingStarted { room_number, .. }
            | ConferenceEvent::MeetingEnded { room_number, .. }
            | ConferenceEvent::RoomDestroyed { room_number, .. }
            | ConferenceEvent::RoomLocked { room_number, .. }
            | ConferenceEvent::RoomUnlocked { room_number, .. }
            | ConferenceEvent::RecordingStarted { room_number, .. }
            | ConferenceEvent::RecordingStopped { room_number, .. }
            | ConferenceEvent::PinFailed { room_number, .. }
            | ConferenceEvent::MeetingCreated { room_number, .. }
            | ConferenceEvent::MeetingUpdated { room_number, .. }
            | ConferenceEvent::MeetingDeleted { room_number, .. }
            | ConferenceEvent::DialOutInvited { room_number, .. }
            | ConferenceEvent::DialOutFailed { room_number, .. }
            | ConferenceEvent::VideoStateChanged { room_number, .. }
            | ConferenceEvent::ContentStarted { room_number, .. }
            | ConferenceEvent::ContentStopped { room_number, .. }
            | ConferenceEvent::VideoOverload { room_number, .. }
            | ConferenceEvent::VideoLayoutChanged { room_number, .. }
            | ConferenceEvent::ActiveSpeakerChanged { room_number, .. } => {
                Some(room_number.clone())
            }
        };

        // Notify RFC 4575 subscribers for this room
        if let Some(ref rn) = room_number {
            self.notify_conference_subscribers(rn);
        }

        let event_bus = Arc::clone(&self.event_bus);
        let platform_event = PlatformEvent::Conference(event);
        tokio::spawn(async move {
            if let Err(e) = event_bus.publish(platform_event).await {
                warn!(error = %e, "Failed to publish conference event");
            }
        });
    }

    /// Send NOTIFY to all RFC 4575 subscribers for a given room.
    fn notify_conference_subscribers(&self, room_number: &str) {
        // Collect active subscribers for this room (and prune expired ones)
        let now = Instant::now();
        let mut expired = Vec::new();
        let mut targets: Vec<SipUri> = Vec::new();

        for entry in self.conference_subscriptions.iter() {
            if entry.value().room_number == room_number {
                if entry.value().expires_at <= now {
                    expired.push(entry.key().clone());
                } else {
                    targets.push(entry.value().contact_uri.clone());
                }
            }
        }

        // Remove expired subscriptions
        for call_id in expired {
            self.conference_subscriptions.remove(&call_id);
        }

        if targets.is_empty() {
            return;
        }

        // Build conference state XML
        let state_xml = if let Ok(room) = self.bridge.get_room(room_number) {
            build_conference_state_xml(room_number, &room)
        } else {
            // Room was destroyed, send empty state
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <conference-info xmlns=\"urn:ietf:params:xml:ns:conference-info\"\n  \
                   entity=\"sip:{}@conference\" state=\"full\" version=\"1\">\n  \
                   <users count=\"0\"/>\n\
                 </conference-info>\n",
                room_number,
            )
        };

        let uac = Arc::clone(&self.uac);
        tokio::spawn(async move {
            for target_uri in targets {
                let _ = uac
                    .send_unsolicited_notify(
                        &target_uri,
                        "conference",
                        "application/conference-info+xml",
                        &state_xml,
                    )
                    .await;
            }
        });
    }

    /// Remove a participant by SIP call ID (called on BYE).
    /// Remove a session for `reason` (`Hangup { by: None }` for a BYE from
    /// the far end), writing the leg's record.
    fn remove_session_for(&self, call_id: &str, reason: fcp_sip::TerminationReason) {
        if let Some((_, mut session)) = self.sessions.remove(call_id) {
            // A leg that was being moved leaves as a migration, whether
            // its holder reported first or simply hung up (phase 5).
            let migrated_to = self.migrations.take(call_id).map(|m| m.target_node);
            self.write_leg_record(&session, reason, migrated_to.as_deref());
            // Their own recording, if one ran, ends with them.
            self.finish_participant_recording(&session.room_number, &session.participant_id);
            // Update secondary room index
            if let Some(mut ids) = self.room_sessions.get_mut(&session.room_number) {
                ids.retain(|id| id != call_id);
                if ids.is_empty() {
                    drop(ids);
                    self.room_sessions.remove(&session.room_number);
                }
            }
            info!(
                call_id = %call_id,
                room = %session.room_number,
                participant = %session.participant_id,
                migrated_to = ?migrated_to,
                "Participant left conference"
            );

            // A BFCP endpoint's ids, and the floor it held, go with it (7e).
            if let Some(a) = &session.bfcp {
                self.bfcp.leave(&session.room_number, a.user_id);
            }

            // Abort the audio bridge task and validate it actually stops
            session.audio_bridge_handle.abort();
            let abort_handle = session.audio_bridge_handle;

            // Deallocate the RTP port pair after confirming bridge abort (async)
            let port_pool = Arc::clone(&self.port_pool);
            let port_pair = session.port_pair;
            let dealloc_call_id = call_id.to_string();
            tokio::spawn(async move {
                // Wait up to 5s for the audio bridge to actually stop
                match tokio::time::timeout(Duration::from_secs(5), abort_handle).await {
                    Ok(Ok(())) => {}                     // Normal exit
                    Ok(Err(e)) if e.is_cancelled() => {} // Aborted successfully
                    Ok(Err(e)) => {
                        warn!(
                            call_id = %dealloc_call_id,
                            error = %e,
                            "Audio bridge task panicked during abort"
                        );
                    }
                    Err(_) => {
                        warn!(
                            call_id = %dealloc_call_id,
                            "Audio bridge task did not stop within 5s after abort — possible orphan"
                        );
                    }
                }
                debug!(
                    call_id = %dealloc_call_id,
                    rtp_port = port_pair.rtp_port,
                    rtcp_port = port_pair.rtcp_port,
                    "Deallocating RTP port pair"
                );
                port_pool.deallocate(port_pair).await;
            });

            // The video leg: its bridge (when it has its own task) and ports.
            // The room drops the source and subscription with the participant.
            if let Some(video) = session.video.take() {
                if let Some(handle) = video.bridge_handle {
                    handle.abort();
                }
                self.release_video_pair_later(video.port_pair);
            }
            if let Some(content) = session.content.take() {
                if let Some(handle) = content.bridge_handle {
                    handle.abort();
                }
                self.release_video_pair_later(content.port_pair);
            }

            // Publish leave event (trunks are infrastructure, not participants)
            self.sync_room_registry(&session.room_number);
            if session.trunk_peer.is_none() {
                self.publish_event(ConferenceEvent::ParticipantLeft {
                    room_number: session.room_number.clone(),
                    participant_id: session.participant_id.clone(),
                    migrated_to,
                    tenant_id: session.tenant_id,
                    timestamp: SystemTime::now(),
                });
            }

            // Remove from forge-conference room
            if let Ok(room) = self.bridge.get_room(&session.room_number) {
                let _ = room.remove_participant(&session.participant_id);

                // Publish MeetingEnded when last participant leaves
                if room.participant_count() == 0 {
                    self.publish_event(ConferenceEvent::MeetingEnded {
                        room_number: session.room_number.clone(),
                        tenant_id: session.tenant_id,
                        timestamp: SystemTime::now(),
                    });
                }

                // Auto-destroy empty rooms if configured
                if room.participant_count() == 0
                    && self.conference_config.read().auto_destroy_empty_rooms
                {
                    let room_number = session.room_number.clone();
                    let bridge = Arc::clone(&self.bridge);
                    let storage = Arc::clone(&self.storage);
                    let event_bus = Arc::clone(&self.event_bus);
                    let tenant_id = session.tenant_id;

                    // Clean up DTMF handler for this room
                    self.dtmf_handlers.remove(&room_number);

                    // Clean up meeting cache for this room
                    self.meeting_cache.remove(&(tenant_id, room_number.clone()));

                    tokio::spawn(async move {
                        use fcp_core::storage::ConferenceStore;
                        // Re-check participant count — a new participant may have joined
                        // between our check and this task executing
                        if let Ok(room) = bridge.get_room(&room_number) {
                            if room.participant_count() > 0 {
                                debug!(room = %room_number, "Skipping auto-destroy: room is no longer empty");
                                return;
                            }
                        }
                        let should_destroy = match storage
                            .get_meeting_by_room_number(&tenant_id, &room_number)
                            .await
                        {
                            Ok(Some(meeting)) => !meeting.permanent,
                            _ => true,
                        };
                        if should_destroy {
                            debug!(room = %room_number, "Destroying empty conference room");
                            let _ = bridge.delete_room(&room_number);

                            let event = PlatformEvent::Conference(ConferenceEvent::RoomDestroyed {
                                room_number,
                                tenant_id,
                                timestamp: SystemTime::now(),
                            });
                            let _ = event_bus.publish(event).await;
                        }
                    });
                }
            }
        } else {
            debug!(call_id = %call_id, "No conference session found for BYE");
        }
    }

    /// Where this node's conference leg records go, and the region they
    /// name. Set once at start-up (`[cdr]`); unset, no records are written.
    pub fn set_cdr(&mut self, writer: fcp_sip::CdrWriter, region: Option<String>) {
        self.cdr = Some(writer);
        self.region = region;
    }

    /// The record of a conference leg that ended for `reason`
    /// (`docs/CDR_PIPELINE.md` §5): `kind = conference`, the call id the
    /// leg arrived with, the participant, the room, packet counts.
    fn write_leg_record(
        &self,
        session: &ParticipantSession,
        reason: fcp_sip::TerminationReason,
        migrated_to: Option<&str>,
    ) {
        let Some(writer) = &self.cdr else {
            return;
        };
        let reason = match (migrated_to, reason) {
            (Some(node), _) => fcp_sip::TerminationReason::system(
                fcp_sip::SystemCause::Migration,
                format!("Moved to node {node}"),
            ),
            // A BYE from the far end: the participant, or the destination
            // of a dial-out.
            (None, fcp_sip::TerminationReason::Hangup { by: None }) => {
                fcp_sip::TerminationReason::hangup_by(if session.is_dial_out {
                    fcp_core::storage::HangupParty::Callee
                } else {
                    fcp_core::storage::HangupParty::Caller
                })
            }
            (None, reason) => reason,
        };
        let record = fcp_cdr::RecordBuilder::new(
            fcp_core::storage::CdrKind::Conference,
            session.record.call_id,
            session.tenant_id,
        )
        .sip_call_id(session.call_id.clone())
        .from(
            session.record.from_uri.clone(),
            session.display_name.clone(),
        )
        .to(
            session
                .record
                .to_uri
                .clone()
                .unwrap_or_else(|| format!("conference:{}", session.room_number)),
            None,
        )
        .started_at(session.joined_at)
        .answered_at(Some(session.joined_at))
        .ended_at(Some(SystemTime::now()))
        .reason(reason)
        .media_anchored(true)
        .codecs(Some(session.codec_name.clone()), None)
        .rtp_packets(
            session.stats.packets_sent.load(Ordering::Relaxed),
            session.stats.packets_received.load(Ordering::Relaxed),
        )
        .node(Some(self.node_id.clone()), self.region.clone())
        .custom_data(Some(serde_json::json!({
            "room_number": session.room_number,
            "participant_id": session.participant_id,
            "is_host": session.is_host,
            "dial_out": session.is_dial_out,
            "trunk_peer": session.trunk_peer,
            "joined_by_token": session.joined_by_token,
            "video": session.video.is_some(),
            "content": session.content.is_some(),
            "migrated_from": session.record.migrated_from,
            "migrated_to": migrated_to,
        })))
        .build();
        writer.write(record);
    }

    /// Build a forge-conference RoomConfig from a Meeting's settings.
    ///
    /// Validates max_channels against the node-level participant limit to prevent
    /// individual rooms from being configured beyond what the node can support.
    fn meeting_to_room_config(&self, meeting: &Meeting) -> RoomConfig {
        let conf = self.conference_config.read();
        let max_per_node = conf.max_participants_per_node;
        let mut max_channels = if meeting.max_participants > 0 {
            Some(meeting.max_participants)
        } else {
            None
        };

        // Cap room max_channels at node limit (if both are set)
        if let Some(room_max) = max_channels {
            if max_per_node > 0 && room_max > max_per_node {
                warn!(
                    room = %meeting.room_number,
                    room_max = room_max,
                    node_max = max_per_node,
                    "Room max_participants exceeds node limit, capping"
                );
                max_channels = Some(max_per_node);
            }
        }

        // Resolve sound file path: per-meeting override takes priority over global config.
        // Relative paths are resolved against announcements_dir.
        let resolve_sound =
            |meeting_val: &Option<String>, global_val: &Option<String>| -> Option<Option<String>> {
                let chosen = meeting_val.as_ref().or(global_val.as_ref());
                chosen.map(|p| {
                    let path = std::path::Path::new(p);
                    if path.is_absolute() {
                        Some(p.clone())
                    } else {
                        Some(format!("{}/{}", conf.announcements_dir, p))
                    }
                })
            };

        RoomConfig {
            guest_pin: meeting.guest_pin.clone(),
            host_pin: meeting.host_pin.clone(),
            require_guest_pin: Some(meeting.require_guest_pin),
            max_pin_attempts: None,
            default_locked: Some(meeting.default_locked),
            enable_dtmf: None,
            dtmf_commands: None,
            max_channels,
            wait_for_moderator: Some(meeting.wait_for_moderator),
            min_users: None,
            min_recording_participants: if meeting.auto_record { Some(1) } else { None },
            join_sound: if meeting.play_entry_tone {
                resolve_sound(&meeting.join_sound, &conf.join_sound)
            } else {
                Some(None) // Explicitly disable
            },
            exit_sound: if meeting.play_exit_tone {
                resolve_sound(&meeting.exit_sound, &conf.exit_sound)
            } else {
                Some(None)
            },
            alone_sound: None,
            invalid_pin_sound: resolve_sound(&meeting.invalid_pin_sound, &conf.invalid_pin_sound),
            locked_sound: None,
            unlocked_sound: None,
            kicked_sound: None,
            max_members_sound: None,
            moh_sound: resolve_sound(&meeting.moh_sound, &conf.moh_sound),
            pin_prompt_sound: resolve_sound(&meeting.pin_prompt_sound, &conf.pin_prompt_sound),
            recording_started_sound: None,
            recording_stopped_sound: None,
        }
    }

    /// Return the cached local SIP URI (parsed once at construction — no runtime panics).
    fn parse_local_sip_uri(&self) -> SipUri {
        self.local_sip_uri.clone()
    }

    /// Check whether PIN authentication is required for a meeting.
    fn meeting_requires_pin(meeting: &Meeting) -> bool {
        meeting.require_guest_pin && (meeting.guest_pin.is_some() || meeting.host_pin.is_some())
    }

    /// Negotiate SDES-SRTP from a parsed crypto line.
    ///
    /// Decodes the remote key material, generates a local key, and returns both
    /// the `SrtpContext` (for the audio bridge) and the `SrtpSdpContext` (for
    /// the SDP answer).
    /// The SDP answer: one section per offered section, video answered
    /// when `video` is given and a codec agrees, rejected (port 0)
    /// otherwise. An offer that does not parse gets the legacy audio-only
    /// answer.
    fn build_answer(
        &self,
        offer: &str,
        audio_rtp_port: u16,
        audio_srtp: Option<&SrtpSdpContext>,
        video: Option<VideoAnswer<'_>>,
        content: Option<VideoAnswer<'_>>,
        bfcp: Option<fcp_media::BfcpAnswer<'_>>,
    ) -> fcp_media::Answer {
        match construct_answer(
            offer,
            &AnswerOptions {
                server_ip: &self.local_addr,
                audio_rtp_port,
                audio_srtp,
                video,
                content,
                bfcp,
            },
        ) {
            Ok(answer) => answer,
            Err(e) => {
                debug!(error = %e, "SDP offer did not parse; answering audio only");
                fcp_media::Answer {
                    sdp: construct_answer_for_a_leg(
                        offer,
                        &self.local_addr,
                        audio_rtp_port,
                        audio_srtp,
                    ),
                    video: None,
                    content: None,
                    bfcp: None,
                }
            }
        }
    }

    /// The BFCP answer that repeats a session's assignment on a re-INVITE
    /// or UPDATE, when the offer still carries a labelled content section
    /// and this node still listens.
    fn bfcp_answer_for<'a>(
        &self,
        session: &ParticipantSession,
        label: Option<&'a str>,
    ) -> Option<fcp_media::BfcpAnswer<'a>> {
        let a = session.bfcp.as_ref()?;
        let bind = self.conference_config.read().bfcp_bind?;
        Some(fcp_media::BfcpAnswer {
            port: bind.port(),
            conference_id: a.conference_id,
            user_id: a.user_id,
            floor_id: a.floor_id,
            label: label?,
        })
    }

    /// Give a port pair back once whatever used it has had a moment to stop.
    fn release_video_pair_later(&self, pair: PortPair) {
        let pool = Arc::clone(&self.port_pool);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            pool.deallocate(pair).await;
        });
    }

    /// Give back a video port pair that will not be used after all.
    fn release_video_port(&self, pair: &mut Option<PortPair>) {
        if let Some(p) = pair.take() {
            let pool = Arc::clone(&self.port_pool);
            tokio::spawn(async move { pool.deallocate(p).await });
        }
    }

    /// Give back a participant's ports when the join fails after allocation.
    fn release_ports(&self, audio: PortPair, video: Option<PortPair>) {
        let pool = Arc::clone(&self.port_pool);
        tokio::spawn(async move {
            pool.deallocate(audio).await;
            if let Some(v) = video {
                pool.deallocate(v).await;
            }
        });
    }

    /// Attach a negotiated video leg to the room and spawn its bridge.
    /// Returns the leg with its handle, or `None` (ports released) when
    /// the room refused.
    ///
    /// `may_share` is the meeting's `video_content_share` policy for this
    /// participant; it matters only to a content leg (§15.6).
    fn start_video_leg(
        &self,
        room: &Arc<forge_conference::ConferenceRoom>,
        participant_id: &str,
        display_name: Option<&str>,
        tenant_id: TenantId,
        room_number: &str,
        mut leg: VideoLeg,
        may_share: bool,
    ) -> Option<VideoLeg> {
        let Some(backend) = self.video_backend.as_ref() else {
            self.release_video_pair_later(leg.port_pair);
            return None;
        };
        let sink = self.video_event_sink(tenant_id, room_number);
        // A BFCP endpoint's floor is explicit (7e): its policy is noted,
        // and its implicit sending switched off, whichever way it goes.
        if leg.kind == StreamKind::Content {
            self.bfcp
                .set_may_share(room_number, participant_id, may_share);
        }
        let fut = match leg.kind {
            StreamKind::Camera => {
                video_bridge::attach_video(room, backend, participant_id, display_name, &leg, sink)
                    .map(|f| Box::pin(f) as std::pin::Pin<Box<dyn Future<Output = ()> + Send>>)
            }
            StreamKind::Content => {
                video_bridge::attach_content(room, backend, participant_id, &leg, sink, may_share)
                    .map(|f| Box::pin(f) as std::pin::Pin<Box<dyn Future<Output = ()> + Send>>)
            }
        };
        match fut {
            Some(fut) => {
                leg.bridge_handle = Some(tokio::spawn(fut));
                Some(leg)
            }
            None => {
                self.release_video_pair_later(leg.port_pair);
                None
            }
        }
    }

    /// Stop one of a participant's video legs — the camera's or the
    /// shared screen's: the bridge, the room's source and subscription,
    /// the ports. The participant stays in the call.
    fn tear_down_leg(&self, session: &mut ParticipantSession, kind: StreamKind) {
        let slot = match kind {
            StreamKind::Camera => &mut session.video,
            StreamKind::Content => &mut session.content,
        };
        if let Some(leg) = slot.take() {
            if let Some(handle) = leg.bridge_handle {
                handle.abort();
            }
            if let Ok(room) = self.bridge.get_room(&session.room_number) {
                match kind {
                    StreamKind::Camera => {
                        video_bridge::detach_video(&room, &session.participant_id)
                    }
                    StreamKind::Content => {
                        video_bridge::detach_content(&room, &session.participant_id)
                    }
                }
            }
            self.release_video_pair_later(leg.port_pair);
        }
    }

    /// A video leg for a participant adding a stream mid-call — the camera
    /// or a shared screen: the meeting must allow it, the codec must
    /// agree, SRTP rules as at join. Ports and the socket are allocated
    /// here; the leg is not yet attached. Returns the leg and the
    /// meeting's `video_content_share` policy.
    async fn prepare_leg(
        &self,
        kind: StreamKind,
        tenant_id: &TenantId,
        room_number: &str,
        sdp_offer: &str,
        conf: &ConferenceSection,
    ) -> Option<(VideoLeg, String)> {
        let backend = self.video_backend.as_ref()?;
        let meeting = self
            .lookup_meeting(tenant_id, room_number)
            .await
            .ok()
            .flatten()?;
        if kind == StreamKind::Content && meeting.video_content_share == "off" {
            debug!(room = %room_number, "re-INVITE: a shared screen offered; sharing is off for this meeting");
            return None;
        }
        let usable = video_bridge::usable_codecs(backend, conf.video_native_codecs_only);
        let (settings, codecs) = video_bridge::plan_video(
            &meeting,
            &usable,
            Duration::from_secs(conf.content_idle_secs),
        )?;
        let crypto = match kind {
            StreamKind::Camera => parse_video_sdes_crypto(sdp_offer),
            StreamKind::Content => parse_content_sdes_crypto(sdp_offer),
        };
        let (srtp_context, srtp_sdp) = if conf.enable_srtp {
            match crypto.first().map(Self::negotiate_srtp) {
                Some(Ok((ctx, sdp_ctx))) => {
                    (Some(Arc::new(tokio::sync::RwLock::new(ctx))), Some(sdp_ctx))
                }
                Some(Err(e)) => {
                    warn!(room = %room_number, error = %e, "re-INVITE: video SRTP negotiation failed; video rejected");
                    return None;
                }
                None => {
                    info!(room = %room_number, "re-INVITE: video offered without crypto while SRTP is on; rejected");
                    return None;
                }
            }
        } else {
            (None, None)
        };
        let pair = match self.port_pool.allocate().await {
            Ok(p) => p,
            Err(e) => {
                warn!(room = %room_number, error = %e, "re-INVITE: no port pair left for video");
                return None;
            }
        };
        // The answer builder decides codec and payload type; probe it now
        // so the leg is complete before the session is touched.
        let answer = VideoAnswer {
            rtp_port: pair.rtp_port,
            codecs: &codecs,
            srtp: srtp_sdp.as_ref(),
        };
        let probe = match kind {
            StreamKind::Camera => {
                self.build_answer(sdp_offer, pair.rtp_port, None, Some(answer), None, None)
            }
            StreamKind::Content => {
                self.build_answer(sdp_offer, pair.rtp_port, None, None, Some(answer), None)
            }
        };
        let negotiated = match kind {
            StreamKind::Camera => probe.video,
            StreamKind::Content => probe.content,
        };
        let Some(negotiated) = negotiated else {
            self.release_video_port(&mut Some(pair));
            return None;
        };
        let socket = match RtpSocketPair::new(pair, RtpSocketConfig::default()).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                warn!(rtp_port = pair.rtp_port, error = %e, "re-INVITE: failed to create video socket pair");
                self.release_video_port(&mut Some(pair));
                return None;
            }
        };
        let fallback = match kind {
            StreamKind::Camera => fcp_media::remote_video_endpoint(sdp_offer),
            StreamKind::Content => fcp_media::remote_content_endpoint(sdp_offer),
        };
        if let Some(remote) = negotiated.remote.or(fallback) {
            socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote,
                    rtcp_addr: Some(SocketAddr::new(remote.ip(), remote.port().wrapping_add(1))),
                })
                .await;
        }
        Some((
            VideoLeg {
                kind,
                port_pair: pair,
                socket,
                srtp_context,
                srtp_sdp,
                negotiated,
                settings,
                stats: Arc::new(VideoLegStats::default()),
                bridge_handle: None,
            },
            meeting.video_content_share,
        ))
    }

    /// A kept leg on a re-INVITE: the peer's address may have moved and
    /// its key may have rolled. Returns our key for the answer when it
    /// rolled.
    fn refresh_leg(
        &self,
        call_id: &str,
        leg: &VideoLeg,
        new_remote: Option<SocketAddr>,
        crypto: Option<&fcp_media::SdesCryptoLine>,
    ) -> Option<SrtpSdpContext> {
        if let Some(new_remote) = new_remote {
            let socket = Arc::clone(&leg.socket);
            tokio::spawn(async move {
                socket
                    .set_remote_endpoint(RemoteEndpoint {
                        rtp_addr: new_remote,
                        rtcp_addr: Some(SocketAddr::new(
                            new_remote.ip(),
                            new_remote.port().wrapping_add(1),
                        )),
                    })
                    .await;
            });
        }
        let crypto = crypto?;
        match (Self::negotiate_srtp(crypto), leg.srtp_context.as_ref()) {
            (Ok((new_ctx, sdp_ctx)), Some(lock)) => {
                info!(call_id = %call_id, stream = %leg.kind, "re-INVITE: video SRTP key rollover");
                let lock = Arc::clone(lock);
                tokio::spawn(async move {
                    *lock.write().await = new_ctx;
                });
                Some(sdp_ctx)
            }
            (Ok(_), None) => {
                warn!(call_id = %call_id, stream = %leg.kind, "re-INVITE: video started plain and cannot switch to SRTP mid-call; staying plain");
                None
            }
            (Err(e), _) => {
                warn!(error = %e, stream = %leg.kind, "video SRTP renegotiation failed, keeping existing keys");
                None
            }
        }
    }

    fn negotiate_srtp(crypto: &fcp_media::SdesCryptoLine) -> Result<(SrtpContext, SrtpSdpContext)> {
        let (key, salt, profile) = decode_sdes_key_params(&crypto.key_params, &crypto.suite)?;
        let remote_key = SrtpKeyMaterial::new(key, salt, profile)
            .map_err(|e| anyhow::anyhow!("Invalid SRTP key material: {}", e))?;

        let (local_key, b64) = generate_srtp_key_material()?;

        let mut ctx = SrtpContext::new();
        ctx.set_remote_key(remote_key);
        ctx.set_local_key(local_key);

        let sdp_ctx = SrtpSdpContext {
            local_key_b64: format!("inline:{}", b64),
            suite: crypto.suite.clone(),
            tag: crypto.tag,
        };

        debug!(suite = %crypto.suite, tag = crypto.tag, "SRTP negotiated");
        Ok((ctx, sdp_ctx))
    }
}

/// Play pre-loaded PCM samples directly to a participant's RTP socket.
///
/// Used for PIN prompts, invalid PIN sounds, and MOH — situations where the
/// participant is NOT yet in the mixer so we can't use `play_sound_to_all()`.
///
/// The samples are expected at the mixer sample rate. If the codec operates at
/// a different rate, the audio is resampled before encoding.
pub async fn play_prompt_to_rtp(
    rtp_socket: &RtpSocketPair,
    samples: &[i16],
    encoder: &mut dyn AudioCodec,
    payload_type: u8,
    frame_size: usize,
    srtp_context: &Option<Arc<tokio::sync::RwLock<SrtpContext>>>,
    mixer_sample_rate: u32,
) {
    let codec_rate = encoder.native_format().sample_rate;
    let need_resample = codec_rate != mixer_sample_rate;

    // Resample from mixer rate to codec rate if needed
    let codec_samples = if need_resample {
        match forge_resampler::Resampler::new(mixer_sample_rate, codec_rate, 1) {
            Ok(mut rs) => match rs.resample(samples) {
                Ok(resampled) => resampled,
                Err(e) => {
                    warn!(error = %e, "Resample failed for prompt playback");
                    return;
                }
            },
            Err(e) => {
                warn!(error = %e, "Failed to create resampler for prompt");
                return;
            }
        }
    } else {
        samples.to_vec()
    };

    let mut seq: u16 = rand::random();
    let ssrc: u32 = rand::random();
    let mut timestamp: u32 = rand::random();
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    // Consume the first immediate tick
    interval.tick().await;

    for chunk in codec_samples.chunks(frame_size) {
        // Pad final short chunk with silence
        let frame = if chunk.len() < frame_size {
            let mut padded = chunk.to_vec();
            padded.resize(frame_size, 0);
            padded
        } else {
            chunk.to_vec()
        };

        match encoder.encode(&frame) {
            Ok(encoded) => {
                let packet = forge_rtp::RtpPacket::build(
                    payload_type,
                    seq,
                    timestamp,
                    ssrc,
                    bytes::Bytes::from(encoded),
                    false,
                );

                let send_result = if let Some(ref ctx) = srtp_context {
                    let packet_bytes = packet.to_bytes();
                    let protect_result = {
                        let mut guard = ctx.write().await;
                        guard.protect_rtp(&packet_bytes)
                    };
                    match protect_result {
                        Ok(protected) => rtp_socket.send_rtp_raw(&protected).await,
                        Err(e) => {
                            debug!(error = %e, "SRTP protect error during prompt playback");
                            Ok(0)
                        }
                    }
                } else {
                    rtp_socket.send_rtp(&packet).await
                };

                if let Err(e) = send_result {
                    debug!(error = %e, "RTP send error during prompt playback");
                    return;
                }

                seq = seq.wrapping_add(1);
                timestamp = timestamp.wrapping_add(frame_size as u32);
            }
            Err(e) => {
                debug!(error = %e, "Encode error during prompt playback");
            }
        }

        // Pace at 20ms intervals for real-time playback
        interval.tick().await;
    }
}

/// Inter-node conference trunks (see [`crate::trunks`]).
impl ConferenceCallHandler {
    /// Get or create a room and make sure its frame clock is running.
    /// Every room this server mixes runs on the frame clock: it is what
    /// lets each participant, the recorder, an AI tap and every trunk
    /// read the same frame without consuming it from one another.
    fn room_for(&self, room_number: &str) -> Result<Arc<forge_conference::ConferenceRoom>> {
        let room = self
            .bridge
            .get_or_create_room(room_number, None)
            .context("Failed to create conference room")?;
        room.start_frame_clock()
            .context("Failed to start the room's frame clock")?;
        Ok(room)
    }

    /// [`room_for`](Self::room_for) for a caller joining: when this call
    /// creates the room on this node, the room inherits the cluster-wide
    /// lock flag (phase 3), so a room locked on the nodes already hosting
    /// it is locked here too before anyone is admitted.
    async fn room_for_join(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
    ) -> Result<Arc<forge_conference::ConferenceRoom>> {
        let existed = self.bridge.get_room(room_number).is_ok();
        let room = self.room_for(room_number)?;
        if existed {
            return Ok(room);
        }
        let registry = self.room_registry.read().unwrap().clone();
        if let Some(registry) = registry {
            if registry
                .room_locked(&tenant_id.to_string(), room_number)
                .await
            {
                room.lock();
                info!(room = %room_number, "Room created locked: inherited the cluster-wide lock");
            }
        }
        Ok(room)
    }

    /// Lock or unlock the room on this node only. `Err` when the room is
    /// not active here.
    pub fn set_room_locked(&self, room_number: &str, locked: bool) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|_| anyhow::anyhow!("Conference room not active: {}", room_number))?;
        if locked {
            room.lock();
        } else {
            room.unlock();
        }
        Ok(())
    }

    /// This node's cluster id.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Peer nodes this node has a trunk with for a room, whichever side
    /// dialed.
    pub fn room_trunk_peers(&self, room_number: &str) -> Vec<String> {
        let Some(call_ids) = self.room_sessions.get(room_number) else {
            return Vec::new();
        };
        let mut peers: Vec<String> = call_ids
            .iter()
            .filter_map(|call_id| self.sessions.get(call_id))
            .filter_map(|session| session.trunk_peer.clone())
            .collect();
        peers.sort();
        peers.dedup();
        peers
    }

    /// Hang up every trunk to `peer` in a room: local teardown at once,
    /// the BYE sent in the background with a bound. The peer may be the
    /// reason for the hangup — dead, or gone from the cluster — and a
    /// BYE that never gets answered must not stall the reconciler.
    pub async fn hangup_trunk(&self, room_number: &str, peer: &str) {
        let call_ids: Vec<String> = self
            .room_sessions
            .get(room_number)
            .map(|ids| ids.clone())
            .unwrap_or_default();
        for call_id in call_ids {
            let dialog = match self.sessions.get(&call_id) {
                Some(session) if session.trunk_peer.as_deref() == Some(peer) => {
                    session.dialog.clone()
                }
                _ => continue,
            };
            self.remove_session_for(
                &call_id,
                fcp_sip::TerminationReason::system(fcp_sip::SystemCause::Other, "Trunk closed"),
            );
            let uac = Arc::clone(&self.uac);
            let room = room_number.to_string();
            let peer = peer.to_string();
            tokio::spawn(async move {
                match tokio::time::timeout(BYE_TIMEOUT, uac.bye(&dialog)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        warn!(room = %room, peer = %peer, error = %e, "BYE for conference trunk failed")
                    }
                    Err(_) => {
                        debug!(room = %room, peer = %peer, "BYE for conference trunk unanswered")
                    }
                }
            });
        }
    }

    /// Trunks in a room that look dead: joined at least `timeout` ago and
    /// no RTP received for `timeout` (or ever). A peer sends a frame
    /// every 20 ms while its trunk is up, so silence this long means the
    /// peer crashed, restarted, or dropped the session without a BYE.
    pub fn silent_trunks(&self, room_number: &str, timeout: Duration) -> Vec<String> {
        let Some(call_ids) = self.room_sessions.get(room_number) else {
            return Vec::new();
        };
        let mut peers: Vec<String> = call_ids
            .iter()
            .filter_map(|call_id| self.sessions.get(call_id))
            .filter(|session| {
                session.trunk_peer.is_some()
                    && session.joined_at.elapsed().unwrap_or_default() >= timeout
                    && session
                        .stats
                        .last_rx_age()
                        .map(|age| age >= timeout)
                        .unwrap_or(true)
            })
            .filter_map(|session| session.trunk_peer.clone())
            .collect();
        peers.sort();
        peers.dedup();
        peers
    }

    /// Whether trunks this node dials offer SRTP (`cluster.trunk_srtp`).
    pub fn set_trunk_srtp(&self, enabled: bool) {
        self.trunk_srtp.store(enabled, Ordering::Relaxed);
    }

    /// Codec list this node offers a peer, in preference order: wideband
    /// first, filtered by `allowed_codecs`.
    fn trunk_offer_codecs(&self) -> Vec<(&'static str, u8)> {
        const ALL: [(&str, u8); 3] = [("G722", 9), ("PCMU", 0), ("PCMA", 8)];
        let allowed = self.conference_config.read().allowed_codecs.clone();
        let filtered: Vec<(&'static str, u8)> = match allowed {
            Some(list) => ALL
                .iter()
                .copied()
                .filter(|(name, _)| list.iter().any(|a| a.eq_ignore_ascii_case(name)))
                .collect(),
            None => ALL.to_vec(),
        };
        if filtered.is_empty() {
            ALL.to_vec()
        } else {
            filtered
        }
    }

    /// Dial the trunk to `peer_node` for a room this node hosts. Called by
    /// the trunk manager on the lower-id side only.
    pub async fn dial_trunk(
        &self,
        tenant_id: TenantId,
        room_number: &str,
        peer_node: &str,
        target: SocketAddr,
    ) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|_| anyhow::anyhow!("room {} is not hosted on this node", room_number))?;
        if self.countable_participants(room_number) == 0 {
            bail!("room {} has no local participants to trunk", room_number);
        }
        let conf = self.conference_config.read().clone();

        let port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port pair for conference trunk")?;
        let dealloc = |pool: Arc<PortPool>, pair: PortPair| {
            tokio::spawn(async move { pool.deallocate(pair).await });
        };

        let ip = self
            .local_addr
            .split(':')
            .next()
            .unwrap_or(&self.local_addr);
        let codecs: Vec<fcp_media::SdpCodecInfo> = self
            .trunk_offer_codecs()
            .into_iter()
            .map(|(name, pt)| fcp_media::SdpCodecInfo {
                payload_type: pt,
                encoding_name: name.to_string(),
                clock_rate: 8000,
            })
            .collect();
        // SRTP: offer our key per section; the peer's comes back in the
        // answer. Both sections carry their own, as any other leg's do.
        let offer_srtp = self.trunk_srtp.load(Ordering::Relaxed);
        let audio_srtp = offer_srtp
            .then(fcp_media::SrtpOffer::generate)
            .transpose()
            .map_err(|e| anyhow::anyhow!("SRTP key generation failed: {}", e))?;

        // Video on the trunk when the meeting has it and this node can
        // encode something the meeting allows (§15.4, 5b). A meeting that
        // turns video on later gets it when its trunks are next
        // re-established, which reconcile does.
        let planned = self.plan_trunk_video(&tenant_id, room_number, &conf).await;
        let video_srtp = match (&planned, offer_srtp) {
            (Some(_), true) => Some(
                fcp_media::SrtpOffer::generate()
                    .map_err(|e| anyhow::anyhow!("SRTP key generation failed: {}", e))?,
            ),
            _ => None,
        };
        let video_pair = match &planned {
            Some(_) => match self.port_pool.allocate().await {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(room = %room_number, peer = %peer_node, error = %e, "No port pair left for trunk video; trunking audio only");
                    None
                }
            },
            None => None,
        };
        let video_section = match (&planned, video_pair) {
            (Some(plan), Some(pair)) => Some(fcp_media::VideoOfferSection {
                rtp_port: pair.rtp_port,
                payload_type: TRUNK_VIDEO_PAYLOAD_TYPE,
                codec: plan.codec,
                fmtp: None,
                feedback: true,
                srtp: video_srtp.as_ref().map(|o| &o.sdp),
            }),
            _ => None,
        };
        // A shared screen crosses the trunk on its own section when the
        // meeting allows sharing (§15.6, 7d), beside the video one.
        let wants_content =
            planned.as_ref().is_some_and(|p| p.settings.content) && video_pair.is_some();
        let content_srtp = match (wants_content, offer_srtp) {
            (true, true) => Some(
                fcp_media::SrtpOffer::generate()
                    .map_err(|e| anyhow::anyhow!("SRTP key generation failed: {}", e))?,
            ),
            _ => None,
        };
        let content_pair = if wants_content {
            match self.port_pool.allocate().await {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(room = %room_number, peer = %peer_node, error = %e, "No port pair left for trunk content; the screen stays on this node");
                    None
                }
            }
        } else {
            None
        };
        let content_section = match (&planned, content_pair) {
            (Some(plan), Some(pair)) => Some(fcp_media::VideoOfferSection {
                rtp_port: pair.rtp_port,
                payload_type: TRUNK_CONTENT_PAYLOAD_TYPE,
                codec: plan.codec,
                fmtp: None,
                feedback: true,
                srtp: content_srtp.as_ref().map(|o| &o.sdp),
            }),
            _ => None,
        };
        let sdp_offer = match fcp_media::construct_offer(&fcp_media::OfferOptions {
            server_ip: ip,
            origin_id: "0",
            session_name: "FCP Conference Trunk",
            audio: fcp_media::AudioOfferSection {
                rtp_port: port_pair.rtp_port,
                codecs: &codecs,
                telephone_event: None,
                srtp: audio_srtp.as_ref().map(|o| &o.sdp),
                ptime_ms: Some(20),
            },
            video: video_section,
            content: content_section,
        }) {
            Ok(sdp) => sdp,
            Err(e) => {
                for pair in [video_pair, content_pair].into_iter().flatten() {
                    self.release_video_pair_later(pair);
                }
                dealloc(Arc::clone(&self.port_pool), port_pair);
                bail!("could not build the trunk offer: {e}");
            }
        };

        let target_uri = format!("sip:{}@{};transport=tls", room_number, target);
        let mut headers = Headers::new();
        headers
            .push(TRUNK_NODE_HEADER, &self.node_id)
            .map_err(|e| anyhow::anyhow!("trunk header: {e}"))?;
        headers
            .push(TRUNK_TENANT_HEADER, tenant_id.to_string())
            .map_err(|e| anyhow::anyhow!("trunk header: {e}"))?;

        info!(room = %room_number, peer = %peer_node, target = %target_uri, "Sending conference trunk INVITE");
        // Every video port pair goes back if the trunk never comes up.
        let release_video = |handler: &Self, pair: Option<PortPair>| {
            for pair in [pair, content_pair].into_iter().flatten() {
                handler.release_video_pair_later(pair);
            }
        };
        let call_handle = match self
            .uac
            .invite_with_headers(&target_uri, Some(&sdp_offer), headers)
            .await
        {
            Ok(h) => h,
            Err(e) => {
                release_video(self, video_pair);
                dealloc(Arc::clone(&self.port_pool), port_pair);
                bail!("trunk INVITE failed: {}", e);
            }
        };

        let timeout = Duration::from_secs(conf.dial_out_timeout_secs);
        let response = match tokio::time::timeout(timeout, call_handle.await_final()).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                release_video(self, video_pair);
                dealloc(Arc::clone(&self.port_pool), port_pair);
                bail!("trunk call failed: {}", e);
            }
            Err(_) => {
                let _ = call_handle.cancel().await;
                release_video(self, video_pair);
                dealloc(Arc::clone(&self.port_pool), port_pair);
                bail!(
                    "trunk INVITE timed out after {}s",
                    conf.dial_out_timeout_secs
                );
            }
        };
        let status_code = response.code();
        if !(200..300).contains(&status_code) {
            release_video(self, video_pair);
            dealloc(Arc::clone(&self.port_pool), port_pair);
            bail!("peer rejected trunk: {} {}", status_code, response.reason());
        }

        let answer_body = response.body();
        let answer_sdp = std::str::from_utf8(&answer_body[..]).unwrap_or("");
        let answer_codecs = parse_sdp_codecs(answer_sdp);
        let (codec_name, payload_type) = answer_codecs
            .iter()
            .find(|c| !c.encoding_name.eq_ignore_ascii_case("telephone-event"))
            .map(|c| (c.encoding_name.to_uppercase(), c.payload_type))
            .unwrap_or_else(|| ("PCMU".to_string(), 0));
        let frame_size: usize = if codec_name == "G722" { 320 } else { 160 };

        let dialog = call_handle.dialog.read().await.clone();
        let call_id = dialog.id().call_id().to_string();

        // We offered SRTP: the answer must carry the peer's key, or the
        // trunk would silently run in the clear on one side.
        let srtp = match audio_srtp {
            Some(offer) => {
                let crypto_lines = parse_sdes_crypto(answer_sdp);
                let Some(crypto) = crypto_lines.first() else {
                    let _ = tokio::time::timeout(BYE_TIMEOUT, self.uac.bye(&dialog)).await;
                    release_video(self, video_pair);
                    dealloc(Arc::clone(&self.port_pool), port_pair);
                    bail!(
                        "peer {} answered the trunk without SRTP; set cluster.trunk_srtp the same on every conference node",
                        peer_node
                    );
                };
                match offer.accept(crypto) {
                    Ok(ctx) => Some(ctx),
                    Err(e) => {
                        let _ = tokio::time::timeout(BYE_TIMEOUT, self.uac.bye(&dialog)).await;
                        release_video(self, video_pair);
                        dealloc(Arc::clone(&self.port_pool), port_pair);
                        bail!("trunk SRTP answer from {} unusable: {}", peer_node, e);
                    }
                }
            }
            None => None,
        };

        let rtp_socket = match RtpSocketPair::new(port_pair, RtpSocketConfig::default()).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                let _ = self.uac.bye(&dialog).await;
                release_video(self, video_pair);
                dealloc(Arc::clone(&self.port_pool), port_pair);
                bail!("failed to create RTP socket for trunk: {}", e);
            }
        };
        let remote_addr = parse_remote_rtp_addr(answer_sdp);
        if let Some(remote_addr) = remote_addr {
            rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;
        } else {
            warn!(room = %room_number, peer = %peer_node, "Trunk answer has no RTP address");
        }

        // The video half, when we offered one and the peer took it. A
        // peer that answers video without the crypto we asked for is
        // hung up, as it is for audio: half an encrypted trunk is worse
        // than none.
        let video = match (&planned, video_pair) {
            (Some(plan), Some(pair)) => {
                match self
                    .trunk_video_from_answer(answer_sdp, plan, pair, video_srtp, StreamKind::Camera)
                    .await
                {
                    Ok(Some(leg)) => Some(leg),
                    Ok(None) => {
                        self.release_video_pair_later(pair);
                        None
                    }
                    Err(e) => {
                        let _ = tokio::time::timeout(BYE_TIMEOUT, self.uac.bye(&dialog)).await;
                        release_video(self, Some(pair));
                        dealloc(Arc::clone(&self.port_pool), port_pair);
                        bail!("trunk video from {}: {}", peer_node, e);
                    }
                }
            }
            (_, Some(pair)) => {
                self.release_video_pair_later(pair);
                None
            }
            _ => None,
        };
        // The screen's section rides only beside a video one.
        let content = match (&planned, content_pair, video.is_some()) {
            (Some(plan), Some(pair), true) => {
                match self
                    .trunk_video_from_answer(
                        answer_sdp,
                        plan,
                        pair,
                        content_srtp,
                        StreamKind::Content,
                    )
                    .await
                {
                    Ok(Some(leg)) => Some(leg),
                    Ok(None) => {
                        self.release_video_pair_later(pair);
                        None
                    }
                    Err(e) => {
                        let _ = tokio::time::timeout(BYE_TIMEOUT, self.uac.bye(&dialog)).await;
                        self.release_video_pair_later(pair);
                        if let Some(v) = video {
                            self.release_video_pair_later(v.port_pair);
                        }
                        dealloc(Arc::clone(&self.port_pool), port_pair);
                        bail!("trunk content from {}: {}", peer_node, e);
                    }
                }
            }
            (_, Some(pair), _) => {
                self.release_video_pair_later(pair);
                None
            }
            _ => None,
        };

        self.attach_trunk(
            TrunkLeg {
                room,
                room_number: room_number.to_string(),
                peer_node: peer_node.to_string(),
                tenant_id,
                call_id,
                dialog,
                port_pair,
                rtp_socket,
                remote_addr,
                codec_name,
                payload_type,
                frame_size,
                srtp,
                video,
                content,
            },
            &conf,
        )
        .await
    }

    /// What the trunk to a peer would carry as video: the meeting's own
    /// settings, and a codec both the meeting and this node's encoders
    /// allow. `None` when the meeting has no video, the node has no
    /// backend, or nothing agrees.
    async fn plan_trunk_video(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
        conf: &ConferenceSection,
    ) -> Option<TrunkVideoPlan> {
        let backend = self.video_backend.as_ref()?;
        let meeting = self
            .lookup_meeting(tenant_id, room_number)
            .await
            .ok()
            .flatten()?;
        let usable = video_bridge::usable_codecs(backend, conf.video_native_codecs_only);
        let (settings, codecs) = video_bridge::plan_video(
            &meeting,
            &usable,
            Duration::from_secs(conf.content_idle_secs),
        )?;
        // The first codec the meeting prefers that this node can encode.
        let codec = *codecs.first()?;
        Some(TrunkVideoPlan { settings, codec })
    }

    /// Read the peer's answer to one of our video sections — the camera's
    /// or the shared screen's (§15.6, 7d). `Ok(None)` means the peer
    /// declined the section and the trunk goes on without it; `Err` means
    /// it took the section on terms we cannot use, which is a reason to
    /// hang up.
    async fn trunk_video_from_answer(
        &self,
        answer_sdp: &str,
        plan: &TrunkVideoPlan,
        pair: PortPair,
        srtp_offer: Option<fcp_media::SrtpOffer>,
        kind: StreamKind,
    ) -> Result<Option<VideoLeg>> {
        let sections = fcp_media::parse_video_sections(answer_sdp);
        let answered = match kind {
            StreamKind::Camera => sections.camera,
            StreamKind::Content => sections.content,
        };
        let Some(section) = answered.filter(|s| s.port != 0) else {
            return Ok(None);
        };
        let Some(agreed) = section.codecs.iter().find(|c| c.codec == plan.codec) else {
            return Ok(None);
        };
        let (srtp_context, srtp_sdp) = match srtp_offer {
            Some(offer) => {
                let sdp = offer.sdp.clone();
                let crypto = match kind {
                    StreamKind::Camera => parse_video_sdes_crypto(answer_sdp),
                    StreamKind::Content => parse_content_sdes_crypto(answer_sdp),
                };
                let Some(crypto) = crypto.into_iter().next() else {
                    bail!(
                        "the peer answered {} without SRTP while the trunk is encrypted",
                        kind.name()
                    );
                };
                let ctx = offer
                    .accept(&crypto)
                    .map_err(|e| anyhow::anyhow!("video SRTP answer unusable: {e}"))?;
                (Some(Arc::new(tokio::sync::RwLock::new(ctx))), Some(sdp))
            }
            None => (None, None),
        };
        let socket = Arc::new(
            RtpSocketPair::new(pair, RtpSocketConfig::default())
                .await
                .map_err(|e| anyhow::anyhow!("video socket: {e}"))?,
        );
        if let Some(remote) = section.remote {
            socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote,
                    rtcp_addr: Some(SocketAddr::new(remote.ip(), remote.port().wrapping_add(1))),
                })
                .await;
        }
        Ok(Some(VideoLeg {
            kind,
            port_pair: pair,
            socket,
            srtp_context,
            srtp_sdp,
            negotiated: fcp_media::NegotiatedVideo {
                label: section.label.clone(),
                codec: agreed.codec,
                payload_type: agreed.payload_type,
                fmtp: agreed.fmtp.clone(),
                remote: section.remote,
                pli: section.feedback,
                fir: section.feedback,
                nack: section.feedback,
                remote_crypto: None,
            },
            settings: plan.settings.clone(),
            stats: Arc::new(VideoLegStats::default()),
            bridge_handle: None,
        }))
    }

    /// Answer a trunk INVITE from a peer node. The peer certificate must
    /// name the node the INVITE claims to come from; nothing else in the
    /// message is trusted for admission.
    #[allow(clippy::too_many_arguments)]
    async fn handle_trunk_invite(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        peer: SocketAddr,
        peer_identity: Option<Arc<PeerIdentity>>,
        peer_node: &str,
        tenant: Option<String>,
    ) -> Result<()> {
        let reject =
            |code: u16, reason: &str| UserAgentServer::create_response(request, code, reason);

        let Some(identity) = peer_identity else {
            warn!(peer = %peer, node = %peer_node, "Trunk INVITE without a verified client certificate");
            handle
                .send_final(reject(403, "Trunk Requires Mutual TLS"))
                .await;
            return Ok(());
        };
        if peer_node.is_empty()
            || peer_node == self.node_id
            || !trunks::identity_names_node(&identity, peer_node)
        {
            warn!(
                peer = %peer,
                node = %peer_node,
                identity = %identity,
                "Trunk INVITE: certificate does not name the claimed node"
            );
            handle
                .send_final(reject(403, "Certificate Does Not Name Node"))
                .await;
            return Ok(());
        }
        let tenant_id: TenantId = match tenant.as_deref().map(str::parse) {
            Some(Ok(t)) => t,
            _ => {
                handle.send_final(reject(400, "Missing Trunk Tenant")).await;
                return Ok(());
            }
        };
        let Some(room_number) = extract_room_number(request) else {
            handle.send_final(reject(400, "Bad Room")).await;
            return Ok(());
        };
        // A trunk only makes sense between two nodes that both have
        // callers in the room. Without local callers there is nothing to
        // send, and the peer's own reconcile will not hold the trunk.
        if self.countable_participants(&room_number) == 0 {
            handle.send_final(reject(480, "Room Not Hosted Here")).await;
            return Ok(());
        }
        let Ok(room) = self.bridge.get_room(&room_number) else {
            handle.send_final(reject(480, "Room Not Hosted Here")).await;
            return Ok(());
        };
        let conf = self.conference_config.read().clone();

        let body = request.body();
        if body.is_empty() {
            handle.send_final(reject(400, "Missing SDP")).await;
            return Ok(());
        }
        let sdp_offer = std::str::from_utf8(&body[..]).context("SDP body is not valid UTF-8")?;
        let offered_codecs = parse_sdp_codecs(sdp_offer);
        let Some((codec_name, payload_type, frame_size)) =
            pick_codec(&offered_codecs, conf.allowed_codecs.as_deref())
        else {
            handle.send_final(reject(488, "Not Acceptable Here")).await;
            return Ok(());
        };

        // SRTP whenever the peer offers it: the dialing side decides, and
        // a peer that offered a key expects ours back.
        let (srtp, srtp_sdp) = match parse_sdes_crypto(sdp_offer).first() {
            Some(crypto) => match Self::negotiate_srtp(crypto) {
                Ok((ctx, sdp_ctx)) => (Some(ctx), Some(sdp_ctx)),
                Err(e) => {
                    warn!(room = %room_number, peer = %peer_node, error = %e, "Trunk SRTP offer unusable");
                    handle.send_final(reject(488, "Not Acceptable Here")).await;
                    return Ok(());
                }
            },
            None => (None, None),
        };

        let port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port pair for conference trunk")?;

        // The peer offered video: answer it when this node's meeting has
        // video too and a codec agrees. The same lookup the dialling side
        // makes, so both ends reach the same answer.
        let mut video_pair = None;
        let mut video_answer_srtp = None;
        let mut plan = None;
        if fcp_media::parse_video_offer(sdp_offer).is_some_and(|v| v.port != 0) {
            plan = self.plan_trunk_video(&tenant_id, &room_number, &conf).await;
            if plan.is_some() {
                match parse_video_sdes_crypto(sdp_offer)
                    .first()
                    .map(Self::negotiate_srtp)
                {
                    Some(Ok((ctx, sdp_ctx))) => {
                        video_answer_srtp = Some((Arc::new(tokio::sync::RwLock::new(ctx)), sdp_ctx))
                    }
                    Some(Err(e)) => {
                        warn!(room = %room_number, peer = %peer_node, error = %e, "Trunk video SRTP offer unusable; answering audio only");
                        plan = None;
                    }
                    // The dialling side encrypts both sections or
                    // neither, so a video section with no key on an
                    // otherwise encrypted trunk is not one we answer.
                    None if srtp.is_some() => {
                        warn!(room = %room_number, peer = %peer_node, "Trunk video offered without crypto on an encrypted trunk; answering audio only");
                        plan = None;
                    }
                    None => {}
                }
            }
            if plan.is_some() {
                match self.port_pool.allocate().await {
                    Ok(p) => video_pair = Some(p),
                    Err(e) => {
                        warn!(room = %room_number, peer = %peer_node, error = %e, "No port pair left for trunk video; answering audio only");
                        plan = None;
                    }
                }
            }
        }
        // The peer offered a shared screen's section too: answer it when
        // the meeting allows sharing, on its own port and key (7d).
        let mut content_pair = None;
        let mut content_answer_srtp = None;
        let content_offered = fcp_media::parse_video_sections(sdp_offer)
            .content
            .is_some_and(|c| c.port != 0);
        if content_offered
            && video_pair.is_some()
            && plan.as_ref().is_some_and(|p| p.settings.content)
        {
            let mut take = true;
            match parse_content_sdes_crypto(sdp_offer)
                .first()
                .map(Self::negotiate_srtp)
            {
                Some(Ok((ctx, sdp_ctx))) => {
                    content_answer_srtp = Some((Arc::new(tokio::sync::RwLock::new(ctx)), sdp_ctx))
                }
                Some(Err(e)) => {
                    warn!(room = %room_number, peer = %peer_node, error = %e, "Trunk content SRTP offer unusable; answering without the screen");
                    take = false;
                }
                None if srtp.is_some() => {
                    warn!(room = %room_number, peer = %peer_node, "Trunk content offered without crypto on an encrypted trunk; answering without the screen");
                    take = false;
                }
                None => {}
            }
            if take {
                match self.port_pool.allocate().await {
                    Ok(p) => content_pair = Some(p),
                    Err(e) => {
                        warn!(room = %room_number, peer = %peer_node, error = %e, "No port pair left for trunk content; answering without the screen");
                    }
                }
            }
        }
        let trunk_codecs: Vec<_> = plan.as_ref().map(|p| vec![p.codec]).unwrap_or_default();
        let answer = match (&plan, video_pair) {
            (Some(_), Some(pair)) => self.build_answer(
                sdp_offer,
                port_pair.rtp_port,
                srtp_sdp.as_ref(),
                Some(VideoAnswer {
                    rtp_port: pair.rtp_port,
                    codecs: &trunk_codecs,
                    srtp: video_answer_srtp.as_ref().map(|(_, sdp)| sdp),
                }),
                content_pair.map(|cp| VideoAnswer {
                    rtp_port: cp.rtp_port,
                    codecs: &trunk_codecs,
                    srtp: content_answer_srtp.as_ref().map(|(_, sdp)| sdp),
                }),
                None,
            ),
            _ => fcp_media::Answer {
                sdp: construct_answer_for_a_leg(
                    sdp_offer,
                    &self.local_addr,
                    port_pair.rtp_port,
                    srtp_sdp.as_ref(),
                ),
                video: None,
                content: None,
                bfcp: None,
            },
        };
        let sdp_answer = answer.sdp;
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri);
        let (response, dialog) = match uas.accept_invite(request, Some(&sdp_answer)) {
            Ok(v) => v,
            Err(e) => {
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                for pair in [video_pair, content_pair].into_iter().flatten() {
                    self.release_video_pair_later(pair);
                }
                return Err(anyhow::anyhow!("Failed to create 200 OK for trunk: {}", e));
            }
        };
        handle.send_final(response).await;

        let rtp_socket = match RtpSocketPair::new(port_pair, RtpSocketConfig::default()).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                let _ = self.uac.bye(&dialog).await;
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                for pair in [video_pair, content_pair].into_iter().flatten() {
                    self.release_video_pair_later(pair);
                }
                return Err(anyhow::anyhow!(
                    "Failed to create RTP socket for trunk: {}",
                    e
                ));
            }
        };

        // The video half of the answer, when one was agreed, and the
        // screen's section beside it.
        let video = match (&plan, video_pair, answer.video) {
            (Some(plan), Some(pair), Some(negotiated)) => {
                self.trunk_leg_from_answer(
                    StreamKind::Camera,
                    pair,
                    negotiated,
                    fcp_media::remote_video_endpoint(sdp_offer),
                    video_answer_srtp,
                    plan.settings.clone(),
                )
                .await
            }
            (_, Some(pair), _) => {
                self.release_video_pair_later(pair);
                None
            }
            _ => None,
        };
        let content = match (&plan, content_pair, answer.content, video.is_some()) {
            (Some(plan), Some(pair), Some(negotiated), true) => {
                self.trunk_leg_from_answer(
                    StreamKind::Content,
                    pair,
                    negotiated,
                    fcp_media::remote_content_endpoint(sdp_offer),
                    content_answer_srtp,
                    plan.settings.clone(),
                )
                .await
            }
            (_, Some(pair), _, _) => {
                self.release_video_pair_later(pair);
                None
            }
            _ => None,
        };
        let remote_addr = parse_remote_rtp_addr(sdp_offer);
        if let Some(remote_addr) = remote_addr {
            rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;
        } else {
            warn!(room = %room_number, peer = %peer_node, "Trunk offer has no RTP address");
        }

        // A trunk to this peer that we still hold is stale: the peer would
        // not dial again if it thought it had one.
        let stale: Vec<String> = self
            .room_sessions
            .get(&room_number)
            .map(|ids| ids.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|id| {
                self.sessions
                    .get(id)
                    .map(|s| s.trunk_peer.as_deref() == Some(peer_node))
                    .unwrap_or(false)
            })
            .collect();
        for call_id in stale {
            info!(room = %room_number, peer = %peer_node, call_id = %call_id, "Replacing stale conference trunk");
            self.remove_session_for(
                &call_id,
                fcp_sip::TerminationReason::system(fcp_sip::SystemCause::Other, "Trunk replaced"),
            );
        }

        let call_id = extract_call_id(request);
        self.attach_trunk(
            TrunkLeg {
                room,
                room_number,
                peer_node: peer_node.to_string(),
                tenant_id,
                call_id,
                dialog,
                port_pair,
                rtp_socket,
                remote_addr,
                codec_name,
                payload_type,
                frame_size,
                srtp,
                video,
                content,
            },
            &conf,
        )
        .await
    }

    /// One answered video section of a trunk as a leg: its socket pointed
    /// at the peer, its key. `None` (and the ports given back) when the
    /// socket cannot be made; the trunk goes on without the section.
    async fn trunk_leg_from_answer(
        &self,
        kind: StreamKind,
        pair: PortPair,
        negotiated: fcp_media::NegotiatedVideo,
        offered_remote: Option<SocketAddr>,
        srtp: Option<(Arc<tokio::sync::RwLock<SrtpContext>>, SrtpSdpContext)>,
        settings: forge_conference::video::VideoRoomSettings,
    ) -> Option<VideoLeg> {
        let socket = match RtpSocketPair::new(pair, RtpSocketConfig::default()).await {
            Ok(socket) => Arc::new(socket),
            Err(e) => {
                warn!(stream = kind.name(), error = %e, "Failed to create the trunk's socket; without the section");
                self.release_video_pair_later(pair);
                return None;
            }
        };
        if let Some(remote) = negotiated.remote.or(offered_remote) {
            socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote,
                    rtcp_addr: Some(SocketAddr::new(remote.ip(), remote.port().wrapping_add(1))),
                })
                .await;
        }
        let (srtp_context, srtp_sdp) = match srtp {
            Some((ctx, sdp)) => (Some(ctx), Some(sdp)),
            None => (None, None),
        };
        Some(VideoLeg {
            kind,
            port_pair: pair,
            socket,
            srtp_context,
            srtp_sdp,
            negotiated,
            settings,
            stats: Arc::new(VideoLegStats::default()),
            bridge_handle: None,
        })
    }

    /// Join an established trunk leg to the room: a virtual participant,
    /// an audio bridge in trunk mode (send-mix of local callers only),
    /// and a session so BYE and reconcile can find it.
    async fn attach_trunk(&self, leg: TrunkLeg, conf: &ConferenceSection) -> Result<()> {
        let participant_id = trunks::trunk_participant_id(&leg.peer_node);
        if let Err(e) = leg.room.add_virtual_participant(&participant_id) {
            let _ = self.uac.bye(&leg.dialog).await;
            let pool = Arc::clone(&self.port_pool);
            let pair = leg.port_pair;
            tokio::spawn(async move { pool.deallocate(pair).await });
            bail!("failed to add trunk participant: {}", e);
        }

        let encoder = create_codec_instance(&leg.codec_name)?;
        let decoder = create_codec_instance(&leg.codec_name)?;
        let stats = Arc::new(AudioStats::new());
        let srtp_context = leg.srtp.map(|ctx| Arc::new(tokio::sync::RwLock::new(ctx)));
        let audio_bridge_handle = tokio::spawn(audio_bridge::run_audio_bridge(
            Arc::clone(&leg.room),
            participant_id.clone(),
            Arc::clone(&leg.rtp_socket),
            encoder,
            decoder,
            leg.payload_type,
            leg.frame_size,
            conf.jitter_buffer_ms,
            None, // no DTMF over a trunk
            8000,
            None,
            false,
            Arc::clone(&stats),
            srtp_context.clone(),
            conf.sample_rate,
            None,
            None,
            None,
            None,
            None,
            true,
        ));

        // The peer's composite as one tile here, and this node's callers
        // going back down the trunk. Video is a bonus on a trunk: if it
        // cannot be started the audio trunk still stands.
        let video = leg.video.and_then(|v| {
            self.start_trunk_video(
                &leg.room,
                &participant_id,
                &leg.peer_node,
                leg.tenant_id,
                &leg.room_number,
                v,
            )
        });
        // The screen's section only beside a working video one.
        let content = match (leg.content, video.is_some()) {
            (Some(c), true) => self.start_trunk_content(
                &leg.room,
                &participant_id,
                leg.tenant_id,
                &leg.room_number,
                c,
            ),
            (Some(c), false) => {
                self.release_video_pair_later(c.port_pair);
                None
            }
            (None, _) => None,
        };

        self.room_sessions
            .entry(leg.room_number.clone())
            .or_default()
            .push(leg.call_id.clone());
        self.sessions.insert(
            leg.call_id.clone(),
            ParticipantSession {
                call_id: leg.call_id,
                room_number: leg.room_number.clone(),
                participant_id: participant_id.clone(),
                is_host: false,
                is_dial_out: false,
                trunk_peer: Some(leg.peer_node.clone()),
                audio_bridge_handle,
                port_pair: leg.port_pair,
                rtp_socket: leg.rtp_socket,
                remote_addr: leg.remote_addr.unwrap_or(UNKNOWN_REMOTE),
                codec_name: leg.codec_name.clone(),
                tenant_id: leg.tenant_id,
                dialog: leg.dialog,
                joined_at: SystemTime::now(),
                te_pt: None,
                stats,
                display_name: Some(format!("Trunk: {}", leg.peer_node)),
                caller_id: None,
                srtp_context,
                last_refreshed: Instant::now(),
                video,
                content,
                joined_by_token: false,
                bfcp: None,
                record: LegRecord::own(
                    format!("trunk:{}", leg.peer_node),
                    Some(format!("conference:{}", leg.room_number)),
                ),
            },
        );
        info!(
            room = %leg.room_number,
            peer = %leg.peer_node,
            participant = %participant_id,
            codec = %leg.codec_name,
            "Conference trunk connected"
        );
        Ok(())
    }

    /// Put a trunk's content section on the room: the peer's shared
    /// screen as a remote content source, and this node's presenter's
    /// screen going back to it (§15.6, 7d).
    fn start_trunk_content(
        &self,
        room: &Arc<forge_conference::ConferenceRoom>,
        participant_id: &str,
        tenant_id: TenantId,
        room_number: &str,
        mut leg: VideoLeg,
    ) -> Option<VideoLeg> {
        let Some(backend) = self.video_backend.as_ref() else {
            self.release_video_pair_later(leg.port_pair);
            return None;
        };
        let sink = self.video_event_sink(tenant_id, room_number);
        match video_bridge::attach_trunk_content(room, backend, participant_id, &leg, sink) {
            Some(fut) => {
                leg.bridge_handle = Some(tokio::spawn(fut));
                info!(room = %room_number, trunk = %participant_id, "Conference trunk carries a shared screen");
                Some(leg)
            }
            None => {
                self.release_video_pair_later(leg.port_pair);
                None
            }
        }
    }

    /// Put a trunk's video on the room: the peer's composite as a remote
    /// tile, and a local-only composite going back to it.
    fn start_trunk_video(
        &self,
        room: &Arc<forge_conference::ConferenceRoom>,
        participant_id: &str,
        peer_node: &str,
        tenant_id: TenantId,
        room_number: &str,
        mut leg: VideoLeg,
    ) -> Option<VideoLeg> {
        let Some(backend) = self.video_backend.as_ref() else {
            self.release_video_pair_later(leg.port_pair);
            return None;
        };
        let sink = self.video_event_sink(tenant_id, room_number);
        match video_bridge::attach_trunk_video(room, backend, participant_id, peer_node, &leg, sink)
        {
            Some(fut) => {
                leg.bridge_handle = Some(tokio::spawn(fut));
                info!(
                    room = %room_number,
                    peer = %peer_node,
                    codec = %leg.negotiated.codec.sdp_name(),
                    "Conference trunk carries video"
                );
                Some(leg)
            }
            None => {
                self.release_video_pair_later(leg.port_pair);
                None
            }
        }
    }
}

/// An established trunk leg, before it is attached to the room.
/// How long a BYE may wait for its answer before the session is closed
/// regardless.
const BYE_TIMEOUT: Duration = Duration::from_secs(5);
/// The payload type a trunk offers its video on. A trunk is FCP at both
/// ends, so one number in the dynamic range is enough.
const TRUNK_VIDEO_PAYLOAD_TYPE: u8 = 97;
/// The payload type a trunk offers a shared screen on (§15.6, 7d).
const TRUNK_CONTENT_PAYLOAD_TYPE: u8 = 98;

/// A peer node's claim to a room's content floor (§15.6, 7d): who is
/// sharing there and since when, in milliseconds since the Unix epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentClaim {
    pub participant_id: String,
    pub display_name: String,
    pub since_ms: u64,
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
/// BYEs in flight at once during shutdown.
const SHUTDOWN_BYE_CONCURRENCY: usize = 32;

struct TrunkLeg {
    room: Arc<forge_conference::ConferenceRoom>,
    room_number: String,
    peer_node: String,
    tenant_id: TenantId,
    call_id: String,
    dialog: Dialog,
    port_pair: PortPair,
    rtp_socket: Arc<RtpSocketPair>,
    remote_addr: Option<SocketAddr>,
    codec_name: String,
    payload_type: u8,
    frame_size: usize,
    /// SDES-SRTP keys when negotiated on the trunk.
    srtp: Option<SrtpContext>,
    /// The peer's composite, when the trunk carries video: one tile
    /// here, and this node's callers going back (§15.4, 5b).
    video: Option<VideoLeg>,
    /// The shared screen's section, when the meeting allows sharing and
    /// the peer took it (§15.6, 7d).
    content: Option<VideoLeg>,
}

/// What a trunk would carry as video, decided before the offer goes out.
#[derive(Debug, Clone)]
struct TrunkVideoPlan {
    /// The room's own video settings, as the meeting sets them.
    settings: forge_conference::video::VideoRoomSettings,
    /// The codec offered: the meeting's first that this node encodes.
    codec: forge_core::VideoCodec,
}

#[async_trait::async_trait]
impl fcp_sip::conference::ConferenceHandler for ConferenceCallHandler {
    async fn can_handle(&self, request: &sip_core::Request) -> bool {
        // A trunk INVITE names its tenant in a header and is authorized by
        // the peer certificate; handle_invite does that. Deciding here by
        // the Request-URI host would resolve the wrong tenant.
        if sip_parse::header(request.headers(), TRUNK_NODE_HEADER).is_some() {
            return true;
        }
        let room_number = match extract_room_number(request) {
            Some(rn) => rn,
            None => {
                debug!(uri = %request.uri(), "can_handle: extract_room_number returned None");
                return false;
            }
        };

        let tenant_id = match self.resolve_tenant(request).await {
            Ok(tenant_id) => tenant_id,
            Err(e) => {
                warn!(room = %room_number, error = %e, "can_handle: resolve_tenant failed");
                return false;
            }
        };
        match self.lookup_meeting(&tenant_id, &room_number).await {
            Ok(Some(meeting)) => {
                debug!(room = %room_number, tenant = %tenant_id, enabled = meeting.enabled, "can_handle: meeting found");
                meeting.enabled
            }
            Ok(None) => {
                debug!(room = %room_number, tenant = %tenant_id, "can_handle: no meeting found for tenant");
                false
            }
            Err(e) => {
                warn!(room = %room_number, tenant = %tenant_id, error = %e, "can_handle: lookup_meeting error");
                false
            }
        }
    }

    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        peer: SocketAddr,
        peer_identity: Option<Arc<PeerIdentity>>,
    ) -> Result<()> {
        let _invite_start = Instant::now();

        // 0. Reject new calls during shutdown draining. Trunk INVITEs
        // still land: the callers already here must keep hearing the
        // rest of the room until they leave.
        if self.draining.load(Ordering::Relaxed)
            && sip_parse::header(request.headers(), TRUNK_NODE_HEADER).is_none()
        {
            info!(peer = %peer, "Rejecting INVITE during shutdown drain");
            let reject = UserAgentServer::create_response(request, 503, "Service Unavailable");
            handle.send_final(reject).await;
            return Ok(());
        }

        // 0a. Per-IP INVITE rate limiting
        let max_rate = self.conference_config.read().max_invite_rate_per_ip;
        if !self.check_invite_rate(peer.ip(), max_rate) {
            warn!(peer = %peer.ip(), max_rate, "INVITE rate limit exceeded");
            let reject = UserAgentServer::create_response(request, 486, "Busy Here");
            handle.send_final(reject).await;
            return Ok(());
        }

        // 0a'. Inter-node conference trunk: authorized by the peer
        // certificate, admitted past the caller-facing checks below.
        if let Some(node) = sip_parse::header(request.headers(), TRUNK_NODE_HEADER) {
            let node = node.as_str().trim().to_string();
            let tenant = sip_parse::header(request.headers(), TRUNK_TENANT_HEADER)
                .map(|v| v.as_str().trim().to_string());
            return self
                .handle_trunk_invite(request, handle, peer, peer_identity, &node, tenant)
                .await;
        }

        // 0b. Per-node participant limit (OOM protection)
        let max_per_node = self.conference_config.read().max_participants_per_node;
        if max_per_node > 0 && self.sessions.len() >= max_per_node {
            warn!(
                current = self.sessions.len(),
                max = max_per_node,
                "Node participant limit reached, rejecting INVITE"
            );
            let reject = UserAgentServer::create_response(request, 503, "Service Unavailable");
            handle.send_final(reject).await;
            return Ok(());
        }

        // 0b'. A migration ticket (phase 5): a caller moving here from a
        // draining node, to be admitted as the participant they already
        // were — past PIN, lock and waiting room. Refused when it does
        // not verify, was used before, or no cluster secret is set.
        let migrate = match sip_parse::header(request.headers(), fcp_core::MIGRATION_TICKET_HEADER)
        {
            Some(value) => {
                let secret = self.cluster_secret.read().unwrap().clone();
                let Some(secret) = secret else {
                    warn!(%peer, "Migration ticket received but cluster.secret is not set");
                    let reject =
                        UserAgentServer::create_response(request, 403, "Migration Not Configured");
                    handle.send_final(reject).await;
                    return Ok(());
                };
                match MigrationTicket::verify(value.as_str(), secret.as_bytes()) {
                    Ok(ticket) if self.used_tickets.admit(&ticket) => Some(ticket),
                    Ok(ticket) => {
                        warn!(%peer, participant = %ticket.participant_id, "Migration ticket used twice");
                        let reject = UserAgentServer::create_response(
                            request,
                            403,
                            "Migration Ticket Already Used",
                        );
                        handle.send_final(reject).await;
                        return Ok(());
                    }
                    Err(e) => {
                        warn!(%peer, error = %e, "Migration ticket refused");
                        let reject = UserAgentServer::create_response(
                            request,
                            403,
                            "Invalid Migration Ticket",
                        );
                        handle.send_final(reject).await;
                        return Ok(());
                    }
                }
            }
            None => None,
        };

        // 0c. A join token (a browser through the WebRTC gateway): the
        // token names the tenant, room, participant and role, and stands
        // in for the PIN. Refused outright when it does not verify or no
        // secret is configured; an INVITE without one takes the usual path.
        let join = match sip_parse::header(request.headers(), fcp_core::JOIN_TOKEN_HEADER) {
            Some(value) => {
                let secret = self
                    .conference_config
                    .read()
                    .join_secret
                    .clone()
                    .filter(|s| !s.is_empty());
                let Some(secret) = secret else {
                    warn!(%peer, "Join token received but conference.join_secret is not set");
                    let reject = UserAgentServer::create_response(
                        request,
                        403,
                        "Join Tokens Not Configured",
                    );
                    handle.send_final(reject).await;
                    return Ok(());
                };
                match fcp_core::JoinToken::verify(value.as_str(), secret.as_bytes()) {
                    Ok(token) => Some(token),
                    Err(e) => {
                        warn!(%peer, error = %e, "Join token refused");
                        let reject =
                            UserAgentServer::create_response(request, 403, "Invalid Join Token");
                        handle.send_final(reject).await;
                        return Ok(());
                    }
                }
            }
            None => None,
        };

        // 1. Extract room number (the ticket's or the token's, when there is one)
        let room_number = match (&migrate, &join) {
            (Some(ticket), _) => ticket.room_number.clone(),
            (None, Some(token)) => token.room_number.clone(),
            (None, None) => extract_room_number(request)
                .context("Failed to extract room number from Request-URI")?,
        };

        // 2. Extract tenant and look up meeting
        let tenant_id = match (&migrate, &join) {
            (Some(ticket), _) => ticket.tenant_id,
            (None, Some(token)) => token.tenant_id,
            (None, None) => self.resolve_tenant(request).await?,
        };
        let meeting = self
            .lookup_meeting(&tenant_id, &room_number)
            .await?
            .context("Meeting not found")?;

        if !meeting.enabled {
            let reject = UserAgentServer::create_response(request, 403, "Conference Disabled");
            handle.send_final(reject).await;
            return Ok(());
        }

        // Snapshot config for use throughout this method (avoids holding lock across awaits)
        let conf = self.conference_config.read().clone();

        // 3. Extract SDP offer from INVITE body
        let body = request.body();
        if body.is_empty() {
            let reject = UserAgentServer::create_response(request, 400, "Missing SDP");
            handle.send_final(reject).await;
            return Ok(());
        }
        let sdp_offer = std::str::from_utf8(&body[..]).context("SDP body is not valid UTF-8")?;

        // 4. Parse codecs and pick first supported one (filtered by allowed_codecs config)
        let offered_codecs = parse_sdp_codecs(sdp_offer);
        let allowed = conf.allowed_codecs.as_deref();
        let codec_result = pick_codec(&offered_codecs, allowed);
        let (codec_name, payload_type, frame_size) = match codec_result {
            Some(c) => c,
            None => {
                let reject = UserAgentServer::create_response(request, 488, "Not Acceptable Here");
                handle.send_final(reject).await;
                return Ok(());
            }
        };

        // Video: what the meeting allows, restricted to what this node can
        // run. Decided now so the ports and the answer follow it; anything
        // offered beyond that is rejected properly (port 0) in the answer.
        let offered_sections = fcp_media::parse_video_sections(sdp_offer);
        let video_plan = self.video_backend.as_ref().and_then(|backend| {
            let offered = offered_sections
                .camera
                .as_ref()
                .or(offered_sections.content.as_ref())?;
            let usable = video_bridge::usable_codecs(backend, conf.video_native_codecs_only);
            let plan = video_bridge::plan_video(
                &meeting,
                &usable,
                Duration::from_secs(conf.content_idle_secs),
            );
            if plan.is_none() {
                debug!(
                    room = %room_number,
                    codecs = ?offered.codecs.iter().map(|c| c.codec).collect::<Vec<_>>(),
                    video_enabled = meeting.video_enabled,
                    "Offer carries video; not enabled for this meeting or no usable codec, rejected in the answer"
                );
            }
            plan
        });
        // A shared screen's section (§15.6) is answered beside the camera
        // when the meeting allows sharing at all; whether this caller may
        // take the floor is the policy's, applied when the leg attaches.
        let content_offered = video_plan.is_some()
            && offered_sections.content.is_some()
            && meeting.video_content_share != "off";

        // Video budget (design §9.3): a join whose video would not fit on
        // this node is refused the way the participant cap is, before the
        // 180. Placement should have sent it where the budget is; this is
        // the guard for the race. The units are held until the next
        // recompute sees the participant in its room.
        if let (Some((settings, codecs)), Some(capacity)) =
            (video_plan.as_ref(), self.video_capacity.as_ref())
        {
            let room_video = self
                .bridge
                .get_room(&room_number)
                .ok()
                .and_then(|room| room.video());
            let cost = capacity.join_cost(settings, codecs, room_video.as_deref());
            if !capacity.try_reserve(&cost) {
                warn!(
                    room = %room_number,
                    %cost,
                    device = %capacity.device(),
                    left = capacity.left(),
                    total = capacity.total(),
                    "Video budget exhausted on this node, rejecting INVITE"
                );
                let reject = UserAgentServer::create_response(request, 503, "Service Unavailable");
                handle.send_final(reject).await;
                return Ok(());
            }
            debug!(room = %room_number, %cost, left = capacity.left(), "Video join reserved");
        }

        // Also look for telephone-event payload type and clock rate for DTMF
        let te_pt = find_telephone_event_pt(&offered_codecs, sdp_offer);
        let dtmf_clock_rate = find_telephone_event_clock_rate(&offered_codecs, sdp_offer);

        debug!(
            room = %room_number,
            codec = %codec_name,
            pt = payload_type,
            te_pt = ?te_pt,
            "Negotiated codec for conference participant"
        );

        // 5. Send 180 Ringing
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri.clone());
        let ringing = uas.create_ringing(request);
        handle.send_provisional(ringing).await;

        // 6. Allocate RTP port pair (and a second one for video)
        let port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port pair")?;
        let mut video_pair = match (&video_plan, &offered_sections.camera) {
            (Some(_), Some(_)) => match self.port_pool.allocate().await {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(room = %room_number, error = %e, "No port pair left for video; joining audio-only");
                    None
                }
            },
            _ => None,
        };
        // And a third for the shared screen.
        let mut content_pair = if content_offered {
            match self.port_pool.allocate().await {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(room = %room_number, error = %e, "No port pair left for content; the shared screen is rejected");
                    None
                }
            }
        } else {
            None
        };

        // 7. Negotiate SRTP (if enabled and offered by client)
        let (srtp_context, srtp_sdp) = if conf.enable_srtp {
            let crypto_lines = parse_sdes_crypto(sdp_offer);
            if let Some(crypto) = crypto_lines.first() {
                match Self::negotiate_srtp(crypto) {
                    Ok((ctx, sdp_ctx)) => {
                        (Some(Arc::new(tokio::sync::RwLock::new(ctx))), Some(sdp_ctx))
                    }
                    Err(e) => {
                        warn!(error = %e, "SRTP negotiation failed, falling back to plain RTP");
                        (None, None)
                    }
                }
            } else {
                (None, None) // Client didn't offer crypto
            }
        } else {
            (None, None)
        };

        // 7a. Video SRTP: the section has its own key. A room with SRTP on
        // refuses plaintext video (design §13).
        let (video_srtp_context, video_srtp_sdp) = if video_pair.is_some() && conf.enable_srtp {
            match parse_video_sdes_crypto(sdp_offer)
                .first()
                .map(Self::negotiate_srtp)
            {
                Some(Ok((ctx, sdp_ctx))) => {
                    (Some(Arc::new(tokio::sync::RwLock::new(ctx))), Some(sdp_ctx))
                }
                Some(Err(e)) => {
                    warn!(error = %e, "Video SRTP negotiation failed; video rejected");
                    self.release_video_port(&mut video_pair);
                    (None, None)
                }
                None => {
                    info!(room = %room_number, "Video offered without crypto while SRTP is on; video rejected");
                    self.release_video_port(&mut video_pair);
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        // 7a'. The shared screen's key: its own, like the camera's.
        let (content_srtp_context, content_srtp_sdp) = if content_pair.is_some() && conf.enable_srtp
        {
            match parse_content_sdes_crypto(sdp_offer)
                .first()
                .map(Self::negotiate_srtp)
            {
                Some(Ok((ctx, sdp_ctx))) => {
                    (Some(Arc::new(tokio::sync::RwLock::new(ctx))), Some(sdp_ctx))
                }
                Some(Err(e)) => {
                    warn!(error = %e, "Content SRTP negotiation failed; the shared screen is rejected");
                    self.release_video_port(&mut content_pair);
                    (None, None)
                }
                None => {
                    info!(room = %room_number, "Shared screen offered without crypto while SRTP is on; rejected");
                    self.release_video_port(&mut content_pair);
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        // 7b. Construct SDP answer (one section per offered section)
        let video_codecs: Vec<forge_core::VideoCodec> = video_plan
            .as_ref()
            .map(|(_, c)| c.clone())
            .unwrap_or_default();
        // BFCP (§15.6, 7e): a room system's floor control section is
        // answered beside its content section — the floor names the
        // section by its label — when this node listens for it. The ids
        // are reserved now and bound to the participant once admitted.
        let content_label = offered_sections
            .content
            .as_ref()
            .and_then(|c| c.label.clone());
        let bfcp_offer = conf.bfcp_bind.and_then(|bind| {
            fcp_media::parse_bfcp_offer(sdp_offer)
                .filter(|o| o.answerable())
                .map(|o| (bind, o))
        });
        let mut bfcp_assignment = match (&bfcp_offer, content_pair.is_some(), &content_label) {
            (Some((_, offer)), true, Some(_)) => {
                Some(self.bfcp.reserve(&room_number, offer.transport))
            }
            _ => None,
        };
        let bfcp_answer = match (&bfcp_assignment, &bfcp_offer, &content_label) {
            (Some(a), Some((bind, _)), Some(label)) => Some(fcp_media::BfcpAnswer {
                port: bind.port(),
                conference_id: a.conference_id,
                user_id: a.user_id,
                floor_id: a.floor_id,
                label,
            }),
            _ => None,
        };
        let answer = self.build_answer(
            sdp_offer,
            port_pair.rtp_port,
            srtp_sdp.as_ref(),
            video_pair.map(|vp| VideoAnswer {
                rtp_port: vp.rtp_port,
                codecs: &video_codecs,
                srtp: video_srtp_sdp.as_ref(),
            }),
            content_pair.map(|cp| VideoAnswer {
                rtp_port: cp.rtp_port,
                codecs: &video_codecs,
                srtp: content_srtp_sdp.as_ref(),
            }),
            bfcp_answer,
        );
        let sdp_answer = answer.sdp;
        if answer.bfcp.is_none() {
            if let Some(a) = bfcp_assignment.take() {
                self.bfcp.leave(&room_number, a.user_id);
            }
        }
        let mut negotiated_video = answer.video;
        if negotiated_video.is_none() {
            self.release_video_port(&mut video_pair);
        }
        let mut negotiated_content = answer.content;
        if negotiated_content.is_none() {
            self.release_video_port(&mut content_pair);
        }

        // 7c. SDP answer-offer consistency check: verify answer codec was in the offer
        let answer_codecs = parse_sdp_codecs(&sdp_answer);
        for ac in &answer_codecs {
            if !ac.encoding_name.eq_ignore_ascii_case("telephone-event")
                && !offered_codecs.iter().any(|oc| {
                    oc.encoding_name.eq_ignore_ascii_case(&ac.encoding_name)
                        && oc.clock_rate == ac.clock_rate
                })
            {
                warn!(
                    room = %room_number,
                    answer_codec = %ac.encoding_name,
                    "SDP consistency: answer codec not found in offer"
                );
            }
        }

        // 8. Create 200 OK with SDP answer using UAS
        let (mut response, dialog) = uas
            .accept_invite(request, Some(&sdp_answer))
            .context("Failed to create 200 OK")?;

        // 8b. RFC 4028: Add Session-Expires and Min-SE headers if session timers are enabled
        if conf.session_expires_secs > 0 {
            // Check if client requested a Session-Expires (from their INVITE)
            let client_se = sip_parse::header(request.headers(), "Session-Expires").and_then(|v| {
                v.as_str()
                    .split(';')
                    .next()
                    .and_then(|n| n.trim().parse::<u64>().ok())
            });
            let min_se: u64 = conf.min_session_expires_secs;

            let negotiated_se = match client_se {
                Some(requested) if requested < min_se => {
                    // Client's value is too small — but we already sent 200 OK;
                    // use our configured value (proper 422 would require intercepting earlier)
                    conf.session_expires_secs
                }
                Some(requested) => requested.min(conf.session_expires_secs),
                None => conf.session_expires_secs,
            };

            let _ = response.headers_mut().push(
                "Session-Expires",
                format!("{};refresher=uac", negotiated_se),
            );
            let _ = response.headers_mut().push("Min-SE", min_se.to_string());
            let _ = response.headers_mut().push("Require", "timer");
        }

        // 9. Send 200 OK
        handle.send_final(response).await;

        // 10. Create RTP socket pair (deallocate port on failure to prevent leak)
        let rtp_socket = match RtpSocketPair::new(port_pair, RtpSocketConfig::default()).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                warn!(
                    rtp_port = port_pair.rtp_port,
                    error = %e,
                    "Failed to create RTP socket pair, deallocating port"
                );
                self.release_ports(port_pair, video_pair);
                return Err(anyhow::anyhow!("Failed to create RTP socket pair: {}", e));
            }
        };

        // 11. Parse remote RTP address from SDP offer
        if let Some(remote_addr) = parse_remote_rtp_addr(sdp_offer) {
            rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;

            debug!(
                remote = %remote_addr,
                local_rtp_port = port_pair.rtp_port,
                "RTP endpoints configured"
            );
        } else {
            warn!("Could not parse remote RTP address from SDP offer");
        }

        // 11b. The video socket pair, pointed at the video section's endpoint
        let video_socket = match (video_pair, negotiated_video.as_ref()) {
            (Some(vp), Some(nv)) => {
                match RtpSocketPair::new(vp, RtpSocketConfig::default()).await {
                    Ok(s) => {
                        let s = Arc::new(s);
                        if let Some(remote) = nv
                            .remote
                            .or_else(|| fcp_media::remote_video_endpoint(sdp_offer))
                        {
                            s.set_remote_endpoint(RemoteEndpoint {
                                rtp_addr: remote,
                                rtcp_addr: Some(SocketAddr::new(
                                    remote.ip(),
                                    remote.port().wrapping_add(1),
                                )),
                            })
                            .await;
                        }
                        Some(s)
                    }
                    Err(e) => {
                        warn!(rtp_port = vp.rtp_port, error = %e, "Failed to create video socket pair; joining audio-only");
                        self.release_video_port(&mut video_pair);
                        negotiated_video = None;
                        None
                    }
                }
            }
            _ => None,
        };
        // 11c. The shared screen's socket pair, pointed at its own section.
        let content_socket = match (content_pair, negotiated_content.as_ref()) {
            (Some(cp), Some(nc)) => {
                match RtpSocketPair::new(cp, RtpSocketConfig::default()).await {
                    Ok(s) => {
                        let s = Arc::new(s);
                        if let Some(remote) = nc
                            .remote
                            .or_else(|| fcp_media::remote_content_endpoint(sdp_offer))
                        {
                            s.set_remote_endpoint(RemoteEndpoint {
                                rtp_addr: remote,
                                rtcp_addr: Some(SocketAddr::new(
                                    remote.ip(),
                                    remote.port().wrapping_add(1),
                                )),
                            })
                            .await;
                        }
                        Some(s)
                    }
                    Err(e) => {
                        warn!(rtp_port = cp.rtp_port, error = %e, "Failed to create content socket pair; the shared screen is dropped");
                        self.release_video_port(&mut content_pair);
                        negotiated_content = None;
                        None
                    }
                }
            }
            _ => None,
        };
        let mut pending_content = match (
            content_socket,
            negotiated_content,
            content_pair,
            &video_plan,
        ) {
            (Some(socket), Some(negotiated), Some(cp), Some((settings, _))) => {
                info!(
                    room = %room_number,
                    codec = %negotiated.codec,
                    payload_type = negotiated.payload_type,
                    content_port = cp.rtp_port,
                    srtp = content_srtp_context.is_some(),
                    "Shared screen negotiated"
                );
                Some(VideoLeg {
                    kind: StreamKind::Content,
                    port_pair: cp,
                    socket,
                    srtp_context: content_srtp_context,
                    srtp_sdp: content_srtp_sdp.clone(),
                    negotiated,
                    settings: settings.clone(),
                    stats: Arc::new(VideoLegStats::default()),
                    bridge_handle: None,
                })
            }
            _ => None,
        };
        let mut pending_video = match (video_socket, negotiated_video, video_pair, &video_plan) {
            (Some(socket), Some(negotiated), Some(vp), Some((settings, _))) => {
                let srtp_sdp = video_srtp_sdp.clone();
                info!(
                    room = %room_number,
                    codec = %negotiated.codec,
                    payload_type = negotiated.payload_type,
                    video_port = vp.rtp_port,
                    srtp = video_srtp_context.is_some(),
                    "Video negotiated"
                );
                Some(VideoLeg {
                    kind: StreamKind::Camera,
                    port_pair: vp,
                    socket,
                    srtp_context: video_srtp_context,
                    srtp_sdp,
                    negotiated,
                    settings: settings.clone(),
                    stats: Arc::new(VideoLegStats::default()),
                    bridge_handle: None,
                })
            }
            _ => None,
        };

        // 12. Generate participant ID and extract caller info
        let call_id = extract_call_id(request);
        let (mut display_name, mut caller_id) = extract_caller_info(request);
        if let Some(token) = &join {
            display_name = Some(token.display_name.clone());
        }
        if let Some(ticket) = &migrate {
            display_name = ticket.display_name.clone();
            caller_id = ticket.caller_id.clone();
        }
        let short_uuid = uuid::Uuid::new_v4().to_string();
        let participant_id = match &migrate {
            // The same participant as on the node they came from.
            Some(ticket) => ticket.participant_id.clone(),
            None => format!("{}-{}", &call_id[..call_id.len().min(8)], &short_uuid[..8]),
        };

        // 13. Get or create conference room, configure with meeting settings
        let room = self.room_for_join(&tenant_id, &room_number).await?;

        // Wire room configuration from meeting → forge-conference
        let room_config = self.meeting_to_room_config(&meeting);
        let global_config = ConferenceConfig::default();
        if let Err(e) = room.configure(room_config, &global_config) {
            warn!(room = %room_number, error = %e, "Failed to configure room (may already be configured)");
        }

        // 14. PIN authentication (if required) — a migration ticket or a
        // join token settles the role instead, with no PIN collected.
        let is_host = if let Some(ticket) = &migrate {
            info!(
                room = %room_number,
                participant = %participant_id,
                from = %ticket.from_node,
                host = ticket.is_host,
                "Admitted by migration ticket"
            );
            ticket.is_host
        } else if let Some(token) = &join {
            let host = token.role == fcp_core::JoinRole::Host;
            info!(
                room = %room_number,
                participant = %participant_id,
                host,
                "Admitted by join token"
            );
            host
        } else if Self::meeting_requires_pin(&meeting) {
            let max_attempts = conf.max_pin_attempts;
            let pin_timeout = Duration::from_secs(conf.pin_timeout_secs);
            let mixer_sample_rate = conf.sample_rate;
            let (info_digits_tx, info_digits_rx) = tokio::sync::mpsc::unbounded_channel();
            let pin_decoder = create_codec_instance(&codec_name)?;
            let mut pin_collector =
                PinDtmfCollector::new(dtmf_clock_rate, info_digits_rx, pin_decoder, payload_type);

            // Load pre-loaded sounds from the room for prompt playback.
            // If a sound file isn't configured but the meeting has TTS text, synthesize it.
            let sounds = room.get_sounds();
            let mut pin_prompt_samples = sounds.as_ref().and_then(|s| s.pin_prompt_sound.clone());
            let mut invalid_pin_samples = sounds.as_ref().and_then(|s| s.invalid_pin_sound.clone());

            // TTS fallback: synthesize prompts from meeting text if no sound file loaded
            if let Some(ref tts) = self.tts_service {
                if pin_prompt_samples.is_none() {
                    if let Some(ref text) = meeting.pin_prompt_tts {
                        match tts.synthesize(text, None, None).await {
                            Ok(path) => {
                                let player =
                                    forge_conference::AudioFeedbackPlayer::new(mixer_sample_rate);
                                if let Some(pcm) = player.load_audio_file(&path) {
                                    debug!(room = %room_number, "TTS: synthesized pin_prompt");
                                    pin_prompt_samples = Some(Arc::new(pcm));
                                }
                            }
                            Err(e) => {
                                warn!(room = %room_number, error = %e, "TTS pin_prompt synthesis failed")
                            }
                        }
                    }
                }
                if invalid_pin_samples.is_none() {
                    if let Some(ref text) = meeting.invalid_pin_tts {
                        match tts.synthesize(text, None, None).await {
                            Ok(path) => {
                                let player =
                                    forge_conference::AudioFeedbackPlayer::new(mixer_sample_rate);
                                if let Some(pcm) = player.load_audio_file(&path) {
                                    debug!(room = %room_number, "TTS: synthesized invalid_pin");
                                    invalid_pin_samples = Some(Arc::new(pcm));
                                }
                            }
                            Err(e) => {
                                warn!(room = %room_number, error = %e, "TTS invalid_pin synthesis failed")
                            }
                        }
                    }
                }
            }

            // Prompt store fallback: load pre-synthesized audio from the Prompt system.
            // These are seeded at startup and can be (re-)synthesized via the admin UI
            // Audio Prompts page using Kokoro TTS.
            if pin_prompt_samples.is_none() || invalid_pin_samples.is_none() {
                if let Some(prompt_store) = self.storage.prompt_store() {
                    let player = forge_conference::AudioFeedbackPlayer::new(mixer_sample_rate);
                    if pin_prompt_samples.is_none() {
                        if let Ok(Some(prompt)) = prompt_store
                            .get_prompt_by_name(&tenant_id, "conf_pin_prompt")
                            .await
                        {
                            if let Some(ref path) = prompt.audio_path {
                                if let Some(pcm) = player.load_audio_file(path) {
                                    debug!(room = %room_number, "Loaded pin_prompt from Prompt store");
                                    pin_prompt_samples = Some(Arc::new(pcm));
                                }
                            }
                        }
                    }
                    if invalid_pin_samples.is_none() {
                        if let Ok(Some(prompt)) = prompt_store
                            .get_prompt_by_name(&tenant_id, "conf_invalid_pin")
                            .await
                        {
                            if let Some(ref path) = prompt.audio_path {
                                if let Some(pcm) = player.load_audio_file(path) {
                                    debug!(room = %room_number, "Loaded invalid_pin from Prompt store");
                                    invalid_pin_samples = Some(Arc::new(pcm));
                                }
                            }
                        }
                    }
                }
            }

            // Create a temporary encoder for prompt playback (same codec as participant)
            let mut prompt_encoder = create_codec_instance(&codec_name)?;

            self.pending_pin_auth.insert(
                call_id.clone(),
                PendingPinAuth {
                    room_number: room_number.clone(),
                    participant_id: participant_id.clone(),
                    info_digits_tx,
                },
            );

            let mut authenticated = false;
            let mut is_host_auth = false;

            for attempt in 1..=max_attempts {
                // Brief pause between retry attempts to drain leftover DTMF retransmissions
                if attempt > 1 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                // Play PIN prompt in the background so DTMF collection
                // starts immediately (caller can "type through" the prompt).
                let prompt_handle = if let Some(ref samples) = pin_prompt_samples {
                    debug!(room = %room_number, participant = %participant_id, "Playing PIN prompt (interruptible)");
                    let prompt_socket = Arc::clone(&rtp_socket);
                    let prompt_samples = Arc::clone(samples);
                    let prompt_codec = codec_name.clone();
                    let prompt_srtp = srtp_context.clone();
                    let prompt_ms = mixer_sample_rate;
                    Some(tokio::spawn(async move {
                        if let Ok(mut enc) = create_codec_instance(&prompt_codec) {
                            play_prompt_to_rtp(
                                &prompt_socket,
                                &prompt_samples,
                                enc.as_mut(),
                                payload_type,
                                frame_size,
                                &prompt_srtp,
                                prompt_ms,
                            )
                            .await;
                        }
                    }))
                } else {
                    if attempt == 1 {
                        warn!(
                            room = %room_number,
                            "No PIN prompt audio configured — caller hears silence. \
                             Set conference.pin_prompt_sound in config, or set \
                             pin_prompt_tts on the meeting with TTS enabled."
                        );
                    }
                    None
                };

                debug!(
                    room = %room_number,
                    participant = %participant_id,
                    attempt = attempt,
                    "Waiting for PIN entry via DTMF"
                );

                // Collect DTMF concurrently with prompt playback
                let pin_result = pin_collector
                    .collect_pin_dtmf(&rtp_socket, te_pt, pin_timeout)
                    .await;

                // Stop prompt playback if still running
                if let Some(h) = prompt_handle {
                    h.abort();
                }

                match pin_result {
                    Some(ref pin) if pin.is_empty() => {
                        // Empty PIN (user pressed # immediately) — treat as incorrect
                        warn!(room = %room_number, participant = %participant_id, attempt, "Empty PIN submitted");
                        if let Some(ref samples) = invalid_pin_samples {
                            play_prompt_to_rtp(
                                &rtp_socket,
                                samples,
                                prompt_encoder.as_mut(),
                                payload_type,
                                frame_size,
                                &srtp_context,
                                mixer_sample_rate,
                            )
                            .await;
                        }
                        continue;
                    }
                    Some(pin) => {
                        let result = room.authenticate_participant(&participant_id, &pin);
                        match result {
                            PinAuthResult::GuestAuthenticated => {
                                info!(room = %room_number, participant = %participant_id, "Guest PIN accepted");
                                authenticated = true;
                                is_host_auth = false;
                                break;
                            }
                            PinAuthResult::HostAuthenticated => {
                                info!(room = %room_number, participant = %participant_id, "Host PIN accepted");
                                authenticated = true;
                                is_host_auth = true;
                                break;
                            }
                            PinAuthResult::IncorrectPin { attempts_remaining } => {
                                warn!(
                                    room = %room_number,
                                    participant = %participant_id,
                                    attempts_remaining = attempts_remaining,
                                    "Incorrect PIN"
                                );
                                // Play invalid PIN sound
                                if let Some(ref samples) = invalid_pin_samples {
                                    play_prompt_to_rtp(
                                        &rtp_socket,
                                        samples,
                                        prompt_encoder.as_mut(),
                                        payload_type,
                                        frame_size,
                                        &srtp_context,
                                        mixer_sample_rate,
                                    )
                                    .await;
                                }
                            }
                            PinAuthResult::LockedOut { .. } => {
                                warn!(room = %room_number, participant = %participant_id, "PIN locked out");
                                self.pending_pin_auth.remove(&call_id);
                                self.publish_event(ConferenceEvent::PinFailed {
                                    room_number: room_number.clone(),
                                    participant_id: participant_id.clone(),
                                    tenant_id,
                                    timestamp: SystemTime::now(),
                                });
                                // Send BYE to disconnect the answered call
                                if let Err(e) = self.uac.bye(&dialog).await {
                                    warn!(error = %e, "Failed to send BYE after PIN lockout");
                                }
                                self.release_ports(
                                    port_pair,
                                    pending_video.as_ref().map(|v| v.port_pair),
                                );
                                return Ok(());
                            }
                            PinAuthResult::NoPinRequired => {
                                authenticated = true;
                                break;
                            }
                        }
                    }
                    None => {
                        // Timeout — no DTMF received
                        warn!(room = %room_number, participant = %participant_id, "PIN collection timed out");
                        break;
                    }
                }
            }

            if !authenticated {
                self.pending_pin_auth.remove(&call_id);
                info!(room = %room_number, participant = %participant_id, "PIN authentication failed, rejecting");
                self.publish_event(ConferenceEvent::PinFailed {
                    room_number: room_number.clone(),
                    participant_id: participant_id.clone(),
                    tenant_id,
                    timestamp: SystemTime::now(),
                });
                // Send BYE to disconnect the answered call
                if let Err(e) = self.uac.bye(&dialog).await {
                    warn!(error = %e, "Failed to send BYE after PIN failure");
                }
                self.release_ports(port_pair, pending_video.as_ref().map(|v| v.port_pair));
                return Ok(());
            }

            self.pending_pin_auth.remove(&call_id);
            is_host_auth
        } else {
            false
        };

        // 15. Add participant to room (enforces capacity/locked/wait-for-moderator)
        //
        // Defense-in-depth: check the meeting's wait_for_moderator flag directly
        // at the handler level. The room's add_participant also checks, but the
        // meeting is the authoritative source. If both the meeting says "wait" and
        // there are no hosts, we enforce waiting regardless of room config state.
        info!(
            room = %room_number,
            participant = %participant_id,
            is_host = is_host,
            wait_for_moderator = meeting.wait_for_moderator,
            room_host_count = room.host_count(),
            room_wfm_flag = room.wait_for_moderator_enabled(),
            "Adding participant to conference"
        );

        // Determine if the participant should wait for a moderator.
        // Check the meeting config directly (handler-level) as the authoritative source.
        let should_wait = if migrate.is_none()
            && meeting.wait_for_moderator
            && !is_host
            && room.host_count() == 0
        {
            info!(
                room = %room_number,
                participant = %participant_id,
                "Guest must wait for moderator (meeting.wait_for_moderator=true, no hosts present)"
            );
            true
        } else {
            false
        };

        // If the handler already determined the participant should wait, skip
        // add_participant (which would add to the mixer) and go straight to
        // the waiting flow. Otherwise, let add_participant handle it normally.
        let should_wait = if should_wait {
            true
        } else {
            // A migrant was admitted once already: no lock, no waiting room.
            let added = match &migrate {
                Some(_) => room.add_participant_admitted(&participant_id, is_host),
                None => room.add_participant_quiet(&participant_id, is_host),
            };
            match added {
                Ok(()) => false,
                Err(ConferenceError::WaitingForModerator) => true,
                Err(ConferenceError::ConferenceFull(max)) => {
                    warn!(room = %room_number, max = max, "Conference is full, rejecting participant");
                    if let Err(e) = self.uac.bye(&dialog).await {
                        warn!(error = %e, "Failed to send BYE after conference full");
                    }
                    self.release_ports(port_pair, pending_video.as_ref().map(|v| v.port_pair));
                    return Ok(());
                }
                Err(ConferenceError::ConferenceLocked) => {
                    warn!(room = %room_number, "Conference is locked, rejecting participant");
                    if let Err(e) = self.uac.bye(&dialog).await {
                        warn!(error = %e, "Failed to send BYE after conference locked");
                    }
                    self.release_ports(port_pair, pending_video.as_ref().map(|v| v.port_pair));
                    return Ok(());
                }
                Err(e) => {
                    warn!(room = %room_number, error = %e, "Failed to add participant");
                    if let Err(bye_err) = self.uac.bye(&dialog).await {
                        warn!(error = %bye_err, "Failed to send BYE after add_participant error");
                    }
                    self.release_ports(port_pair, pending_video.as_ref().map(|v| v.port_pair));
                    return Ok(());
                }
            }
        };

        if should_wait {
            info!(
                room = %room_number,
                participant = %participant_id,
                "Participant waiting for moderator"
            );
            // Ensure participant is in waiting_participants (add_participant may
            // have added them if it returned WaitingForModerator, or we may need
            // to add them here if the handler-level check pre-empted add_participant).
            if !room.waiting_participants().contains(&participant_id) {
                // Handler-level enforcement: manually insert into waiting list.
                // Use a low-level DashMap insert via a helper if available, or
                // call add_participant which will do it for us (it will return
                // WaitingForModerator since we know the conditions are met).
                let _ = room.add_participant(&participant_id, false);
                // The above call should return Err(WaitingForModerator) and
                // insert the participant into waiting_participants.
            }

            // Spawn a task that polls until they appear in the mixer (when a
            // host joins and releases them), then start the audio bridge.
            let bridge_room = Arc::clone(&room);
            let bridge_participant = participant_id.clone();
            let bridge_socket = Arc::clone(&rtp_socket);
            let event_bus = Arc::clone(&self.event_bus);
            let rn = room_number.clone();
            let pid = participant_id.clone();
            let encoder = create_codec_instance(&codec_name)?;
            let decoder = create_codec_instance(&codec_name)?;
            let jitter_buffer_ms = conf.jitter_buffer_ms;
            let mixer_sample_rate = conf.sample_rate;

            let dtmf_handler = if conf.dtmf_enabled {
                Some(self.get_or_create_dtmf_handler(&room_number))
            } else {
                None
            };
            let wait_stats = Arc::new(AudioStats::new());
            let wait_stats_clone = Arc::clone(&wait_stats);
            let wait_srtp = srtp_context.clone();
            let max_wait_secs = conf.max_wait_for_moderator_secs;
            let announcement_interval_secs = conf.moh_announcement_interval_secs;
            let wait_uac = Arc::clone(&self.uac);
            // The handler itself, for the auto-record check once a
            // moderator lets the room in.
            let wait_handler = self.me();
            let wait_dialog = dialog.clone();
            let wait_call_id = call_id.clone();
            let wait_by_token = join.is_some();
            // The video leg starts with the audio bridge once admitted; the
            // session keeps its own handle-less copy for teardown and the API.
            let wait_video = pending_video.as_ref().map(VideoLeg::shared);
            let wait_content = pending_content.as_ref().map(VideoLeg::shared);
            // Waiting participants are not hosts: the policy for a guest.
            let wait_may_share = video_bridge::may_share(&meeting.video_content_share, false);
            let wait_backend = self.video_backend.clone();
            let wait_display_name = display_name.clone();
            let wait_sink = self.video_event_sink(tenant_id, &room_number);

            // Load announcement sound (short TTS message like "Please wait...")
            let mut announcement_samples: Option<Arc<Vec<i16>>> = None;
            if let Some(prompt_store) = self.storage.prompt_store() {
                if let Ok(Some(prompt)) = prompt_store
                    .get_prompt_by_name(&tenant_id, "conf_wait_announcement")
                    .await
                {
                    if let Some(ref path) = prompt.audio_path {
                        let player = forge_conference::AudioFeedbackPlayer::new(conf.sample_rate);
                        if let Some(pcm) = player.load_audio_file(path) {
                            debug!(room = %room_number, "Loaded announcement from conf_moh prompt");
                            announcement_samples = Some(Arc::new(pcm));
                        }
                    }
                }
            }

            // Load hold music (separate from the announcement).
            // Priority: meeting moh_sound → room sounds → conf_moh_music prompt
            let mut music_samples: Option<Arc<Vec<i16>>> =
                room.get_sounds().and_then(|s| s.moh_sound.clone());
            if music_samples.is_none() {
                if let Some(prompt_store) = self.storage.prompt_store() {
                    if let Ok(Some(prompt)) = prompt_store
                        .get_prompt_by_name(&tenant_id, "conf_moh_music")
                        .await
                    {
                        if let Some(ref path) = prompt.audio_path {
                            let player =
                                forge_conference::AudioFeedbackPlayer::new(conf.sample_rate);
                            if let Some(pcm) = player.load_audio_file(path) {
                                debug!(room = %room_number, "Loaded hold music from conf_moh_music prompt");
                                music_samples = Some(Arc::new(pcm));
                            }
                        }
                    }
                }
            }

            let moh_codec_name = codec_name.clone();
            let moh_payload_type = payload_type;
            let moh_frame_size = frame_size;
            let moh_rtp_socket = Arc::clone(&rtp_socket);
            let moh_srtp = srtp_context.clone();
            let wait_muted_samples = self.dtmf_muted_samples.clone();
            let wait_unmuted_samples = self.dtmf_unmuted_samples.clone();
            let wait_locked_samples = self.dtmf_locked_samples.clone();
            let wait_unlocked_samples = self.dtmf_unlocked_samples.clone();
            let wait_ending_samples = self.dtmf_ending_samples.clone();

            let wait_handle = tokio::spawn(async move {
                // Spawn a background task that plays: announcement → music/silence → repeat
                let moh_cancel = tokio_util::sync::CancellationToken::new();
                let has_audio = announcement_samples.is_some() || music_samples.is_some();
                let moh_handle = if has_audio {
                    let cancel = moh_cancel.clone();
                    let rtp_sock = Arc::clone(&moh_rtp_socket);
                    let srtp = moh_srtp.clone();
                    let codec_name = moh_codec_name.clone();
                    let ms_rate = mixer_sample_rate;
                    Some(tokio::spawn(async move {
                        let mut moh_encoder = match create_codec_instance(&codec_name) {
                            Ok(e) => e,
                            Err(e) => {
                                warn!(error = %e, "Failed to create MOH encoder");
                                return;
                            }
                        };
                        loop {
                            // 1. Play the announcement (short TTS message)
                            if let Some(ref announcement) = announcement_samples {
                                play_prompt_to_rtp(
                                    &rtp_sock,
                                    announcement,
                                    moh_encoder.as_mut(),
                                    moh_payload_type,
                                    moh_frame_size,
                                    &srtp,
                                    ms_rate,
                                )
                                .await;
                                if cancel.is_cancelled() {
                                    break;
                                }
                            }

                            // 2. Play hold music for the announcement interval,
                            //    or wait silently if no music is configured.
                            let interval = Duration::from_secs(announcement_interval_secs.max(5));
                            let music_start = Instant::now();
                            if let Some(ref music) = music_samples {
                                // Loop the music file until the interval expires
                                while music_start.elapsed() < interval {
                                    play_prompt_to_rtp(
                                        &rtp_sock,
                                        music,
                                        moh_encoder.as_mut(),
                                        moh_payload_type,
                                        moh_frame_size,
                                        &srtp,
                                        ms_rate,
                                    )
                                    .await;
                                    if cancel.is_cancelled() {
                                        break;
                                    }
                                }
                            } else {
                                // No music configured — send silence packets to
                                // keep the RTP stream alive during the interval.
                                let silence_frame = vec![0i16; moh_frame_size];
                                let ptime = Duration::from_millis(
                                    (moh_frame_size as u64 * 1000) / ms_rate as u64,
                                );
                                while music_start.elapsed() < interval {
                                    play_prompt_to_rtp(
                                        &rtp_sock,
                                        &silence_frame,
                                        moh_encoder.as_mut(),
                                        moh_payload_type,
                                        moh_frame_size,
                                        &srtp,
                                        ms_rate,
                                    )
                                    .await;
                                    // pace silence at the frame rate
                                    tokio::time::sleep(ptime).await;
                                    if cancel.is_cancelled() {
                                        break;
                                    }
                                }
                            }

                            if cancel.is_cancelled() {
                                break;
                            }
                        }
                    }))
                } else {
                    None
                };

                // Poll every 500ms waiting to be released into the mixer
                let wait_start = Instant::now();
                let max_wait = if max_wait_secs > 0 {
                    Some(Duration::from_secs(max_wait_secs))
                } else {
                    None // unlimited
                };
                loop {
                    tokio::time::sleep(Duration::from_millis(500)).await;

                    // Check timeout
                    if let Some(limit) = max_wait {
                        if wait_start.elapsed() > limit {
                            warn!(
                                room = %rn,
                                participant = %pid,
                                timeout_secs = max_wait_secs,
                                waited_secs = wait_start.elapsed().as_secs(),
                                "Wait-for-moderator timeout, disconnecting participant"
                            );
                            // Stop MOH
                            moh_cancel.cancel();
                            if let Some(h) = &moh_handle {
                                h.abort();
                            }

                            // Remove from room (handles waiting list cleanup)
                            let _ = bridge_room.remove_participant(&pid);

                            // Send BYE to disconnect the SIP call
                            if let Err(e) = wait_uac.bye(&wait_dialog).await {
                                warn!(error = %e, "Failed to send BYE after wait-for-moderator timeout");
                            }

                            // Publish leave event so downstream systems know about the timeout
                            let timeout_event =
                                PlatformEvent::Conference(ConferenceEvent::ParticipantLeft {
                                    room_number: rn.clone(),
                                    participant_id: pid.clone(),
                                    migrated_to: None,
                                    tenant_id,
                                    timestamp: SystemTime::now(),
                                });
                            let _ = event_bus.publish(timeout_event).await;
                            break;
                        }
                    }

                    // Check if we're still in the waiting list
                    let still_waiting = bridge_room
                        .waiting_participants()
                        .contains(&bridge_participant);

                    if !still_waiting {
                        // Stop MOH playback — moderator arrived or we were removed
                        moh_cancel.cancel();
                        if let Some(h) = &moh_handle {
                            h.abort();
                        }

                        // We were either released into mixer or removed entirely.
                        // Check if we're now in the mixer.
                        if bridge_room.participants().contains(&bridge_participant) {
                            info!(
                                room = %rn,
                                participant = %pid,
                                "Moderator arrived, participant released"
                            );

                            // Publish join event
                            let event =
                                PlatformEvent::Conference(ConferenceEvent::ParticipantJoined {
                                    room_number: rn.clone(),
                                    participant_id: pid.clone(),
                                    migrated_from: None,
                                    is_host: false,
                                    has_video: wait_video.is_some(),
                                    tenant_id,
                                    timestamp: SystemTime::now(),
                                });
                            let _ = event_bus.publish(event).await;

                            // Auto-record check after moderator releases waiting participants
                            if !bridge_room.is_recording() {
                                if let Some(cfg) = bridge_room.get_effective_config() {
                                    if cfg.min_recording_participants > 0
                                        && bridge_room.participant_count()
                                            >= cfg.min_recording_participants
                                    {
                                        match &wait_handler {
                                            Some(handler) => {
                                                if let Err(e) = handler
                                                    .start_room_recording(
                                                        &rn,
                                                        fcp_core::RecordingTrigger::Auto,
                                                    )
                                                    .await
                                                {
                                                    tracing::warn!(
                                                        room = %rn,
                                                        error = %e,
                                                        "Auto-record failed (after moderator join)"
                                                    );
                                                }
                                            }
                                            None => tracing::warn!(
                                                room = %rn,
                                                "Auto-record skipped: the handler is shutting down"
                                            ),
                                        }
                                    }
                                }
                            }

                            // Someone may be sharing already: a browser
                            // admitted now is told, as at a direct join.
                            if wait_by_token {
                                if let Some(handler) = &wait_handler {
                                    handler.tell_floor_on_join(&wait_call_id, &rn);
                                }
                            }

                            // Start the audio bridge, and the video bridge
                            // beside it when video was negotiated.
                            // The content leg first, as at a direct join.
                            let content = wait_content.as_ref().and_then(|leg| {
                                video_bridge::attach_content(
                                    &bridge_room,
                                    wait_backend.as_deref()?,
                                    &pid,
                                    leg,
                                    wait_sink.clone(),
                                    wait_may_share,
                                )
                            });
                            let video = wait_video.as_ref().and_then(|leg| {
                                video_bridge::attach_video(
                                    &bridge_room,
                                    wait_backend.as_deref()?,
                                    &pid,
                                    wait_display_name.as_deref(),
                                    leg,
                                    wait_sink.clone(),
                                )
                            });
                            let audio = audio_bridge::run_audio_bridge(
                                Arc::clone(&bridge_room),
                                pid.clone(),
                                bridge_socket,
                                encoder,
                                decoder,
                                payload_type,
                                frame_size,
                                jitter_buffer_ms,
                                te_pt,
                                dtmf_clock_rate,
                                dtmf_handler,
                                false, // waiting participants are not hosts
                                wait_stats_clone,
                                wait_srtp.clone(),
                                mixer_sample_rate,
                                wait_muted_samples.clone(),
                                wait_unmuted_samples.clone(),
                                wait_locked_samples.clone(),
                                wait_unlocked_samples.clone(),
                                wait_ending_samples.clone(),
                                false,
                            );
                            match (video, content) {
                                (Some(video), Some(content)) => {
                                    tokio::join!(audio, video, content);
                                }
                                (Some(video), None) => {
                                    tokio::join!(audio, video);
                                }
                                (None, Some(content)) => {
                                    tokio::join!(audio, content);
                                }
                                (None, None) => audio.await,
                            }
                        }
                        break;
                    }
                }
            });

            // Store session with the wait handle
            let remote_addr = parse_remote_rtp_addr(sdp_offer).unwrap_or(UNKNOWN_REMOTE);
            self.room_sessions
                .entry(room_number.clone())
                .or_default()
                .push(call_id.clone());
            if let Some(a) = &bfcp_assignment {
                self.bfcp
                    .bind(&room_number, a.user_id, &room, &participant_id);
            }
            self.sessions.insert(
                call_id.clone(),
                ParticipantSession {
                    call_id,
                    room_number,
                    participant_id,
                    is_host,
                    is_dial_out: false,
                    trunk_peer: None,
                    audio_bridge_handle: wait_handle,
                    port_pair,
                    rtp_socket: Arc::clone(&rtp_socket),
                    remote_addr,
                    codec_name,
                    tenant_id,
                    dialog: dialog.clone(),
                    joined_at: SystemTime::now(),
                    te_pt,
                    stats: wait_stats,
                    display_name: display_name.clone(),
                    caller_id: caller_id.clone(),
                    srtp_context: srtp_context.clone(),
                    last_refreshed: Instant::now(),
                    video: pending_video.take(),
                    content: pending_content.take(),
                    joined_by_token: join.is_some()
                        || migrate.as_ref().is_some_and(|t| t.token_join),
                    bfcp: bfcp_assignment.clone(),
                    record: LegRecord::from_request(request, None),
                },
            );

            self.handler_latency.record(
                &self.handler_latency.invite_count,
                &self.handler_latency.invite_total_us,
                _invite_start.elapsed(),
            );
            return Ok(());
        }

        info!(
            call_id = %call_id,
            room = %room_number,
            participant = %participant_id,
            codec = %codec_name,
            is_host = is_host,
            "Participant joined conference"
        );

        // Insert into room_sessions index first (atomically check if we're the first)
        {
            let mut entry = self.room_sessions.entry(room_number.clone()).or_default();
            let is_first = entry.is_empty();
            entry.push(call_id.clone());
            drop(entry); // release DashMap lock before publishing events

            // Publish MeetingStarted if this is the first session for this room
            if is_first {
                self.publish_event(ConferenceEvent::MeetingStarted {
                    room_number: room_number.clone(),
                    tenant_id,
                    timestamp: SystemTime::now(),
                });
            }
        }

        // A migrant arrives as they were: muted, at their gain, with
        // their video off if a host had turned it off.
        if let Some(ticket) = &migrate {
            if ticket.muted {
                if let Err(e) = room
                    .set_participant_state(&participant_id, forge_mixer::ParticipantState::Muted)
                {
                    warn!(participant = %participant_id, error = %e, "Could not restore mute");
                }
            }
            if (ticket.gain - 1.0).abs() > f32::EPSILON {
                if let Err(e) = room.set_participant_gain(&participant_id, ticket.gain) {
                    warn!(participant = %participant_id, error = %e, "Could not restore gain");
                }
            }
            if !ticket.video {
                if let Some(video) = room.video() {
                    video.set_participant_video_enabled(&participant_id, false);
                }
            }
        }

        // Publish join event
        self.sync_room_registry(&room_number);
        self.publish_event(ConferenceEvent::ParticipantJoined {
            room_number: room_number.clone(),
            participant_id: participant_id.clone(),
            migrated_from: migrate.as_ref().map(|t| t.from_node.clone()),
            is_host,
            has_video: pending_video.is_some(),
            tenant_id,
            timestamp: SystemTime::now(),
        });

        // Auto-record: start recording when participant threshold is reached
        if !room.is_recording() {
            if let Some(cfg) = room.get_effective_config() {
                if cfg.min_recording_participants > 0
                    && room.participant_count() >= cfg.min_recording_participants
                {
                    if let Err(e) = self
                        .start_room_recording(&room_number, fcp_core::RecordingTrigger::Auto)
                        .await
                    {
                        warn!(room = %room_number, error = %e, "Auto-record failed");
                    }
                }
            }
        }

        // 15b. Play welcome message before starting the audio bridge.
        //      Try TTS with a short timeout; fall back to static prompt if slow.
        {
            // Count OTHER real participants (exclude audio feedback + self)
            let total_in_mixer = room.participant_count();
            let others_count = total_in_mixer.saturating_sub(2); // -1 for audio_feedback, -1 for self
            let mixer_rate = conf.sample_rate;
            let mut welcome_encoder = create_codec_instance(&codec_name)?;
            let player = forge_conference::AudioFeedbackPlayer::new(mixer_rate);
            let mut welcome_played = false;

            // Whoever joins a room being recorded is named on its row.
            self.note_recording_participant(&room_number, &participant_id)
                .await;

            // A room being recorded tells whoever joins, when the
            // tenant's policy asks for an announcement (§11,
            // docs/COMPLIANCE.md). Before the welcome, so nobody speaks
            // first.
            if let Some(consent) = self.recording_consent_audio(&room_number) {
                play_prompt_to_rtp(
                    &rtp_socket,
                    &consent,
                    welcome_encoder.as_mut(),
                    payload_type,
                    frame_size,
                    &srtp_context,
                    mixer_rate,
                )
                .await;
                debug!(room = %room_number, participant = %participant_id, "Recording announcement played to the joiner");
            }

            // Try TTS with 5s timeout — Kokoro cache hits are <100ms, first synthesis 1-3s
            if let Some(ref tts) = self.tts_service {
                let welcome_text = if let Some(ref custom) = meeting.welcome_tts {
                    custom.replace("{count}", &others_count.to_string())
                } else {
                    let others = if others_count == 0 {
                        "You are the only participant in this conference.".to_string()
                    } else if others_count == 1 {
                        "There is 1 other participant in the conference.".to_string()
                    } else {
                        format!(
                            "There are {} other participants in the conference.",
                            others_count
                        )
                    };
                    format!("You have joined the conference. {}", others)
                };
                info!(room = %room_number, text = %welcome_text, "Synthesizing welcome message");
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    tts.synthesize(&welcome_text, None, None),
                )
                .await
                {
                    Ok(Ok(path)) => {
                        if let Some(pcm) = player.load_audio_file(&path) {
                            play_prompt_to_rtp(
                                &rtp_socket,
                                &pcm,
                                welcome_encoder.as_mut(),
                                payload_type,
                                frame_size,
                                &srtp_context,
                                mixer_rate,
                            )
                            .await;
                            welcome_played = true;
                        }
                    }
                    Ok(Err(e)) => {
                        warn!(room = %room_number, error = %e, "TTS welcome synthesis failed")
                    }
                    Err(_) => {
                        warn!(room = %room_number, "TTS welcome timed out (5s), using static prompt")
                    }
                }
            }

            // Fallback: static prompt from the Prompt store
            if !welcome_played {
                if let Some(prompt_store) = self.storage.prompt_store() {
                    let tid = tenant_id;
                    if let Ok(Some(prompt)) =
                        prompt_store.get_prompt_by_name(&tid, "conf_welcome").await
                    {
                        if let Some(ref path) = prompt.audio_path {
                            if let Some(pcm) = player.load_audio_file(path) {
                                play_prompt_to_rtp(
                                    &rtp_socket,
                                    &pcm,
                                    welcome_encoder.as_mut(),
                                    payload_type,
                                    frame_size,
                                    &srtp_context,
                                    mixer_rate,
                                )
                                .await;
                            }
                        }
                    }
                }
            }
        }

        // 15c. Now that the welcome message has played, trigger the join tone
        //       so other participants hear the beep right before audio bridges.
        room.play_join_sound();

        // 16. Create codec instances for the audio bridge
        let encoder = create_codec_instance(&codec_name)?;
        let decoder = create_codec_instance(&codec_name)?;

        // 17. Create DTMF handler and stats for audio bridge
        let dtmf_handler = if conf.dtmf_enabled {
            let handler = self.get_or_create_dtmf_handler(&room_number);
            // Propagate host status so DTMF host commands work immediately
            if is_host {
                handler.set_host(&participant_id, true);
            }
            Some(handler)
        } else {
            None
        };
        let stats = Arc::new(AudioStats::new());

        // 18. Spawn audio bridge task
        let bridge_room = Arc::clone(&room);
        let bridge_participant = participant_id.clone();
        let bridge_socket = Arc::clone(&rtp_socket);
        let stats_clone = Arc::clone(&stats);
        let bridge_srtp = srtp_context.clone();
        let audio_bridge_handle = tokio::spawn(audio_bridge::run_audio_bridge(
            bridge_room,
            bridge_participant,
            bridge_socket,
            encoder,
            decoder,
            payload_type,
            frame_size,
            conf.jitter_buffer_ms,
            te_pt,
            dtmf_clock_rate,
            dtmf_handler,
            is_host,
            stats_clone,
            bridge_srtp,
            conf.sample_rate,
            self.dtmf_muted_samples.clone(),
            self.dtmf_unmuted_samples.clone(),
            self.dtmf_locked_samples.clone(),
            self.dtmf_unlocked_samples.clone(),
            self.dtmf_ending_samples.clone(),
            false,
        ));

        // 18b. Spawn the video bridges beside it: the camera's and the
        // shared screen's. Whether this caller may take the floor is the
        // meeting's policy for a host or a guest (§15.6).
        // The content leg first: a caller with one gets the cameras alone
        // on their main section, which the camera's attach decides by
        // looking for it.
        let may_share = video_bridge::may_share(&meeting.video_content_share, is_host);
        let content = pending_content.take().and_then(|leg| {
            self.start_video_leg(
                &room,
                &participant_id,
                display_name.as_deref(),
                tenant_id,
                &room_number,
                leg,
                may_share,
            )
        });
        let video = pending_video.take().and_then(|leg| {
            self.start_video_leg(
                &room,
                &participant_id,
                display_name.as_deref(),
                tenant_id,
                &room_number,
                leg,
                may_share,
            )
        });

        // 19. Store session (room_sessions already updated above for MeetingStarted dedup)
        let remote_addr = parse_remote_rtp_addr(sdp_offer).unwrap_or(UNKNOWN_REMOTE);
        if join.is_some() {
            // Told after the insert below has made the session findable.
            self.tell_floor_on_join(&call_id, &room_number);
        }
        if let Some(a) = &bfcp_assignment {
            self.bfcp
                .bind(&room_number, a.user_id, &room, &participant_id);
        }
        self.sessions.insert(
            call_id.clone(),
            ParticipantSession {
                call_id,
                room_number,
                participant_id,
                is_host,
                is_dial_out: false,
                trunk_peer: None,
                audio_bridge_handle,
                port_pair,
                rtp_socket,
                remote_addr,
                codec_name,
                tenant_id,
                dialog,
                joined_at: SystemTime::now(),
                te_pt,
                stats,
                display_name,
                caller_id,
                srtp_context,
                last_refreshed: Instant::now(),
                video,
                content,
                joined_by_token: join.is_some(),
                bfcp: bfcp_assignment.clone(),
                record: LegRecord::from_request(
                    request,
                    migrate.as_ref().map(|t| t.from_node.clone()),
                ),
            },
        );

        self.handler_latency.record(
            &self.handler_latency.invite_count,
            &self.handler_latency.invite_total_us,
            _invite_start.elapsed(),
        );
        Ok(())
    }

    async fn handle_reinvite(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        let reinvite_start = Instant::now();
        let call_id = extract_call_id(request);
        let conf = self.conference_config.read().clone();

        // Look up session — if not found, return Err so caller falls through
        // to the B2BUA. A snapshot: the map is not held across the async
        // work below (a video leg added mid-call needs ports and a socket).
        let (remote_ip, room_number, tenant_id, has_video, has_content, is_host) = {
            let session = self
                .sessions
                .get(&call_id)
                .ok_or_else(|| anyhow::anyhow!("No conference session for call_id: {}", call_id))?;
            (
                session.remote_addr.ip(),
                session.room_number.clone(),
                session.tenant_id,
                session.video.is_some(),
                session.content.is_some(),
                session.is_host,
            )
        };

        // Rate-limit re-INVITEs using the participant's known remote IP
        if !self.check_invite_rate(remote_ip, conf.max_invite_rate_per_ip) {
            warn!(call_id = %call_id, peer = %remote_ip, "re-INVITE rate limit exceeded");
            return Ok(UserAgentServer::create_response(request, 486, "Busy Here"));
        }

        // Parse new SDP body
        let body = request.body();
        let sdp_offer =
            std::str::from_utf8(&body[..]).context("re-INVITE SDP body is not valid UTF-8")?;

        // Video added mid-call (a new active video section, for the camera
        // or a shared screen): prepared before the session is locked.
        let offered_sections = fcp_media::parse_video_sections(sdp_offer);
        let offered_video = offered_sections.camera.filter(|v| v.port != 0);
        let offered_content = offered_sections.content.filter(|v| v.port != 0);
        let new_video = if !has_video && offered_video.is_some() {
            self.prepare_leg(
                StreamKind::Camera,
                &tenant_id,
                &room_number,
                sdp_offer,
                &conf,
            )
            .await
        } else {
            None
        };
        let new_content = if !has_content && offered_content.is_some() {
            self.prepare_leg(
                StreamKind::Content,
                &tenant_id,
                &room_number,
                sdp_offer,
                &conf,
            )
            .await
        } else {
            None
        };

        let mut session = self
            .sessions
            .get_mut(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No conference session for call_id: {}", call_id))?;

        // Check for remote address change
        if let Some(new_remote) = parse_remote_rtp_addr(sdp_offer) {
            if new_remote != session.remote_addr {
                info!(
                    call_id = %call_id,
                    old = %session.remote_addr,
                    new = %new_remote,
                    "re-INVITE: remote RTP address changed"
                );
                session.remote_addr = new_remote;
                let rtp_socket = Arc::clone(&session.rtp_socket);
                tokio::spawn(async move {
                    rtp_socket
                        .set_remote_endpoint(RemoteEndpoint {
                            rtp_addr: new_remote,
                            rtcp_addr: None,
                        })
                        .await;
                });
            }
        }

        // Check for codec change — reject if re-INVITE offers no supported codec
        let offered_codecs = parse_sdp_codecs(sdp_offer);
        let allowed = conf.allowed_codecs.as_deref();
        let codec_pick = pick_codec(&offered_codecs, allowed);
        if codec_pick.is_none() && !offered_codecs.is_empty() {
            // re-INVITE offered codecs but none are supported — reject
            warn!(
                call_id = %call_id,
                offered = ?offered_codecs.iter().map(|c| &c.encoding_name).collect::<Vec<_>>(),
                "re-INVITE: no supported codec in offer, rejecting with 488"
            );
            return Ok(UserAgentServer::create_response(
                request,
                488,
                "Not Acceptable Here",
            ));
        }
        if let Some((new_codec, new_pt, new_frame_size)) = codec_pick {
            if new_codec != session.codec_name {
                info!(
                    call_id = %call_id,
                    old_codec = %session.codec_name,
                    new_codec = %new_codec,
                    "re-INVITE: codec changed, restarting audio bridge"
                );

                // Abort old audio bridge — take ownership of the handle
                let old_handle = std::mem::replace(
                    &mut session.audio_bridge_handle,
                    tokio::spawn(async {}), // temporary placeholder
                );
                old_handle.abort();

                // Create new codec instances
                let encoder = create_codec_instance(&new_codec)?;
                let decoder = create_codec_instance(&new_codec)?;

                // Get room reference
                let room = self
                    .bridge
                    .get_room(&session.room_number)
                    .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

                // Spawn new audio bridge that waits for old bridge to stop first
                // This prevents the race where both bridges access the mixer simultaneously
                let bridge_room = Arc::clone(&room);
                let bridge_participant = session.participant_id.clone();
                let bridge_socket = Arc::clone(&session.rtp_socket);
                let jitter_buffer_ms = conf.jitter_buffer_ms;
                let reinvite_mixer_rate = conf.sample_rate;
                let reinvite_dtmf = if conf.dtmf_enabled {
                    Some(self.get_or_create_dtmf_handler(&session.room_number))
                } else {
                    None
                };
                let reinvite_stats = Arc::clone(&session.stats);
                let reinvite_is_host = session.is_host;
                let reinvite_is_trunk = session.trunk_peer.is_some();
                let reinvite_te_pt = session.te_pt;
                let reinvite_dtmf_clock_rate =
                    find_telephone_event_clock_rate(&offered_codecs, sdp_offer);
                let reinvite_srtp = session.srtp_context.clone();
                let reinvite_muted = self.dtmf_muted_samples.clone();
                let reinvite_unmuted = self.dtmf_unmuted_samples.clone();
                let reinvite_locked = self.dtmf_locked_samples.clone();
                let reinvite_unlocked = self.dtmf_unlocked_samples.clone();
                let reinvite_ending = self.dtmf_ending_samples.clone();
                let new_handle = tokio::spawn(async move {
                    // Wait for old bridge to actually stop (with timeout) before starting new one
                    match tokio::time::timeout(Duration::from_millis(500), old_handle).await {
                        Ok(_) => {}
                        Err(_) => warn!("Old audio bridge did not stop within 500ms on re-INVITE"),
                    }

                    audio_bridge::run_audio_bridge(
                        bridge_room,
                        bridge_participant,
                        bridge_socket,
                        encoder,
                        decoder,
                        new_pt,
                        new_frame_size,
                        jitter_buffer_ms,
                        reinvite_te_pt,
                        reinvite_dtmf_clock_rate,
                        reinvite_dtmf,
                        reinvite_is_host,
                        reinvite_stats,
                        reinvite_srtp,
                        reinvite_mixer_rate,
                        reinvite_muted,
                        reinvite_unmuted,
                        reinvite_locked,
                        reinvite_unlocked,
                        reinvite_ending,
                        reinvite_is_trunk,
                    )
                    .await;
                });

                session.audio_bridge_handle = new_handle;
                session.codec_name = new_codec;
            }
        }

        // Check for SRTP key rollover (new a=crypto line in re-INVITE)
        let srtp_sdp = if conf.enable_srtp {
            let crypto_lines = parse_sdes_crypto(sdp_offer);
            if let Some(crypto) = crypto_lines.first() {
                match Self::negotiate_srtp(crypto) {
                    Ok((new_ctx, sdp_ctx)) => {
                        info!(call_id = %call_id, "re-INVITE: SRTP key rollover");
                        if let Some(ref srtp_lock) = session.srtp_context {
                            let srtp_lock = Arc::clone(srtp_lock);
                            tokio::spawn(async move {
                                *srtp_lock.write().await = new_ctx;
                            });
                        } else {
                            // Upgrading from plain RTP to SRTP
                            session.srtp_context =
                                Some(Arc::new(tokio::sync::RwLock::new(new_ctx)));
                        }
                        Some(sdp_ctx)
                    }
                    Err(e) => {
                        warn!(error = %e, "SRTP renegotiation failed, keeping existing keys");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // Video section: removed by the peer, kept (address change, key
        // rollover), or added. The camera's and the shared screen's alike.
        let mut video_codec: Vec<forge_core::VideoCodec> = Vec::new();
        let mut video_srtp_sdp: Option<SrtpSdpContext> = None;
        if session.video.is_some() && offered_video.is_none() {
            info!(call_id = %call_id, "re-INVITE: video removed by the peer");
            self.tear_down_leg(&mut session, StreamKind::Camera);
        } else if let Some(leg) = session.video.as_ref() {
            let new_remote = offered_video
                .as_ref()
                .and_then(|v| v.remote)
                .or_else(|| fcp_media::remote_video_endpoint(sdp_offer));
            let crypto = parse_video_sdes_crypto(sdp_offer);
            video_srtp_sdp = self.refresh_leg(
                &call_id,
                leg,
                new_remote,
                crypto.first().filter(|_| conf.enable_srtp),
            );
            video_codec = vec![leg.negotiated.codec];
        } else if let Some((leg, policy)) = new_video {
            let started = match self.bridge.get_room(&room_number) {
                Ok(room) => self.start_video_leg(
                    &room,
                    &session.participant_id,
                    session.display_name.as_deref(),
                    tenant_id,
                    &room_number,
                    leg,
                    video_bridge::may_share(&policy, is_host),
                ),
                Err(_) => {
                    self.release_video_pair_later(leg.port_pair);
                    None
                }
            };
            if let Some(leg) = started {
                info!(
                    call_id = %call_id,
                    codec = %leg.negotiated.codec,
                    video_port = leg.port_pair.rtp_port,
                    "re-INVITE: video added"
                );
                video_codec = vec![leg.negotiated.codec];
                video_srtp_sdp = leg.srtp_sdp.clone();
                session.video = Some(leg);
            }
        }
        let mut content_codec: Vec<forge_core::VideoCodec> = Vec::new();
        let mut content_srtp_sdp: Option<SrtpSdpContext> = None;
        if session.content.is_some() && offered_content.is_none() {
            info!(call_id = %call_id, "re-INVITE: the shared screen's section removed by the peer");
            self.tear_down_leg(&mut session, StreamKind::Content);
        } else if let Some(leg) = session.content.as_ref() {
            let new_remote = offered_content
                .as_ref()
                .and_then(|v| v.remote)
                .or_else(|| fcp_media::remote_content_endpoint(sdp_offer));
            let crypto = parse_content_sdes_crypto(sdp_offer);
            content_srtp_sdp = self.refresh_leg(
                &call_id,
                leg,
                new_remote,
                crypto.first().filter(|_| conf.enable_srtp),
            );
            content_codec = vec![leg.negotiated.codec];
        } else if let Some((leg, policy)) = new_content {
            let started = match self.bridge.get_room(&room_number) {
                Ok(room) => self.start_video_leg(
                    &room,
                    &session.participant_id,
                    session.display_name.as_deref(),
                    tenant_id,
                    &room_number,
                    leg,
                    video_bridge::may_share(&policy, is_host),
                ),
                Err(_) => {
                    self.release_video_pair_later(leg.port_pair);
                    None
                }
            };
            if let Some(leg) = started {
                info!(
                    call_id = %call_id,
                    codec = %leg.negotiated.codec,
                    content_port = leg.port_pair.rtp_port,
                    "re-INVITE: shared screen added"
                );
                content_codec = vec![leg.negotiated.codec];
                content_srtp_sdp = leg.srtp_sdp.clone();
                session.content = Some(leg);
            }
        }

        // Refresh session timer (re-INVITE counts as a session refresh per RFC 4028)
        session.last_refreshed = Instant::now();

        // Construct SDP answer with current local RTP ports; a BFCP
        // endpoint keeps the ids it was given (7e).
        let bfcp_label = fcp_media::parse_video_sections(sdp_offer)
            .content
            .and_then(|c| c.label);
        let bfcp_answer = self.bfcp_answer_for(&session, bfcp_label.as_deref());
        let answer = self.build_answer(
            sdp_offer,
            session.port_pair.rtp_port,
            srtp_sdp.as_ref(),
            session.video.as_ref().map(|leg| VideoAnswer {
                rtp_port: leg.port_pair.rtp_port,
                codecs: &video_codec,
                srtp: video_srtp_sdp.as_ref(),
            }),
            session.content.as_ref().map(|leg| VideoAnswer {
                rtp_port: leg.port_pair.rtp_port,
                codecs: &content_codec,
                srtp: content_srtp_sdp.as_ref(),
            }),
            bfcp_answer,
        );
        if session.video.is_some() && answer.video.is_none() {
            info!(call_id = %call_id, "re-INVITE: the video codec is no longer offered; video removed");
            self.tear_down_leg(&mut session, StreamKind::Camera);
        }
        if session.content.is_some() && answer.content.is_none() {
            info!(call_id = %call_id, "re-INVITE: the shared screen's codec is no longer offered; content removed");
            self.tear_down_leg(&mut session, StreamKind::Content);
        }
        let sdp_answer = answer.sdp;

        // Build 200 OK response with SDP answer
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri);
        let mut response = uas
            .create_ok(request, Some(&sdp_answer))
            .map_err(|e| anyhow::anyhow!("Failed to create 200 OK for re-INVITE: {}", e))?;

        // RFC 4028: Add Session-Expires and Min-SE headers on re-INVITE 200 OK
        if conf.session_expires_secs > 0 {
            let client_se = sip_parse::header(request.headers(), "Session-Expires").and_then(|v| {
                v.as_str()
                    .split(';')
                    .next()
                    .and_then(|n| n.trim().parse::<u64>().ok())
            });
            let min_se: u64 = conf.min_session_expires_secs;
            let negotiated_se = match client_se {
                Some(requested) if requested < min_se => conf.session_expires_secs,
                Some(requested) => requested.min(conf.session_expires_secs),
                None => conf.session_expires_secs,
            };
            let _ = response.headers_mut().push(
                "Session-Expires",
                format!("{};refresher=uac", negotiated_se),
            );
            let _ = response.headers_mut().push("Min-SE", min_se.to_string());
            let _ = response.headers_mut().push("Require", "timer");
        }

        self.handler_latency.record(
            &self.handler_latency.reinvite_count,
            &self.handler_latency.reinvite_total_us,
            reinvite_start.elapsed(),
        );
        Ok(response)
    }

    fn handle_bye(&self, call_id: &str) -> Result<()> {
        let bye_start = Instant::now();
        let result = if self.sessions.contains_key(call_id) {
            self.remove_session_for(call_id, fcp_sip::TerminationReason::Hangup { by: None });
            Ok(())
        } else if self.pending_pin_auth.remove(call_id).is_some() {
            info!(call_id = %call_id, "BYE received during conference PIN collection");
            Ok(())
        } else {
            anyhow::bail!("No conference session for call_id: {}", call_id)
        };
        self.handler_latency.record(
            &self.handler_latency.bye_count,
            &self.handler_latency.bye_total_us,
            bye_start.elapsed(),
        );
        result
    }

    fn handle_cancel(&self, call_id: &str) -> Result<bool> {
        if self.sessions.contains_key(call_id) {
            info!(call_id = %call_id, "CANCEL received for conference participant");
            self.remove_session_for(call_id, fcp_sip::TerminationReason::Cancelled);
            Ok(true)
        } else if self.pending_pin_auth.remove(call_id).is_some() {
            info!(call_id = %call_id, "CANCEL received during conference PIN collection");
            Ok(true)
        } else {
            Ok(false) // Not a conference call — let B2BUA handle it
        }
    }

    fn handle_update(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        let call_id = extract_call_id(request);

        // Look up session — if not found, return Err so caller falls through to B2BUA
        let mut session = self
            .sessions
            .get_mut(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No conference session for call_id: {}", call_id))?;

        // Parse SDP if present (UPDATE may have no body for session refresh)
        let body = request.body();
        if !body.is_empty() {
            let sdp_offer =
                std::str::from_utf8(&body[..]).context("UPDATE SDP body is not valid UTF-8")?;

            // Check for remote address change (e.g., hold/resume or IP migration)
            if let Some(new_remote) = parse_remote_rtp_addr(sdp_offer) {
                if new_remote != session.remote_addr {
                    info!(
                        call_id = %call_id,
                        old = %session.remote_addr,
                        new = %new_remote,
                        "UPDATE: remote RTP address changed"
                    );
                    session.remote_addr = new_remote;
                    let rtp_socket = Arc::clone(&session.rtp_socket);
                    tokio::spawn(async move {
                        rtp_socket
                            .set_remote_endpoint(RemoteEndpoint {
                                rtp_addr: new_remote,
                                rtcp_addr: None,
                            })
                            .await;
                    });
                }
            }
        }

        // Refresh session timer (UPDATE counts as a session refresh per RFC 4028)
        session.last_refreshed = Instant::now();

        let conf = self.conference_config.read().clone();

        // Build 200 OK response (with SDP answer if request had SDP)
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri);

        let mut response = if !body.is_empty() {
            let sdp_offer = std::str::from_utf8(&body[..]).unwrap_or("");
            let video_codec: Vec<forge_core::VideoCodec> = session
                .video
                .as_ref()
                .map(|leg| vec![leg.negotiated.codec])
                .unwrap_or_default();
            let content_codec: Vec<forge_core::VideoCodec> = session
                .content
                .as_ref()
                .map(|leg| vec![leg.negotiated.codec])
                .unwrap_or_default();
            let bfcp_label = fcp_media::parse_video_sections(sdp_offer)
                .content
                .and_then(|c| c.label);
            let bfcp_answer = self.bfcp_answer_for(&session, bfcp_label.as_deref());
            let sdp_answer = self
                .build_answer(
                    sdp_offer,
                    session.port_pair.rtp_port,
                    None,
                    session.video.as_ref().map(|leg| VideoAnswer {
                        rtp_port: leg.port_pair.rtp_port,
                        codecs: &video_codec,
                        srtp: None,
                    }),
                    session.content.as_ref().map(|leg| VideoAnswer {
                        rtp_port: leg.port_pair.rtp_port,
                        codecs: &content_codec,
                        srtp: None,
                    }),
                    bfcp_answer,
                )
                .sdp;
            uas.create_ok(request, Some(&sdp_answer))
                .map_err(|e| anyhow::anyhow!("Failed to create 200 OK for UPDATE: {}", e))?
        } else {
            uas.create_ok(request, None)
                .map_err(|e| anyhow::anyhow!("Failed to create 200 OK for UPDATE: {}", e))?
        };

        // RFC 4028: Add Session-Expires and Min-SE headers on UPDATE 200 OK
        if conf.session_expires_secs > 0 {
            let client_se = sip_parse::header(request.headers(), "Session-Expires").and_then(|v| {
                v.as_str()
                    .split(';')
                    .next()
                    .and_then(|n| n.trim().parse::<u64>().ok())
            });
            let min_se: u64 = conf.min_session_expires_secs;
            let negotiated_se = match client_se {
                Some(requested) if requested < min_se => conf.session_expires_secs,
                Some(requested) => requested.min(conf.session_expires_secs),
                None => conf.session_expires_secs,
            };
            let _ = response.headers_mut().push(
                "Session-Expires",
                format!("{};refresher=uac", negotiated_se),
            );
            let _ = response.headers_mut().push("Min-SE", min_se.to_string());
            let _ = response.headers_mut().push("Require", "timer");
        }

        Ok(response)
    }

    fn handle_info(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        let call_id = extract_call_id(request);
        let body = request.body();

        if let Some(pending) = self.pending_pin_auth.get(&call_id) {
            if !body.is_empty() {
                let body_str = String::from_utf8_lossy(body);
                for line in body_str.lines() {
                    let line = line.trim();
                    if let Some(signal) = line
                        .strip_prefix("Signal=")
                        .or_else(|| line.strip_prefix("signal="))
                    {
                        let digit_str = signal.trim();
                        if let Some(ch) = digit_str.chars().next() {
                            if DtmfDigit::from_char(ch).is_ok() {
                                debug!(
                                    call_id = %call_id,
                                    room = %pending.room_number,
                                    participant = %pending.participant_id,
                                    digit = %digit_str,
                                    "INFO DTMF received during PIN collection"
                                );
                                let _ = pending.info_digits_tx.send(ch);
                            }
                        }
                    }
                }
            }

            let local_sip_uri = self.parse_local_sip_uri();
            let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri);
            return uas
                .create_ok(request, None)
                .map_err(|e| anyhow::anyhow!("Failed to create 200 OK for INFO: {}", e));
        }

        // Look up session — if not found, return Err so caller falls through to B2BUA
        let session = self
            .sessions
            .get(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No conference session for call_id: {}", call_id))?;

        // Parse DTMF from application/dtmf-relay body
        if !body.is_empty() {
            let body_str = String::from_utf8_lossy(body);
            // Format: "Signal=5\r\nDuration=160\r\n"
            for line in body_str.lines() {
                let line = line.trim();
                if let Some(signal) = line
                    .strip_prefix("Signal=")
                    .or_else(|| line.strip_prefix("signal="))
                {
                    let digit_str = signal.trim();
                    if let Some(ch) = digit_str.chars().next() {
                        if let Ok(_digit) = DtmfDigit::from_char(ch) {
                            debug!(
                                call_id = %call_id,
                                room = %session.room_number,
                                digit = %digit_str,
                                "INFO DTMF received"
                            );
                            // Route to DTMF handler if enabled
                            if self.conference_config.read().dtmf_enabled {
                                let handler = self.get_or_create_dtmf_handler(&session.room_number);
                                let _ = handler.process_digit(&session.participant_id, ch);
                            }
                        }
                    }
                }
            }
        }

        // Always respond 200 OK to INFO
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri);
        let response = uas
            .create_ok(request, None)
            .map_err(|e| anyhow::anyhow!("Failed to create 200 OK for INFO: {}", e))?;

        Ok(response)
    }

    async fn handle_subscribe(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()> {
        let subscribe_start = Instant::now();
        // Rate-limit SUBSCRIBEs — extract source from Via header
        let conf = self.conference_config.read().clone();
        if let Some(via_ip) = extract_via_ip(request) {
            if !self.check_invite_rate(via_ip, conf.max_invite_rate_per_ip) {
                warn!(peer = %via_ip, "SUBSCRIBE rate limit exceeded");
                let reject = UserAgentServer::create_response(request, 486, "Busy Here");
                handle.send_final(reject).await;
                return Ok(());
            }
        }

        // Only handle Event: conference subscriptions
        let event_header = sip_parse::header(request.headers(), "Event")
            .ok_or_else(|| anyhow::anyhow!("No Event header in SUBSCRIBE"))?;
        if !event_header.as_str().starts_with("conference") {
            anyhow::bail!("Not a conference event SUBSCRIBE");
        }

        // Extract room number from Request-URI
        let room_number = extract_room_number(request)
            .ok_or_else(|| anyhow::anyhow!("Cannot extract room number from SUBSCRIBE URI"))?;

        // Verify room exists (either active or configured)
        let room_active = self.bridge.get_room(&room_number).is_ok();
        let tenant_id = self.resolve_tenant(request).await?;
        let meeting_exists = self
            .lookup_meeting(&tenant_id, &room_number)
            .await?
            .is_some();
        if !room_active && !meeting_exists {
            let response = UserAgentServer::create_response(request, 404, "Conference Not Found");
            handle.send_final(response).await;
            return Ok(());
        }

        // Authorization: subscriber must be an active participant in this room.
        // This prevents arbitrary SIP endpoints from subscribing to conference state.
        let call_id_sub = extract_call_id(request);
        let is_participant = self
            .sessions
            .iter()
            .any(|entry| entry.value().room_number == room_number);
        // Also allow if the subscriber's Call-ID matches an existing session
        let is_session = self.sessions.contains_key(&call_id_sub);
        if !is_participant && !is_session {
            warn!(
                room = %room_number,
                call_id = %call_id_sub,
                "SUBSCRIBE rejected: subscriber is not a conference participant"
            );
            let response = UserAgentServer::create_response(request, 403, "Forbidden");
            handle.send_final(response).await;
            return Ok(());
        }

        // Parse Expires header (default 3600)
        let expires_secs: u64 = sip_parse::header(request.headers(), "Expires")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);

        let call_id = extract_call_id(request);

        // If Expires=0, this is an unsubscribe
        if expires_secs == 0 {
            self.conference_subscriptions.remove(&call_id);
            info!(call_id = %call_id, room = %room_number, "Conference subscription terminated");
            let mut response = UserAgentServer::create_response(request, 200, "OK");
            let _ = response.headers_mut().push("Expires", "0");
            handle.send_final(response).await;
            return Ok(());
        }

        // Extract subscriber Contact URI for NOTIFY delivery
        let contact_uri = sip_parse::header(request.headers(), "Contact")
            .and_then(|c| {
                let s = c.as_str();
                // Strip angle brackets if present
                let uri_str = if let Some(inner) = s.strip_prefix('<') {
                    inner.split('>').next().unwrap_or(s)
                } else {
                    s.split(';').next().unwrap_or(s)
                };
                SipUri::parse(uri_str).ok()
            })
            .unwrap_or_else(|| self.parse_local_sip_uri());

        // Prune expired subscriptions and cap total to prevent unbounded growth
        const MAX_SUBSCRIPTIONS: usize = 10_000;
        if self.conference_subscriptions.len() >= MAX_SUBSCRIPTIONS {
            let now = Instant::now();
            self.conference_subscriptions
                .retain(|_, sub| sub.expires_at > now);
            if self.conference_subscriptions.len() >= MAX_SUBSCRIPTIONS {
                warn!(
                    count = self.conference_subscriptions.len(),
                    "Subscription limit reached, rejecting new subscription"
                );
                let response =
                    UserAgentServer::create_response(request, 503, "Service Unavailable");
                handle.send_final(response).await;
                return Ok(());
            }
        }

        // Create/refresh subscription
        self.conference_subscriptions.insert(
            call_id.clone(),
            ConferenceSubscription {
                room_number: room_number.clone(),
                contact_uri: contact_uri.clone(),
                expires_at: Instant::now() + Duration::from_secs(expires_secs),
            },
        );

        info!(
            call_id = %call_id,
            room = %room_number,
            expires = expires_secs,
            "Conference event subscription created"
        );

        // Send 200 OK with Expires
        let mut response = UserAgentServer::create_response(request, 200, "OK");
        let _ = response
            .headers_mut()
            .push("Expires", expires_secs.to_string());
        handle.send_final(response).await;

        // Send initial NOTIFY with current conference state
        if room_active {
            if let Ok(room) = self.bridge.get_room(&room_number) {
                let state_xml = build_conference_state_xml(&room_number, &room);
                let _ = self
                    .uac
                    .send_unsolicited_notify(
                        &contact_uri,
                        "conference",
                        "application/conference-info+xml",
                        &state_xml,
                    )
                    .await;
            }
        }

        self.handler_latency.record(
            &self.handler_latency.subscribe_count,
            &self.handler_latency.subscribe_total_us,
            subscribe_start.elapsed(),
        );
        Ok(())
    }

    fn handle_refer(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        let refer_start = Instant::now();
        // Rate-limit REFERs
        let conf = self.conference_config.read().clone();
        if let Some(via_ip) = extract_via_ip(request) {
            if !self.check_invite_rate(via_ip, conf.max_invite_rate_per_ip) {
                warn!(peer = %via_ip, "REFER rate limit exceeded");
                return Ok(UserAgentServer::create_response(request, 486, "Busy Here"));
            }
        }

        // Extract Refer-To header to determine target
        let refer_to = sip_parse::header(request.headers(), "Refer-To")
            .ok_or_else(|| anyhow::anyhow!("No Refer-To header in REFER"))?;

        // Parse the Refer-To URI to extract room number
        let refer_to_str = refer_to.as_str();
        let room_number = if let Some(rest) = refer_to_str.strip_prefix("sip:") {
            rest.split('@')
                .next()
                .filter(|u| !u.is_empty())
                .map(String::from)
        } else if let Some(rest) = refer_to_str.strip_prefix("<sip:") {
            rest.split('@')
                .next()
                .filter(|u| !u.is_empty())
                .map(String::from)
        } else {
            None
        };

        let room_number = room_number
            .filter(|r| validate_room_number(r))
            .ok_or_else(|| anyhow::anyhow!("Refer-To does not target a conference room"))?;

        // Check if this room exists
        if self.bridge.get_room(&room_number).is_err() {
            anyhow::bail!("Conference room not active: {}", room_number);
        }

        // Detect attended transfer: Refer-To URI contains a Replaces parameter
        // e.g. <sip:conf@host?Replaces=callid%3Bto-tag%3D...%3Bfrom-tag%3D...>
        let is_attended = refer_to_str.contains("Replaces=") || refer_to_str.contains("replaces=");
        let replaces_call_id = if is_attended {
            // Extract the Replaces call-id from the Refer-To URI query
            refer_to_str
                .split("Replaces=")
                .nth(1)
                .or_else(|| refer_to_str.split("replaces=").nth(1))
                .and_then(|r| {
                    // URL-decode %3B → ; to parse call-id;to-tag=...;from-tag=...
                    let decoded = r.replace("%3B", ";").replace("%3b", ";");
                    decoded.split(';').next().map(|s| s.to_string())
                })
        } else {
            None
        };

        // Accept the REFER with 202 Accepted
        // For attended transfers: the referee will send a new INVITE with Replaces header
        // For blind transfers: the referee will send a new INVITE to the conference room
        info!(
            room = %room_number,
            refer_to = %refer_to_str,
            attended = is_attended,
            replaces_call_id = ?replaces_call_id,
            "REFER accepted for conference room"
        );

        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri);
        let response = uas
            .create_ok(request, None)
            .map_err(|e| anyhow::anyhow!("Failed to create 202 for REFER: {}", e))?;

        self.handler_latency.record(
            &self.handler_latency.refer_count,
            &self.handler_latency.refer_total_us,
            refer_start.elapsed(),
        );
        Ok(response)
    }

    /// The report on a REFER this node sent as it drained (phase 5): a
    /// `message/sipfrag` NOTIFY from the caller's leg. 2xx means the
    /// caller is on the other node, so this leg is dropped; a failure
    /// leaves them here. Any NOTIFY on a session this node holds is
    /// answered 200, whether or not it is migrating.
    fn handle_notify(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        let call_id = extract_call_id(request);
        if !self.sessions.contains_key(&call_id) {
            anyhow::bail!("No conference session for call_id: {}", call_id)
        }
        let body = String::from_utf8_lossy(request.body());
        let status = migration::sipfrag_status(&body);
        match status.and_then(|code| self.migrations.on_notify(&call_id, code)) {
            Some(Notice::Succeeded) => {
                info!(call_id = %call_id, "Participant is on the other node; dropping this leg");
                if let Some(me) = self.me() {
                    let call_id = call_id.clone();
                    tokio::spawn(async move { me.finish_migration(&call_id).await });
                } else {
                    self.remove_session_for(
                        &call_id,
                        fcp_sip::TerminationReason::system(
                            fcp_sip::SystemCause::Migration,
                            "Moved to another node",
                        ),
                    );
                }
            }
            Some(Notice::Failed(code)) => {
                warn!(call_id = %call_id, code, "Migration failed; the caller stays here")
            }
            Some(Notice::Pending) => debug!(call_id = %call_id, "Migration in progress"),
            None => {
                debug!(call_id = %call_id, status = ?status, "NOTIFY on a session not migrating")
            }
        }
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri);
        uas.create_ok(request, None)
            .map_err(|e| anyhow::anyhow!("Failed to create 200 for NOTIFY: {}", e))
    }
}

/// Truncate a string to `max_len` bytes for use in filenames,
/// ensuring we don't cut in the middle of a UTF-8 character.
fn truncate_for_filename(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    // Find the last valid char boundary at or before max_len
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// XML-escape a string for safe embedding in XML attributes and text nodes.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Build a minimal RFC 4575 conference-info XML document for a room.
fn build_conference_state_xml(
    room_number: &str,
    room: &forge_conference::ConferenceRoom,
) -> String {
    let participants = room.participants();
    let safe_room = xml_escape(room_number);
    let mut users_xml = String::new();
    // Cap XML output to prevent excessive memory usage with very large rooms
    let max_users_in_xml: usize = 1000;
    for pid in participants.iter().take(max_users_in_xml) {
        let safe_pid = xml_escape(pid);
        users_xml.push_str(&format!(
            "    <user entity=\"sip:{}@conference\">\n      <display-text>{}</display-text>\n      <endpoint entity=\"sip:{}@conference\">\n        <status>connected</status>\n      </endpoint>\n    </user>\n",
            safe_pid, safe_pid, safe_pid,
        ));
    }
    if participants.len() > max_users_in_xml {
        users_xml.push_str("    <!-- additional participants truncated -->\n");
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <conference-info xmlns=\"urn:ietf:params:xml:ns:conference-info\"\n  \
           entity=\"sip:{}@conference\" state=\"full\" version=\"1\">\n  \
           <conference-description>\n    <display-text>Conference {}</display-text>\n  \
           </conference-description>\n  \
           <users count=\"{}\">\n{}\
           </users>\n\
         </conference-info>\n",
        safe_room,
        safe_room,
        participants.len(),
        users_xml,
    )
}

/// Extract the room number from a SIP Request-URI user part.
///
/// Supports both `sip:` and `tel:` URI schemes:
/// - `sip:1234@host` → `"1234"`
/// - `tel:+18005551234` → `"+18005551234"`
/// - `tel:1234` → `"1234"`
pub fn extract_room_number(request: &sip_core::Request) -> Option<String> {
    let uri = request.uri();
    let uri_str = uri.to_string();

    let candidate = if let Some(rest) = uri_str.strip_prefix("sip:") {
        // SIP URI: extract user part from sip:user@host
        rest.split('@')
            .next()
            .filter(|u| !u.is_empty())
            .map(String::from)
    } else if let Some(rest) = uri_str.strip_prefix("tel:") {
        // tel URI: extract phone number from tel:+number or tel:number
        // Strip parameters (e.g. tel:+1234;phone-context=example.com)
        let number = rest.split(';').next().unwrap_or(rest).trim();
        if !number.is_empty() {
            Some(number.to_string())
        } else {
            None
        }
    } else {
        None
    };

    candidate.filter(|r| validate_room_number(r))
}

/// Validate a room number string for safe use.
///
/// Rules:
/// - Max 64 characters
/// - Only alphanumeric, `-`, `_`, `.`, `+`, `*`, `#`
/// - No control characters, no whitespace, no path separators
pub fn validate_room_number(room: &str) -> bool {
    !room.is_empty()
        && room.len() <= 64
        && room
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+*#".contains(c))
}

/// Extract source IP from the top Via header's sent-by address.
///
/// Used for rate-limiting methods that don't receive the peer SocketAddr directly.
fn extract_via_ip(request: &sip_core::Request) -> Option<std::net::IpAddr> {
    sip_parse::header(request.headers(), "Via").and_then(|v| {
        // Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK...
        let after_proto = v.as_str().split_whitespace().nth(1)?;
        let host_port = after_proto.split(';').next()?;
        // Try parsing as ip:port first, then just ip
        if let Ok(addr) = host_port.parse::<std::net::SocketAddr>() {
            Some(addr.ip())
        } else {
            host_port.split(':').next()?.parse().ok()
        }
    })
}

/// Extract Call-ID from a SIP request.
fn extract_call_id(request: &sip_core::Request) -> String {
    request
        .headers()
        .get("Call-ID")
        .map(|v| v.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Extract display name and caller ID from SIP request headers.
///
/// Checks P-Asserted-Identity first (trusted network identity), then falls back
/// to the From header. Returns (display_name, caller_id).
fn extract_caller_info(request: &sip_core::Request) -> (Option<String>, Option<String>) {
    // Try P-Asserted-Identity first (more trusted on SBC/trunk calls)
    if let Ok(Some(pai)) = sip_core::parse_p_asserted_identity(request.headers()) {
        if let Some(identity) = pai.identities().next() {
            let display = identity.display_name().map(|s| s.to_string());
            let caller = identity
                .uri()
                .as_sip()
                .and_then(|u| u.user().map(|s| s.to_string()));
            if display.is_some() || caller.is_some() {
                return (display, caller);
            }
        }
    }

    // Fall back to From header
    if let Some(from_val) = request.headers().get_smol("From") {
        if let Some(from) = sip_parse::parse_from_header(from_val) {
            let name_addr = from.inner();
            let display = name_addr.display_name().map(|s| s.to_string());
            let caller = name_addr
                .sip_uri()
                .and_then(|u| u.user().map(|s| s.to_string()));
            return (display, caller);
        }
    }

    (None, None)
}

/// Parse the remote audio RTP endpoint from an SDP offer: the first active
/// audio section's port with its own `c=` line, or the session-level one.
/// A video section's `c=` never leaks into the audio endpoint.
fn parse_remote_rtp_addr(sdp: &str) -> Option<SocketAddr> {
    fcp_media::remote_audio_endpoint(sdp)
}

/// Pick the first supported codec from an SDP offer.
///
/// If `allowed` is `Some`, only codecs in that list are considered.
/// Returns (codec_name, payload_type, frame_size_in_samples).
fn pick_codec(
    codecs: &[fcp_media::SdpCodecInfo],
    allowed: Option<&[String]>,
) -> Option<(String, u8, usize)> {
    for codec in codecs {
        let name_upper = codec.encoding_name.to_uppercase();
        // Check against allowed list if configured
        if let Some(allowed_list) = allowed {
            if !allowed_list
                .iter()
                .any(|a| a.eq_ignore_ascii_case(&name_upper))
            {
                continue;
            }
        }
        match name_upper.as_str() {
            "PCMU" => return Some(("PCMU".to_string(), codec.payload_type, 160)),
            "PCMA" => return Some(("PCMA".to_string(), codec.payload_type, 160)),
            "G722" => return Some(("G722".to_string(), codec.payload_type, 320)),
            _ => continue,
        }
    }
    None
}

/// Extract the telephone-event clock rate from SDP codecs.
///
/// Returns the clock rate from the telephone-event codec entry (typically 8000),
/// falling back to 8000 if not found. This ensures the DTMF detector uses the
/// correct sample rate rather than assuming 8000 Hz.
fn find_telephone_event_clock_rate(codecs: &[fcp_media::SdpCodecInfo], sdp: &str) -> u32 {
    for codec in codecs {
        if codec.encoding_name.eq_ignore_ascii_case("telephone-event") {
            return codec.clock_rate;
        }
    }
    // Fallback: parse SDP lines manually for clock rate
    for line in sdp.lines() {
        let line = line.trim();
        // a=rtpmap:101 telephone-event/8000
        if line.starts_with("a=rtpmap:") && line.contains("telephone-event") {
            if let Some(slash_pos) = line.rfind('/') {
                if let Ok(rate) = line[slash_pos + 1..].parse::<u32>() {
                    return rate;
                }
            }
        }
    }
    8000 // default
}

/// Find telephone-event payload type from the SDP.
///
/// Looks for `a=rtpmap:<pt> telephone-event/8000` in the SDP text, or
/// checks parsed codecs for a "telephone-event" encoding name.
fn find_telephone_event_pt(codecs: &[fcp_media::SdpCodecInfo], sdp: &str) -> Option<u8> {
    // Check parsed codecs first
    for codec in codecs {
        if codec.encoding_name.eq_ignore_ascii_case("telephone-event") {
            return Some(codec.payload_type);
        }
    }

    // Fallback: parse SDP lines manually
    for line in sdp.lines() {
        let line = line.trim();
        if line.starts_with("a=rtpmap:") && line.contains("telephone-event") {
            // a=rtpmap:101 telephone-event/8000
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(pt_str) = parts.first() {
                if let Some(pt_num) = pt_str.strip_prefix("a=rtpmap:") {
                    if let Ok(pt) = pt_num.parse::<u8>() {
                        return Some(pt);
                    }
                }
            }
        }
    }

    None
}

/// Convert a DtmfDigit to its character representation for DTMF command processing.
///
/// Returns `None` for Hash because `#` is used as a command terminator in conference
/// DTMF menus (e.g. `*1#` = mute toggle). This intentionally differs from
/// `audio_bridge::dtmf_digit_to_char` which returns `Some('#')` for general
/// audio bridge event reporting.
fn dtmf_digit_to_char(digit: &DtmfDigit) -> Option<char> {
    match digit {
        DtmfDigit::Zero => Some('0'),
        DtmfDigit::One => Some('1'),
        DtmfDigit::Two => Some('2'),
        DtmfDigit::Three => Some('3'),
        DtmfDigit::Four => Some('4'),
        DtmfDigit::Five => Some('5'),
        DtmfDigit::Six => Some('6'),
        DtmfDigit::Seven => Some('7'),
        DtmfDigit::Eight => Some('8'),
        DtmfDigit::Nine => Some('9'),
        DtmfDigit::Star => Some('*'),
        DtmfDigit::Hash => None, // '#' used as terminator — see doc comment above
        DtmfDigit::A => Some('A'),
        DtmfDigit::B => Some('B'),
        DtmfDigit::C => Some('C'),
        DtmfDigit::D => Some('D'),
    }
}

/// Create a codec instance by name for audio encoding/decoding.
pub fn create_codec_instance(codec_name: &str) -> Result<Box<dyn AudioCodec>> {
    match codec_name {
        "PCMU" => Ok(Box::new(g711::G711MuLaw::new(8000))),
        "PCMA" => Ok(Box::new(g711::G711ALaw::new(8000))),
        "G722" => Ok(Box::new(g722::G722Codec::new(g722::G722BitRate::Rate64k))),
        other => anyhow::bail!("Unsupported codec: {}", other),
    }
}

impl FloorNotifier for ConferenceCallHandler {
    fn floor_changed(&self, room_number: &str, mut notice: FloorNotice) {
        // A BFCP endpoint hears the room taking the floor from it as a
        // `Revoked` (§15.6, 7e); the rest is the gateway sessions'.
        if notice.floor == "released" {
            self.bfcp.floor_released(
                room_number,
                &notice.participant_id,
                notice.reason.as_deref().unwrap_or("ended"),
            );
        }
        let Some(me) = self.me() else {
            return;
        };
        // A peer's screen holds the floor under its trunk's tile; the
        // browsers here are told the presenter's name from the peer's
        // claim, when it has arrived (§15.6, 7d).
        if let (Some(node), Some(_)) = (
            trunks::trunk_peer_node(&notice.participant_id),
            notice.presenter.as_ref(),
        ) {
            if let Some(claim) = self.content_claims.get(&format!("{room_number}|{node}")) {
                notice.presenter = Some(claim.value().0.display_name.clone());
            }
        }
        // A refusal is the refused participant's business alone; the
        // floor changing hands is everyone's.
        let only_theirs = notice.floor == "refused";
        let targets: Vec<(String, bool)> = self
            .room_sessions
            .get(room_number)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.sessions.get(id))
                    .filter(|s| s.joined_by_token)
                    .map(|s| (s.call_id.clone(), s.participant_id == notice.participant_id))
                    .filter(|(_, mine)| *mine || !only_theirs)
                    .collect()
            })
            .unwrap_or_default();
        for (call_id, mine) in targets {
            let me = Arc::clone(&me);
            let notice = notice.clone();
            tokio::spawn(async move { me.send_floor_info(call_id, notice, mine).await });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_room_number() {
        let uri = "sip:8001@192.168.1.100";
        let user = uri
            .strip_prefix("sip:")
            .and_then(|rest| rest.split('@').next())
            .filter(|u| !u.is_empty());
        assert_eq!(user, Some("8001"));
    }

    #[test]
    fn test_extract_room_number_tel_uri() {
        // Verify tel: URI parsing logic directly
        let tel = "tel:+18005551234";
        let number = tel
            .strip_prefix("tel:")
            .and_then(|rest| rest.split(';').next())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        assert_eq!(number, Some("+18005551234"));

        // tel: with phone-context parameter
        let tel2 = "tel:1234;phone-context=example.com";
        let number2 = tel2
            .strip_prefix("tel:")
            .and_then(|rest| rest.split(';').next())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        assert_eq!(number2, Some("1234"));

        // Plain tel: local number
        let tel3 = "tel:8001";
        let number3 = tel3
            .strip_prefix("tel:")
            .and_then(|rest| rest.split(';').next())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        assert_eq!(number3, Some("8001"));
    }

    #[test]
    fn test_validate_room_number() {
        // Valid room numbers
        assert!(validate_room_number("8001"));
        assert!(validate_room_number("room-1"));
        assert!(validate_room_number("conf_room.2"));
        assert!(validate_room_number("+18005551234"));
        assert!(validate_room_number("*67"));
        assert!(validate_room_number("#100"));
        assert!(validate_room_number("a")); // min length 1

        // Invalid room numbers
        assert!(!validate_room_number("")); // empty
        assert!(!validate_room_number("room 1")); // space
        assert!(!validate_room_number("room/1")); // path separator
        assert!(!validate_room_number("room\\1")); // backslash
        assert!(!validate_room_number("room\x001")); // control char
        assert!(!validate_room_number("room;drop")); // semicolon
        assert!(!validate_room_number(&"x".repeat(65))); // too long
        assert!(validate_room_number(&"x".repeat(64))); // exactly max
    }

    #[test]
    fn test_parse_remote_rtp_addr() {
        let sdp = "v=0\r\no=- 1234 1234 IN IP4 192.168.1.50\r\nc=IN IP4 192.168.1.50\r\nm=audio 10000 RTP/AVP 0 8\r\n";
        let addr = parse_remote_rtp_addr(sdp).unwrap();
        assert_eq!(addr, "192.168.1.50:10000".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn test_parse_remote_rtp_addr_ignores_video_connection() {
        let sdp = "v=0\r\no=- 1 1 IN IP4 192.168.1.50\r\nc=IN IP4 192.168.1.50\r\n\
                   m=audio 10000 RTP/AVP 0 8\r\nm=video 10002 RTP/AVP 96\r\nc=IN IP4 10.9.9.9\r\n";
        let addr = parse_remote_rtp_addr(sdp).unwrap();
        assert_eq!(addr, "192.168.1.50:10000".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn test_parse_remote_rtp_addr_missing() {
        let sdp = "v=0\r\no=- 1234 1234 IN IP4 0.0.0.0\r\n";
        assert!(parse_remote_rtp_addr(sdp).is_none());
    }

    #[test]
    fn test_pick_codec_pcmu() {
        let codecs = vec![
            fcp_media::SdpCodecInfo {
                payload_type: 0,
                encoding_name: "PCMU".to_string(),
                clock_rate: 8000,
            },
            fcp_media::SdpCodecInfo {
                payload_type: 8,
                encoding_name: "PCMA".to_string(),
                clock_rate: 8000,
            },
        ];
        let (name, pt, frame_size) = pick_codec(&codecs, None).unwrap();
        assert_eq!(name, "PCMU");
        assert_eq!(pt, 0);
        assert_eq!(frame_size, 160);
    }

    #[test]
    fn test_pick_codec_g722() {
        let codecs = vec![fcp_media::SdpCodecInfo {
            payload_type: 9,
            encoding_name: "G722".to_string(),
            clock_rate: 8000,
        }];
        let (name, pt, frame_size) = pick_codec(&codecs, None).unwrap();
        assert_eq!(name, "G722");
        assert_eq!(pt, 9);
        assert_eq!(frame_size, 320);
    }

    #[test]
    fn test_pick_codec_none() {
        let codecs = vec![fcp_media::SdpCodecInfo {
            payload_type: 96,
            encoding_name: "opus".to_string(),
            clock_rate: 48000,
        }];
        assert!(pick_codec(&codecs, None).is_none());
    }

    #[test]
    fn test_pick_codec_allowed_filter() {
        let codecs = vec![
            fcp_media::SdpCodecInfo {
                payload_type: 0,
                encoding_name: "PCMU".to_string(),
                clock_rate: 8000,
            },
            fcp_media::SdpCodecInfo {
                payload_type: 8,
                encoding_name: "PCMA".to_string(),
                clock_rate: 8000,
            },
            fcp_media::SdpCodecInfo {
                payload_type: 9,
                encoding_name: "G722".to_string(),
                clock_rate: 8000,
            },
        ];
        // Only allow PCMA
        let allowed = vec!["PCMA".to_string()];
        let (name, pt, _) = pick_codec(&codecs, Some(&allowed)).unwrap();
        assert_eq!(name, "PCMA");
        assert_eq!(pt, 8);

        // Allow none of the offered codecs
        let allowed = vec!["G729".to_string()];
        assert!(pick_codec(&codecs, Some(&allowed)).is_none());
    }

    #[test]
    fn test_find_telephone_event_pt() {
        let sdp = "v=0\r\nm=audio 10000 RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\n";
        let codecs = vec![fcp_media::SdpCodecInfo {
            payload_type: 0,
            encoding_name: "PCMU".to_string(),
            clock_rate: 8000,
        }];
        assert_eq!(find_telephone_event_pt(&codecs, sdp), Some(101));
    }

    #[test]
    fn test_dtmf_digit_to_char() {
        assert_eq!(dtmf_digit_to_char(&DtmfDigit::Zero), Some('0'));
        assert_eq!(dtmf_digit_to_char(&DtmfDigit::Nine), Some('9'));
        assert_eq!(dtmf_digit_to_char(&DtmfDigit::Star), Some('*'));
        assert_eq!(dtmf_digit_to_char(&DtmfDigit::Hash), None);
    }
}
