//! Concrete QueueCallHandler implementing the QueueHandler trait.
//!
//! Wires together QueueManager (distribution logic), RTP (media),
//! and SIP signaling (UAC/UAS) to handle queue-bound INVITE calls.

use crate::queue_audio::{AnnouncementCache, MohCache, PromptCache};
use crate::queue_recording::{Leg, NoticeTone, NoticeWriter, QueueRecordings};
use anyhow::{bail, Context, Result};
use dashmap::DashMap;
use fcp_core::storage::CdrKind;
use fcp_core::{TenantId, TenantStore};
use fcp_events::EventPublisher;
use fcp_media::{construct_answer_for_a_leg, extract_telephone_event_pt, parse_sdp_codecs};
use fcp_queue::{QueueManager, QueueSystemConfig, QueuedCall};
use fcp_sip::prelude::SipUserAgentClient;
use forge_dtmf::Rfc2833Detector;
use forge_rtp::{PortPair, PortPool, RemoteEndpoint, RtpSocketConfig, RtpSocketPair};
use sip_core::Uri;
use sip_uas::UserAgentServer;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// How long a queue call's recorder is given to write what it still
/// holds and close its file once the call has ended.
const RECORDING_FINISH: Duration = Duration::from_secs(5);

/// Supervisor monitoring/coaching mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupervisorMode {
    /// Listen only — supervisor hears both caller and agent but neither hears supervisor.
    Monitor,
    /// Coach — supervisor can speak to the agent only; caller cannot hear supervisor.
    Whisper,
    /// Full three-way — all three parties can hear each other.
    Barge,
}

impl std::fmt::Display for SupervisorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupervisorMode::Monitor => write!(f, "monitor"),
            SupervisorMode::Whisper => write!(f, "whisper"),
            SupervisorMode::Barge => write!(f, "barge"),
        }
    }
}

/// Per-call state for a caller waiting in a queue.
#[allow(dead_code)]
struct QueueSession {
    /// SIP Call-ID.
    call_id: String,
    /// Name of the queue this call is waiting in.
    queue_name: String,
    /// Tenant context.
    tenant_id: TenantId,
    /// Caller's SIP URI (From header).
    caller_uri: String,
    /// Caller's display name.
    caller_display: Option<String>,

    // --- Caller A-leg ---
    /// Allocated RTP/RTCP port pair for the caller leg.
    port_pair: PortPair,
    /// RTP socket pair for the caller leg.
    rtp_socket: Arc<RtpSocketPair>,
    /// Negotiated codec name (e.g., "PCMU").
    codec_name: String,
    /// RTP payload type for the negotiated codec.
    payload_type: u8,
    /// Frame size in samples for the negotiated codec.
    _frame_size: usize,
    /// SIP dialog for the caller leg.
    dialog: sip_dialog::Dialog,

    // --- MOH task (aborted when agent connects or caller leaves) ---
    moh_handle: Option<JoinHandle<()>>,
    /// Pre-encoded MOH frames for this session's codec+class (if available).
    moh_frames: Option<Arc<Vec<Vec<u8>>>>,

    // --- Agent B-leg (populated when agent answers) ---
    agent_port_pair: Option<PortPair>,
    agent_rtp_socket: Option<Arc<RtpSocketPair>>,
    agent_dialog: Option<sip_dialog::Dialog>,
    bridge_a_to_b: Option<JoinHandle<()>>,
    bridge_b_to_a: Option<JoinHandle<()>>,

    // --- DTMF listener (for exit options / callback) ---
    dtmf_listener_handle: Option<JoinHandle<()>>,

    // --- Recording (if enabled) ---
    recording_handle: Option<JoinHandle<()>>,
    recording_path: Option<String>,
    /// What the parties were told, when the queue records the call.
    recording_consent: Option<fcp_core::RecordingConsent>,

    // --- Supervisor (monitor/whisper/barge) ---
    supervisor_port_pair: Option<PortPair>,
    supervisor_rtp_socket: Option<Arc<RtpSocketPair>>,
    supervisor_dialog: Option<sip_dialog::Dialog>,
    supervisor_mode: Option<SupervisorMode>,
    supervisor_id: Option<String>,
    supervisor_tasks: Vec<JoinHandle<()>>,

    // --- State tracking ---
    state: QueueCallState,
    /// Whether this session is a campaign-originated call (both legs outbound).
    is_campaign_call: bool,
    /// Agent ID currently being rung (if any).
    ringing_agent: Option<String>,
    /// Handle for cancelling an in-progress agent ring task.
    ring_task_handle: Option<JoinHandle<()>>,
    /// Sender to cancel an in-progress agent INVITE (sends SIP CANCEL).
    ring_cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// When this call was enqueued.
    enqueued_at: Instant,
    /// When an agent answered this call (for CDR duration calculation).
    agent_answered_at: Option<Instant>,
    /// Agent's negotiated codec name (set when agent answers with SDP).
    agent_codec_name: Option<String>,
    /// The B2BUA call the caller's INVITE created; ended with the session,
    /// and the queue record's `call_id`. `None` for calls the queue placed
    /// itself (callbacks, campaigns).
    b2bua_call_id: Option<fcp_sip::CallId>,
}

/// The call id a queue segment's record and recording carry.
fn queue_record_call_id(b2bua_call_id: Option<fcp_sip::CallId>) -> String {
    b2bua_call_id
        .map(|id| fcp_core::CallId::from_uuid(*id.as_uuid()))
        .unwrap_or_default()
        .to_string()
}

/// State machine for a queue call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueCallState {
    /// Caller is waiting in the queue, hearing MOH.
    WaitingInQueue,
    /// An agent is being rung for this caller.
    RingingAgent,
    /// Agent has answered, RTP bridge is active.
    Connected,
    /// Call has been routed to the overflow destination (B2BUA bridged).
    OverflowedToDestination,
}

/// Concrete handler for ACD/Queue calls.
///
/// Implements `fcp_sip::queue::QueueHandler` and is plugged into
/// `SipMessageHandler` to intercept INVITEs destined for queue extensions.
/// Configuration for position announcements during MOH.
#[derive(Clone)]
struct AnnounceConfig {
    cache: Arc<AnnouncementCache>,
    interval_secs: u32,
    queue_manager: Arc<QueueManager>,
    /// Whether to announce estimated wait time after position.
    announce_eta: bool,
    /// Language code for announcements (e.g., "en", "es").
    language: String,
}

pub struct QueueCallHandler {
    queue_manager: Arc<QueueManager>,
    config: QueueSystemConfig,
    default_tenant_id: TenantId,
    sessions: Arc<DashMap<String, QueueSession>>,
    local_uri: String,
    local_addr: String,
    port_pool: Arc<PortPool>,
    event_bus: Arc<dyn EventPublisher>,
    uac: Arc<SipUserAgentClient>,
    announcement_cache: Option<Arc<AnnouncementCache>>,
    moh_cache: Option<Arc<MohCache>>,
    prompt_cache: Option<Arc<PromptCache>>,
    storage: Arc<fcp_core::DynStorage>,
    /// Where a finished queue segment's record goes (`docs/CDR_PIPELINE.md`).
    cdr: fcp_sip::CdrWriter,
    /// This node, on every record.
    node_id: String,

    /// The B2BUA's calls, ended with their queue sessions
    /// (`docs/CDR_PIPELINE.md` §5).
    call_manager: Arc<fcp_sip::CallManager>,
    /// Maps callee Call-ID ↔ agent Call-ID for campaign calls (reverse BYE lookup).
    campaign_call_id_map: Arc<DashMap<String, String>>,
    /// Speech for a recording announcement when the tenant's prompt has no
    /// audio.
    tts: Option<Arc<fcp_media::TtsService>>,
}

impl QueueCallHandler {
    /// The user an agent's extension names — their username, else the
    /// extension on their record — if they may work.
    async fn user_of_extension(
        &self,
        extension: &str,
        tenant_id: &TenantId,
    ) -> Option<fcp_core::User> {
        use fcp_core::CredentialStore;
        let user = match self
            .storage
            .get_user_by_sip_name(extension, tenant_id)
            .await
        {
            Ok(Some(user)) => Some(user),
            _ => self
                .storage
                .list_users_in_tenant(tenant_id, &Default::default())
                .await
                .ok()
                .and_then(|users| {
                    users
                        .into_iter()
                        .find(|u| u.extension.as_deref() == Some(extension))
                }),
        };
        user.filter(|u| u.may_use_services())
    }

    /// The groups (by id) of `user`.
    async fn groups_of_user(&self, user: &fcp_core::User) -> Vec<String> {
        let Some(groups) = self.storage.group_store() else {
            return Vec::new();
        };
        match groups.groups_of_user(&user.id).await {
            Ok(groups) => groups.into_iter().map(|g| g.id.to_string()).collect(),
            Err(e) => {
                warn!(user = %user.username, error = %e, "Could not read an agent's groups");
                Vec::new()
            }
        }
    }

    /// The node's hold music (`path`) for a queue whose class has no
    /// audio, so a waiting caller is never left on dead air.
    pub fn with_default_moh(mut self, path: Option<&std::path::Path>) -> Self {
        if let Some(path) = path {
            let cache = self
                .moh_cache
                .take()
                .map(|c| Arc::try_unwrap(c).unwrap_or_default())
                .unwrap_or_default()
                .with_default(path);
            self.moh_cache = (!cache.is_empty()).then(|| Arc::new(cache));
        }
        self
    }

    /// Speech for recording announcements when a tenant's prompt has no
    /// audio (`docs/COMPLIANCE_FOLLOWUPS.md` §8.2).
    pub fn with_tts_service(mut self, tts: Option<Arc<fcp_media::TtsService>>) -> Self {
        self.tts = tts;
        self
    }

