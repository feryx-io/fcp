use anyhow::{bail, Context, Result};
use dashmap::DashMap;
use fcp_core::storage::TenantStore;
use fcp_core::{DynStorage, Meeting, TenantId};
use fcp_events::{ConferenceEvent, EventPublisher, PlatformEvent};
use fcp_media::{
    construct_answer_for_a_leg, decode_sdes_key_params, generate_srtp_key_material,
    parse_sdes_crypto, parse_sdp_codecs, SrtpSdpContext,
};
use fcp_sip::prelude::SipUserAgentClient;
use forge_codecs::{g711, g722, AudioCodec};
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
use sip_core::{SipUri, Uri};
use sip_dialog::Dialog;
use sip_uas::UserAgentServer;
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::audio_bridge::{self, AudioStats};
use crate::config::{AiSection, ConferenceSection};

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
}

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
    /// Secondary index: room_number → Vec<call_id> for O(1) room-to-session lookups.
    room_sessions: DashMap<String, Vec<String>>,
    /// Latency telemetry for trait methods (INVITE, re-INVITE, BYE, etc.)
    pub handler_latency: HandlerLatency,
    /// Optional TTS service for synthesizing prompts from text
    tts_service: Option<Arc<fcp_media::TtsService>>,
    /// Calls that have been answered and are waiting for PIN input.
    pending_pin_auth: DashMap<String, PendingPinAuth>,
    /// Pre-loaded DTMF feedback audio: "Muted" / "Unmuted" PCM samples
    pub dtmf_muted_samples: Option<Arc<Vec<i16>>>,
    pub dtmf_unmuted_samples: Option<Arc<Vec<i16>>>,
    pub dtmf_locked_samples: Option<Arc<Vec<i16>>>,
    pub dtmf_unlocked_samples: Option<Arc<Vec<i16>>>,
    pub dtmf_ending_samples: Option<Arc<Vec<i16>>>,
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
            room_sessions: DashMap::new(),
            handler_latency: HandlerLatency::new(),
            tts_service: None,
            pending_pin_auth: DashMap::new(),
            dtmf_muted_samples: None,
            dtmf_unmuted_samples: None,
            dtmf_locked_samples: None,
            dtmf_unlocked_samples: None,
            dtmf_ending_samples: None,
        }
    }

    /// Set the TTS service for synthesizing prompts from meeting text fields.
    pub fn set_tts_service(&mut self, tts: Arc<fcp_media::TtsService>) {
        self.tts_service = Some(tts);
    }

    /// Signal that this handler should reject new INVITEs (graceful shutdown draining).
    pub fn start_draining(&self) {
        self.draining.store(true, Ordering::Relaxed);
        info!("Conference handler entering drain mode — rejecting new INVITEs");
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

        let cache_key = (tenant_id.clone(), room_number.to_string());
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
            .remove(&(tenant_id.clone(), room_number.to_string()));
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
        }

        Ok(json)
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
        }))
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

        self.remove_session(&call_id);
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

        let room = self
            .bridge
            .get_or_create_room(room_number, None)
            .context("Failed to create conference room")?;

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
        if status_code < 200 || status_code >= 300 {
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
        self.publish_event(ConferenceEvent::ParticipantJoined {
            room_number: room_number.to_string(),
            participant_id: participant_id.clone(),
            is_host: false,
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
            self.remove_session(call_id);
        }

        // Stop recording if active
        if let Ok(room) = self.bridge.get_room(room_number) {
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

        // Send BYE and remove each session
        for call_id in &call_ids {
            if let Some(session) = self.sessions.get(call_id) {
                if let Err(e) = self.uac.bye(&session.dialog).await {
                    warn!(call_id = %call_id, error = %e, "Failed to send BYE during shutdown");
                }
                drop(session); // Release DashMap ref before remove_session
            }
            self.remove_session(call_id);
        }

        // Stop recordings and delete all active rooms
        for room_id in self.bridge.list_rooms() {
            if let Ok(room) = self.bridge.get_room(&room_id) {
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
            .filter(|entry| entry.value().last_refreshed.elapsed() > max_age)
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
            self.remove_session(call_id);
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
                .map_or(false, |elapsed| elapsed > Duration::from_secs(10));
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
            | ConferenceEvent::DialOutFailed { room_number, .. } => Some(room_number.clone()),
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
    fn remove_session(&self, call_id: &str) {
        if let Some((_, session)) = self.sessions.remove(call_id) {
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
                "Participant left conference"
            );

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

            // Publish leave event
            self.publish_event(ConferenceEvent::ParticipantLeft {
                room_number: session.room_number.clone(),
                participant_id: session.participant_id.clone(),
                tenant_id: session.tenant_id,
                timestamp: SystemTime::now(),
            });

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

#[async_trait::async_trait]
impl fcp_sip::conference::ConferenceHandler for ConferenceCallHandler {
    async fn can_handle(&self, request: &sip_core::Request) -> bool {
        let room_number = match extract_room_number(request) {
            Some(rn) => rn,
            None => return false,
        };

        let tenant_id = match self.resolve_tenant(request).await {
            Ok(tenant_id) => tenant_id,
            Err(_) => return false,
        };
        match self.lookup_meeting(&tenant_id, &room_number).await {
            Ok(Some(meeting)) => meeting.enabled,
            _ => false,
        }
    }

    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        peer: SocketAddr,
    ) -> Result<()> {
        let _invite_start = Instant::now();

        // 0. Reject new calls during shutdown draining
        if self.draining.load(Ordering::Relaxed) {
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

        // 1. Extract room number
        let room_number = extract_room_number(request)
            .context("Failed to extract room number from Request-URI")?;

        // 2. Extract tenant and look up meeting
        let tenant_id = self.resolve_tenant(request).await?;
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

        // 6. Allocate RTP port pair
        let port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port pair")?;

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

        // 7b. Construct SDP answer
        let sdp_answer = construct_answer_for_a_leg(
            sdp_offer,
            &self.local_addr,
            port_pair.rtp_port,
            srtp_sdp.as_ref(),
        );

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
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
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

        // 12. Generate participant ID and extract caller info
        let call_id = extract_call_id(request);
        let (display_name, caller_id) = extract_caller_info(request);
        let short_uuid = uuid::Uuid::new_v4().to_string();
        let participant_id = format!("{}-{}", &call_id[..call_id.len().min(8)], &short_uuid[..8]);

        // 13. Get or create conference room, configure with meeting settings
        let room = self
            .bridge
            .get_or_create_room(&room_number, None)
            .context("Failed to create conference room")?;

        // Wire room configuration from meeting → forge-conference
        let room_config = self.meeting_to_room_config(&meeting);
        let global_config = ConferenceConfig::default();
        if let Err(e) = room.configure(room_config, &global_config) {
            warn!(room = %room_number, error = %e, "Failed to configure room (may already be configured)");
        }

        // 14. PIN authentication (if required)
        let is_host = if Self::meeting_requires_pin(&meeting) {
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
                                let pool = Arc::clone(&self.port_pool);
                                tokio::spawn(async move { pool.deallocate(port_pair).await });
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
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
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
        let should_wait = if meeting.wait_for_moderator && !is_host && room.host_count() == 0 {
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
            match room.add_participant_quiet(&participant_id, is_host) {
                Ok(()) => false,
                Err(ConferenceError::WaitingForModerator) => true,
                Err(ConferenceError::ConferenceFull(max)) => {
                    warn!(room = %room_number, max = max, "Conference is full, rejecting participant");
                    if let Err(e) = self.uac.bye(&dialog).await {
                        warn!(error = %e, "Failed to send BYE after conference full");
                    }
                    let pool = Arc::clone(&self.port_pool);
                    tokio::spawn(async move { pool.deallocate(port_pair).await });
                    return Ok(());
                }
                Err(ConferenceError::ConferenceLocked) => {
                    warn!(room = %room_number, "Conference is locked, rejecting participant");
                    if let Err(e) = self.uac.bye(&dialog).await {
                        warn!(error = %e, "Failed to send BYE after conference locked");
                    }
                    let pool = Arc::clone(&self.port_pool);
                    tokio::spawn(async move { pool.deallocate(port_pair).await });
                    return Ok(());
                }
                Err(e) => {
                    warn!(room = %room_number, error = %e, "Failed to add participant");
                    if let Err(bye_err) = self.uac.bye(&dialog).await {
                        warn!(error = %bye_err, "Failed to send BYE after add_participant error");
                    }
                    let pool = Arc::clone(&self.port_pool);
                    tokio::spawn(async move { pool.deallocate(port_pair).await });
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
            let recordings_dir = conf.recordings_dir.clone();
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
            let wait_dialog = dialog.clone();

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
                                    is_host: false,
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
                                        let ts = SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs();
                                        let rec_path = format!(
                                            "{}/{}-{}-{}.wav",
                                            recordings_dir,
                                            rn,
                                            ts,
                                            uuid::Uuid::new_v4().as_simple()
                                        );
                                        match bridge_room.start_recording(&rec_path, None).await {
                                            Ok(()) => {
                                                tracing::info!(
                                                    room = %rn,
                                                    path = %rec_path,
                                                    "Auto-recording started (after moderator join)"
                                                );
                                                let rec_event = PlatformEvent::Conference(
                                                    ConferenceEvent::RecordingStarted {
                                                        room_number: rn.clone(),
                                                        tenant_id,
                                                        timestamp: SystemTime::now(),
                                                    },
                                                );
                                                let _ = event_bus.publish(rec_event).await;
                                            }
                                            Err(e) => {
                                                tracing::warn!(
                                                    room = %rn,
                                                    error = %e,
                                                    "Auto-record failed (after moderator join)"
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            // Start audio bridge
                            audio_bridge::run_audio_bridge(
                                bridge_room,
                                pid,
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
                            )
                            .await;
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
            self.sessions.insert(
                call_id.clone(),
                ParticipantSession {
                    call_id,
                    room_number,
                    participant_id,
                    is_host,
                    is_dial_out: false,
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

        // Publish join event
        self.publish_event(ConferenceEvent::ParticipantJoined {
            room_number: room_number.clone(),
            participant_id: participant_id.clone(),
            is_host,
            tenant_id,
            timestamp: SystemTime::now(),
        });

        // Auto-record: start recording when participant threshold is reached
        if !room.is_recording() {
            if let Some(cfg) = room.get_effective_config() {
                if cfg.min_recording_participants > 0
                    && room.participant_count() >= cfg.min_recording_participants
                {
                    let timestamp = SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let path = format!(
                        "{}/{}-{}-{}.wav",
                        conf.recordings_dir,
                        room_number,
                        timestamp,
                        uuid::Uuid::new_v4().as_simple()
                    );
                    match room.start_recording(&path, None).await {
                        Ok(()) => {
                            info!(room = %room_number, path = %path, "Auto-recording started");
                            self.publish_event(ConferenceEvent::RecordingStarted {
                                room_number: room_number.clone(),
                                tenant_id,
                                timestamp: SystemTime::now(),
                            });
                        }
                        Err(e) => {
                            warn!(room = %room_number, error = %e, "Auto-record failed");
                        }
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
        ));

        // 19. Store session (room_sessions already updated above for MeetingStarted dedup)
        let remote_addr = parse_remote_rtp_addr(sdp_offer).unwrap_or(UNKNOWN_REMOTE);
        self.sessions.insert(
            call_id.clone(),
            ParticipantSession {
                call_id,
                room_number,
                participant_id,
                is_host,
                is_dial_out: false,
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
            },
        );

        self.handler_latency.record(
            &self.handler_latency.invite_count,
            &self.handler_latency.invite_total_us,
            _invite_start.elapsed(),
        );
        Ok(())
    }

    fn handle_reinvite(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        let reinvite_start = Instant::now();
        let call_id = extract_call_id(request);
        let conf = self.conference_config.read().clone();

        // Look up session — if not found, return Err so caller falls through to B2BUA
        let mut session = self
            .sessions
            .get_mut(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No conference session for call_id: {}", call_id))?;

        // Rate-limit re-INVITEs using the participant's known remote IP
        if !self.check_invite_rate(session.remote_addr.ip(), conf.max_invite_rate_per_ip) {
            warn!(call_id = %call_id, peer = %session.remote_addr.ip(), "re-INVITE rate limit exceeded");
            return Ok(UserAgentServer::create_response(request, 486, "Busy Here"));
        }

        // Parse new SDP body
        let body = request.body();
        let sdp_offer =
            std::str::from_utf8(&body[..]).context("re-INVITE SDP body is not valid UTF-8")?;

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

        // Refresh session timer (re-INVITE counts as a session refresh per RFC 4028)
        session.last_refreshed = Instant::now();

        // Construct SDP answer with current local RTP port
        let sdp_answer = construct_answer_for_a_leg(
            sdp_offer,
            &self.local_addr,
            session.port_pair.rtp_port,
            srtp_sdp.as_ref(),
        );

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
            self.remove_session(call_id);
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
            self.remove_session(call_id);
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
            let sdp_answer = construct_answer_for_a_leg(
                sdp_offer,
                &self.local_addr,
                session.port_pair.rtp_port,
                None,
            );
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
                let body_str = String::from_utf8_lossy(&body);
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
            let body_str = String::from_utf8_lossy(&body);
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

/// Parse remote RTP endpoint from an SDP offer.
///
/// Extracts the IP from the `c=IN IP4 <addr>` line and the port from
/// the `m=audio <port>` line, combining them into a SocketAddr.
fn parse_remote_rtp_addr(sdp: &str) -> Option<SocketAddr> {
    let mut ip: Option<&str> = None;
    let mut port: Option<u16> = None;

    for line in sdp.lines() {
        let line = line.trim();
        if line.starts_with("c=IN IP4 ") || line.starts_with("c=IN ip4 ") {
            // c=IN IP4 192.168.1.100
            ip = line.split_whitespace().nth(2);
        } else if line.starts_with("c=IN IP6 ") || line.starts_with("c=IN ip6 ") {
            ip = line.split_whitespace().nth(2);
        } else if line.starts_with("m=audio ") {
            // m=audio 10000 RTP/AVP 0 8 9
            if let Some(port_str) = line.split_whitespace().nth(1) {
                port = port_str.parse().ok();
            }
        }
    }

    match (ip, port) {
        (Some(ip_str), Some(p)) => format!("{}:{}", ip_str, p).parse().ok(),
        _ => None,
    }
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