    /// Create a new QueueCallHandler.
    ///
    /// Accepts a shared `Arc<QueueManager>` so the same instance can be
    /// shared with the REST API for agent management and stats queries.
    /// Spawns per-queue distribution loops that periodically try to match
    /// waiting callers with available agents.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        queue_manager: Arc<QueueManager>,
        config: QueueSystemConfig,
        default_tenant_id: TenantId,
        local_uri: String,
        local_addr: String,
        port_pool: Arc<PortPool>,
        event_bus: Arc<dyn EventPublisher>,
        uac: Arc<SipUserAgentClient>,
        storage: Arc<fcp_core::DynStorage>,
        cdr: fcp_sip::CdrWriter,
        node_id: String,
        call_manager: Arc<fcp_sip::CallManager>,
    ) -> Self {
        // Load announcement audio if configured
        let announcement_cache = config.queue_audio_dir.as_ref().and_then(|dir| {
            let cache = AnnouncementCache::load(std::path::Path::new(dir));
            if cache.is_empty() {
                info!("Queue announcement cache is empty (no audio files found)");
                None
            } else {
                info!(dir = %dir, "Queue announcement audio loaded");
                Some(Arc::new(cache))
            }
        });

        // Load MOH audio if configured
        let moh_cache = config.queue_audio_dir.as_ref().and_then(|dir| {
            let cache = MohCache::load(std::path::Path::new(dir));
            if cache.is_empty() {
                info!("MOH cache is empty (no audio files found)");
                None
            } else {
                info!(dir = %dir, "MOH audio loaded");
                Some(Arc::new(cache))
            }
        });

        // Load prompt audio (entry/connect prompts) if configured
        let prompt_cache = config.queue_audio_dir.as_ref().and_then(|dir| {
            // Collect unique prompt filenames from all queue configs
            let mut prompt_files: Vec<String> = Vec::new();
            for qc in config.queues.values() {
                if let Some(ref p) = qc.entry_prompt {
                    if !prompt_files.contains(p) {
                        prompt_files.push(p.clone());
                    }
                }
                if let Some(ref p) = qc.connect_prompt {
                    if !prompt_files.contains(p) {
                        prompt_files.push(p.clone());
                    }
                }
            }
            if prompt_files.is_empty() {
                None
            } else {
                // Collect unique languages from all queue configs
                let mut languages: Vec<String> = Vec::new();
                for qc in config.queues.values() {
                    if let Some(ref lang) = qc.announcement_language {
                        if !languages.contains(lang) {
                            languages.push(lang.clone());
                        }
                    }
                }
                if !languages.contains(&"en".to_string()) {
                    languages.push("en".to_string());
                }
                let cache = PromptCache::load(std::path::Path::new(dir), &prompt_files, &languages);
                if cache.is_empty() {
                    None
                } else {
                    Some(Arc::new(cache))
                }
            }
        });

        let handler = Self {
            queue_manager,
            config: config.clone(),
            default_tenant_id,
            sessions: Arc::new(DashMap::new()),
            local_uri,
            local_addr,
            port_pool,
            event_bus,
            uac,
            announcement_cache,
            moh_cache,
            prompt_cache,
            storage,
            cdr,
            node_id,
            call_manager,
            campaign_call_id_map: Arc::new(DashMap::new()),
            tts: None,
        };

        // Spawn distribution loops for each configured queue
        for queue_name in config.queues.keys() {
            handler.spawn_distribution_loop(queue_name.clone(), config.max_ring_time_secs);
        }

        // Start daily stats reset scheduler
        handler.queue_manager.start_daily_reset();

        handler
    }

    /// Whether `call_id` is a call this queue system holds the media of.
    pub fn has_session(&self, call_id: &str) -> bool {
        self.sessions.contains_key(call_id)
    }

    /// Expose the QueueManager for agent login/logout via REST API.
    // Kept as part of the module's intended API; not currently called from this binary.
    #[allow(dead_code)]
    pub fn queue_manager(&self) -> &Arc<QueueManager> {
        &self.queue_manager
    }

    /// Update the queue_name on an active session (after a queue-to-queue transfer).
    pub fn update_session_queue(&self, call_id: &str, new_queue: &str) {
        if let Some(mut session) = self.sessions.get_mut(call_id) {
            session.queue_name = new_queue.to_string();
        }
    }

    /// Spawn a distribution loop for a dynamically added queue.
    /// Called after reload_config() adds new queues.
    pub fn spawn_distribution_loop_for(&self, queue_name: String) {
        let ring_timeout = self.config.max_ring_time_secs;
        self.spawn_distribution_loop(queue_name, ring_timeout);
    }

    /// Build an AnnounceConfig if position announcements are enabled for the given queue.
    fn build_announce_config(&self, queue_name: &str) -> Option<AnnounceConfig> {
        let cache = self.announcement_cache.as_ref()?;

        let queue_cfg = self
            .config
            .queues
            .get(queue_name)
            .or_else(|| self.config.queues.values().find(|qc| qc.announce_position))?;

        if !queue_cfg.announce_position {
            return None;
        }

        let language = queue_cfg
            .announcement_language
            .as_deref()
            .unwrap_or("en")
            .to_string();

        let interval = queue_cfg.announce_interval_secs;
        let announce_eta = queue_cfg.announce_eta && cache.has_eta(&language);

        Some(AnnounceConfig {
            cache: Arc::clone(cache),
            interval_secs: interval,
            queue_manager: Arc::clone(&self.queue_manager),
            announce_eta,
            language,
        })
    }

    /// Resolve MOH frames for a given queue and codec.
    /// Looks up the queue's `moh_class` in the MOH cache, falling back to
    /// `"default"` for a queue with none (or an empty one, as the API
    /// stores it) or a class with no audio.
    fn resolve_moh_frames(&self, queue_name: &str, codec_name: &str) -> Option<Arc<Vec<Vec<u8>>>> {
        let cache = self.moh_cache.as_ref()?;
        let moh_class = self
            .queue_manager
            .get_queue_config(queue_name)
            .and_then(|c| c.moh_class.clone())
            .filter(|class| !class.is_empty())
            .unwrap_or_else(|| "default".to_string());
        let frames = cache
            .get_frames(&moh_class, codec_name)
            .or_else(|| cache.get_frames("default", codec_name))?;
        Some(Arc::new(frames.to_vec()))
    }

    /// Parse the local SIP URI.
    fn parse_local_sip_uri(&self) -> sip_core::SipUri {
        Uri::parse(&self.local_uri)
            .ok()
            .and_then(|u| u.as_sip().cloned())
            .unwrap_or_else(|| {
                Uri::parse("sip:queue@localhost")
                    .unwrap()
                    .as_sip()
                    .cloned()
                    .unwrap()
            })
    }

    /// Extract the user part from a SIP Request-URI.
    fn extract_extension(request: &sip_core::Request) -> Option<String> {
        let uri = request.uri();
        let uri_str = uri.to_string();
        if let Some(rest) = uri_str.strip_prefix("sip:") {
            rest.split('@')
                .next()
                .filter(|u| !u.is_empty())
                .map(String::from)
        } else if let Some(rest) = uri_str.strip_prefix("tel:") {
            let number = rest.split(';').next().unwrap_or(rest).trim();
            if !number.is_empty() {
                Some(number.to_string())
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Extract the Call-ID header from a SIP request.
    fn extract_call_id(request: &sip_core::Request) -> String {
        request
            .headers()
            .get("Call-ID")
            .map(|v| v.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    }

    /// Extract a tenant domain from SIP addressing.
    fn extract_tenant_domain(request: &sip_core::Request) -> Option<String> {
        // The Request-URI's host, without its port or parameters: a call to
        // `sip:7000@127.0.0.1:5070` is to a host literal, not to a tenant
        // domain called "127.0.0.1:5070".
        let request_uri = request.uri().to_string();
        if let Ok(parsed) = sip_core::SipUri::parse(request_uri.trim()) {
            let domain = parsed.host().trim().to_string();
            if !domain.is_empty() && domain != "localhost" && domain != "127.0.0.1" {
                return Some(domain);
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
        if let Some(domain) = Self::extract_tenant_domain(request) {
            if fcp_core::is_host_literal(&domain) {
                return self
                    .default_tenant_fallback()
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("tenant context required"));
            }

            return match self.storage.get_tenant_by_sip_domain(&domain).await {
                Ok(Some(tenant)) => Ok(tenant.id),
                Ok(None) => match self.storage.get_tenant_by_name(&domain).await {
                    Ok(Some(tenant)) => {
                        warn!(
                            domain = %domain,
                            tenant_id = %tenant.id,
                            "Queue tenant resolved via legacy name match; add the domain to sip_domains"
                        );
                        Ok(tenant.id)
                    }
                    Ok(None) => bail!("Unknown tenant domain '{}'", domain),
                    Err(e) => Err(e).context("failed to resolve tenant from SIP domain"),
                },
                Err(e) => Err(e).context("failed to resolve tenant from SIP domain"),
            };
        }

        self.default_tenant_fallback()
            .await?
            .ok_or_else(|| anyhow::anyhow!("tenant context required"))
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

    /// Extract caller info (display_name, caller_uri) from From header.
    fn extract_caller_info(request: &sip_core::Request) -> (Option<String>, String) {
        if let Some(from_val) = request.headers().get_smol("From") {
            if let Some(from) = sip_parse::parse_from_header(from_val) {
                let name_addr = from.inner();
                let display = name_addr.display_name().map(|s| s.to_string());
                let caller = name_addr
                    .sip_uri()
                    .map(|u| u.as_str().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                return (display, caller);
            }
        }
        (None, "unknown".to_string())
    }

    /// Parse remote RTP endpoint from an SDP offer.
    fn parse_remote_rtp_addr(sdp: &str) -> Option<SocketAddr> {
        let mut ip: Option<&str> = None;
        let mut port: Option<u16> = None;

        for line in sdp.lines() {
            let line = line.trim();
            if line.starts_with("c=IN IP4 ")
                || line.starts_with("c=IN ip4 ")
                || line.starts_with("c=IN IP6 ")
                || line.starts_with("c=IN ip6 ")
            {
                ip = line.split_whitespace().nth(2);
            } else if line.starts_with("m=audio ") {
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
    /// Returns (codec_name, payload_type, frame_size_in_samples).
    fn pick_codec(codecs: &[fcp_media::SdpCodecInfo]) -> Option<(String, u8, usize)> {
        for codec in codecs {
            let name_upper = codec.encoding_name.to_uppercase();
            match name_upper.as_str() {
                "PCMU" => return Some(("PCMU".to_string(), codec.payload_type, 160)),
                "PCMA" => return Some(("PCMA".to_string(), codec.payload_type, 160)),
                "G722" => return Some(("G722".to_string(), codec.payload_type, 320)),
                _ => continue,
            }
        }
        None
    }

    /// Spawn a MOH (Music On Hold) playback task that sends RTP packets
    /// to the caller at 20ms intervals. If `moh_frames` is provided,
    /// plays real MOH audio looping through the frames; otherwise sends
    /// silence. Periodic position announcements interrupt MOH if configured.
    fn spawn_moh(
        rtp_socket: Arc<RtpSocketPair>,
        codec_name: &str,
        payload_type: u8,
        call_id: String,
        announce: Option<AnnounceConfig>,
        moh_frames: Option<Arc<Vec<Vec<u8>>>>,
    ) -> JoinHandle<()> {
        let silence_frame: Vec<u8> = match codec_name {
            "PCMU" => vec![0xFFu8; 160], // G.711 μ-law silence
            "PCMA" => vec![0xD5u8; 160], // G.711 A-law silence
            "G722" => vec![0x00u8; 160], // G.722 silence
            _ => vec![0xFFu8; 160],      // Default to μ-law
        };

        let seq = Arc::new(AtomicU16::new(0));
        let ts = Arc::new(AtomicU32::new(0));
        let pt = payload_type;
        let codec = codec_name.to_string();
        // Generate a random SSRC from a UUID (avoids adding rand dependency)
        let ssrc = (uuid::Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(20));
            let mut last_announcement = Instant::now();
            let mut moh_index: usize = 0;

            loop {
                interval.tick().await;

                // Check if it's time for a position announcement
                if let Some(ref ac) = announce {
                    let elapsed = last_announcement.elapsed().as_secs() as u32;
                    if elapsed >= ac.interval_secs {
                        last_announcement = Instant::now();

                        if let Some(position) = ac.queue_manager.get_caller_position(&call_id) {
                            // Collect all announcement frames: position + optional ETA
                            let mut all_frames: Vec<&[u8]> = Vec::new();

                            if let Some(pos_frames) =
                                ac.cache.get_position_frames(position, &codec, &ac.language)
                            {
                                for f in pos_frames {
                                    all_frames.push(f);
                                }
                            }

                            // Add ETA frames if enabled
                            if ac.announce_eta {
                                if let Some(eta_secs) =
                                    ac.queue_manager.estimate_wait_secs(&call_id)
                                {
                                    let eta_mins = ((eta_secs + 30) / 60).max(1) as usize; // Round to nearest minute, minimum 1
                                    if eta_mins <= 60 {
                                        // Small gap between position and ETA (200ms = 10 silence frames at 20ms)
                                        // We'll handle this by adding silence in between
                                        if let Some(eta_frames) =
                                            ac.cache.get_eta_frames(eta_mins, &codec, &ac.language)
                                        {
                                            debug!(
                                                call_id = %call_id,
                                                eta_mins = eta_mins,
                                                "Including ETA in announcement"
                                            );
                                            for f in eta_frames {
                                                all_frames.push(f);
                                            }
                                        }
                                    }
                                }
                            }

                            if !all_frames.is_empty() {
                                debug!(
                                    call_id = %call_id,
                                    position = position,
                                    frames = all_frames.len(),
                                    "Playing position announcement"
                                );

                                // Play all announcement frames
                                for frame in &all_frames {
                                    let sequence = seq.fetch_add(1, Ordering::Relaxed);
                                    let timestamp = ts.fetch_add(160, Ordering::Relaxed);

                                    let mut packet = Vec::with_capacity(12 + frame.len());
                                    packet.push(0x80);
                                    packet.push(pt);
                                    packet.extend_from_slice(&sequence.to_be_bytes());
                                    packet.extend_from_slice(&timestamp.to_be_bytes());
                                    packet.extend_from_slice(&ssrc.to_be_bytes());
                                    packet.extend_from_slice(frame);

                                    if let Err(e) = rtp_socket.send_rtp_raw(&packet).await {
                                        debug!(error = %e, "Announcement send failed");
                                        return;
                                    }

                                    // Pace at 20ms per frame
                                    tokio::time::sleep(Duration::from_millis(20)).await;
                                }

                                // Skip the regular MOH tick since we just played audio
                                continue;
                            }
                        }
                    }
                }

                // Select the frame to send: real MOH or silence
                let frame = if let Some(ref frames) = moh_frames {
                    if !frames.is_empty() {
                        let f = &frames[moh_index % frames.len()];
                        moh_index += 1;
                        f.as_slice()
                    } else {
                        silence_frame.as_slice()
                    }
                } else {
                    silence_frame.as_slice()
                };

                let sequence = seq.fetch_add(1, Ordering::Relaxed);
                let timestamp = ts.fetch_add(160, Ordering::Relaxed);

                let mut packet = Vec::with_capacity(12 + frame.len());
                packet.push(0x80);
                packet.push(pt);
                packet.extend_from_slice(&sequence.to_be_bytes());
                packet.extend_from_slice(&timestamp.to_be_bytes());
                packet.extend_from_slice(&ssrc.to_be_bytes());
                packet.extend_from_slice(frame);

                if let Err(e) = rtp_socket.send_rtp_raw(&packet).await {
                    debug!(error = %e, "MOH send failed (caller may have hung up)");
                    break;
                }
            }
        })
    }

    /// Spawn the per-queue distribution loop.
    ///
    /// Runs every 500ms, checking for waiting calls and attempting to
    /// match them with available agents via `QueueManager::select_agent()`.
    fn spawn_distribution_loop(&self, queue_name: String, ring_timeout_secs: u32) {
        let sessions = Arc::clone(&self.sessions);
        let queue_manager = self.queue_manager.clone();
        let port_pool = Arc::clone(&self.port_pool);
        let local_addr = self.local_addr.clone();
        let local_uri = self.local_uri.clone();
        let uac = Arc::clone(&self.uac);
        let event_bus = Arc::clone(&self.event_bus);
        let announce_config = self.build_announce_config(&queue_name);
        let prompt_cache = self.prompt_cache.clone();
        let recording_dir = self
            .config
            .recording_dir
            .clone()
            .map(|dir| QueueRecordings {
                dir,
                store: self.storage.call_recording_store().cloned(),
                storage: Arc::clone(&self.storage),
                tts: self.tts.clone(),
            });
        let campaign_call_id_map = Arc::clone(&self.campaign_call_id_map);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;

                if !queue_manager.is_enabled() {
                    continue;
                }

                let Some(queue_tenant_id) = queue_manager.get_queue_tenant(&queue_name) else {
                    debug!(queue = %queue_name, "Skipping distribution tick for missing queue");
                    continue;
                };

                // Check for callers who have exceeded max wait time
                if let Some(config) = queue_manager.get_queue_config(&queue_name) {
                    if config.max_wait_time_secs > 0 {
                        let max_wait = Duration::from_secs(config.max_wait_time_secs as u64);
                        let expired: Vec<String> = sessions
                            .iter()
                            .filter(|entry| {
                                let s = entry.value();
                                s.queue_name == queue_name
                                    && s.state == QueueCallState::WaitingInQueue
                                    && s.enqueued_at.elapsed() > max_wait
                            })
                            .map(|entry| entry.key().clone())
                            .collect();

                        for call_id in expired {
                            let overflow_dests = config.effective_overflow_destinations();
                            if !overflow_dests.is_empty() {
                                info!(
                                    call_id = %call_id,
                                    queue = %queue_name,
                                    destinations = ?overflow_dests,
                                    "Max wait time exceeded, routing to overflow"
                                );
                                overflow_expired_call_multi(
                                    &call_id,
                                    &overflow_dests,
                                    &queue_name,
                                    &sessions,
                                    &queue_manager,
                                    &uac,
                                    &port_pool,
                                    &local_addr,
                                    &local_uri,
                                    &event_bus,
                                )
                                .await;
                            } else {
                                warn!(
                                    call_id = %call_id,
                                    queue = %queue_name,
                                    "Max wait time exceeded, no overflow destination, sending BYE"
                                );
                                // Send BYE to caller and clean up
                                if let Some((_, session)) = sessions.remove(&call_id) {
                                    if let Some(moh) = session.moh_handle {
                                        moh.abort();
                                    }
                                    let bye_uac = Arc::clone(&uac);
                                    let bye_dialog = session.dialog.clone();
                                    tokio::spawn(async move {
                                        let _ = bye_uac.bye(&bye_dialog).await;
                                    });
                                    let pool = Arc::clone(&port_pool);
                                    let pp = session.port_pair;
                                    tokio::spawn(async move { pool.deallocate(pp).await });
                                    queue_manager.dequeue_call(&call_id);
                                }
                            }
                        }
                    }
                }

                // Escalate priorities for calls that have been waiting
                queue_manager.escalate_priorities(&queue_name);

                // Collect call_ids that are WaitingInQueue for this queue
                let waiting_call_ids: Vec<String> = sessions
                    .iter()
                    .filter(|entry| {
                        let s = entry.value();
                        s.queue_name == queue_name && s.state == QueueCallState::WaitingInQueue
                    })
                    .map(|entry| entry.key().clone())
                    .collect();

                // Prune callbacks that exceeded max attempts
                let expired_callbacks = queue_manager.remove_expired_callbacks(&queue_name);
                for expired in &expired_callbacks {
                    let cb_uri = expired.callback_uri.as_deref().unwrap_or("");
                    fcp_queue::events::callback_failed(
                        &Some(Arc::clone(&event_bus)),
                        &queue_name,
                        &expired.call_id,
                        cb_uri,
                        "max_attempts_exceeded",
                        expired.callback_attempts,
                        &expired.tenant_id,
                    )
                    .await;
                }

                // Also collect callback call_ids (in QueueManager queue but no session)
                // Using get_ready_callbacks() which enforces cooldown between attempts
                let callback_call_ids: Vec<(String, String)> = queue_manager
                    .get_ready_callbacks(&queue_name)
                    .into_iter()
                    .filter(|(call_id, _)| !sessions.contains_key(call_id))
                    .collect();

                // Process normal waiting calls first
                let no_waiting = waiting_call_ids.is_empty();
                for call_id in waiting_call_ids {
                    // Try to select an agent for this call
                    let plan = match queue_manager.select_agent(&queue_name) {
                        Some(p) => p,
                        None => break, // No available agents, stop trying for this tick
                    };

                    // Mark agent as ringing
                    queue_manager
                        .mark_agent_ringing(&plan.agent_id, &call_id)
                        .await;

                    // Update session state to RingingAgent
                    if let Some(mut session) = sessions.get_mut(&call_id) {
                        session.state = QueueCallState::RingingAgent;
                        session.ringing_agent = Some(plan.agent_id.clone());

                        // Stop MOH and DTMF listener while ringing agent
                        if let Some(moh) = session.moh_handle.take() {
                            moh.abort();
                        }
                        if let Some(dtmf) = session.dtmf_listener_handle.take() {
                            dtmf.abort();
                        }
                    } else {
                        continue;
                    }

                    // Create cancel channel for caller BYE during ring
                    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

                    // Spawn the agent ring task
                    let ring_handle = Self::spawn_agent_ring(
                        call_id.clone(),
                        plan,
                        ring_timeout_secs,
                        Arc::clone(&sessions),
                        queue_manager.clone(),
                        Arc::clone(&port_pool),
                        local_addr.clone(),
                        local_uri.clone(),
                        Arc::clone(&uac),
                        Arc::clone(&event_bus),
                        announce_config.clone(),
                        cancel_rx,
                        prompt_cache.clone(),
                        recording_dir.clone(),
                    );

                    // Store ring task handle and cancel sender for BYE/CANCEL
                    if let Some(mut session) = sessions.get_mut(&call_id) {
                        session.ring_task_handle = Some(ring_handle);
                        session.ring_cancel_tx = Some(cancel_tx);
                    }
                }

                // Process callback calls
                for (call_id, callback_uri) in callback_call_ids {
                    let plan = match queue_manager.select_agent(&queue_name) {
                        Some(p) => p,
                        None => break,
                    };

                    queue_manager
                        .mark_agent_ringing(&plan.agent_id, &call_id)
                        .await;

                    spawn_callback_ring(
                        call_id,
                        callback_uri,
                        plan,
                        ring_timeout_secs,
                        Arc::clone(&sessions),
                        queue_manager.clone(),
                        Arc::clone(&port_pool),
                        local_addr.clone(),
                        local_uri.clone(),
                        Arc::clone(&uac),
                        Arc::clone(&event_bus),
                        queue_name.clone(),
                        queue_tenant_id,
                    );
                }

                // ---- Campaign / Predictive Dialer ----
                // If campaign is enabled and running, auto-dial numbers when agents are idle
                if let Some(config) = queue_manager.get_queue_config(&queue_name) {
                    if config.campaign_enabled && queue_manager.is_campaign_running(&queue_name) {
                        let available = queue_manager.count_available_agents(&queue_name);
                        let active = queue_manager.campaign_active_dial_count(&queue_name);
                        let dials_target =
                            (available as f32 * config.campaign_dial_ratio).ceil() as usize;
                        let dials_needed = dials_target.saturating_sub(active);

                        for _ in 0..dials_needed {
                            let number = match queue_manager.next_campaign_number(&queue_name) {
                                Some(n) => n,
                                None => {
                                    // No more numbers — stop campaign
                                    queue_manager.set_campaign_running(&queue_name, false);
                                    break;
                                }
                            };

                            let dest = number.destination.clone();
                            queue_manager.add_campaign_active_dial(&queue_name, &dest);

                            // Emit event
                            fcp_queue::events::campaign_call_placed(
                                &Some(Arc::clone(&event_bus)),
                                &queue_name,
                                &dest,
                                &queue_tenant_id,
                            )
                            .await;

                            // Spawn outbound INVITE task
                            let qm = queue_manager.clone();
                            let qn = queue_name.clone();
                            let uac2 = Arc::clone(&uac);
                            let eb = Arc::clone(&event_bus);
                            let pp = Arc::clone(&port_pool);
                            let la = local_addr.clone();
                            let lu = local_uri.clone();
                            let sessions2 = Arc::clone(&sessions);
                            let ring_timeout = ring_timeout_secs;
                            let abandon_timeout = config.campaign_abandon_timeout_secs;
                            let ac2 = announce_config.clone();
                            let pc2 = prompt_cache.clone();
                            let rd2 = recording_dir.clone();
                            let tid = queue_tenant_id;
                            let ccm = Arc::clone(&campaign_call_id_map);

                            tokio::spawn(async move {
                                campaign_dial_task(
                                    dest,
                                    number,
                                    qn,
                                    qm,
                                    uac2,
                                    eb,
                                    pp,
                                    la,
                                    lu,
                                    sessions2,
                                    ring_timeout,
                                    abandon_timeout,
                                    ac2,
                                    pc2,
                                    rd2,
                                    tid,
                                    ccm,
                                )
                                .await;
                            });
                        }
                    }
                }

                // Check if queue is fully drained (draining + no waiting + no active sessions)
                if queue_manager.is_queue_draining(&queue_name) && no_waiting {
                    let active = sessions.iter().any(|e| e.value().queue_name == queue_name);
                    if !active {
                        let events = queue_manager.events_ref();
                        fcp_queue::events::queue_drained(&events, &queue_name, &queue_tenant_id)
                            .await;
                    }
                }
            }
        });
    }

    /// Spawn a task that INVITEs an agent and, on answer, bridges RTP.
    #[allow(clippy::too_many_arguments)]
    fn spawn_agent_ring(
        call_id: String,
        plan: fcp_queue::DistributionPlan,
        ring_timeout_secs: u32,
        sessions: Arc<DashMap<String, QueueSession>>,
        queue_manager: Arc<QueueManager>,
        port_pool: Arc<PortPool>,
        local_addr: String,
        local_uri: String,
        uac: Arc<SipUserAgentClient>,
        _event_bus: Arc<dyn EventPublisher>,
        announce_config: Option<AnnounceConfig>,
        cancel_rx: tokio::sync::oneshot::Receiver<()>,
        prompt_cache: Option<Arc<PromptCache>>,
        recording_dir: Option<QueueRecordings>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let agent_id = plan.agent_id.clone();
            let agent_extension = plan.agent_extension.clone();
            let ring_timeout = Duration::from_secs(ring_timeout_secs as u64);

            // 1. Allocate B-leg RTP port pair
            let agent_port_pair = match port_pool.allocate().await {
                Ok(pp) => pp,
                Err(e) => {
                    warn!(
                        call_id = %call_id,
                        agent = %agent_id,
                        error = %e,
                        "Failed to allocate RTP port for agent leg"
                    );
                    Self::return_to_waiting(
                        &call_id,
                        &agent_id,
                        &sessions,
                        &queue_manager,
                        announce_config.clone(),
                    )
                    .await;
                    return;
                }
            };

            // 2. Build multi-codec SDP offer for agent leg
            let sdp_offer = build_multi_codec_sdp_offer(&local_addr, agent_port_pair.rtp_port);

            // 3. INVITE the agent
            let agent_uri = format!("sip:{}@{}", agent_extension, extract_domain(&local_uri));
            let call_handle = match uac.invite(&agent_uri, Some(&sdp_offer)).await {
                Ok(h) => h,
                Err(e) => {
                    warn!(
                        call_id = %call_id,
                        agent = %agent_id,
                        error = %e,
                        "Failed to INVITE agent"
                    );
                    port_pool.deallocate(agent_port_pair).await;
                    Self::return_to_waiting(
                        &call_id,
                        &agent_id,
                        &sessions,
                        &queue_manager,
                        announce_config.clone(),
                    )
                    .await;
                    return;
                }
            };

            // 4. Wait for final response with timeout, or caller cancel
            let result = tokio::select! {
                r = tokio::time::timeout(ring_timeout, call_handle.await_final()) => r,
                _ = cancel_rx => {
                    // Caller hung up while agent was ringing — send SIP CANCEL
                    info!(
                        call_id = %call_id,
                        agent = %agent_id,
                        "Caller hung up, cancelling agent INVITE"
                    );
                    let _ = call_handle.cancel().await;
                    port_pool.deallocate(agent_port_pair).await;
                    queue_manager.agent_no_answer_for_cancel(&agent_id, &call_id).await;
                    return;
                }
            };

            match result {
                Ok(Ok(response))
                    if response.start_line().code() >= 200
                        && response.start_line().code() < 300 =>
                {
                    // Agent answered (200 OK)
                    info!(
                        call_id = %call_id,
                        agent = %agent_id,
                        "Agent answered queue call"
                    );

                    // Parse SDP answer from agent to determine chosen codec
                    let agent_body = response.body();
                    let agent_sdp = std::str::from_utf8(&agent_body[..]).unwrap_or("");
                    let (agent_codec_name, agent_pt) = parse_agent_codec(agent_sdp);
                    debug!(
                        call_id = %call_id,
                        agent = %agent_id,
                        agent_codec = %agent_codec_name,
                        agent_pt = agent_pt,
                        "Agent codec negotiated"
                    );

                    // Create B-leg RTP socket
                    let agent_rtp_socket =
                        match RtpSocketPair::new(agent_port_pair, RtpSocketConfig::default()).await
                        {
                            Ok(s) => Arc::new(s),
                            Err(e) => {
                                warn!(error = %e, "Failed to create agent RTP socket");
                                port_pool.deallocate(agent_port_pair).await;
                                Self::return_to_waiting(
                                    &call_id,
                                    &agent_id,
                                    &sessions,
                                    &queue_manager,
                                    announce_config.clone(),
                                )
                                .await;
                                return;
                            }
                        };

                    // Set agent remote RTP endpoint
                    if let Some(remote_addr) = Self::parse_remote_rtp_addr(agent_sdp) {
                        agent_rtp_socket
                            .set_remote_endpoint(RemoteEndpoint {
                                rtp_addr: remote_addr,
                                rtcp_addr: None,
                            })
                            .await;
                    }

                    // Get the agent's dialog
                    let agent_dialog = call_handle.dialog.read().await.clone();

                    // Update QueueManager
                    queue_manager.agent_answered(&agent_id, &call_id).await;

                    // Play connect prompt to caller if configured
                    if let Some(ref pc) = prompt_cache {
                        if let Some(session) = sessions.get(&call_id) {
                            let queue_name = session.queue_name.clone();
                            let codec_name = session.codec_name.clone();
                            let pt = session.payload_type;
                            let caller_socket = Arc::clone(&session.rtp_socket);
                            drop(session);

                            if let Some(qc) = queue_manager.get_queue_config(&queue_name) {
                                if let Some(ref prompt_name) = qc.connect_prompt {
                                    if let Some(frames) = pc.get_frames(prompt_name, &codec_name) {
                                        debug!(call_id = %call_id, prompt = %prompt_name, "Playing connect prompt");
                                        play_prompt_frames(&caller_socket, frames, pt).await;
                                    }
                                }
                            }
                        }
                    }

                    // Connect caller and agent: when the queue records, both
                    // hear the tenant's announcement before they hear each
                    // other (docs/COMPLIANCE_FOLLOWUPS.md §8.2).
                    let caller_side = sessions.get(&call_id).map(|session| {
                        (
                            Leg {
                                socket: Arc::clone(&session.rtp_socket),
                                codec: session.codec_name.clone(),
                                payload_type: session.payload_type,
                            },
                            session.queue_name.clone(),
                            session.tenant_id,
                            session.b2bua_call_id,
                        )
                    });
                    if let Some((caller, queue_name, tenant_id, b2bua_call_id)) = caller_side {
                        let agent = Leg {
                            socket: Arc::clone(&agent_rtp_socket),
                            codec: agent_codec_name.clone(),
                            payload_type: agent_pt,
                        };
                        if caller.codec != agent.codec {
                            info!(
                                call_id = %call_id,
                                caller_codec = %caller.codec,
                                agent_codec = %agent.codec,
                                "Codecs differ, using transcoding bridge"
                            );
                        }
                        let recording =
                            match (&recording_dir, queue_manager.get_queue_config(&queue_name)) {
                                (Some(recordings), Some(qc)) if qc.record_calls => {
                                    Some(RecordingPlan {
                                        recordings,
                                        tenant_id,
                                        call_id: queue_record_call_id(b2bua_call_id),
                                        format: qc.recording_format.clone(),
                                    })
                                }
                                _ => None,
                            };
                        let still_here = || sessions.contains_key(&call_id);
                        let bridge = connect_legs(&caller, &agent, recording, still_here).await;
                        match (bridge, sessions.get_mut(&call_id)) {
                            (Some(bridge), Some(mut session)) => {
                                if let Some(path) = &bridge.recording_path {
                                    info!(call_id = %call_id, path = %path, "Call recording started");
                                }
                                session.bridge_a_to_b = Some(bridge.a_to_b);
                                session.bridge_b_to_a = Some(bridge.b_to_a);
                                session.recording_handle = bridge.recording;
                                session.recording_path = bridge.recording_path;
                                session.recording_consent = bridge.recording_consent;
                                session.state = QueueCallState::Connected;
                                session.agent_answered_at = Some(Instant::now());
                                session.agent_codec_name = Some(agent_codec_name.clone());
                                session.agent_port_pair = Some(agent_port_pair);
                                session.agent_rtp_socket = Some(agent_rtp_socket);
                                session.agent_dialog = Some(agent_dialog);
                                session.ringing_agent = None;
                                session.ring_task_handle = None;
                            }
                            (Some(bridge), None) => {
                                bridge.abandon(recording_dir.as_ref()).await;
                                info!(call_id = %call_id, "Caller left while the legs were being connected");
                            }
                            (None, _) => {
                                info!(call_id = %call_id, "Caller left during the recording announcement");
                            }
                        }
                    }
                }
                Ok(Ok(response)) => {
                    // Agent rejected (4xx/5xx/6xx)
                    warn!(
                        call_id = %call_id,
                        agent = %agent_id,
                        status = response.start_line().code(),
                        "Agent rejected queue call"
                    );
                    port_pool.deallocate(agent_port_pair).await;
                    Self::return_to_waiting(
                        &call_id,
                        &agent_id,
                        &sessions,
                        &queue_manager,
                        announce_config.clone(),
                    )
                    .await;
                }
                Ok(Err(e)) => {
                    // Error during call setup
                    warn!(
                        call_id = %call_id,
                        agent = %agent_id,
                        error = %e,
                        "Error during agent INVITE"
                    );
                    port_pool.deallocate(agent_port_pair).await;
                    Self::return_to_waiting(
                        &call_id,
                        &agent_id,
                        &sessions,
                        &queue_manager,
                        announce_config.clone(),
                    )
                    .await;
                }
                Err(_) => {
                    // Ring timeout — cancel the agent INVITE
                    info!(
                        call_id = %call_id,
                        agent = %agent_id,
                        timeout_secs = ring_timeout_secs,
                        "Agent ring timeout, cancelling INVITE"
                    );
                    let _ = call_handle.cancel().await;
                    port_pool.deallocate(agent_port_pair).await;
                    Self::return_to_waiting(
                        &call_id,
                        &agent_id,
                        &sessions,
                        &queue_manager,
                        announce_config.clone(),
                    )
                    .await;
                }
            }
        })
    }

    /// Return a call to WaitingInQueue state after a failed agent attempt.
    async fn return_to_waiting(
        call_id: &str,
        agent_id: &str,
        sessions: &DashMap<String, QueueSession>,
        queue_manager: &QueueManager,
        announce: Option<AnnounceConfig>,
    ) {
        queue_manager.agent_no_answer(agent_id, call_id).await;

        if let Some(mut session) = sessions.get_mut(call_id) {
            session.state = QueueCallState::WaitingInQueue;
            session.ringing_agent = None;
            session.ring_task_handle = None;

            // Restart MOH
            let moh_frames = session.moh_frames.clone();
            let moh = Self::spawn_moh(
                Arc::clone(&session.rtp_socket),
                &session.codec_name,
                session.payload_type,
                call_id.to_string(),
                announce,
                moh_frames,
            );
            session.moh_handle = Some(moh);
        }
    }

    /// End the B2BUA call for a caller the queue refused with `response`.
    fn refuse_call(&self, b2bua_call_id: fcp_sip::CallId, response: &sip_core::Response) {
        let _ = self.call_manager.terminate_call_with(
            &b2bua_call_id,
            fcp_sip::TerminationReason::Failed {
                status: Some(response.start_line().code()),
                detail: format!(
                    "Queue: {} {}",
                    response.start_line().code(),
                    response.start_line().reason()
                ),
            },
        );
    }

    /// Remove and clean up a queue session, inserting a CDR for the call.
    fn remove_session(&self, call_id: &str) {
        if let Some((_, session)) = self.sessions.remove(call_id) {
            info!(
                call_id = %call_id,
                queue = %session.queue_name,
                state = ?session.state,
                "Queue session removed"
            );

            // The call ends with its session; its record and the segment's
            // share the call's id.
            if let Some(b2bua_call_id) = session.b2bua_call_id {
                let _ = self.call_manager.terminate_call_with(
                    &b2bua_call_id,
                    fcp_sip::TerminationReason::Hangup { by: None },
                );
            }
            // The segment's record, off the call path through the writer.
            self.cdr.write(queue_record(
                session.b2bua_call_id,
                call_id,
                session.tenant_id,
                &session.queue_name,
                &session.caller_uri,
                session.caller_display.as_deref(),
                &session.codec_name,
                session.agent_codec_name.as_deref(),
                session.enqueued_at,
                session.agent_answered_at,
                session.state,
                session.ringing_agent.as_deref(),
                session.recording_path.as_deref(),
                session.recording_consent,
                &self.node_id,
            ));

            // Abort MOH if running
            if let Some(moh) = session.moh_handle {
                moh.abort();
            }

            // Abort DTMF listener if running
            if let Some(dtmf) = session.dtmf_listener_handle {
                dtmf.abort();
            }

            // Send cancel signal to ring task (triggers SIP CANCEL to agent)
            if let Some(cancel_tx) = session.ring_cancel_tx {
                let _ = cancel_tx.send(());
            }

            // Abort ring task as fallback (in case select! hasn't started yet)
            if let Some(ring_task) = session.ring_task_handle {
                ring_task.abort();
            }

            // Abort RTP bridge tasks if running
            if let Some(bridge) = session.bridge_a_to_b {
                bridge.abort();
            }
            if let Some(bridge) = session.bridge_b_to_a {
                bridge.abort();
            }

            // Stop recording if active
            if let Some(mut rec_handle) = session.recording_handle {
                // The bridge tasks are gone, so the recorder's senders are
                // dropped: it writes what it still holds and closes the
                // file. Cutting it off here instead would lose the last of
                // the call — as much as a [`QUIET`] window of it.
                let path = session.recording_path.clone();
                let ended = call_id.to_string();
                tokio::spawn(async move {
                    if tokio::time::timeout(RECORDING_FINISH, &mut rec_handle)
                        .await
                        .is_err()
                    {
                        warn!(call_id = %ended, "A queue recording did not finish; stopping it");
                        rec_handle.abort();
                    }
                });
                if let Some(ref path) = path {
                    info!(call_id = %call_id, path = %path, "Recording stopped");
                }
                // Its row closes when the segment's record is stored; close it
                // now, so a record that never arrives leaves no open row.
                // The file is named for its row.
                let id = session.recording_path.as_deref().and_then(recording_id_of);
                if let (Some(store), Some(id)) = (self.storage.call_recording_store().cloned(), id)
                {
                    tokio::spawn(async move {
                        if let Err(e) = store
                            .finish_call_recording(&id, std::time::SystemTime::now())
                            .await
                        {
                            warn!(error = %e, "Could not close a queue recording's row");
                        }
                    });
                }
            }

            // Stop supervisor if active
            for task in session.supervisor_tasks {
                task.abort();
            }
            if let Some(ref sup_dialog) = session.supervisor_dialog {
                let uac = Arc::clone(&self.uac);
                let dialog = sup_dialog.clone();
                tokio::spawn(async move {
                    if let Err(e) = uac.bye(&dialog).await {
                        warn!(error = %e, "Failed to send BYE to supervisor");
                    }
                });
            }
            if let Some(sup_port) = session.supervisor_port_pair {
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(sup_port).await });
            }

            // Send BYE on agent dialog if connected
            if let Some(ref agent_dialog) = session.agent_dialog {
                let uac = Arc::clone(&self.uac);
                let dialog = agent_dialog.clone();
                tokio::spawn(async move {
                    if let Err(e) = uac.bye(&dialog).await {
                        warn!(error = %e, "Failed to send BYE to agent");
                    }
                });
            }

            // For campaign calls, we originated both legs — also BYE the callee
            if session.is_campaign_call {
                let uac = Arc::clone(&self.uac);
                let dialog = session.dialog.clone();
                tokio::spawn(async move {
                    if let Err(e) = uac.bye(&dialog).await {
                        warn!(error = %e, "Failed to send BYE to campaign callee");
                    }
                });
            }

            // Deallocate A-leg and B-leg port pairs
            let port_pool = Arc::clone(&self.port_pool);
            let a_port = session.port_pair;
            let b_port = session.agent_port_pair;
            tokio::spawn(async move {
                port_pool.deallocate(a_port).await;
                if let Some(bp) = b_port {
                    port_pool.deallocate(bp).await;
                }
            });

            // Notify QueueManager if still in queue
            if session.state != QueueCallState::Connected
                && session.state != QueueCallState::OverflowedToDestination
            {
                let qm = self.queue_manager.clone();
                let cid = call_id.to_string();
                tokio::spawn(async move {
                    qm.caller_abandoned(&cid).await;
                });
            } else {
                // Agent was connected — mark call completed so agent goes to WrapUp
                if let Some(ref agent_id) = session.ringing_agent {
                    self.queue_manager.agent_call_completed(agent_id);
                }
            }
        }
    }

    /// Start supervisor monitoring on a connected call.
    ///
    /// Allocates a C-leg, INVITEs the supervisor's extension, and sets up
    /// the appropriate RTP bridge mode (Monitor, Whisper, or Barge).
    pub async fn start_supervisor(
        &self,
        call_id: &str,
        supervisor_ext: &str,
        mode: SupervisorMode,
    ) -> Result<()> {
        // Validate call is connected
        {
            let session = self.sessions.get(call_id).context("Call not found")?;
            if session.state != QueueCallState::Connected {
                bail!("Call is not connected (state: {:?})", session.state);
            }
            if session.supervisor_mode.is_some() {
                bail!("Supervisor already active on this call");
            }
        }

        // Allocate C-leg RTP port pair
        let sup_port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port for supervisor")?;

        // Build SDP offer for supervisor
        let codec_name = {
            let session = self.sessions.get(call_id).context("Call not found")?;
            session.codec_name.clone()
        };
        let sdp_offer = build_sdp_offer(&self.local_addr, sup_port_pair.rtp_port, &codec_name);

        // INVITE the supervisor
        let sup_uri = format!("sip:{}@{}", supervisor_ext, extract_domain(&self.local_uri));
        let call_handle = match self.uac.invite(&sup_uri, Some(&sdp_offer)).await {
            Ok(h) => h,
            Err(e) => {
                self.port_pool.deallocate(sup_port_pair).await;
                bail!("Failed to INVITE supervisor: {}", e);
            }
        };

        // Wait for answer with 30s timeout
        let result = tokio::time::timeout(Duration::from_secs(30), call_handle.await_final()).await;

        match result {
            Ok(Ok(response))
                if response.start_line().code() >= 200 && response.start_line().code() < 300 =>
            {
                let agent_body = response.body();
                let sup_sdp = std::str::from_utf8(&agent_body[..]).unwrap_or("");

                let sup_rtp_socket =
                    match RtpSocketPair::new(sup_port_pair, RtpSocketConfig::default()).await {
                        Ok(s) => Arc::new(s),
                        Err(e) => {
                            self.port_pool.deallocate(sup_port_pair).await;
                            bail!("Failed to create supervisor RTP socket: {}", e);
                        }
                    };

                if let Some(remote_addr) = Self::parse_remote_rtp_addr(sup_sdp) {
                    sup_rtp_socket
                        .set_remote_endpoint(RemoteEndpoint {
                            rtp_addr: remote_addr,
                            rtcp_addr: None,
                        })
                        .await;
                }

                let sup_dialog = call_handle.dialog.read().await.clone();

                // Tear down existing A-B bridge, replace with supervisor bridge
                let mut session = self.sessions.get_mut(call_id).context("Call not found")?;
                if let Some(bridge) = session.bridge_a_to_b.take() {
                    bridge.abort();
                }
                if let Some(bridge) = session.bridge_b_to_a.take() {
                    bridge.abort();
                }

                let a_socket = Arc::clone(&session.rtp_socket);
                let b_socket = session
                    .agent_rtp_socket
                    .as_ref()
                    .context("No agent RTP socket")?;
                let b_socket = Arc::clone(b_socket);
                let c_socket = Arc::clone(&sup_rtp_socket);
                // The supervisor was offered the caller's codec.
                let codecs = [
                    session.codec_name.clone(),
                    session
                        .agent_codec_name
                        .clone()
                        .unwrap_or_else(|| session.codec_name.clone()),
                    session.codec_name.clone(),
                ];

                let tasks = spawn_supervisor_bridge(a_socket, b_socket, c_socket, mode, codecs);

                session.supervisor_port_pair = Some(sup_port_pair);
                session.supervisor_rtp_socket = Some(sup_rtp_socket);
                session.supervisor_dialog = Some(sup_dialog);
                session.supervisor_mode = Some(mode);
                session.supervisor_id = Some(supervisor_ext.to_string());
                session.supervisor_tasks = tasks;

                let tenant_id = session.tenant_id;
                drop(session);

                // Publish event
                let events = Some(Arc::clone(&self.event_bus));
                fcp_queue::events::supervisor_joined(
                    &events,
                    call_id,
                    supervisor_ext,
                    &mode.to_string(),
                    &tenant_id,
                )
                .await;

                info!(
                    call_id = %call_id,
                    supervisor = %supervisor_ext,
                    mode = %mode,
                    "Supervisor joined call"
                );
                Ok(())
            }
            _ => {
                self.port_pool.deallocate(sup_port_pair).await;
                bail!("Supervisor did not answer");
            }
        }
    }

    /// Remove the supervisor from a call and restore normal A-B bridge.
    pub async fn stop_supervisor(&self, call_id: &str) -> Result<()> {
        let mut session = self.sessions.get_mut(call_id).context("Call not found")?;

        if session.supervisor_mode.is_none() {
            bail!("No supervisor active on this call");
        }

        // Abort supervisor bridge tasks
        for task in session.supervisor_tasks.drain(..) {
            task.abort();
        }

        // Send BYE to supervisor
        if let Some(ref sup_dialog) = session.supervisor_dialog {
            let uac = Arc::clone(&self.uac);
            let dialog = sup_dialog.clone();
            tokio::spawn(async move {
                if let Err(e) = uac.bye(&dialog).await {
                    warn!(error = %e, "Failed to send BYE to supervisor");
                }
            });
        }

        // Deallocate supervisor port
        if let Some(sup_port) = session.supervisor_port_pair.take() {
            let pool = Arc::clone(&self.port_pool);
            tokio::spawn(async move { pool.deallocate(sup_port).await });
        }

        let supervisor_id = session.supervisor_id.take().unwrap_or_default();
        let tenant_id = session.tenant_id;

        session.supervisor_rtp_socket = None;
        session.supervisor_dialog = None;
        session.supervisor_mode = None;

        // Restore normal A-B bridge
        let a_socket = Arc::clone(&session.rtp_socket);
        let b_socket = session
            .agent_rtp_socket
            .as_ref()
            .context("No agent RTP socket")?;
        let b_socket = Arc::clone(b_socket);

        let (bridge_a_to_b, bridge_b_to_a) = spawn_rtp_bridge(a_socket, b_socket);
        session.bridge_a_to_b = Some(bridge_a_to_b);
        session.bridge_b_to_a = Some(bridge_b_to_a);

        drop(session);

        let events = Some(Arc::clone(&self.event_bus));
        fcp_queue::events::supervisor_left(&events, call_id, &supervisor_id, &tenant_id).await;

        info!(call_id = %call_id, supervisor = %supervisor_id, "Supervisor left call");
        Ok(())
    }

    /// Change the supervisor mode on an active supervised call.
    pub async fn change_supervisor_mode(
        &self,
        call_id: &str,
        new_mode: SupervisorMode,
    ) -> Result<()> {
        let mut session = self.sessions.get_mut(call_id).context("Call not found")?;

        if session.supervisor_mode.is_none() {
            bail!("No supervisor active on this call");
        }

        let old_mode = session.supervisor_mode.unwrap();
        if old_mode == new_mode {
            return Ok(());
        }

        // Abort old bridge tasks
        for task in session.supervisor_tasks.drain(..) {
            task.abort();
        }

        // Create new bridge with different mode
        let a_socket = Arc::clone(&session.rtp_socket);
        let b_socket = session
            .agent_rtp_socket
            .as_ref()
            .context("No agent RTP socket")?;
        let b_socket = Arc::clone(b_socket);
        let c_socket = session
            .supervisor_rtp_socket
            .as_ref()
            .context("No supervisor RTP socket")?;
        let c_socket = Arc::clone(c_socket);
        let codecs = [
            session.codec_name.clone(),
            session
                .agent_codec_name
                .clone()
                .unwrap_or_else(|| session.codec_name.clone()),
            session.codec_name.clone(),
        ];

        let tasks = spawn_supervisor_bridge(a_socket, b_socket, c_socket, new_mode, codecs);
        session.supervisor_tasks = tasks;
        session.supervisor_mode = Some(new_mode);

        let supervisor_id = session.supervisor_id.clone().unwrap_or_default();
        drop(session);

        info!(
            call_id = %call_id,
            supervisor = %supervisor_id,
            old_mode = %old_mode,
            new_mode = %new_mode,
            "Supervisor mode changed"
        );
        Ok(())
    }

    /// Route a call to the overflow destination using B2BUA bridging.
    /// Keeps the caller's A-leg alive and INVITEs the overflow target.
    async fn overflow_call(&self, call_id: &str, overflow_dest: &str, reason: &str) -> Result<()> {
        // Queue chaining: if destination is "queue:<name>", transfer to another queue
        if let Some(target_queue) = overflow_dest.strip_prefix("queue:") {
            let from_queue = {
                let session = self.sessions.get(call_id).context("Session not found")?;
                session.queue_name.clone()
            };
            let position = self
                .queue_manager
                .transfer_to_queue(call_id, &from_queue, target_queue)
                .await?;

            // Update session's queue_name
            if let Some(mut session) = self.sessions.get_mut(call_id) {
                session.queue_name = target_queue.to_string();
                session.state = QueueCallState::WaitingInQueue;
            }

            info!(
                call_id = %call_id,
                from = %from_queue,
                to = %target_queue,
                position = position,
                reason = %reason,
                "Queue overflow: transferred to another queue"
            );
            return Ok(());
        }

        let (codec_name, caller_rtp, queue_name, tenant_id) = {
            let session = self
                .sessions
                .get(call_id)
                .context("Session not found for overflow")?;
            (
                session.codec_name.clone(),
                Arc::clone(&session.rtp_socket),
                session.queue_name.clone(),
                session.tenant_id,
            )
        };

        let dest_uri = parse_overflow_destination(overflow_dest, &extract_domain(&self.local_uri));

        // Allocate B-leg RTP port pair
        let b_port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port for overflow leg")?;

        // Build SDP offer for overflow target
        let sdp_offer = build_sdp_offer(&self.local_addr, b_port_pair.rtp_port, &codec_name);

        // INVITE the overflow destination
        let call_handle = match self.uac.invite(&dest_uri, Some(&sdp_offer)).await {
            Ok(h) => h,
            Err(e) => {
                self.port_pool.deallocate(b_port_pair).await;
                return Err(anyhow::anyhow!("Failed to INVITE overflow dest: {}", e));
            }
        };

        // Wait for final response (with a generous timeout)
        let result = tokio::time::timeout(Duration::from_secs(30), call_handle.await_final()).await;

        match result {
            Ok(Ok(response))
                if response.start_line().code() >= 200 && response.start_line().code() < 300 =>
            {
                // Overflow target answered
                info!(
                    call_id = %call_id,
                    dest = %dest_uri,
                    reason = %reason,
                    "Overflow destination answered"
                );

                let agent_body = response.body();
                let agent_sdp = std::str::from_utf8(&agent_body[..]).unwrap_or("");

                // Create B-leg RTP socket
                let b_rtp_socket =
                    match RtpSocketPair::new(b_port_pair, RtpSocketConfig::default()).await {
                        Ok(s) => Arc::new(s),
                        Err(e) => {
                            self.port_pool.deallocate(b_port_pair).await;
                            return Err(anyhow::anyhow!(
                                "Failed to create overflow RTP socket: {}",
                                e
                            ));
                        }
                    };

                // Set remote RTP endpoint
                if let Some(remote_addr) = Self::parse_remote_rtp_addr(agent_sdp) {
                    b_rtp_socket
                        .set_remote_endpoint(RemoteEndpoint {
                            rtp_addr: remote_addr,
                            rtcp_addr: None,
                        })
                        .await;
                }

                let overflow_dialog = call_handle.dialog.read().await.clone();

                // Start RTP bridge and update session
                if let Some(mut session) = self.sessions.get_mut(call_id) {
                    // Stop MOH if running
                    if let Some(moh) = session.moh_handle.take() {
                        moh.abort();
                    }

                    let (bridge_a_to_b, bridge_b_to_a) =
                        spawn_rtp_bridge(Arc::clone(&caller_rtp), Arc::clone(&b_rtp_socket));

                    session.state = QueueCallState::OverflowedToDestination;
                    session.agent_port_pair = Some(b_port_pair);
                    session.agent_rtp_socket = Some(b_rtp_socket);
                    session.agent_dialog = Some(overflow_dialog);
                    session.bridge_a_to_b = Some(bridge_a_to_b);
                    session.bridge_b_to_a = Some(bridge_b_to_a);
                    session.ringing_agent = None;
                    session.ring_task_handle = None;
                }

                // Dequeue from QueueManager
                self.queue_manager.dequeue_call(call_id);

                // Publish overflow event
                fcp_queue::events::caller_overflowed(
                    &Some(Arc::clone(&self.event_bus)),
                    &queue_name,
                    call_id,
                    overflow_dest,
                    reason,
                    &tenant_id,
                )
                .await;

                Ok(())
            }
            _ => {
                // Overflow target rejected or timed out
                self.port_pool.deallocate(b_port_pair).await;
                Err(anyhow::anyhow!(
                    "Overflow destination {} did not answer",
                    dest_uri
                ))
            }
        }
    }

    /// Try a list of overflow destinations in order. Returns Ok on first success.
    /// If all fail, sends BYE to the caller and cleans up the session.
    async fn overflow_call_multi(&self, call_id: &str, destinations: &[String], reason: &str) {
        for (i, dest) in destinations.iter().enumerate() {
            info!(
                call_id = %call_id,
                dest = %dest,
                attempt = i + 1,
                total = destinations.len(),
                reason = %reason,
                "Trying overflow destination"
            );
            match self.overflow_call(call_id, dest, reason).await {
                Ok(()) => return,
                Err(e) => {
                    warn!(
                        call_id = %call_id,
                        dest = %dest,
                        error = %e,
                        remaining = destinations.len() - i - 1,
                        "Overflow destination failed, trying next"
                    );
                }
            }
        }
        // All destinations exhausted — BYE the caller
        warn!(
            call_id = %call_id,
            reason = %reason,
            "All overflow destinations exhausted, sending BYE"
        );
        self.remove_session(call_id);
    }
}

#[async_trait::async_trait]
impl fcp_sip::queue::QueueHandler for QueueCallHandler {
    fn extension_of(&self, tenant_id: &fcp_core::TenantId, queue: &str) -> Option<String> {
        if !self.queue_manager.is_enabled() {
            return None;
        }
        if let Some(config) = self
            .queue_manager
            .get_queue_config_for_tenant(queue, tenant_id)
        {
            return Some(config.extension);
        }
        self.queue_manager
            .get_queue_for_extension_in_tenant(tenant_id, queue)
            .map(|_| queue.to_string())
    }

    async fn can_handle(&self, request: &sip_core::Request) -> bool {
        if !self.queue_manager.is_enabled() {
            return false;
        }
        let tenant_id = match self.resolve_tenant(request).await {
            Ok(tenant_id) => tenant_id,
            Err(_) => return false,
        };
        let extension = match Self::extract_extension(request) {
            Some(ext) => ext,
            None => return false,
        };
        self.queue_manager
            .get_queue_for_extension_in_tenant(&tenant_id, &extension)
            .is_some()
    }

    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        _peer: SocketAddr,
        b2bua_call_id: fcp_sip::CallId,
    ) -> Result<()> {
        // 1. Extract extension and look up queue
        let extension = Self::extract_extension(request)
            .context("Failed to extract extension from Request-URI")?;
        let call_id = Self::extract_call_id(request);
        let tenant_id = match self.resolve_tenant(request).await {
            Ok(tenant_id) => tenant_id,
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Rejecting queue INVITE: tenant resolution failed");
                let status = if e.to_string().contains("Unknown tenant domain")
                    || e.to_string().contains("tenant context required")
                {
                    403
                } else {
                    503
                };
                let reason = if e.to_string().contains("Unknown tenant domain") {
                    "Unknown tenant domain"
                } else if status == 403 {
                    "Tenant context required"
                } else {
                    "Tenant lookup unavailable"
                };
                let response = sip_uas::UserAgentServer::create_response(request, status, reason);
                handle.send_final(response.clone()).await;
                self.refuse_call(b2bua_call_id, &response);
                return Ok(());
            }
        };
        let queue_name = self
            .queue_manager
            .get_queue_for_extension_in_tenant(&tenant_id, &extension)
            .context("No queue found for extension")?;
        let (caller_display, caller_uri) = Self::extract_caller_info(request);

        info!(
            call_id = %call_id,
            queue = %queue_name,
            caller = %caller_uri,
            "Queue INVITE received"
        );

        // 1b. Check if queue is open (schedule check)
        if !self.queue_manager.is_queue_open(&queue_name) {
            if let Some(dest) = self.queue_manager.get_closed_destination(&queue_name) {
                info!(
                    call_id = %call_id,
                    queue = %queue_name,
                    dest = %dest,
                    "Queue is closed, routing to closed destination"
                );
                // Return closed error with destination info — caller (B2BUA) will handle routing
                bail!("Queue '{}' is closed; route to {}", queue_name, dest);
            } else {
                info!(
                    call_id = %call_id,
                    queue = %queue_name,
                    "Queue is closed, no destination configured, rejecting"
                );
                let reject =
                    UserAgentServer::create_response(request, 480, "Temporarily Unavailable");
                handle.send_final(reject.clone()).await;
                self.refuse_call(b2bua_call_id, &reject);
                return Ok(());
            }
        }

        // 1c. Check if queue is draining
        if self.queue_manager.is_queue_draining(&queue_name) {
            if let Some(dest) = self.queue_manager.get_closed_destination(&queue_name) {
                info!(
                    call_id = %call_id,
                    queue = %queue_name,
                    dest = %dest,
                    "Queue is draining, routing to closed destination"
                );
                bail!("Queue '{}' is draining; route to {}", queue_name, dest);
            } else {
                info!(
                    call_id = %call_id,
                    queue = %queue_name,
                    "Queue is draining, no destination configured, rejecting"
                );
                let reject =
                    UserAgentServer::create_response(request, 480, "Temporarily Unavailable");
                handle.send_final(reject.clone()).await;
                self.refuse_call(b2bua_call_id, &reject);
                return Ok(());
            }
        }

        // 2. Extract and parse SDP offer
        // The SDP alone or the SDP part of a multipart body.
        let Some(sdp_offer) = fcp_sip::body::request_sdp(request) else {
            let reject = UserAgentServer::create_response(request, 400, "Missing SDP");
            handle.send_final(reject.clone()).await;
            self.refuse_call(b2bua_call_id, &reject);
            return Ok(());
        };
        let sdp_offer = sdp_offer.as_str();

        // 3. Parse codecs and pick first supported one
        let offered_codecs = parse_sdp_codecs(sdp_offer);
        let (codec_name, payload_type, frame_size) = match Self::pick_codec(&offered_codecs) {
            Some(c) => c,
            None => {
                let reject = UserAgentServer::create_response(request, 488, "Not Acceptable Here");
                handle.send_final(reject.clone()).await;
                self.refuse_call(b2bua_call_id, &reject);
                return Ok(());
            }
        };

        debug!(
            call_id = %call_id,
            queue = %queue_name,
            codec = %codec_name,
            pt = payload_type,
            "Negotiated codec for queue caller"
        );

        // 4. Send 180 Ringing
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri.clone());
        let ringing = uas.create_ringing(request);
        handle.send_provisional(ringing).await;

        // 5. Allocate RTP port pair, bound before it is answered on
        let (port_pair, rtp_socket) = fcp_media::bound_port_pair(&self.port_pool)
            .await
            .context("Failed to allocate RTP port pair")?;
        let rtp_socket = Arc::new(rtp_socket);

        // 6. Construct SDP answer
        let sdp_answer = construct_answer_for_a_leg(
            sdp_offer,
            &self.local_addr,
            port_pair.rtp_port,
            None, // No SRTP for queue calls (future enhancement)
        );

        // 7. Create 200 OK with SDP answer
        let (response, dialog) = uas
            .accept_invite(request, Some(&sdp_answer))
            .context("Failed to create 200 OK")?;

        // 8. Send 200 OK
        handle.send_final(response).await;
        // The call is answered, by the queue.
        let _ = self
            .call_manager
            .update_call(&b2bua_call_id, |call| call.establish());

        // 10. Set remote RTP endpoint from caller's SDP
        if let Some(remote_addr) = Self::parse_remote_rtp_addr(sdp_offer) {
            rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;

            debug!(
                call_id = %call_id,
                remote = %remote_addr,
                local_rtp_port = port_pair.rtp_port,
                "RTP endpoints configured for queue caller"
            );
        } else {
            warn!(call_id = %call_id, "Could not parse remote RTP address from SDP offer");
        }

        // 10b. Play entry prompt if configured
        if let Some(ref config) = self.queue_manager.get_queue_config(&queue_name) {
            if let Some(ref prompt_name) = config.entry_prompt {
                if let Some(ref prompt_cache) = self.prompt_cache {
                    if let Some(frames) = prompt_cache.get_frames(prompt_name, &codec_name) {
                        debug!(call_id = %call_id, prompt = %prompt_name, "Playing entry prompt");
                        play_prompt_frames(&rtp_socket, frames, payload_type).await;
                    }
                }
            }
        }

        // 11. Start MOH playback (with position announcements if configured)
        let moh_frames = self.resolve_moh_frames(&queue_name, &codec_name);
        let moh_handle = Self::spawn_moh(
            Arc::clone(&rtp_socket),
            &codec_name,
            payload_type,
            call_id.clone(),
            self.build_announce_config(&queue_name),
            moh_frames.clone(),
        );

        // 12. Enqueue call in QueueManager
        let queued_call = QueuedCall {
            id: uuid::Uuid::new_v4().to_string(),
            call_id: call_id.clone(),
            queue_name: queue_name.clone(),
            tenant_id,
            caller_uri: caller_uri.clone(),
            caller_display: caller_display.clone(),
            priority: 0,
            original_priority: 0,
            enqueued_at: Instant::now(),
            position: 0,
            attempts: 0,
            is_callback: false,
            callback_uri: None,
            callback_attempts: 0,
            last_callback_attempt: None,
        };

        match self.queue_manager.enqueue_call(queued_call).await {
            Ok(position) => {
                info!(
                    call_id = %call_id,
                    queue = %queue_name,
                    position = position,
                    "Caller enqueued"
                );
            }
            Err(e) => {
                // Queue full — try overflow destinations before disconnecting
                let overflow_dests = self.queue_manager.get_overflow_destinations(&queue_name);

                if !overflow_dests.is_empty() {
                    info!(
                        call_id = %call_id,
                        queue = %queue_name,
                        destinations = ?overflow_dests,
                        "Queue full, routing to overflow destinations"
                    );

                    // Store session so overflow_call can access it
                    self.sessions.insert(
                        call_id.clone(),
                        QueueSession {
                            call_id: call_id.clone(),
                            queue_name: queue_name.clone(),
                            tenant_id,
                            caller_uri,
                            caller_display,
                            port_pair,
                            rtp_socket,
                            codec_name,
                            payload_type,
                            _frame_size: frame_size,
                            dialog,
                            moh_handle: Some(moh_handle),
                            moh_frames: moh_frames.clone(),
                            dtmf_listener_handle: None,
                            agent_port_pair: None,
                            agent_rtp_socket: None,
                            agent_dialog: None,
                            bridge_a_to_b: None,
                            bridge_b_to_a: None,
                            recording_handle: None,
                            recording_path: None,
                            recording_consent: None,
                            supervisor_port_pair: None,
                            supervisor_rtp_socket: None,
                            supervisor_dialog: None,
                            supervisor_mode: None,
                            supervisor_id: None,
                            supervisor_tasks: Vec::new(),
                            state: QueueCallState::WaitingInQueue,
                            is_campaign_call: false,
                            ringing_agent: None,
                            ring_task_handle: None,
                            ring_cancel_tx: None,
                            enqueued_at: Instant::now(),
                            agent_answered_at: None,
                            agent_codec_name: None,
                            b2bua_call_id: Some(b2bua_call_id),
                        },
                    );

                    self.overflow_call_multi(&call_id, &overflow_dests, "queue_full")
                        .await;
                } else {
                    warn!(
                        call_id = %call_id,
                        queue = %queue_name,
                        error = %e,
                        "Queue full, no overflow destination, sending BYE"
                    );
                    moh_handle.abort();
                    let uac = Arc::clone(&self.uac);
                    let bye_dialog = dialog.clone();
                    tokio::spawn(async move {
                        let _ = uac.bye(&bye_dialog).await;
                    });
                    let pool = Arc::clone(&self.port_pool);
                    tokio::spawn(async move { pool.deallocate(port_pair).await });
                }
                return Ok(());
            }
        }

        // 13. Spawn DTMF listener if the queue has exit options or callback
        let queue_config = self.queue_manager.get_queue_config(&queue_name);
        let has_dtmf = queue_config
            .as_ref()
            .map(|c| {
                !c.dtmf_exit_options.is_empty()
                    || (c.callback_enabled && c.callback_digit.is_some())
            })
            .unwrap_or(false);

        let dtmf_listener_handle = if has_dtmf {
            let te_pt = extract_telephone_event_pt(sdp_offer).unwrap_or(101);
            let qc = queue_config.unwrap();
            Some(spawn_dtmf_listener(
                call_id.clone(),
                Arc::clone(&rtp_socket),
                te_pt,
                qc.dtmf_exit_options.clone(),
                qc.callback_enabled,
                qc.callback_digit.clone(),
                caller_uri.clone(),
                queue_name.clone(),
                Arc::clone(&self.sessions),
                self.queue_manager.clone(),
                Arc::clone(&self.uac),
                Arc::clone(&self.port_pool),
                self.local_addr.clone(),
                self.local_uri.clone(),
                Arc::clone(&self.event_bus),
                self.default_tenant_id,
            ))
        } else {
            None
        };

        // 14. Store session
        self.sessions.insert(
            call_id.clone(),
            QueueSession {
                call_id,
                queue_name,
                tenant_id,
                caller_uri,
                caller_display,
                port_pair,
                rtp_socket,
                codec_name,
                payload_type,
                _frame_size: frame_size,
                dialog,
                moh_handle: Some(moh_handle),
                moh_frames,
                dtmf_listener_handle,
                agent_port_pair: None,
                agent_rtp_socket: None,
                agent_dialog: None,
                bridge_a_to_b: None,
                bridge_b_to_a: None,
                recording_handle: None,
                recording_path: None,
                recording_consent: None,
                supervisor_port_pair: None,
                supervisor_rtp_socket: None,
                supervisor_dialog: None,
                supervisor_mode: None,
                supervisor_id: None,
                supervisor_tasks: Vec::new(),
                state: QueueCallState::WaitingInQueue,
                is_campaign_call: false,
                ringing_agent: None,
                ring_task_handle: None,
                ring_cancel_tx: None,
                enqueued_at: Instant::now(),
                agent_answered_at: None,
                agent_codec_name: None,
                b2bua_call_id: Some(b2bua_call_id),
            },
        );

        Ok(())
    }

    fn handle_bye(&self, call_id: &str) -> Result<()> {
        if self.sessions.contains_key(call_id) {
            self.remove_session(call_id);
            // For campaign calls, also clean up the other leg
            if let Some((_, other_call_id)) = self.campaign_call_id_map.remove(call_id) {
                self.campaign_call_id_map.remove(&other_call_id);
                if self.sessions.contains_key(&other_call_id) {
                    self.remove_session(&other_call_id);
                }
            }
            Ok(())
        } else {
            // Check if this BYE is for the agent leg of a campaign call
            if let Some((_, callee_call_id)) = self.campaign_call_id_map.remove(call_id) {
                self.campaign_call_id_map.remove(&callee_call_id);
                if self.sessions.contains_key(&callee_call_id) {
                    self.remove_session(&callee_call_id);
                }
                return Ok(());
            }
            anyhow::bail!("No queue session for call_id: {}", call_id)
        }
    }

    fn handle_cancel(&self, call_id: &str) -> Result<bool> {
        if self.sessions.contains_key(call_id) {
            info!(call_id = %call_id, "CANCEL received for queue caller");
            self.remove_session(call_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn agent_login(&self, extension: &str, tenant_id: &TenantId) -> Result<()> {
        // Find which queues this extension belongs to from config
        let config = self.queue_manager.get_system_config();
        // A phone dials in by its user's name; a queue may list that user
        // by name or by extension, or take its agents from a group
        // (docs/USERS.md §5).
        let user = self.user_of_extension(extension, tenant_id).await;
        let groups = match &user {
            Some(user) if config.queues.values().any(|qc| qc.agent_group_id.is_some()) => {
                self.groups_of_user(user).await
            }
            _ => Vec::new(),
        };
        let names: Vec<&str> = std::iter::once(extension)
            .chain(user.iter().map(|u| u.username.as_str()))
            .chain(user.iter().filter_map(|u| u.extension.as_deref()))
            .collect();
        let queue_names: Vec<String> = config
            .queues
            .iter()
            .filter(|(_, qc)| {
                qc.agents.iter().any(|a| names.contains(&a.as_str()))
                    || qc
                        .agent_group_id
                        .as_ref()
                        .is_some_and(|g| groups.contains(g))
            })
            .map(|(name, _)| name.clone())
            .collect();

        if queue_names.is_empty() {
            anyhow::bail!(
                "Extension '{}' is not configured as an agent in any queue",
                extension
            );
        }

        info!(
            extension = %extension,
            queues = ?queue_names,
            "Agent logging in via feature code"
        );

        // Look up skills from config for this agent extension
        let skills = config
            .queues
            .values()
            .find_map(|qc| qc.agent_skills.get(extension))
            .cloned()
            .unwrap_or_default();

        self.queue_manager
            .agent_login(extension, extension, &queue_names, *tenant_id, skills)
            .await
    }

    async fn agent_logout(&self, extension: &str) -> Result<()> {
        info!(extension = %extension, "Agent logging out via feature code");
        self.queue_manager.agent_logout(extension).await
    }
}

/// Spawn two tokio tasks that forward raw RTP packets between A-leg and B-leg sockets.
/// If `recording_tx` channels are provided, a copy of each packet's decoded audio
/// is sent to the recording task.
/// Spawn two tokio tasks that forward raw RTP packets between A-leg and B-leg sockets.
fn spawn_rtp_bridge(
    a_socket: Arc<RtpSocketPair>,
    b_socket: Arc<RtpSocketPair>,
) -> (JoinHandle<()>, JoinHandle<()>) {
    spawn_rtp_bridge_inner(a_socket, b_socket, None, None, None)
}

/// Spawn RTP bridge tasks forwarding packets between two legs in the same
/// codec. With recording channels, each direction's packets are copied to
/// the recorder before forwarding; with a notice tone (and the legs' codec),
/// its beep is written over the audio forwarded while it sounds.
fn spawn_rtp_bridge_inner(
    a_socket: Arc<RtpSocketPair>,
    b_socket: Arc<RtpSocketPair>,
    a_record_tx: Option<tokio::sync::broadcast::Sender<bytes::Bytes>>,
    b_record_tx: Option<tokio::sync::broadcast::Sender<bytes::Bytes>>,
    notice: Option<(Arc<NoticeTone>, forge_codecs::AudioCodecType)>,
) -> (JoinHandle<()>, JoinHandle<()>) {
    let a_to_b_a = Arc::clone(&a_socket);
    let a_to_b_b = Arc::clone(&b_socket);
    let b_to_a_a = Arc::clone(&a_socket);
    let b_to_a_b = Arc::clone(&b_socket);
    let mut a_to_b_notice = notice
        .clone()
        .map(|(tone, codec)| NoticeWriter::new(tone, codec, None));
    let mut b_to_a_notice = notice.map(|(tone, codec)| NoticeWriter::new(tone, codec, None));

    let a_to_b = tokio::spawn(async move {
        loop {
            match a_to_b_a.recv_rtp_raw().await {
                Ok((data, _addr)) => {
                    if data.is_empty() {
                        break;
                    }
                    if let Some(ref tx) = a_record_tx {
                        let _ = tx.send(data.clone());
                    }
                    let beeped = a_to_b_notice.as_mut().and_then(|n| n.apply(&data));
                    let out = beeped.as_deref().unwrap_or(&data);
                    if let Err(e) = a_to_b_b.send_rtp_raw(out).await {
                        debug!(error = %e, "A->B bridge send failed");
                        break;
                    }
                }
                Err(e) => {
                    debug!(error = %e, "A->B bridge recv failed");
                    break;
                }
            }
        }
    });

    let b_to_a = tokio::spawn(async move {
        loop {
            match b_to_a_b.recv_rtp_raw().await {
                Ok((data, _addr)) => {
                    if data.is_empty() {
                        break;
                    }
                    if let Some(ref tx) = b_record_tx {
                        let _ = tx.send(data.clone());
                    }
                    let beeped = b_to_a_notice.as_mut().and_then(|n| n.apply(&data));
                    let out = beeped.as_deref().unwrap_or(&data);
                    if let Err(e) = b_to_a_a.send_rtp_raw(out).await {
                        debug!(error = %e, "B->A bridge send failed");
                        break;
                    }
                }
                Err(e) => {
                    debug!(error = %e, "B->A bridge recv failed");
                    break;
                }
            }
        }
    });

    (a_to_b, b_to_a)
}

/// Spawn a transcoding RTP bridge between two sockets with different codecs.
///
/// Each direction decodes from the source codec, resamples if needed, and re-encodes
/// to the destination codec. The RTP payload type byte is rewritten for each direction.
/// Optional recording broadcast channels can capture raw packets for recording.
#[allow(clippy::too_many_arguments)]
fn spawn_transcoding_rtp_bridge(
    a_socket: Arc<RtpSocketPair>,
    b_socket: Arc<RtpSocketPair>,
    a_codec: forge_codecs::AudioCodecType,
    b_codec: forge_codecs::AudioCodecType,
    a_pt: u8,
    b_pt: u8,
    a_record_tx: Option<tokio::sync::broadcast::Sender<bytes::Bytes>>,
    b_record_tx: Option<tokio::sync::broadcast::Sender<bytes::Bytes>>,
    notice: Option<Arc<NoticeTone>>,
) -> (JoinHandle<()>, JoinHandle<()>) {
    // A→B direction: decode A's codec, encode to B's codec, rewrite PT to b_pt
    let a_to_b_a = Arc::clone(&a_socket);
    let a_to_b_b = Arc::clone(&b_socket);
    let mut a_to_b_notice = notice
        .clone()
        .map(|tone| NoticeWriter::new(tone, b_codec, Some(b_pt)));
    let mut b_to_a_notice = notice.map(|tone| NoticeWriter::new(tone, a_codec, Some(a_pt)));

    let a_to_b = tokio::spawn(async move {
        let src_format =
            forge_transcoder::AudioFormat::new(codec_sample_rate_for_type(a_codec), 1, a_codec);
        let dst_format =
            forge_transcoder::AudioFormat::new(codec_sample_rate_for_type(b_codec), 1, b_codec);
        let mut transcoder = match forge_transcoder::Transcoder::new(src_format, dst_format) {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "Failed to create A->B transcoder");
                return;
            }
        };

        loop {
            match a_to_b_a.recv_rtp_raw().await {
                Ok((data, _addr)) => {
                    if data.is_empty() || data.len() < 12 {
                        break;
                    }
                    // Send raw copy for recording if configured
                    if let Some(ref tx) = a_record_tx {
                        let _ = tx.send(data.clone());
                    }
                    // Extract RTP header and payload
                    let payload = &data[12..];
                    // Skip DTMF (telephone-event) packets
                    let rtp_pt = data[1] & 0x7F;
                    if rtp_pt == 101 {
                        // Forward DTMF as-is (telephone-event is codec-independent)
                        if let Err(e) = a_to_b_b.send_rtp_raw(&data).await {
                            debug!(error = %e, "A->B transcoding bridge send failed");
                            break;
                        }
                        continue;
                    }
                    // Transcode the payload
                    match transcoder.transcode(payload) {
                        Ok(transcoded) => {
                            // Rebuild RTP packet with new payload and rewritten PT
                            let mut out_packet = Vec::with_capacity(12 + transcoded.len());
                            out_packet.extend_from_slice(&data[..12]);
                            // Rewrite payload type (byte 1, lower 7 bits)
                            out_packet[1] = (data[1] & 0x80) | (b_pt & 0x7F);
                            out_packet.extend_from_slice(&transcoded);
                            if let Some(beeped) =
                                a_to_b_notice.as_mut().and_then(|n| n.apply(&out_packet))
                            {
                                out_packet = beeped;
                            }
                            if let Err(e) = a_to_b_b.send_rtp_raw(&out_packet).await {
                                debug!(error = %e, "A->B transcoding bridge send failed");
                                break;
                            }
                        }
                        Err(e) => {
                            debug!(error = %e, "A->B transcode failed, skipping packet");
                        }
                    }
                }
                Err(e) => {
                    debug!(error = %e, "A->B transcoding bridge recv failed");
                    break;
                }
            }
        }
    });

    // B→A direction: decode B's codec, encode to A's codec, rewrite PT to a_pt
    let b_to_a_b = Arc::clone(&b_socket);
    let b_to_a_a = Arc::clone(&a_socket);

    let b_to_a = tokio::spawn(async move {
        let src_format =
            forge_transcoder::AudioFormat::new(codec_sample_rate_for_type(b_codec), 1, b_codec);
        let dst_format =
            forge_transcoder::AudioFormat::new(codec_sample_rate_for_type(a_codec), 1, a_codec);
        let mut transcoder = match forge_transcoder::Transcoder::new(src_format, dst_format) {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "Failed to create B->A transcoder");
                return;
            }
        };

        loop {
            match b_to_a_b.recv_rtp_raw().await {
                Ok((data, _addr)) => {
                    if data.is_empty() || data.len() < 12 {
                        break;
                    }
                    if let Some(ref tx) = b_record_tx {
                        let _ = tx.send(data.clone());
                    }
                    let payload = &data[12..];
                    let rtp_pt = data[1] & 0x7F;
                    if rtp_pt == 101 {
                        if let Err(e) = b_to_a_a.send_rtp_raw(&data).await {
                            debug!(error = %e, "B->A transcoding bridge send failed");
                            break;
                        }
                        continue;
                    }
                    match transcoder.transcode(payload) {
                        Ok(transcoded) => {
                            let mut out_packet = Vec::with_capacity(12 + transcoded.len());
                            out_packet.extend_from_slice(&data[..12]);
                            out_packet[1] = (data[1] & 0x80) | (a_pt & 0x7F);
                            out_packet.extend_from_slice(&transcoded);
                            if let Some(beeped) =
                                b_to_a_notice.as_mut().and_then(|n| n.apply(&out_packet))
                            {
                                out_packet = beeped;
                            }
                            if let Err(e) = b_to_a_a.send_rtp_raw(&out_packet).await {
                                debug!(error = %e, "B->A transcoding bridge send failed");
                                break;
                            }
                        }
                        Err(e) => {
                            debug!(error = %e, "B->A transcode failed, skipping packet");
                        }
                    }
                }
                Err(e) => {
                    debug!(error = %e, "B->A transcoding bridge recv failed");
                    break;
                }
            }
        }
    });

    (a_to_b, b_to_a)
}

/// Get the native sample rate for a codec type.
fn codec_sample_rate_for_type(codec: forge_codecs::AudioCodecType) -> u32 {
    match codec {
        forge_codecs::AudioCodecType::G722 => 16000,
        _ => 8000,
    }
}

/// A queue recording to make when legs are connected.
struct RecordingPlan<'a> {
    recordings: &'a QueueRecordings,
    tenant_id: TenantId,
    /// The call id the row and the segment's record carry.
    call_id: String,
    format: String,
}

/// What connecting a caller and an agent started.
struct Bridge {
    a_to_b: JoinHandle<()>,
    b_to_a: JoinHandle<()>,
    recording: Option<JoinHandle<()>>,
    recording_path: Option<String>,
    /// What the parties were told, when the queue records; on the record.
    recording_consent: Option<fcp_core::RecordingConsent>,
}

impl Bridge {
    /// The caller left before the bridge was stored: stop it, and close the
    /// recording's row.
    async fn abandon(self, recordings: Option<&QueueRecordings>) {
        self.a_to_b.abort();
        self.b_to_a.abort();
        if let Some(recording) = self.recording {
            recording.abort();
        }
        let id = self.recording_path.as_deref().and_then(recording_id_of);
        if let (Some(store), Some(id)) = (recordings.and_then(|r| r.store.as_ref()), id) {
            if let Err(e) = store
                .finish_call_recording(&id, std::time::SystemTime::now())
                .await
            {
                warn!(error = %e, "Could not close an abandoned queue recording's row");
            }
        }
    }
}

/// The recording id a queue recording's file is named for.
fn recording_id_of(path: &str) -> Option<fcp_core::RecordingId> {
    std::path::Path::new(path)
        .file_stem()?
        .to_str()?
        .parse::<fcp_core::RecordingId>()
        .ok()
}

/// Connect `caller` and `agent` with an RTP bridge. When the queue records
/// the call, its consent step comes first — both legs hear the tenant's
/// announcement before they hear each other, and a refused announcement
/// records nothing — then the recording's row, the mixed recorder and the
/// notice tone (`docs/COMPLIANCE_FOLLOWUPS.md` §8.2). `None` when
/// `still_here` says the call went away during the announcement.
async fn connect_legs(
    caller: &Leg,
    agent: &Leg,
    recording: Option<RecordingPlan<'_>>,
    still_here: impl Fn() -> bool,
) -> Option<Bridge> {
    let mut consent = None;
    let mut path = None;
    let mut notice = None;
    if let Some(plan) = recording {
        let step =
            crate::queue_recording::consent_step(plan.recordings, &plan.tenant_id, caller, agent)
                .await;
        if !still_here() {
            return None;
        }
        if step.records() {
            path = plan
                .recordings
                .begin(plan.tenant_id, plan.call_id, &plan.format, step.consent)
                .await;
            if path.is_some() {
                consent = Some(step.consent);
                notice = step.notice;
            }
        } else {
            consent = Some(step.consent);
        }
    }

    let channels = path.as_ref().map(|_| {
        (
            tokio::sync::broadcast::channel::<bytes::Bytes>(256),
            tokio::sync::broadcast::channel::<bytes::Bytes>(256),
        )
    });
    let (senders, receivers) = match channels {
        Some(((a_tx, a_rx), (b_tx, b_rx))) => (Some((a_tx, b_tx)), Some((a_rx, b_rx))),
        None => (None, None),
    };
    let (a_tx, b_tx) = match senders {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };
    let caller_codec = codec_name_to_type(&caller.codec);
    let agent_codec = codec_name_to_type(&agent.codec);
    let (a_to_b, b_to_a) = if caller.codec != agent.codec {
        spawn_transcoding_rtp_bridge(
            Arc::clone(&caller.socket),
            Arc::clone(&agent.socket),
            caller_codec,
            agent_codec,
            caller.payload_type,
            agent.payload_type,
            a_tx,
            b_tx,
            notice,
        )
    } else {
        spawn_rtp_bridge_inner(
            Arc::clone(&caller.socket),
            Arc::clone(&agent.socket),
            a_tx,
            b_tx,
            notice.map(|tone| (tone, caller_codec)),
        )
    };
    let recording = match (receivers, &path) {
        (Some((a_rx, b_rx)), Some(path)) => Some(crate::queue_recording::spawn_mixed_recording(
            a_rx,
            b_rx,
            caller,
            agent,
            path.clone(),
        )),
        _ => None,
    };
    Some(Bridge {
        a_to_b,
        b_to_a,
        recording,
        recording_path: path,
        recording_consent: consent,
    })
}

/// The queue call's media with a supervisor on it
/// (`docs/ENTERPRISE_CALLING.md` §3.11): the caller (A), the agent (B) and
/// the supervisor (C), each on a socket of its own. Every packet is decoded
/// into forge-mixer's [`SupervisionMix`], the core forge-engine's third
/// party uses too; a leg that does not hear the supervisor keeps the other
/// leg's packets relayed as they came, and every 20 ms the supervisor is
/// sent both legs and a leg that hears the supervisor (the agent on a
/// whisper, both on a barge) is sent the other leg with the supervisor
/// summed in, each on an RTP stream of its own in its own codec.
fn spawn_supervisor_bridge(
    a_socket: Arc<RtpSocketPair>,
    b_socket: Arc<RtpSocketPair>,
    c_socket: Arc<RtpSocketPair>,
    mode: SupervisorMode,
    codecs: [String; 3],
) -> Vec<JoinHandle<()>> {
    use forge_mixer::{CallParty, SupervisionMix, SupervisionMode, Voice};
    let mode = match mode {
        SupervisorMode::Monitor => SupervisionMode::Monitor,
        // The agent is coached; the caller never hears it.
        SupervisorMode::Whisper => SupervisionMode::Whisper(CallParty::B),
        SupervisorMode::Barge => SupervisionMode::Barge,
    };
    let codecs = codecs.map(|name| codec_name_to_type(&name.to_ascii_uppercase()));
    let mix = Arc::new(std::sync::Mutex::new(SupervisionMix::new(
        mode,
        SUPERVISION_FRAME,
    )));
    let mut tasks = Vec::new();
    let sources = [
        (
            Voice::A,
            Arc::clone(&a_socket),
            codecs[0],
            (!mode.heard_by(CallParty::B)).then(|| Arc::clone(&b_socket)),
        ),
        (
            Voice::B,
            Arc::clone(&b_socket),
            codecs[1],
            (!mode.heard_by(CallParty::A)).then(|| Arc::clone(&a_socket)),
        ),
        (Voice::Supervisor, Arc::clone(&c_socket), codecs[2], None),
    ];
    for (voice, socket, codec, relay_to) in sources {
        let mix = Arc::clone(&mix);
        tasks.push(tokio::spawn(async move {
            let mut decoder = supervision_codec(codec);
            loop {
                let data = match socket.recv_rtp_raw().await {
                    Ok((data, _)) if !data.is_empty() => data,
                    _ => break,
                };
                if let Some(dest) = &relay_to {
                    if dest.send_rtp_raw(&data).await.is_err() {
                        break;
                    }
                }
                if data.len() <= 12 || data[1] & 0x7f == 101 {
                    continue;
                }
                let Some(pcm) = decoder.as_mut().and_then(|d| d.decode(&data[12..]).ok()) else {
                    continue;
                };
                let pcm = resample_linear(&pcm, codec_sample_rate_for_type(codec), 8000);
                mix.lock().expect("mix").write(voice, &pcm);
            }
        }));
    }
    let destinations = [
        (Some(CallParty::A), a_socket, codecs[0]),
        (Some(CallParty::B), b_socket, codecs[1]),
        (None, c_socket, codecs[2]),
    ];
    tasks.push(tokio::spawn(async move {
        let mut streams: Vec<Destination> = destinations
            .into_iter()
            .map(|(party, socket, codec)| Destination {
                party,
                socket,
                codec,
                encoder: supervision_codec(codec),
                stream: SupervisionStream::new(),
            })
            .collect();
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(20));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let frame = mix.lock().expect("mix").tick();
            for Destination {
                party,
                socket,
                codec,
                encoder,
                stream,
            } in streams.iter_mut()
            {
                let samples = match party {
                    None => Some(frame.supervisor.clone()),
                    Some(CallParty::A) => frame.a.clone(),
                    Some(CallParty::B) => frame.b.clone(),
                };
                let (Some(samples), Some(encoder)) = (samples, encoder.as_mut()) else {
                    continue;
                };
                let rate = codec_sample_rate_for_type(*codec);
                let samples = resample_linear(&samples, 8000, rate);
                let Ok(payload) = encoder.encode(&samples) else {
                    continue;
                };
                // G.722's RTP clock is 8 kHz whatever its sample rate.
                let packet =
                    stream.packet(payload_type_for(*codec), SUPERVISION_FRAME as u32, &payload);
                if socket.send_rtp_raw(&packet).await.is_err() {
                    return;
                }
            }
        }
    }));
    tasks
}

/// One leg the supervision bridge sends to: which listener it is (`None`
/// the supervisor), its socket and codec, and its own RTP stream.
struct Destination {
    party: Option<forge_mixer::CallParty>,
    socket: Arc<RtpSocketPair>,
    codec: forge_codecs::AudioCodecType,
    encoder: Option<Box<dyn forge_codecs::AudioCodec>>,
    stream: SupervisionStream,
}

/// Samples per 20 ms frame of the supervision mix (8 kHz).
const SUPERVISION_FRAME: usize = 160;

/// A codec for one stream of a supervised call; `None` for one this bridge
/// cannot decode.
fn supervision_codec(
    codec: forge_codecs::AudioCodecType,
) -> Option<Box<dyn forge_codecs::AudioCodec>> {
    forge_codecs::create_codec(&forge_codecs::AudioFormat::new(
        codec_sample_rate_for_type(codec),
        1,
        codec,
    ))
    .ok()
}

/// The static payload type of a codec the queue offers.
fn payload_type_for(codec: forge_codecs::AudioCodecType) -> u8 {
    match codec {
        forge_codecs::AudioCodecType::PCMA => 8,
        forge_codecs::AudioCodecType::G722 => 9,
        _ => 0,
    }
}

/// Linear resampling, enough for speech between 8 and 16 kHz.
fn resample_linear(samples: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let out_len = (samples.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * f64::from(from) / f64::from(to);
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(samples.len() - 1);
            let frac = pos - lo as f64;
            let lo = samples[lo.min(samples.len() - 1)] as f64;
            (lo + (samples[hi] as f64 - lo) * frac) as i16
        })
        .collect()
}

/// An RTP stream this bridge generates toward one leg.
struct SupervisionStream {
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
    started: bool,
}

impl SupervisionStream {
    fn new() -> Self {
        let random = uuid::Uuid::new_v4().as_u128();
        Self {
            ssrc: random as u32,
            sequence: (random >> 32) as u16,
            timestamp: (random >> 48) as u32,
            started: false,
        }
    }

    fn packet(&mut self, payload_type: u8, clock_step: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + payload.len());
        out.push(0x80);
        let marker = if self.started { 0 } else { 0x80 };
        self.started = true;
        out.push(marker | (payload_type & 0x7f));
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.timestamp.to_be_bytes());
        out.extend_from_slice(&self.ssrc.to_be_bytes());
        out.extend_from_slice(payload);
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(clock_step);
        out
    }
}

/// Build a single-codec SDP offer (used for supervisor, overflow, and callback legs).
fn build_sdp_offer(local_addr: &str, rtp_port: u16, codec_name: &str) -> String {
    let ip = local_addr.split(':').next().unwrap_or(local_addr);

    let (pt, encoding, clock_rate): (u8, &str, u32) = match codec_name {
        "PCMU" => (0, "PCMU", 8000),
        "PCMA" => (8, "PCMA", 8000),
        "G722" => (9, "G722", 8000),
        _ => (0, "PCMU", 8000),
    };

    format!(
        "v=0\r\n\
         o=fcp-queue 0 0 IN IP4 {ip}\r\n\
         s=Queue Call\r\n\
         c=IN IP4 {ip}\r\n\
         t=0 0\r\n\
         m=audio {rtp_port} RTP/AVP {pt} 101\r\n\
         a=rtpmap:{pt} {encoding}/{clock_rate}\r\n\
         a=rtpmap:101 telephone-event/8000\r\n\
         a=fmtp:101 0-16\r\n\
         a=sendrecv\r\n\
         a=ptime:20\r\n",
    )
}

/// Build a multi-codec SDP offer for the agent B-leg.
///
/// Offers all three supported codecs (PCMU, PCMA, G722) so the agent device
/// can choose whichever it supports. Transcoding is handled transparently
/// if the agent picks a different codec than the caller.
fn build_multi_codec_sdp_offer(local_addr: &str, rtp_port: u16) -> String {
    let ip = local_addr.split(':').next().unwrap_or(local_addr);

    format!(
        "v=0\r\n\
         o=fcp-queue 0 0 IN IP4 {ip}\r\n\
         s=Queue Call\r\n\
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
    )
}

/// Parse the agent's chosen codec from their SDP answer.
/// Returns (codec_name, payload_type). Defaults to ("PCMU", 0) if parsing fails.
fn parse_agent_codec(sdp: &str) -> (String, u8) {
    let codecs = parse_sdp_codecs(sdp);
    for codec in &codecs {
        // Skip telephone-event
        let name_upper = codec.encoding_name.to_uppercase();
        if name_upper == "TELEPHONE-EVENT" {
            continue;
        }
        return (name_upper, codec.payload_type);
    }
    ("PCMU".to_string(), 0)
}

/// Map a codec name to forge_codecs::AudioCodecType.
pub(crate) fn codec_name_to_type(name: &str) -> forge_codecs::AudioCodecType {
    match name {
        "PCMU" => forge_codecs::AudioCodecType::PCMU,
        "PCMA" => forge_codecs::AudioCodecType::PCMA,
        "G722" => forge_codecs::AudioCodecType::G722,
        _ => forge_codecs::AudioCodecType::PCMU,
    }
}

/// Parse overflow destination string into a SIP URI.
/// Supports formats:
/// - "sip:user@host" — used as-is
/// - "extension:NNNN" — routed as sip:NNNN@local_domain
/// - bare digits "NNNN" — treated as extension
fn parse_overflow_destination(dest: &str, local_domain: &str) -> String {
    if dest.starts_with("sip:") || dest.starts_with("sips:") {
        dest.to_string()
    } else {
        let ext = dest.strip_prefix("extension:").unwrap_or(dest);
        // Also strip voicemail: prefix etc., treat as extension
        let ext = ext.strip_prefix("voicemail:").unwrap_or(ext);
        format!("sip:{}@{}", ext, local_domain)
    }
}

/// Spawn a callback ring task — two-stage B2BUA.
///
/// 1. INVITE the agent
/// 2. Agent answers → INVITE the callback URI (original caller)
/// 3. Callback answers → bridge agent ↔ callback RTP
///
/// If either fails, return the call to the queue and make the agent available.
#[allow(clippy::too_many_arguments)]
fn spawn_callback_ring(
    call_id: String,
    callback_uri: String,
    plan: fcp_queue::DistributionPlan,
    ring_timeout_secs: u32,
    sessions: Arc<DashMap<String, QueueSession>>,
    queue_manager: Arc<QueueManager>,
    port_pool: Arc<PortPool>,
    local_addr: String,
    local_uri: String,
    uac: Arc<SipUserAgentClient>,
    event_bus: Arc<dyn EventPublisher>,
    queue_name: String,
    queue_tenant_id: TenantId,
) {
    tokio::spawn(async move {
        let agent_id = plan.agent_id.clone();
        let agent_extension = plan.agent_extension.clone();
        let ring_timeout = Duration::from_secs(ring_timeout_secs as u64);
        let domain = extract_domain(&local_uri);
        let events = Some(Arc::clone(&event_bus));

        // Look up tenant_id from the queued call
        let tenant_id = queue_manager
            .get_queued_call(&call_id)
            .map(|c| c.tenant_id)
            .unwrap_or(queue_tenant_id);

        // 1. Allocate agent B-leg RTP ports
        let agent_port_pair = match port_pool.allocate().await {
            Ok(pp) => pp,
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Callback: failed to allocate agent RTP port");
                queue_manager.agent_no_answer(&agent_id, &call_id).await;
                return;
            }
        };

        // 2. INVITE the agent
        let agent_uri = format!("sip:{}@{}", agent_extension, domain);
        let sdp_offer = build_sdp_offer(&local_addr, agent_port_pair.rtp_port, "PCMU");

        let agent_handle = match uac.invite(&agent_uri, Some(&sdp_offer)).await {
            Ok(h) => h,
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Callback: failed to INVITE agent");
                port_pool.deallocate(agent_port_pair).await;
                queue_manager.agent_no_answer(&agent_id, &call_id).await;
                return;
            }
        };

        // 3. Wait for agent to answer (Stage 1 — agent no-answer IS the agent's fault)
        let agent_result = tokio::time::timeout(ring_timeout, agent_handle.await_final()).await;

        let agent_response = match agent_result {
            Ok(Ok(resp)) if resp.start_line().code() >= 200 && resp.start_line().code() < 300 => {
                resp
            }
            Ok(Ok(_)) | Ok(Err(_)) => {
                port_pool.deallocate(agent_port_pair).await;
                queue_manager.agent_no_answer(&agent_id, &call_id).await;
                return;
            }
            Err(_) => {
                let _ = agent_handle.cancel().await;
                port_pool.deallocate(agent_port_pair).await;
                queue_manager.agent_no_answer(&agent_id, &call_id).await;
                return;
            }
        };

        info!(call_id = %call_id, agent = %agent_id, "Callback: agent answered, calling back caller");

        // Create agent RTP socket
        let agent_rtp_socket = match RtpSocketPair::new(agent_port_pair, RtpSocketConfig::default())
            .await
        {
            Ok(s) => Arc::new(s),
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Callback: failed to create agent RTP socket");
                port_pool.deallocate(agent_port_pair).await;
                let _ = uac.bye(&agent_handle.dialog.read().await.clone()).await;
                queue_manager
                    .agent_callback_recipient_failed(&agent_id, &call_id)
                    .await;
                return;
            }
        };

        // Set agent remote endpoint
        let agent_body = agent_response.body();
        let agent_sdp = std::str::from_utf8(&agent_body[..]).unwrap_or("");
        if let Some(remote_addr) = QueueCallHandler::parse_remote_rtp_addr(agent_sdp) {
            agent_rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;
        }

        let agent_dialog = agent_handle.dialog.read().await.clone();

        // 4. Allocate callback C-leg RTP ports
        let callback_port_pair = match port_pool.allocate().await {
            Ok(pp) => pp,
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Callback: failed to allocate callback RTP port");
                let _ = uac.bye(&agent_dialog).await;
                port_pool.deallocate(agent_port_pair).await;
                queue_manager
                    .agent_callback_recipient_failed(&agent_id, &call_id)
                    .await;
                return;
            }
        };

        // 5. INVITE the callback URI (original caller)
        let cb_sdp = build_sdp_offer(&local_addr, callback_port_pair.rtp_port, "PCMU");
        let cb_dest = parse_overflow_destination(&callback_uri, &domain);

        let cb_handle = match uac.invite(&cb_dest, Some(&cb_sdp)).await {
            Ok(h) => h,
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Callback: failed to INVITE callback URI");
                let _ = uac.bye(&agent_dialog).await;
                port_pool.deallocate(agent_port_pair).await;
                port_pool.deallocate(callback_port_pair).await;
                // Stage 2 failure — don't penalize agent
                queue_manager
                    .agent_callback_recipient_failed(&agent_id, &call_id)
                    .await;
                let attempts = queue_manager
                    .get_queued_call(&call_id)
                    .map(|c| c.callback_attempts)
                    .unwrap_or(0);
                fcp_queue::events::callback_attempted(
                    &events,
                    &queue_name,
                    &call_id,
                    &callback_uri,
                    attempts,
                    &tenant_id,
                )
                .await;
                return;
            }
        };

        // 6. Wait for callback recipient to answer (Stage 2 — NOT the agent's fault)
        let cb_result =
            tokio::time::timeout(Duration::from_secs(30), cb_handle.await_final()).await;

        let cb_response = match cb_result {
            Ok(Ok(resp)) if resp.start_line().code() >= 200 && resp.start_line().code() < 300 => {
                resp
            }
            Ok(Ok(_)) | Ok(Err(_)) => {
                warn!(call_id = %call_id, "Callback: recipient did not answer");
                let _ = uac.bye(&agent_dialog).await;
                port_pool.deallocate(agent_port_pair).await;
                port_pool.deallocate(callback_port_pair).await;
                // Stage 2 failure — don't penalize agent
                queue_manager
                    .agent_callback_recipient_failed(&agent_id, &call_id)
                    .await;
                let attempts = queue_manager
                    .get_queued_call(&call_id)
                    .map(|c| c.callback_attempts)
                    .unwrap_or(0);
                fcp_queue::events::callback_attempted(
                    &events,
                    &queue_name,
                    &call_id,
                    &callback_uri,
                    attempts,
                    &tenant_id,
                )
                .await;
                return;
            }
            Err(_) => {
                let _ = cb_handle.cancel().await;
                let _ = uac.bye(&agent_dialog).await;
                port_pool.deallocate(agent_port_pair).await;
                port_pool.deallocate(callback_port_pair).await;
                // Stage 2 timeout — don't penalize agent
                queue_manager
                    .agent_callback_recipient_failed(&agent_id, &call_id)
                    .await;
                let attempts = queue_manager
                    .get_queued_call(&call_id)
                    .map(|c| c.callback_attempts)
                    .unwrap_or(0);
                fcp_queue::events::callback_attempted(
                    &events,
                    &queue_name,
                    &call_id,
                    &callback_uri,
                    attempts,
                    &tenant_id,
                )
                .await;
                return;
            }
        };

        info!(call_id = %call_id, callback_uri = %callback_uri, "Callback: recipient answered, bridging");

        // Create callback RTP socket
        let cb_rtp_socket = match RtpSocketPair::new(callback_port_pair, RtpSocketConfig::default())
            .await
        {
            Ok(s) => Arc::new(s),
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Callback: failed to create callback RTP socket");
                let _ = uac.bye(&agent_dialog).await;
                let _ = uac.bye(&cb_handle.dialog.read().await.clone()).await;
                port_pool.deallocate(agent_port_pair).await;
                port_pool.deallocate(callback_port_pair).await;
                queue_manager.agent_no_answer(&agent_id, &call_id).await;
                return;
            }
        };

        // Set callback remote endpoint
        let cb_body = cb_response.body();
        let cb_sdp_answer = std::str::from_utf8(&cb_body[..]).unwrap_or("");
        if let Some(remote_addr) = QueueCallHandler::parse_remote_rtp_addr(cb_sdp_answer) {
            cb_rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;
        }

        let cb_dialog = cb_handle.dialog.read().await.clone();

        // 7. Dequeue the call and update agent state
        let cb_attempts = queue_manager
            .get_queued_call(&call_id)
            .map(|c| c.callback_attempts)
            .unwrap_or(0);
        queue_manager.agent_answered(&agent_id, &call_id).await;

        // Publish CallbackCompleted event
        fcp_queue::events::callback_completed(
            &events,
            &queue_name,
            &call_id,
            &callback_uri,
            &agent_id,
            cb_attempts + 1,
            &tenant_id,
        )
        .await;

        // 8. Bridge agent ↔ callback RTP
        let (bridge_a_to_b, bridge_b_to_a) =
            spawn_rtp_bridge(Arc::clone(&agent_rtp_socket), Arc::clone(&cb_rtp_socket));

        // 9. Create a session to track this bridged call for cleanup on BYE
        sessions.insert(
            call_id.clone(),
            QueueSession {
                call_id: call_id.clone(),
                queue_name: String::new(), // No longer in a queue
                tenant_id,
                caller_uri: callback_uri,
                caller_display: None,
                port_pair: callback_port_pair,
                rtp_socket: cb_rtp_socket,
                codec_name: "PCMU".to_string(),
                payload_type: 0,
                _frame_size: 160,
                dialog: cb_dialog,
                moh_handle: None,
                moh_frames: None,
                dtmf_listener_handle: None,
                agent_port_pair: Some(agent_port_pair),
                agent_rtp_socket: Some(agent_rtp_socket),
                agent_dialog: Some(agent_dialog),
                bridge_a_to_b: Some(bridge_a_to_b),
                bridge_b_to_a: Some(bridge_b_to_a),
                recording_handle: None,
                recording_path: None,
                recording_consent: None,
                supervisor_port_pair: None,
                supervisor_rtp_socket: None,
                supervisor_dialog: None,
                supervisor_mode: None,
                supervisor_id: None,
                supervisor_tasks: Vec::new(),
                state: QueueCallState::Connected,
                is_campaign_call: false,
                ringing_agent: Some(agent_id),
                ring_task_handle: None,
                ring_cancel_tx: None,
                enqueued_at: Instant::now(),
                agent_answered_at: Some(Instant::now()),
                agent_codec_name: None,
                b2bua_call_id: None,
            },
        );
    });
}

/// Spawn a DTMF listener task that monitors the caller's RTP stream for
/// RFC 2833 telephone-event packets. When a digit matching `exit_options`
/// is detected, the call is routed to the configured destination.
/// When callback is enabled and the callback digit is pressed, the call
/// is converted to a callback request.
#[allow(clippy::too_many_arguments)]
fn spawn_dtmf_listener(
    call_id: String,
    rtp_socket: Arc<RtpSocketPair>,
    te_pt: u8,
    exit_options: std::collections::HashMap<String, String>,
    callback_enabled: bool,
    callback_digit: Option<String>,
    caller_uri: String,
    queue_name: String,
    sessions: Arc<DashMap<String, QueueSession>>,
    queue_manager: Arc<QueueManager>,
    uac: Arc<SipUserAgentClient>,
    port_pool: Arc<PortPool>,
    local_addr: String,
    local_uri: String,
    event_bus: Arc<dyn EventPublisher>,
    default_tenant_id: TenantId,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut detector = Rfc2833Detector::new(8000);

        loop {
            let (data, _addr) = match rtp_socket.recv_rtp_raw().await {
                Ok(d) => d,
                Err(_) => break,
            };

            // Minimum RTP packet: 12-byte header + payload
            if data.len() < 13 {
                continue;
            }

            // Check payload type (lower 7 bits of byte 1)
            let pt = data[1] & 0x7F;
            if pt != te_pt {
                continue;
            }

            // Extract RTP timestamp (bytes 4-7)
            let timestamp = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

            // Feed RFC 2833 payload (after 12-byte RTP header) to detector
            let payload = &data[12..];
            let events = match detector.process_with_timestamp(payload, timestamp) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for event in events {
                if event.event_type != forge_dtmf::DtmfEventType::End {
                    continue;
                }

                let digit_str = event.digit.to_string();
                debug!(
                    call_id = %call_id,
                    digit = %digit_str,
                    "DTMF digit detected in queue"
                );

                // Check callback digit first
                if callback_enabled {
                    if let Some(ref cb_digit) = callback_digit {
                        if digit_str == *cb_digit {
                            info!(
                                call_id = %call_id,
                                callback_uri = %caller_uri,
                                "Callback requested via DTMF"
                            );

                            // Mark call as callback in QueueManager (stays in queue)
                            queue_manager.mark_callback(&call_id, &caller_uri);

                            // Send BYE to caller and clean up session resources
                            if let Some((_, session)) = sessions.remove(&call_id) {
                                if let Some(moh) = session.moh_handle {
                                    moh.abort();
                                }
                                let bye_uac = Arc::clone(&uac);
                                let bye_dialog = session.dialog.clone();
                                tokio::spawn(async move {
                                    let _ = bye_uac.bye(&bye_dialog).await;
                                });
                                let pool = Arc::clone(&port_pool);
                                let pp = session.port_pair;
                                tokio::spawn(async move { pool.deallocate(pp).await });
                            }

                            // Publish callback event
                            fcp_queue::events::callback_initiated(
                                &Some(Arc::clone(&event_bus)),
                                &queue_name,
                                &call_id,
                                &caller_uri,
                                &queue_manager
                                    .get_queued_call(&call_id)
                                    .map(|c| c.tenant_id)
                                    .unwrap_or(default_tenant_id),
                            )
                            .await;

                            return;
                        }
                    }
                }

                // Check DTMF exit options
                if let Some(destination) = exit_options.get(&digit_str) {
                    info!(
                        call_id = %call_id,
                        digit = %digit_str,
                        destination = %destination,
                        "DTMF exit from queue"
                    );

                    if !overflow_expired_call_single(
                        &call_id,
                        destination,
                        &queue_name,
                        &sessions,
                        &queue_manager,
                        &uac,
                        &port_pool,
                        &local_addr,
                        &local_uri,
                        &event_bus,
                    )
                    .await
                    {
                        bye_and_cleanup(&call_id, &sessions, &uac, &port_pool, &queue_manager)
                            .await;
                    }

                    return;
                }
            }
        }
    })
}

/// Try a list of overflow destinations in order for an expired call.
/// On first success, stops. If all fail, sends BYE and cleans up.
#[allow(clippy::too_many_arguments)]
async fn overflow_expired_call_multi(
    call_id: &str,
    destinations: &[String],
    queue_name: &str,
    sessions: &Arc<DashMap<String, QueueSession>>,
    queue_manager: &Arc<QueueManager>,
    uac: &Arc<SipUserAgentClient>,
    port_pool: &Arc<PortPool>,
    local_addr: &str,
    local_uri: &str,
    event_bus: &Arc<dyn EventPublisher>,
) {
    for (i, dest) in destinations.iter().enumerate() {
        info!(
            call_id = %call_id,
            dest = %dest,
            attempt = i + 1,
            total = destinations.len(),
            "Trying overflow destination for expired call"
        );
        if overflow_expired_call_single(
            call_id,
            dest,
            queue_name,
            sessions,
            queue_manager,
            uac,
            port_pool,
            local_addr,
            local_uri,
            event_bus,
        )
        .await
        {
            return; // Success
        }
        warn!(
            call_id = %call_id,
            dest = %dest,
            remaining = destinations.len() - i - 1,
            "Overflow destination failed, trying next"
        );
    }
    // All destinations exhausted — BYE the caller
    warn!(
        call_id = %call_id,
        "All overflow destinations exhausted, sending BYE"
    );
    bye_and_cleanup(call_id, sessions, uac, port_pool, queue_manager).await;
}

/// Route an expired call to a single overflow destination.
/// Returns `true` on success, `false` if this destination failed (caller still alive for retry).
#[allow(clippy::too_many_arguments)]
async fn overflow_expired_call_single(
    call_id: &str,
    overflow_dest: &str,
    queue_name: &str,
    sessions: &Arc<DashMap<String, QueueSession>>,
    queue_manager: &Arc<QueueManager>,
    uac: &Arc<SipUserAgentClient>,
    port_pool: &Arc<PortPool>,
    local_addr: &str,
    local_uri: &str,
    event_bus: &Arc<dyn EventPublisher>,
) -> bool {
    // Queue chaining: if destination is "queue:<name>", transfer to another queue
    if let Some(target_queue) = overflow_dest.strip_prefix("queue:") {
        match queue_manager
            .transfer_to_queue(call_id, queue_name, target_queue)
            .await
        {
            Ok(position) => {
                if let Some(mut session) = sessions.get_mut(call_id) {
                    session.queue_name = target_queue.to_string();
                    session.state = QueueCallState::WaitingInQueue;
                }
                info!(
                    call_id = %call_id,
                    from = %queue_name,
                    to = %target_queue,
                    position = position,
                    "Expired call transferred to another queue"
                );
                return true;
            }
            Err(e) => {
                warn!(
                    call_id = %call_id,
                    error = %e,
                    "Failed to transfer expired call to queue"
                );
                return false;
            }
        }
    }

    let (codec_name, caller_rtp, tenant_id) = {
        let session = match sessions.get(call_id) {
            Some(s) => s,
            None => return false,
        };
        (
            session.codec_name.clone(),
            Arc::clone(&session.rtp_socket),
            session.tenant_id,
        )
    };

    let domain = extract_domain(local_uri);
    let dest_uri = parse_overflow_destination(overflow_dest, &domain);

    // Allocate B-leg RTP port pair
    let b_port_pair = match port_pool.allocate().await {
        Ok(pp) => pp,
        Err(e) => {
            warn!(call_id = %call_id, error = %e, "Failed to allocate overflow RTP port");
            return false;
        }
    };

    let sdp_offer = build_sdp_offer(local_addr, b_port_pair.rtp_port, &codec_name);

    // INVITE the overflow destination
    let call_handle = match uac.invite(&dest_uri, Some(&sdp_offer)).await {
        Ok(h) => h,
        Err(e) => {
            warn!(call_id = %call_id, error = %e, "Failed to INVITE overflow dest");
            port_pool.deallocate(b_port_pair).await;
            return false;
        }
    };

    let result = tokio::time::timeout(Duration::from_secs(30), call_handle.await_final()).await;

    match result {
        Ok(Ok(response))
            if response.start_line().code() >= 200 && response.start_line().code() < 300 =>
        {
            info!(
                call_id = %call_id,
                dest = %dest_uri,
                "Overflow destination answered (max wait time)"
            );

            let agent_body = response.body();
            let agent_sdp = std::str::from_utf8(&agent_body[..]).unwrap_or("");

            let b_rtp_socket = match RtpSocketPair::new(b_port_pair, RtpSocketConfig::default())
                .await
            {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    warn!(call_id = %call_id, error = %e, "Failed to create overflow RTP socket");
                    port_pool.deallocate(b_port_pair).await;
                    return false;
                }
            };

            if let Some(remote_addr) = QueueCallHandler::parse_remote_rtp_addr(agent_sdp) {
                b_rtp_socket
                    .set_remote_endpoint(RemoteEndpoint {
                        rtp_addr: remote_addr,
                        rtcp_addr: None,
                    })
                    .await;
            }

            let overflow_dialog = call_handle.dialog.read().await.clone();

            if let Some(mut session) = sessions.get_mut(call_id) {
                if let Some(moh) = session.moh_handle.take() {
                    moh.abort();
                }

                let (bridge_a_to_b, bridge_b_to_a) =
                    spawn_rtp_bridge(Arc::clone(&caller_rtp), Arc::clone(&b_rtp_socket));

                session.state = QueueCallState::OverflowedToDestination;
                session.agent_port_pair = Some(b_port_pair);
                session.agent_rtp_socket = Some(b_rtp_socket);
                session.agent_dialog = Some(overflow_dialog);
                session.bridge_a_to_b = Some(bridge_a_to_b);
                session.bridge_b_to_a = Some(bridge_b_to_a);
                session.ringing_agent = None;
                session.ring_task_handle = None;
            }

            queue_manager.dequeue_call(call_id);

            fcp_queue::events::caller_overflowed(
                &Some(Arc::clone(event_bus)),
                queue_name,
                call_id,
                overflow_dest,
                "max_wait_time",
                &tenant_id,
            )
            .await;

            true
        }
        _ => {
            // Overflow target rejected or timed out
            warn!(
                call_id = %call_id,
                dest = %dest_uri,
                "Overflow dest did not answer"
            );
            port_pool.deallocate(b_port_pair).await;
            false
        }
    }
}

/// Send BYE to a caller and clean up their session and queue entry.
async fn bye_and_cleanup(
    call_id: &str,
    sessions: &Arc<DashMap<String, QueueSession>>,
    uac: &Arc<SipUserAgentClient>,
    port_pool: &Arc<PortPool>,
    queue_manager: &Arc<QueueManager>,
) {
    if let Some((_, session)) = sessions.remove(call_id) {
        if let Some(moh) = session.moh_handle {
            moh.abort();
        }
        let bye_uac = Arc::clone(uac);
        let bye_dialog = session.dialog.clone();
        tokio::spawn(async move {
            let _ = bye_uac.bye(&bye_dialog).await;
        });
        let pool = Arc::clone(port_pool);
        let pp = session.port_pair;
        tokio::spawn(async move {
            pool.deallocate(pp).await;
        });
    }
    queue_manager.dequeue_call(call_id);
}

/// Extract domain part from a SIP URI string.
/// Play pre-encoded prompt frames to an RTP socket at 20ms intervals.
pub(crate) async fn play_prompt_frames(
    socket: &RtpSocketPair,
    frames: &[Vec<u8>],
    payload_type: u8,
) {
    use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};

    let seq = AtomicU16::new(0);
    let ts = AtomicU32::new(0);
    let ssrc: u32 = std::process::id().wrapping_mul(0x9e3779b9);
    // 20ms at 8kHz; G.722 (PT 9) also uses an 8 kHz RTP clock despite 16 kHz sampling.
    let ts_increment: u32 = 160;

    for frame in frames {
        let s = seq.fetch_add(1, Ordering::Relaxed);
        let t = ts.fetch_add(ts_increment, Ordering::Relaxed);

        // Build RTP packet: 12-byte header + payload
        let mut packet = Vec::with_capacity(12 + frame.len());
        packet.push(0x80); // V=2, no padding, no extension, no CSRC
        packet.push(payload_type & 0x7f);
        packet.extend_from_slice(&s.to_be_bytes());
        packet.extend_from_slice(&t.to_be_bytes());
        packet.extend_from_slice(&ssrc.to_be_bytes());
        packet.extend_from_slice(frame);

        let _ = socket.send_rtp_raw(&packet).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn extract_domain(uri: &str) -> String {
    // Handle "sip:user@host:port" format
    if let Some(rest) = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
    {
        if let Some(at_pos) = rest.find('@') {
            return rest[at_pos + 1..].to_string();
        }
        return rest.to_string();
    }
    // Fallback: return the whole string
    uri.to_string()
}

/// The record of a finished queue segment. Its `call_id` is the B2BUA
/// call's, so the segment sits beside the call's own record; a call the
/// queue placed itself gets one of its own. `sip_call_id` is the SIP Call-ID
/// the segment ran under.
#[allow(clippy::too_many_arguments)]
fn queue_record(
    b2bua_call_id: Option<fcp_sip::CallId>,
    sip_call_id: &str,
    tenant_id: TenantId,
    queue_name: &str,
    caller_uri: &str,
    caller_display: Option<&str>,
    codec_name: &str,
    agent_codec_name: Option<&str>,
    enqueued_at: Instant,
    agent_answered_at: Option<Instant>,
    state: QueueCallState,
    agent_id: Option<&str>,
    recording_path: Option<&str>,
    recording_consent: Option<fcp_core::RecordingConsent>,
    node_id: &str,
) -> fcp_cdr::Record {
    let now = SystemTime::now();
    let total_elapsed = enqueued_at.elapsed();

    // The session keeps Instants; the record wants wall-clock times.
    let start_time = now.checked_sub(total_elapsed).unwrap_or(now);
    let answer_time = agent_answered_at.map(|answered_at| {
        start_time
            .checked_add(answered_at.duration_since(enqueued_at))
            .unwrap_or(now)
    });

    let reason = match state {
        QueueCallState::Connected | QueueCallState::OverflowedToDestination => {
            fcp_cdr::TerminationReason::Hangup { by: None }
        }
        QueueCallState::RingingAgent => fcp_cdr::TerminationReason::NoAnswer,
        QueueCallState::WaitingInQueue => fcp_cdr::TerminationReason::Cancelled,
    };

    let wait_time_ms = agent_answered_at
        .map(|a| a.duration_since(enqueued_at).as_millis() as u64)
        .unwrap_or(total_elapsed.as_millis() as u64);
    let handle_time_ms = agent_answered_at
        .map(|a| a.elapsed().as_millis() as u64)
        .unwrap_or(0);
    let outcome = match state {
        QueueCallState::Connected => "answered",
        QueueCallState::OverflowedToDestination => "overflowed",
        QueueCallState::RingingAgent => "abandoned_ringing",
        QueueCallState::WaitingInQueue => "abandoned_waiting",
    };

    let call_id = b2bua_call_id
        .map(|id| fcp_core::CallId::from_uuid(*id.as_uuid()))
        .unwrap_or_default();
    fcp_cdr::RecordBuilder::new(CdrKind::Queue, call_id, tenant_id)
        .sip_call_id(sip_call_id)
        .from(caller_uri, caller_display.map(|s| s.to_string()))
        .to(format!("queue:{}", queue_name), None)
        .started_at(start_time)
        .answered_at(answer_time)
        .ended_at(Some(now))
        .reason(reason)
        .media_anchored(true)
        .codecs(
            Some(codec_name.to_string()),
            agent_codec_name.map(|s| s.to_string()),
        )
        .recording(recording_path.map(|s| s.to_string()))
        .recording_consent(recording_consent)
        .node(Some(node_id.to_string()), None)
        .custom_data(Some(serde_json::json!({
            "queue_name": queue_name,
            "agent_id": agent_id,
            "wait_time_ms": wait_time_ms,
            "handle_time_ms": handle_time_ms,
            "outcome": outcome,
        })))
        .build()
}

/// Task that dials a campaign number, waits for answer, then bridges to an agent.
#[allow(clippy::too_many_arguments)]
async fn campaign_dial_task(
    destination: String,
    _number: fcp_queue::CampaignNumber,
    queue_name: String,
    queue_manager: Arc<QueueManager>,
    uac: Arc<SipUserAgentClient>,
    event_bus: Arc<dyn EventPublisher>,
    port_pool: Arc<PortPool>,
    local_addr: String,
    local_uri: String,
    sessions: Arc<DashMap<String, QueueSession>>,
    ring_timeout_secs: u32,
    abandon_timeout_secs: u32,
    _announce_config: Option<AnnounceConfig>,
    _prompt_cache: Option<Arc<PromptCache>>,
    recording_dir: Option<QueueRecordings>,
    tenant_id: TenantId,
    campaign_call_id_map: Arc<DashMap<String, String>>,
) {
    let ring_timeout = Duration::from_secs(ring_timeout_secs as u64);

    // 1. Allocate RTP port pair for outbound leg
    let outbound_pp = match port_pool.allocate().await {
        Ok(pp) => pp,
        Err(e) => {
            warn!(dest = %destination, queue = %queue_name, error = %e, "Campaign: failed to allocate RTP port");
            queue_manager.campaign_call_result(
                &queue_name,
                &destination,
                fcp_queue::CampaignCallOutcome::Failed,
            );
            fcp_queue::events::campaign_call_failed(
                &Some(Arc::clone(&event_bus)),
                &queue_name,
                &destination,
                "port_allocation_failed",
                &tenant_id,
            )
            .await;
            return;
        }
    };

    // 2. Build SDP offer and INVITE the destination
    let sdp_offer = build_multi_codec_sdp_offer(&local_addr, outbound_pp.rtp_port);
    let dest_uri = if destination.starts_with("sip:") {
        destination.clone()
    } else {
        format!("sip:{}@{}", destination, extract_domain(&local_uri))
    };

    let call_handle = match uac.invite(&dest_uri, Some(&sdp_offer)).await {
        Ok(h) => h,
        Err(e) => {
            warn!(dest = %destination, queue = %queue_name, error = %e, "Campaign: INVITE failed");
            port_pool.deallocate(outbound_pp).await;
            queue_manager.campaign_call_result(
                &queue_name,
                &destination,
                fcp_queue::CampaignCallOutcome::Failed,
            );
            fcp_queue::events::campaign_call_failed(
                &Some(Arc::clone(&event_bus)),
                &queue_name,
                &destination,
                "invite_failed",
                &tenant_id,
            )
            .await;
            return;
        }
    };

    // 3. Wait for answer or timeout
    let result = tokio::time::timeout(ring_timeout, call_handle.await_final()).await;

    match result {
        Ok(Ok(response))
            if response.start_line().code() >= 200 && response.start_line().code() < 300 =>
        {
            // Callee answered — try to find an available agent
            info!(dest = %destination, queue = %queue_name, "Campaign call answered");

            // Parse callee's codec
            let callee_body = response.body();
            let callee_sdp = std::str::from_utf8(&callee_body[..]).unwrap_or("");
            let (callee_codec, callee_pt) = parse_agent_codec(callee_sdp);

            // Try to select an agent
            match queue_manager.select_agent(&queue_name) {
                Some(plan) => {
                    // Agent available — bridge the call
                    let agent_id = plan.agent_id.clone();
                    queue_manager
                        .mark_agent_ringing(&agent_id, &destination)
                        .await;

                    // Allocate agent RTP port
                    let agent_pp = match port_pool.allocate().await {
                        Ok(pp) => pp,
                        Err(e) => {
                            warn!(dest = %destination, agent = %agent_id, error = %e, "Campaign: agent port alloc failed");
                            let _ = uac.bye(&*call_handle.dialog.read().await).await;
                            port_pool.deallocate(outbound_pp).await;
                            queue_manager.campaign_call_result(
                                &queue_name,
                                &destination,
                                fcp_queue::CampaignCallOutcome::Failed,
                            );
                            return;
                        }
                    };

                    // INVITE agent
                    let agent_sdp = build_multi_codec_sdp_offer(&local_addr, agent_pp.rtp_port);
                    let agent_uri = format!(
                        "sip:{}@{}",
                        plan.agent_extension,
                        extract_domain(&local_uri)
                    );
                    let agent_ring_timeout = Duration::from_secs(ring_timeout_secs as u64);

                    match uac.invite(&agent_uri, Some(&agent_sdp)).await {
                        Ok(agent_handle) => {
                            match tokio::time::timeout(
                                agent_ring_timeout,
                                agent_handle.await_final(),
                            )
                            .await
                            {
                                Ok(Ok(agent_resp))
                                    if agent_resp.start_line().code() >= 200
                                        && agent_resp.start_line().code() < 300 =>
                                {
                                    // Agent answered — bridge RTP
                                    info!(
                                        dest = %destination, agent = %agent_id, queue = %queue_name,
                                        "Campaign: agent answered, bridging"
                                    );
                                    queue_manager.agent_answered(&agent_id, &destination).await;
                                    queue_manager.campaign_call_result(
                                        &queue_name,
                                        &destination,
                                        fcp_queue::CampaignCallOutcome::Answered,
                                    );
                                    fcp_queue::events::campaign_call_connected(
                                        &Some(Arc::clone(&event_bus)),
                                        &queue_name,
                                        &destination,
                                        &agent_id,
                                        &tenant_id,
                                    )
                                    .await;

                                    // Parse agent SDP
                                    let agent_body = agent_resp.body();
                                    let agent_sdp_str =
                                        std::str::from_utf8(&agent_body[..]).unwrap_or("");
                                    let (agent_codec_name, agent_pt) =
                                        parse_agent_codec(agent_sdp_str);

                                    // Create callee RTP socket
                                    let callee_rtp_socket = match RtpSocketPair::new(
                                        outbound_pp,
                                        RtpSocketConfig::default(),
                                    )
                                    .await
                                    {
                                        Ok(s) => Arc::new(s),
                                        Err(e) => {
                                            warn!(error = %e, "Campaign: failed to create callee RTP socket");
                                            let _ =
                                                uac.bye(&*call_handle.dialog.read().await).await;
                                            let _ =
                                                uac.bye(&*agent_handle.dialog.read().await).await;
                                            port_pool.deallocate(outbound_pp).await;
                                            port_pool.deallocate(agent_pp).await;
                                            return;
                                        }
                                    };

                                    // Set callee remote RTP endpoint
                                    if let Some(remote_addr) =
                                        QueueCallHandler::parse_remote_rtp_addr(callee_sdp)
                                    {
                                        callee_rtp_socket
                                            .set_remote_endpoint(RemoteEndpoint {
                                                rtp_addr: remote_addr,
                                                rtcp_addr: None,
                                            })
                                            .await;
                                    }

                                    // Create agent RTP socket
                                    let agent_rtp_socket = match RtpSocketPair::new(
                                        agent_pp,
                                        RtpSocketConfig::default(),
                                    )
                                    .await
                                    {
                                        Ok(s) => Arc::new(s),
                                        Err(e) => {
                                            warn!(error = %e, "Campaign: failed to create agent RTP socket");
                                            let _ =
                                                uac.bye(&*call_handle.dialog.read().await).await;
                                            let _ =
                                                uac.bye(&*agent_handle.dialog.read().await).await;
                                            port_pool.deallocate(outbound_pp).await;
                                            port_pool.deallocate(agent_pp).await;
                                            return;
                                        }
                                    };

                                    // Set agent remote RTP endpoint
                                    if let Some(remote_addr) =
                                        QueueCallHandler::parse_remote_rtp_addr(agent_sdp_str)
                                    {
                                        agent_rtp_socket
                                            .set_remote_endpoint(RemoteEndpoint {
                                                rtp_addr: remote_addr,
                                                rtcp_addr: None,
                                            })
                                            .await;
                                    }

                                    let callee = Leg {
                                        socket: Arc::clone(&callee_rtp_socket),
                                        codec: callee_codec.clone(),
                                        payload_type: callee_pt,
                                    };
                                    let agent = Leg {
                                        socket: Arc::clone(&agent_rtp_socket),
                                        codec: agent_codec_name.clone(),
                                        payload_type: agent_pt,
                                    };
                                    if callee.codec != agent.codec {
                                        info!(
                                            dest = %destination, agent = %agent_id,
                                            callee_codec = %callee.codec, agent_codec = %agent.codec,
                                            "Campaign: codecs differ, using transcoding bridge"
                                        );
                                    }

                                    // When the queue records, both parties hear the
                                    // tenant's announcement before each other
                                    // (docs/COMPLIANCE_FOLLOWUPS.md §8.2).
                                    let recording = match (
                                        &recording_dir,
                                        queue_manager.get_queue_config(&queue_name),
                                    ) {
                                        (Some(recordings), Some(qc)) if qc.record_calls => {
                                            Some(RecordingPlan {
                                                recordings,
                                                tenant_id,
                                                call_id: queue_record_call_id(None),
                                                format: qc.recording_format.clone(),
                                            })
                                        }
                                        _ => None,
                                    };
                                    let Some(bridge) =
                                        connect_legs(&callee, &agent, recording, || true).await
                                    else {
                                        return;
                                    };
                                    if let Some(path) = &bridge.recording_path {
                                        info!(dest = %destination, path = %path, "Campaign call recording started");
                                    }
                                    let recording_consent = bridge.recording_consent;
                                    let (
                                        bridge_a_to_b,
                                        bridge_b_to_a,
                                        recording_handle,
                                        recording_path,
                                    ) = (
                                        bridge.a_to_b,
                                        bridge.b_to_a,
                                        bridge.recording,
                                        bridge.recording_path,
                                    );

                                    // Get dialogs for both legs
                                    let callee_dialog = call_handle.dialog.read().await.clone();
                                    let agent_dialog = agent_handle.dialog.read().await.clone();

                                    // Use callee's Call-ID as the session key
                                    let callee_call_id = callee_dialog.id().call_id().to_string();
                                    let agent_call_id = agent_dialog.id().call_id().to_string();

                                    // Insert QueueSession for the campaign call
                                    sessions.insert(
                                        callee_call_id.clone(),
                                        QueueSession {
                                            call_id: callee_call_id.clone(),
                                            queue_name: queue_name.clone(),
                                            tenant_id,
                                            caller_uri: destination.clone(),
                                            caller_display: None,
                                            port_pair: outbound_pp,
                                            rtp_socket: callee_rtp_socket,
                                            codec_name: callee_codec.clone(),
                                            payload_type: callee_pt,
                                            _frame_size: 160,
                                            dialog: callee_dialog,
                                            moh_handle: None,
                                            moh_frames: None,
                                            dtmf_listener_handle: None,
                                            agent_port_pair: Some(agent_pp),
                                            agent_rtp_socket: Some(agent_rtp_socket),
                                            agent_dialog: Some(agent_dialog),
                                            bridge_a_to_b: Some(bridge_a_to_b),
                                            bridge_b_to_a: Some(bridge_b_to_a),
                                            recording_handle,
                                            recording_path,
                                            recording_consent,
                                            supervisor_port_pair: None,
                                            supervisor_rtp_socket: None,
                                            supervisor_dialog: None,
                                            supervisor_mode: None,
                                            supervisor_id: None,
                                            supervisor_tasks: Vec::new(),
                                            state: QueueCallState::Connected,
                                            is_campaign_call: true,
                                            ringing_agent: Some(agent_id.clone()),
                                            ring_task_handle: None,
                                            ring_cancel_tx: None,
                                            enqueued_at: Instant::now(),
                                            agent_answered_at: Some(Instant::now()),
                                            agent_codec_name: Some(agent_codec_name.clone()),
                                            b2bua_call_id: None,
                                        },
                                    );

                                    // Map callee ↔ agent Call-IDs for reverse BYE lookup
                                    campaign_call_id_map
                                        .insert(callee_call_id.clone(), agent_call_id.clone());
                                    campaign_call_id_map.insert(agent_call_id, callee_call_id);

                                    debug!(
                                        dest = %destination, agent = %agent_id,
                                        "Campaign call bridged (agent codec: {}, callee codec: {})", agent_codec_name, callee_codec
                                    );
                                }
                                _ => {
                                    // Agent didn't answer — abandon the campaign call
                                    info!(dest = %destination, agent = %agent_id, "Campaign: agent no answer, abandoning callee");
                                    let _ = uac.bye(&*call_handle.dialog.read().await).await;
                                    port_pool.deallocate(agent_pp).await;
                                    port_pool.deallocate(outbound_pp).await;
                                    queue_manager.campaign_call_result(
                                        &queue_name,
                                        &destination,
                                        fcp_queue::CampaignCallOutcome::Abandoned,
                                    );
                                    fcp_queue::events::campaign_call_failed(
                                        &Some(Arc::clone(&event_bus)),
                                        &queue_name,
                                        &destination,
                                        "abandoned",
                                        &tenant_id,
                                    )
                                    .await;
                                }
                            }
                        }
                        Err(e) => {
                            warn!(dest = %destination, agent = %agent_id, error = %e, "Campaign: agent INVITE failed");
                            let _ = uac.bye(&*call_handle.dialog.read().await).await;
                            port_pool.deallocate(agent_pp).await;
                            port_pool.deallocate(outbound_pp).await;
                            queue_manager.campaign_call_result(
                                &queue_name,
                                &destination,
                                fcp_queue::CampaignCallOutcome::Abandoned,
                            );
                        }
                    }
                }
                None => {
                    // No agent available — wait for abandon timeout, then hang up
                    info!(dest = %destination, queue = %queue_name, "Campaign: no agent, waiting abandon timeout");
                    let abandon = Duration::from_secs(abandon_timeout_secs as u64);
                    tokio::time::sleep(abandon).await;

                    let _ = uac.bye(&*call_handle.dialog.read().await).await;
                    port_pool.deallocate(outbound_pp).await;
                    queue_manager.campaign_call_result(
                        &queue_name,
                        &destination,
                        fcp_queue::CampaignCallOutcome::Abandoned,
                    );
                    fcp_queue::events::campaign_call_failed(
                        &Some(Arc::clone(&event_bus)),
                        &queue_name,
                        &destination,
                        "abandoned",
                        &tenant_id,
                    )
                    .await;
                }
            }
        }
        Ok(Ok(response))
            if response.start_line().code() == 486 || response.start_line().code() == 600 =>
        {
            // Busy
            info!(dest = %destination, queue = %queue_name, "Campaign call busy");
            port_pool.deallocate(outbound_pp).await;
            queue_manager.campaign_call_result(
                &queue_name,
                &destination,
                fcp_queue::CampaignCallOutcome::Busy,
            );
            fcp_queue::events::campaign_call_failed(
                &Some(Arc::clone(&event_bus)),
                &queue_name,
                &destination,
                "busy",
                &tenant_id,
            )
            .await;
        }
        Ok(Ok(_response)) => {
            // Other failure (4xx, 5xx, 6xx)
            info!(dest = %destination, queue = %queue_name, "Campaign call failed");
            port_pool.deallocate(outbound_pp).await;
            queue_manager.campaign_call_result(
                &queue_name,
                &destination,
                fcp_queue::CampaignCallOutcome::Failed,
            );
            fcp_queue::events::campaign_call_failed(
                &Some(Arc::clone(&event_bus)),
                &queue_name,
                &destination,
                "failed",
                &tenant_id,
            )
            .await;
        }
        Ok(Err(e)) => {
            // Transport/protocol error
            warn!(dest = %destination, queue = %queue_name, error = %e, "Campaign call error");
            port_pool.deallocate(outbound_pp).await;
            queue_manager.campaign_call_result(
                &queue_name,
                &destination,
                fcp_queue::CampaignCallOutcome::Failed,
            );
            fcp_queue::events::campaign_call_failed(
                &Some(Arc::clone(&event_bus)),
                &queue_name,
                &destination,
                "error",
                &tenant_id,
            )
            .await;
        }
        Err(_) => {
            // Ring timeout — no answer
            info!(dest = %destination, queue = %queue_name, "Campaign call no answer (timeout)");
            let _ = call_handle.cancel().await;
            port_pool.deallocate(outbound_pp).await;
            queue_manager.campaign_call_result(
                &queue_name,
                &destination,
                fcp_queue::CampaignCallOutcome::NoAnswer,
            );
            fcp_queue::events::campaign_call_failed(
                &Some(Arc::clone(&event_bus)),
                &queue_name,
                &destination,
                "no_answer",
                &tenant_id,
            )
            .await;
        }
    }
}

#[cfg(test)]
mod supervision_tests {
    use super::*;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::net::UdpSocket;

    /// A bridge socket whose far end is `remote`.
    async fn bridge_socket(pool: &PortPool, remote: SocketAddr) -> Arc<RtpSocketPair> {
        let ports = pool.allocate().await.unwrap();
        let config = RtpSocketConfig {
            bind_addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            ..Default::default()
        };
        let socket = RtpSocketPair::new(ports, config).await.unwrap();
        socket
            .set_remote_endpoint(RemoteEndpoint {
                rtp_addr: remote,
                rtcp_addr: None,
            })
            .await;
        Arc::new(socket)
    }

    fn loud_pcmu(seq: u16) -> Vec<u8> {
        let mut packet = vec![0x80, 0];
        packet.extend(seq.to_be_bytes());
        packet.extend((u32::from(seq) * 160).to_be_bytes());
        packet.extend(0x1234u32.to_be_bytes());
        packet.extend(std::iter::repeat_n(
            forge_codecs::g711::encode_ulaw(8000),
            160,
        ));
        packet
    }

    /// Whether `sink` is sent a loud PCMU packet within `within`.
    async fn hears_loud(sink: &UdpSocket, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        let mut buf = [0u8; 512];
        while tokio::time::Instant::now() < deadline {
            let Ok(Ok((len, _))) =
                tokio::time::timeout(Duration::from_millis(50), sink.recv_from(&mut buf)).await
            else {
                continue;
            };
            if len <= 12 {
                continue;
            }
            let payload = &buf[12..len];
            let loud = payload
                .iter()
                .map(|&b| (forge_codecs::g711::decode_ulaw(b) as i32).abs())
                .sum::<i32>()
                / payload.len() as i32;
            if loud > 2000 {
                return true;
            }
        }
        false
    }

    /// Keep sending loud audio from `from` to `to` until aborted.
    fn speak(from: Arc<UdpSocket>, to: SocketAddr) -> JoinHandle<()> {
        tokio::spawn(async move {
            for seq in 0..500u16 {
                let _ = from.send_to(&loud_pcmu(seq), to).await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    }

    /// The queue's supervisor bridge mixes (`docs/ENTERPRISE_CALLING.md`
    /// §3.11): a monitor hears the caller, a whisper is heard by the agent
    /// and never the caller, a barge by both.
    #[tokio::test]
    async fn the_queue_supervisor_is_mixed_not_interleaved() {
        let pool = PortPool::new(forge_rtp::PortPoolConfig::new(43100, 43200).unwrap());
        let caller = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let agent = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let supervisor = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let a = bridge_socket(&pool, caller.local_addr().unwrap()).await;
        let b = bridge_socket(&pool, agent.local_addr().unwrap()).await;
        let c = bridge_socket(&pool, supervisor.local_addr().unwrap()).await;
        let codecs = || ["PCMU".to_string(), "PCMU".to_string(), "PCMU".to_string()];

        // Monitor: the caller speaks; the supervisor hears, the agent too.
        let tasks = spawn_supervisor_bridge(
            Arc::clone(&a),
            Arc::clone(&b),
            Arc::clone(&c),
            SupervisorMode::Monitor,
            codecs(),
        );
        let talking = speak(Arc::clone(&caller), a.local_rtp_addr().unwrap());
        assert!(
            hears_loud(&supervisor, Duration::from_secs(2)).await,
            "the supervisor hears the caller"
        );
        assert!(
            hears_loud(&agent, Duration::from_secs(2)).await,
            "the agent hears the caller"
        );
        talking.abort();
        tasks.iter().for_each(JoinHandle::abort);

        // Whisper: the supervisor speaks; the agent hears, the caller not.
        let tasks = spawn_supervisor_bridge(
            Arc::clone(&a),
            Arc::clone(&b),
            Arc::clone(&c),
            SupervisorMode::Whisper,
            codecs(),
        );
        let talking = speak(Arc::clone(&supervisor), c.local_rtp_addr().unwrap());
        assert!(
            hears_loud(&agent, Duration::from_secs(2)).await,
            "the agent hears the whisper"
        );
        assert!(
            !hears_loud(&caller, Duration::from_millis(600)).await,
            "the caller never hears a whisper"
        );
        tasks.iter().for_each(JoinHandle::abort);

        // Barge: both hear the supervisor.
        let tasks = spawn_supervisor_bridge(a, Arc::clone(&b), c, SupervisorMode::Barge, codecs());
        assert!(
            hears_loud(&caller, Duration::from_secs(2)).await,
            "the caller hears a barge"
        );
        talking.abort();
        tasks.iter().for_each(JoinHandle::abort);
    }
}
