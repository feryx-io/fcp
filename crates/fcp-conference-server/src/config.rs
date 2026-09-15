use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Conference server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ConferenceServerConfig {
    #[serde(default)]
    pub service: ServiceSection,

    #[serde(default)]
    pub sip: SipSection,

    #[serde(default)]
    pub storage: StorageSection,

    /// The CDR pipeline's node side: where each participant's leg record
    /// spools before it ships (`docs/CDR_PIPELINE.md`).
    #[serde(default)]
    pub cdr: fcp_cdr::NodeConfig,

    #[serde(default)]
    pub http: HttpSection,

    #[serde(default)]
    pub logging: LoggingSection,

    #[serde(default)]
    pub media: MediaSection,

    #[serde(default)]
    pub events: EventsSection,

    #[serde(default)]
    pub b2bua: fcp_b2bua_core::B2buaConfig,

    #[serde(default)]
    pub conference: ConferenceSection,

    #[serde(default)]
    pub trunks: Vec<fcp_sbc_core::TrunkConfig>,

    #[serde(default)]
    pub ai: AiSection,

    #[serde(default)]
    pub tts: TtsSection,

    #[serde(default)]
    pub cluster: ClusterSection,
}

impl ConferenceServerConfig {
    /// The HTTP API address to advertise in cluster membership, where peer
    /// conference nodes send cross-node control requests:
    /// `http.advertised_address` if set, else `http.bind_address` with an
    /// unspecified IP replaced by `sip.local_ip`. Falls back to the bind
    /// address as-is when nothing routable is known.
    pub fn advertised_http_address(&self) -> SocketAddr {
        if let Some(addr) = self.http.advertised_address {
            return addr;
        }
        let bind = self.http.bind_address;
        if !bind.ip().is_unspecified() {
            return bind;
        }
        match self
            .sip
            .local_ip
            .as_deref()
            .and_then(|ip| ip.parse::<std::net::IpAddr>().ok())
        {
            Some(ip) => SocketAddr::new(ip, bind.port()),
            None => bind,
        }
    }
}

/// Text-to-Speech configuration for synthesizing conference audio prompts.
///
/// When enabled, meetings with `pin_prompt_tts` or `invalid_pin_tts` fields
/// will have their text synthesized to audio at room creation time.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsSection {
    /// Enable TTS prompt synthesis (default: false).
    #[serde(default)]
    pub enabled: bool,

    /// TTS provider: "openai", "google_cloud", "local", etc.
    #[serde(default = "default_tts_provider")]
    pub provider: String,

    /// API key for cloud TTS providers (OpenAI, Google Cloud, etc.)
    #[serde(default)]
    pub api_key: Option<String>,

    /// Default language code (BCP 47). Default: "en-US".
    #[serde(default = "default_tts_language")]
    pub language: String,

    /// Default voice name (provider-specific). None = provider default.
    #[serde(default)]
    pub voice: Option<String>,

    /// Path to Python executable for Kokoro local TTS (e.g. ".venv/bin/python3").
    /// Required when provider = "local".
    #[serde(default)]
    pub python_path: Option<String>,

    /// Local TTS engine name (default: "kokoro"). Only used with provider = "local".
    #[serde(default = "default_tts_engine")]
    pub engine: String,

    /// Directory for cached TTS audio files. Default: /var/fcp/conference/tts_cache
    #[serde(default = "default_tts_cache_dir")]
    pub cache_dir: String,

    /// Maximum TTS cache size in megabytes. Default: 100 MB.
    #[serde(default = "default_tts_cache_max_size_mb")]
    pub cache_max_size_mb: usize,
}

fn default_tts_provider() -> String {
    "local".to_string()
}

fn default_tts_engine() -> String {
    "kokoro".to_string()
}

fn default_tts_language() -> String {
    "en-US".to_string()
}

fn default_tts_cache_dir() -> String {
    "/var/fcp/conference/tts_cache".to_string()
}

fn default_tts_cache_max_size_mb() -> usize {
    100
}

impl Default for TtsSection {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_tts_provider(),
            api_key: None,
            python_path: None,
            engine: default_tts_engine(),
            language: default_tts_language(),
            voice: None,
            cache_dir: default_tts_cache_dir(),
            cache_max_size_mb: default_tts_cache_max_size_mb(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,

    #[serde(default = "default_tenant_id")]
    pub default_tenant_id: fcp_core::TenantId,

    /// Maximum time (seconds) to wait for graceful shutdown before force-closing sessions.
    /// Default: 30. Set 0 for unlimited (not recommended).
    #[serde(default = "default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,

    /// On shutdown, how long (seconds) to keep serving the callers already
    /// here — refusing new ones and, in a cluster, advertised as draining so
    /// placement sends new participants elsewhere — before hanging up
    /// whoever is still connected. Default: 0 (hang up at once, as before).
    #[serde(default)]
    pub drain_wait_secs: u64,

    /// Interval in seconds for polling config file changes (hot-reload). Default: 10.
    #[serde(default = "default_config_reload_interval_secs")]
    pub config_reload_interval_secs: u64,
}

fn default_service_type() -> String {
    "conference".to_string()
}

fn default_shutdown_timeout_secs() -> u64 {
    30
}

fn default_config_reload_interval_secs() -> u64 {
    10
}

fn default_node_id() -> String {
    format!("conference-{}", uuid::Uuid::new_v4())
}

fn default_tenant_id() -> fcp_core::TenantId {
    fcp_core::TenantId::nil()
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
            default_tenant_id: default_tenant_id(),
            shutdown_timeout_secs: default_shutdown_timeout_secs(),
            drain_wait_secs: 0,
            config_reload_interval_secs: default_config_reload_interval_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SipSection {
    #[serde(default = "default_sip_bind")]
    pub udp_bind: String,

    #[serde(default = "default_sip_bind")]
    pub tcp_bind: String,

    #[serde(default)]
    pub ws_bind: Option<String>,

    #[serde(default)]
    pub wss_bind: Option<String>,

    #[serde(default)]
    pub tls_cert_path: Option<String>,

    #[serde(default)]
    pub tls_key_path: Option<String>,

    /// SIP-over-TLS listener bind address (e.g. `0.0.0.0:5061`). Needs
    /// `tls_cert_path` / `tls_key_path`. Inter-node conference trunks
    /// arrive here and nowhere else.
    #[serde(default)]
    pub tls_bind: Option<String>,

    /// PEM bundle of the CA(s) that (a) client certificates presented on
    /// the TLS and WSS listeners must chain to and (b) the certificates
    /// of peer nodes this node dials over TLS are accepted under. This is
    /// the cluster CA; it is what authenticates a trunk INVITE.
    #[serde(default)]
    pub tls_client_ca_path: Option<String>,

    /// With `tls_client_ca_path`: `required` (default) refuses a TLS peer
    /// without a client certificate at the handshake; `optional` admits
    /// it with no identity, which a trunk INVITE is then rejected for.
    #[serde(default = "default_tls_client_auth")]
    pub tls_client_auth: String,

    /// Certificate this node presents on outbound TLS (its identity to
    /// peers). Defaults to `tls_cert_path` / `tls_key_path`.
    #[serde(default)]
    pub tls_client_cert_path: Option<String>,

    #[serde(default)]
    pub tls_client_key_path: Option<String>,

    pub local_ip: Option<String>,

    pub local_uri: Option<String>,
}

fn default_sip_bind() -> String {
    "0.0.0.0:5060".to_string()
}

fn default_tls_client_auth() -> String {
    "required".to_string()
}

impl SipSection {
    /// The `(cert, key)` this node presents on outbound TLS, if any.
    pub fn tls_client_identity(&self) -> Option<(&str, &str)> {
        let cert = self
            .tls_client_cert_path
            .as_deref()
            .or(self.tls_cert_path.as_deref())?;
        let key = self
            .tls_client_key_path
            .as_deref()
            .or(self.tls_key_path.as_deref())?;
        Some((cert, key))
    }

    /// The TLS listener address peers should dial: `tls_bind` with an
    /// unspecified IP replaced by `local_ip`. `None` without a TLS
    /// listener, or when the bind is unspecified and no `local_ip` is
    /// set (nothing routable to advertise).
    pub fn advertised_tls_address(&self) -> Option<std::net::SocketAddr> {
        let bind: std::net::SocketAddr = self.tls_bind.as_deref()?.parse().ok()?;
        if !bind.ip().is_unspecified() {
            return Some(bind);
        }
        let ip: std::net::IpAddr = self.local_ip.as_deref()?.parse().ok()?;
        Some(std::net::SocketAddr::new(ip, bind.port()))
    }
}

impl Default for SipSection {
    fn default() -> Self {
        Self {
            udp_bind: default_sip_bind(),
            tcp_bind: default_sip_bind(),
            ws_bind: None,
            wss_bind: None,
            tls_cert_path: None,
            tls_key_path: None,
            tls_bind: None,
            tls_client_ca_path: None,
            tls_client_auth: default_tls_client_auth(),
            tls_client_cert_path: None,
            tls_client_key_path: None,
            local_ip: None,
            local_uri: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageSection {
    /// PostgreSQL connection URL
    #[serde(default = "default_database_url")]
    pub database_url: String,

    /// Streaming read replica (hot standby) that configuration reads are
    /// served from while it is healthy; writes always go to `database_url`.
    /// See `docs/MULTI_REGION.md`.
    #[serde(default)]
    pub read_replica_url: Option<String>,

    /// Replication lag (seconds) beyond which reads return to the primary;
    /// also the read-after-write window. Default: 5.
    #[serde(default = "default_max_replica_lag_secs")]
    pub max_replica_lag_secs: u64,

    /// Maximum database connections
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Redis connection URL for cluster membership.
    #[serde(default)]
    pub redis_url: Option<String>,
}

fn default_max_replica_lag_secs() -> u64 {
    5
}

fn default_database_url() -> String {
    "postgresql://localhost/fcp".to_string()
}

fn default_max_connections() -> u32 {
    50
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            database_url: default_database_url(),
            read_replica_url: None,
            max_replica_lag_secs: default_max_replica_lag_secs(),
            max_connections: default_max_connections(),
            redis_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpSection {
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,

    /// Enable Bearer token authentication on protected endpoints
    #[serde(default)]
    pub auth_enabled: bool,

    /// Bearer token for API authentication (required when auth_enabled = true)
    pub auth_token: Option<String>,

    /// Rate limit for API requests per second. None or 0 = unlimited.
    #[serde(default)]
    pub rate_limit_rps: Option<u32>,

    /// Maximum HTTP request body size in kilobytes (default: 64 KB).
    #[serde(default = "default_max_body_size_kb")]
    pub max_body_size_kb: usize,

    /// Address peer nodes reach this HTTP API on, advertised in cluster
    /// membership for cross-node conference control. Defaults to
    /// `bind_address`, with an unspecified IP (`0.0.0.0`) replaced by
    /// `sip.local_ip`.
    #[serde(default)]
    pub advertised_address: Option<SocketAddr>,
}

fn default_http_bind() -> SocketAddr {
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 8090)
}

fn default_max_body_size_kb() -> usize {
    64
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
            auth_enabled: false,
            auth_token: None,
            rate_limit_rps: None,
            max_body_size_kb: default_max_body_size_kb(),
            advertised_address: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingSection {
    #[serde(default = "default_log_level")]
    pub level: String,

    #[serde(default = "default_log_format")]
    pub format: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "json".to_string()
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaSection {
    #[serde(default = "default_rtp_port_start")]
    pub rtp_port_start: u16,

    #[serde(default = "default_rtp_port_end")]
    pub rtp_port_end: u16,

    #[serde(default = "default_codecs")]
    pub codecs: Vec<String>,

    pub public_ip: Option<String>,
}

fn default_rtp_port_start() -> u16 {
    10000
}

fn default_rtp_port_end() -> u16 {
    20000
}

fn default_codecs() -> Vec<String> {
    vec!["PCMU".to_string(), "PCMA".to_string()]
}

impl Default for MediaSection {
    fn default() -> Self {
        Self {
            rtp_port_start: default_rtp_port_start(),
            rtp_port_end: default_rtp_port_end(),
            codecs: default_codecs(),
            public_ip: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventsSection {
    pub server_url: Option<String>,

    /// Optional bearer token for authenticating to fcp-events-server.
    #[serde(default)]
    pub auth_token: Option<String>,

    /// HTTP timeout for event forwarding requests (default: 5s).
    #[serde(default = "default_events_timeout_secs")]
    pub timeout_secs: u64,

    /// Max retry attempts for failed event delivery (default: 3).
    #[serde(default = "default_events_max_retries")]
    pub max_retries: u32,

    /// Circuit breaker: consecutive failures before opening (default: 5).
    #[serde(default = "default_events_cb_threshold")]
    pub circuit_breaker_threshold: u64,

    /// Circuit breaker: cooldown seconds before half-open probe (default: 30).
    #[serde(default = "default_events_cb_cooldown_secs")]
    pub circuit_breaker_cooldown_secs: u64,

    /// Max concurrent in-flight event forwarding requests (default: 50).
    #[serde(default = "default_events_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_events_timeout_secs() -> u64 {
    5
}
fn default_events_max_retries() -> u32 {
    3
}
fn default_events_cb_threshold() -> u64 {
    5
}
fn default_events_cb_cooldown_secs() -> u64 {
    30
}
fn default_events_max_concurrent() -> usize {
    50
}

impl Default for EventsSection {
    fn default() -> Self {
        Self {
            server_url: None,
            auth_token: None,
            timeout_secs: default_events_timeout_secs(),
            max_retries: default_events_max_retries(),
            circuit_breaker_threshold: default_events_cb_threshold(),
            circuit_breaker_cooldown_secs: default_events_cb_cooldown_secs(),
            max_concurrent: default_events_max_concurrent(),
        }
    }
}

/// AI participant configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AiSection {
    /// Enable AI participant support (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// API key (can also be set via FORGE_AI_API_KEY env var)
    pub api_key: Option<String>,

    /// AI model (default: "gpt-4o-realtime-preview")
    #[serde(default = "default_ai_model")]
    pub model: String,

    /// Voice for AI responses (default: "alloy")
    #[serde(default = "default_ai_voice")]
    pub voice: String,

    /// System instructions for AI behavior
    pub instructions: Option<String>,

    /// Custom API endpoint URL
    pub endpoint: Option<String>,
}

fn default_ai_model() -> String {
    "gpt-4o-realtime-preview".to_string()
}

fn default_ai_voice() -> String {
    "alloy".to_string()
}

impl Default for AiSection {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            model: default_ai_model(),
            voice: default_ai_voice(),
            instructions: None,
            endpoint: None,
        }
    }
}

/// Conference-specific configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConferenceSection {
    /// Audio sample rate (default: 8000 Hz)
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    /// Audio frame size in samples (default: 160 = 20ms at 8kHz)
    #[serde(default = "default_frame_size")]
    pub frame_size: usize,

    /// Enable voice activity detection
    #[serde(default = "default_true")]
    pub enable_vad: bool,

    /// Default max participants for new rooms (0 = unlimited)
    #[serde(default)]
    pub default_max_participants: usize,

    /// Destroy room when all participants leave (unless meeting.permanent)
    #[serde(default = "default_true")]
    pub auto_destroy_empty_rooms: bool,

    /// Directory for conference recordings
    #[serde(default = "default_recordings_dir")]
    pub recordings_dir: String,

    /// Timeout in seconds for PIN entry
    #[serde(default = "default_pin_timeout_secs")]
    pub pin_timeout_secs: u64,

    /// Maximum PIN entry attempts before disconnect
    #[serde(default = "default_max_pin_attempts")]
    pub max_pin_attempts: u32,

    /// Jitter buffer depth in milliseconds (default: 40ms)
    #[serde(default = "default_jitter_buffer_ms")]
    pub jitter_buffer_ms: u64,

    /// Enable DTMF conference control commands (*6=mute, *7=vol up, etc.)
    #[serde(default = "default_true")]
    pub dtmf_enabled: bool,

    /// Meeting lookup cache TTL in seconds (default: 60). Set 0 to disable.
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,

    /// Recording retention in days. None/0 = keep forever.
    #[serde(default)]
    pub recording_retention_days: Option<u32>,

    /// Allowed codecs for conference participants. None = all supported (PCMU, PCMA, G722).
    #[serde(default)]
    pub allowed_codecs: Option<Vec<String>>,

    /// Enable SDES-SRTP for encrypted media. When enabled, the server will
    /// negotiate SRTP with clients that offer `a=crypto:` in their SDP.
    #[serde(default)]
    pub enable_srtp: bool,

    /// Where this node's video rooms run (design §15.7): `host`, or a
    /// GPU as `backend:address` — `cuda:0` for the first NVIDIA device.
    /// A GPU needs the `video-hwaccel` build feature and the FFmpeg it
    /// was built against; a device that does not open at start leaves
    /// the node on the host with a warning. A room on a device decodes,
    /// composes and encodes there, and runs on the host any stage the
    /// device lacks (VP8 and VP9 encoding on NVIDIA). Default: `host`.
    #[serde(default = "default_video_device")]
    pub video_device: String,

    /// Threads in the video codec pool (decode, compose, encode never run
    /// on the async runtime). Default: two fewer than the machine's cores,
    /// at least one.
    #[serde(default)]
    pub video_codec_threads: Option<usize>,

    /// Offer callers only video codecs with a native binding compiled in
    /// (`video-vpx`, `video-openh264`, `video-dav1d` + `video-svt-av1`).
    /// Off, the raw test codec answers for every codec — for loopback
    /// tests only, never for a real phone. Default: on.
    #[serde(default = "default_true")]
    pub video_native_codecs_only: bool,

    /// Seconds the video self-benchmark may take at start, pricing each
    /// usable codec on this node's codec pool before the first cluster
    /// heartbeat (design §9.3). `0` skips it and uses the phase 0 table.
    /// Default: 10.
    #[serde(default = "default_video_benchmark_seconds")]
    pub video_benchmark_seconds: u64,

    /// Share of the codec pool's time the video budget may commit
    /// (0.05–1.0); the rest absorbs content harder than the benchmark's
    /// and the async side's work. Default: 0.8.
    #[serde(default = "default_video_budget_headroom")]
    pub video_budget_headroom: f64,

    /// Seconds a presenter may send nothing before the shared screen's
    /// floor is given up (design §15.6). Judged on packets, not frames:
    /// a slide that has not changed is still being shared. Default: 30.
    #[serde(default = "default_content_idle_secs")]
    pub content_idle_secs: u64,

    /// Where BFCP (RFC 8855) floor control for SIP room systems listens,
    /// UDP and TCP on the same address (design §15.6, 7e): a caller whose
    /// offer carries `m=application … UDP/BFCP` or `TCP/BFCP` beside an
    /// `a=content:slides` section is answered as its floor control server
    /// here. Unset, such sections are rejected and the floor is implicit.
    #[serde(default)]
    pub bfcp_bind: Option<std::net::SocketAddr>,

    /// Directory for pre-recorded announcement WAV files.
    #[serde(default = "default_announcements_dir")]
    pub announcements_dir: String,

    /// Maximum time (seconds) a participant waits for a moderator before being disconnected.
    /// Default: 300 (5 minutes). Set 0 for unlimited.
    #[serde(default = "default_max_wait_for_moderator_secs")]
    pub max_wait_for_moderator_secs: u64,

    /// Interval (seconds) between repeating the wait-for-moderator announcement.
    /// The announcement plays once, then hold music (or silence) plays for this
    /// duration before the announcement repeats. Default: 30.
    #[serde(default = "default_moh_announcement_interval_secs")]
    pub moh_announcement_interval_secs: u64,

    /// Maximum SIP INVITE rate per source IP per second. 0 = unlimited.
    /// Prevents brute-force PIN attacks and INVITE floods. Default: 10.
    #[serde(default = "default_max_invite_rate")]
    pub max_invite_rate_per_ip: u32,

    /// Session timer interval in seconds (RFC 4028). Sessions not refreshed
    /// within this period are considered dead and cleaned up. Default: 1800 (30 min).
    /// Set 0 to disable session timers.
    #[serde(default = "default_session_expires_secs")]
    pub session_expires_secs: u64,

    /// Maximum recording file size in megabytes. Recording stops automatically
    /// when this limit is reached. Default: 500 (500 MB). Set 0 for no limit.
    #[serde(default = "default_max_recording_size_mb")]
    pub max_recording_size_mb: u64,

    /// Maximum total participants across all rooms on this node.
    /// Prevents OOM when many large conferences are hosted on a single node.
    /// Default: 500. Set 0 for no limit (not recommended for production).
    #[serde(default = "default_max_participants_per_node")]
    pub max_participants_per_node: usize,

    /// Maximum announcement file size in megabytes. Default: 10 MB. Set 0 for no limit.
    #[serde(default = "default_max_announcement_size_mb")]
    pub max_announcement_size_mb: u64,

    /// RFC 4028 minimum session expiry in seconds (default: 90).
    #[serde(default = "default_min_session_expires_secs")]
    pub min_session_expires_secs: u64,

    /// Maximum entries in the in-memory meeting cache (default: 10000).
    #[serde(default = "default_meeting_cache_max_size")]
    pub meeting_cache_max_size: usize,

    /// Interval in seconds between recording cleanup runs (default: 3600 = 1 hour).
    #[serde(default = "default_recording_cleanup_interval_secs")]
    pub recording_cleanup_interval_secs: u64,

    /// Interval in seconds between recording size limit checks (default: 30).
    #[serde(default = "default_recording_size_check_interval_secs")]
    pub recording_size_check_interval_secs: u64,

    /// Path to WAV file played to prompt callers for PIN entry.
    /// Relative paths are resolved against `announcements_dir`.
    #[serde(default)]
    pub pin_prompt_sound: Option<String>,

    /// Path to WAV file played after an incorrect PIN entry.
    #[serde(default)]
    pub invalid_pin_sound: Option<String>,

    /// Path to WAV file played as music-on-hold while waiting for a moderator.
    #[serde(default)]
    pub moh_sound: Option<String>,

    /// Timeout in seconds for dial-out INVITE to be answered. Default: 30.
    #[serde(default = "default_dial_out_timeout_secs")]
    pub dial_out_timeout_secs: u64,

    /// Path to join tone WAV file
    #[serde(default)]
    pub join_sound: Option<String>,

    /// Path to exit tone WAV file
    #[serde(default)]
    pub exit_sound: Option<String>,

    /// What to record a room to: `wav` (mixed audio only, as recordings
    /// have always been), `webm` (VP8 or VP9 with Opus), `mp4` (H.264,
    /// HEVC or AV1 with Opus, fragmented — the container every player
    /// opens, and what a GPU node records on its encoder) or `auto`
    /// (design §15.9): MP4 when the meeting has video and this node can
    /// encode H.264 — natively or on its device — WebM when it can encode
    /// VP8 or VP9 instead, WAV otherwise. A build without the
    /// `recording-webm` / `recording-mp4` feature records what it can,
    /// and logs that at startup.
    #[serde(default = "default_recording_format")]
    pub recording_format: String,

    /// The codec a recording's video track uses: `auto` (the container's
    /// first codec this node encodes — H.264 for MP4, VP8 for WebM),
    /// `h264`, `hevc`, `av1` (MP4) or `vp8`, `vp9` (WebM). With
    /// `recording_format = auto` a named codec also picks its container.
    /// Ignored when the node cannot encode it.
    #[serde(default = "default_recording_video_codec")]
    pub recording_video_codec: String,

    /// The size a recording's video is written at, e.g. `720p` or
    /// `640x360`. The room's own is the default and the cap.
    #[serde(default)]
    pub recording_resolution: Option<String>,

    /// Opus bitrate for a recording's audio, in bits per second.
    #[serde(default = "default_recording_audio_bitrate")]
    pub recording_audio_bitrate: u32,

    /// The secret join tokens are signed with — the same value as the
    /// admin API's `[conference] join_secret`. An INVITE carrying an
    /// `X-FCP-Join` header is admitted on the token's say-so (tenant,
    /// room, name, role) with no PIN; without this set, such an INVITE is
    /// refused. Kept out of the config store: it is read from TOML only.
    #[serde(default, skip_serializing)]
    pub join_secret: Option<String>,
}

fn default_recording_format() -> String {
    "auto".to_string()
}

fn default_recording_video_codec() -> String {
    "auto".to_string()
}

fn default_recording_audio_bitrate() -> u32 {
    64_000
}

fn default_announcements_dir() -> String {
    "/var/fcp/conference/announcements".to_string()
}

fn default_sample_rate() -> u32 {
    8000
}

fn default_frame_size() -> usize {
    160
}

fn default_true() -> bool {
    true
}

fn default_video_benchmark_seconds() -> u64 {
    10
}

fn default_video_device() -> String {
    "host".to_string()
}

fn default_content_idle_secs() -> u64 {
    30
}

fn default_video_budget_headroom() -> f64 {
    0.8
}

fn default_recordings_dir() -> String {
    "/var/fcp/conference/recordings".to_string()
}

fn default_pin_timeout_secs() -> u64 {
    10
}

fn default_max_pin_attempts() -> u32 {
    3
}

fn default_jitter_buffer_ms() -> u64 {
    40
}

fn default_cache_ttl_secs() -> u64 {
    5
}

fn default_max_wait_for_moderator_secs() -> u64 {
    300
}

fn default_moh_announcement_interval_secs() -> u64 {
    30
}

fn default_max_invite_rate() -> u32 {
    10
}

fn default_session_expires_secs() -> u64 {
    1800
}

fn default_max_recording_size_mb() -> u64 {
    500
}

fn default_max_participants_per_node() -> usize {
    500
}

fn default_max_announcement_size_mb() -> u64 {
    10
}

fn default_min_session_expires_secs() -> u64 {
    90
}

fn default_meeting_cache_max_size() -> usize {
    10_000
}

fn default_recording_cleanup_interval_secs() -> u64 {
    3600
}

fn default_recording_size_check_interval_secs() -> u64 {
    30
}

fn default_dial_out_timeout_secs() -> u64 {
    30
}

impl Default for ConferenceSection {
    fn default() -> Self {
        Self {
            sample_rate: default_sample_rate(),
            frame_size: default_frame_size(),
            enable_vad: true,
            default_max_participants: 0,
            auto_destroy_empty_rooms: true,
            recordings_dir: default_recordings_dir(),
            pin_timeout_secs: default_pin_timeout_secs(),
            max_pin_attempts: default_max_pin_attempts(),
            jitter_buffer_ms: default_jitter_buffer_ms(),
            dtmf_enabled: true,
            cache_ttl_secs: default_cache_ttl_secs(),
            recording_retention_days: None,
            allowed_codecs: None,
            enable_srtp: false,
            video_device: default_video_device(),
            video_codec_threads: None,
            video_native_codecs_only: true,
            video_benchmark_seconds: default_video_benchmark_seconds(),
            video_budget_headroom: default_video_budget_headroom(),
            content_idle_secs: default_content_idle_secs(),
            bfcp_bind: None,
            announcements_dir: default_announcements_dir(),
            max_wait_for_moderator_secs: default_max_wait_for_moderator_secs(),
            moh_announcement_interval_secs: default_moh_announcement_interval_secs(),
            max_invite_rate_per_ip: default_max_invite_rate(),
            session_expires_secs: default_session_expires_secs(),
            max_recording_size_mb: default_max_recording_size_mb(),
            max_participants_per_node: default_max_participants_per_node(),
            max_announcement_size_mb: default_max_announcement_size_mb(),
            min_session_expires_secs: default_min_session_expires_secs(),
            meeting_cache_max_size: default_meeting_cache_max_size(),
            recording_cleanup_interval_secs: default_recording_cleanup_interval_secs(),
            recording_size_check_interval_secs: default_recording_size_check_interval_secs(),
            pin_prompt_sound: None,
            invalid_pin_sound: None,
            moh_sound: None,
            dial_out_timeout_secs: default_dial_out_timeout_secs(),
            join_sound: None,
            exit_sound: None,
            recording_format: default_recording_format(),
            recording_video_codec: default_recording_video_codec(),
            recording_resolution: None,
            recording_audio_bitrate: default_recording_audio_bitrate(),
            join_secret: None,
        }
    }
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClusterSection {
    /// Enable cluster mode. Requires `storage.redis_url`.
    #[serde(default)]
    pub enabled: bool,

    /// Deployment region this node advertises in cluster membership, e.g.
    /// `us-east`; falls back to the `FCP_REGION` environment variable.
    /// Placement prefers nodes in the caller's region. See
    /// `docs/MULTI_REGION.md`.
    #[serde(default)]
    pub region: Option<String>,

    /// Heartbeat interval in seconds (default: 5).
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,

    /// Heartbeat TTL in seconds (default: 15).
    #[serde(default = "default_heartbeat_ttl")]
    pub heartbeat_ttl_secs: u64,

    /// How often inter-node conference trunks are reconciled against the
    /// conference registry, in seconds (default: 2). Trunks are also
    /// reconciled immediately on every local join and leave; this bounds
    /// how long a peer that started hosting a room waits to be trunked.
    #[serde(default = "default_trunk_reconcile_interval")]
    pub trunk_reconcile_interval_secs: u64,

    /// An inter-node trunk that has received no RTP for this long (seconds)
    /// is presumed dead — its peer crashed, restarted, or lost the session
    /// — and is torn down so the reconciler dials it again. Trunks send a
    /// frame every 20 ms even when silent, so this measures liveness, not
    /// speech. Default: 30. `0` disables the check.
    #[serde(default = "default_trunk_rtp_timeout")]
    pub trunk_rtp_timeout_secs: u64,

    /// Encrypt inter-node trunk media with SDES-SRTP (AES_CM_128_HMAC_SHA1_80),
    /// keyed over the mutual-TLS signaling. Default: true. A dialing node
    /// that offers SRTP refuses an answer without it, so set the same value
    /// on every conference node.
    #[serde(default = "default_trunk_srtp")]
    pub trunk_srtp: bool,

    /// How often a node tells the peers hosting a room who is speaking
    /// on it, so every node runs the same active-speaker rule over the
    /// same candidates (`docs/DISTRIBUTED_CONFERENCING.md`). It has to
    /// be well inside the 800 ms a challenger needs to take the floor,
    /// or the nodes would decide on stale levels. 0 turns cross-node
    /// speaker agreement off: each node then picks from its own callers
    /// and a peer's tile never wins.
    #[serde(default = "default_speaker_interval")]
    pub speaker_interval_ms: u64,

    /// Shared secret conference nodes present to one another for
    /// cross-node control (lock, kick, mute, gain, announce, end) over
    /// each other's `/internal` HTTP routes. The same value on every
    /// conference node. Without it a control request only reaches the
    /// node it was sent to.
    #[serde(default)]
    pub secret: Option<String>,
    /// When this node drains, how long (seconds) the holder of a caller's
    /// leg has to move the caller to another node after the REFER before
    /// the caller is given up on and hung up with the rest
    /// (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5). Default: 15.
    /// Migration needs `secret`.
    #[serde(default = "default_migration_timeout")]
    pub migration_timeout_secs: u64,
}

fn default_migration_timeout() -> u64 {
    15
}

fn default_speaker_interval() -> u64 {
    400
}

fn default_heartbeat_interval() -> u64 {
    5
}
fn default_heartbeat_ttl() -> u64 {
    15
}
fn default_trunk_reconcile_interval() -> u64 {
    2
}
fn default_trunk_rtp_timeout() -> u64 {
    30
}
fn default_trunk_srtp() -> bool {
    true
}

impl Default for ClusterSection {
    fn default() -> Self {
        Self {
            enabled: false,
            region: None,
            heartbeat_interval_secs: default_heartbeat_interval(),
            heartbeat_ttl_secs: default_heartbeat_ttl(),
            trunk_reconcile_interval_secs: default_trunk_reconcile_interval(),
            trunk_rtp_timeout_secs: default_trunk_rtp_timeout(),
            trunk_srtp: default_trunk_srtp(),
            speaker_interval_ms: default_speaker_interval(),
            secret: None,
            migration_timeout_secs: default_migration_timeout(),
        }
    }
}

/// Validate configuration for logical consistency.
/// Called at startup to fail fast on misconfigured values.
pub fn validate_config(config: &ConferenceServerConfig) -> Result<()> {
    // Where video runs.
    match forge_video::frame::MediaDevice::parse(&config.conference.video_device) {
        Some(forge_video::frame::MediaDevice::Host) => {}
        Some(forge_video::frame::MediaDevice::Gpu { backend, .. })
            if ["cuda", "vaapi", "qsv"].contains(&backend.as_str()) => {}
        _ => anyhow::bail!(
            "conference.video_device '{}' is not `host` or a device like `cuda:0`",
            config.conference.video_device
        ),
    }
    // What a room is recorded to.
    let Some(choice) = crate::recording::FormatChoice::parse(&config.conference.recording_format)
    else {
        anyhow::bail!(
            "conference.recording_format '{}' is not one of auto, wav, webm, mp4",
            config.conference.recording_format
        );
    };
    let codec = crate::recording::parse_video_codec(&config.conference.recording_video_codec);
    match codec {
        Ok(None) => {}
        Ok(Some(codec)) => {
            if let Some(container) = choice.container() {
                if !crate::recording::container_carries(container, codec) {
                    anyhow::bail!(
                        "conference.recording_video_codec '{}' is not a codec {} carries ({})",
                        config.conference.recording_video_codec,
                        container.as_str(),
                        crate::recording::codecs_of(container)
                            .iter()
                            .map(|c| crate::recording::codec_config_name(*c))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }
        Err(other) => anyhow::bail!(
            "conference.recording_video_codec '{other}' is not one of auto, h264, hevc, av1, vp8, vp9"
        ),
    }
    if let Some(res) = &config.conference.recording_resolution {
        if forge_video::frame::Resolution::parse(res).is_none() {
            anyhow::bail!(
                "conference.recording_resolution '{res}' is not a size like 720p or 1280x720"
            );
        }
    }
    if config.conference.recording_audio_bitrate < 6_000
        || config.conference.recording_audio_bitrate > 510_000
    {
        anyhow::bail!(
            "conference.recording_audio_bitrate must be between 6000 and 510000 bits per second"
        );
    }

    // RTP port range
    if config.media.rtp_port_start >= config.media.rtp_port_end {
        anyhow::bail!(
            "media.rtp_port_start ({}) must be less than rtp_port_end ({})",
            config.media.rtp_port_start,
            config.media.rtp_port_end
        );
    }

    // SIP bind addresses must be parseable as SocketAddr and WSS requires TLS paths.
    fcp_sip::validate_sip_binds(
        &config.sip.udp_bind,
        &config.sip.tcp_bind,
        config.sip.ws_bind.as_deref(),
        config.sip.wss_bind.as_deref(),
        config.sip.tls_cert_path.as_deref(),
        config.sip.tls_key_path.as_deref(),
    )?;

    // SIP/TLS listener and mutual-TLS material
    if let Some(ref tls_bind) = config.sip.tls_bind {
        tls_bind.parse::<std::net::SocketAddr>().map_err(|e| {
            anyhow::anyhow!("sip.tls_bind '{}' is not a socket address: {}", tls_bind, e)
        })?;
        if config.sip.tls_cert_path.is_none() || config.sip.tls_key_path.is_none() {
            anyhow::bail!("sip.tls_bind requires sip.tls_cert_path and sip.tls_key_path");
        }
    }
    match config.sip.tls_client_auth.as_str() {
        "required" | "optional" => {}
        other => anyhow::bail!(
            "sip.tls_client_auth must be 'required' or 'optional', got '{}'",
            other
        ),
    }
    if config.sip.tls_client_cert_path.is_some() != config.sip.tls_client_key_path.is_some() {
        anyhow::bail!("sip.tls_client_cert_path and sip.tls_client_key_path must be set together");
    }
    if config.cluster.enabled && config.cluster.trunk_reconcile_interval_secs == 0 {
        anyhow::bail!("cluster.trunk_reconcile_interval_secs must be at least 1");
    }

    // Sample rate must be a standard telephony rate
    match config.conference.sample_rate {
        8000 | 16000 | 32000 | 48000 => {}
        other => anyhow::bail!(
            "conference.sample_rate {} is not a supported rate (8000, 16000, 32000, 48000)",
            other
        ),
    }

    // Frame size should be consistent with sample rate (expect 20ms frames)
    let expected_frame_size = (config.conference.sample_rate / 50) as usize; // 20ms
    if config.conference.frame_size != expected_frame_size {
        tracing::warn!(
            configured = config.conference.frame_size,
            expected = expected_frame_size,
            sample_rate = config.conference.sample_rate,
            "conference.frame_size does not match 20ms at configured sample_rate"
        );
    }

    // Allowed codecs (if set) must be recognized names
    if let Some(ref codecs) = config.conference.allowed_codecs {
        let known = ["PCMU", "PCMA", "G722", "G729", "OPUS", "L16"];
        for codec in codecs {
            let upper = codec.to_uppercase();
            if !known.contains(&upper.as_str()) {
                anyhow::bail!(
                    "conference.allowed_codecs: unknown codec '{}' (known: {:?})",
                    codec,
                    known
                );
            }
        }
    }

    validate_http_config(&config.http)?;

    Ok(())
}

fn validate_http_config(http: &HttpSection) -> Result<()> {
    let public_bind = !http.bind_address.ip().is_loopback();
    let has_token = http
        .auth_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());

    if http.auth_enabled && !has_token {
        anyhow::bail!(
            "http.auth_enabled is true but no auth_token is configured — either set auth_token or disable auth"
        );
    }

    if public_bind && !http.auth_enabled {
        anyhow::bail!(
            "http.auth_enabled must be true when binding the conference HTTP API to a non-loopback address ({})",
            http.bind_address
        );
    }

    if public_bind && !has_token {
        anyhow::bail!(
            "http.auth_token must be set when binding the conference HTTP API to a non-loopback address ({})",
            http.bind_address
        );
    }

    Ok(())
}

/// Load configuration from file or use defaults.
pub fn load_config(config_path: Option<&Path>) -> Result<ConferenceServerConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-conference-server.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            ConferenceServerConfig::default()
        }
    };

    info!(
        node_id = %config.service.node_id,
        default_tenant_id = %config.service.default_tenant_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        http_bind = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        trunks = config.trunks.len(),
        "Configuration loaded"
    );

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<ConferenceServerConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: ConferenceServerConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

/// The settings that belong to a host rather than to the service: a node
/// keeps its own from its TOML when it loads a section every conference
/// node shares from the database. Two nodes on one host — or on hosts with
/// different ranges, directories or devices — would otherwise run on the
/// first node's RTP ports and recordings directory.
impl MediaSection {
    pub fn keep_host_settings(&mut self, own: &MediaSection) {
        self.rtp_port_start = own.rtp_port_start;
        self.rtp_port_end = own.rtp_port_end;
        self.public_ip = own.public_ip.clone();
    }
}

impl ConferenceSection {
    pub fn keep_host_settings(&mut self, own: &ConferenceSection) {
        self.recordings_dir = own.recordings_dir.clone();
        self.announcements_dir = own.announcements_dir.clone();
        self.video_device = own.video_device.clone();
        self.video_codec_threads = own.video_codec_threads;
        self.bfcp_bind = own.bfcp_bind;
        // The join secret is never stored.
        self.join_secret = own.join_secret.clone();
    }
}

impl ClusterSection {
    pub fn keep_host_settings(&mut self, own: &ClusterSection) {
        self.region = own.region.clone();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_node_keeps_its_own_host_settings_over_the_shared_section() {
        let mut shared: super::ConferenceServerConfig = toml::from_str(
            r#"
[media]
rtp_port_start = 20000
rtp_port_end = 20100
codecs = ["PCMA"]

[conference]
recordings_dir = "/var/lib/fcp/recordings-a"
recording_format = "wav"

[cluster]
region = "us-east"
trunk_srtp = false
"#,
        )
        .unwrap();
        let own: super::ConferenceServerConfig = toml::from_str(
            r#"
[media]
rtp_port_start = 30000
rtp_port_end = 30100

[conference]
recordings_dir = "/var/lib/fcp/recordings-b"
join_secret = "s3cret"

[cluster]
region = "eu-west"
"#,
        )
        .unwrap();
        shared.media.keep_host_settings(&own.media);
        shared.conference.keep_host_settings(&own.conference);
        shared.cluster.keep_host_settings(&own.cluster);
        assert_eq!(
            (shared.media.rtp_port_start, shared.media.rtp_port_end),
            (30000, 30100)
        );
        assert_eq!(
            shared.media.codecs,
            vec!["PCMA"],
            "service settings stay shared"
        );
        assert_eq!(
            shared.conference.recordings_dir,
            "/var/lib/fcp/recordings-b"
        );
        assert_eq!(shared.conference.recording_format, "wav");
        assert_eq!(shared.conference.join_secret.as_deref(), Some("s3cret"));
        assert_eq!(shared.cluster.region.as_deref(), Some("eu-west"));
        assert!(!shared.cluster.trunk_srtp);
    }

    use super::*;

    #[test]
    fn test_default_config() {
        let config = ConferenceServerConfig::default();
        assert_eq!(config.service.service_type, "conference");
        assert_eq!(config.service.default_tenant_id, fcp_core::TenantId::nil());
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.http.bind_address, "127.0.0.1:8090".parse().unwrap());
        assert_eq!(config.conference.sample_rate, 8000);
        assert_eq!(config.conference.frame_size, 160);
        assert!(config.conference.enable_vad);
        assert!(config.conference.auto_destroy_empty_rooms);
        assert_eq!(config.conference.max_pin_attempts, 3);
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "conference"
node_id = "conf-test-01"
default_tenant_id = "00000000-0000-0000-0000-000000000123"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
local_ip = "192.168.1.100"

[storage]
database_url = "postgresql://localhost/fcp"

[http]
bind_address = "0.0.0.0:8090"

[media]
rtp_port_start = 20000
rtp_port_end = 30000
codecs = ["PCMU", "PCMA"]

[conference]
sample_rate = 8000
frame_size = 160
enable_vad = true
default_max_participants = 100
auto_destroy_empty_rooms = true
recordings_dir = "/data/recordings"
pin_timeout_secs = 15
max_pin_attempts = 5
"#;
        let config: ConferenceServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "conf-test-01");
        assert_eq!(
            config.service.default_tenant_id,
            "00000000-0000-0000-0000-000000000123".parse().unwrap()
        );
        assert_eq!(config.conference.default_max_participants, 100);
        assert_eq!(config.conference.pin_timeout_secs, 15);
        assert_eq!(config.conference.max_pin_attempts, 5);
    }

    #[test]
    fn test_public_http_bind_requires_auth() {
        let http = HttpSection {
            bind_address: "0.0.0.0:8090".parse().unwrap(),
            auth_enabled: false,
            auth_token: None,
            rate_limit_rps: None,
            max_body_size_kb: default_max_body_size_kb(),
            advertised_address: None,
        };
        assert!(validate_http_config(&http).is_err());

        let http = HttpSection {
            bind_address: "0.0.0.0:8090".parse().unwrap(),
            auth_enabled: true,
            auth_token: None,
            rate_limit_rps: None,
            max_body_size_kb: default_max_body_size_kb(),
            advertised_address: None,
        };
        assert!(validate_http_config(&http).is_err());

        let http = HttpSection {
            bind_address: "0.0.0.0:8090".parse().unwrap(),
            auth_enabled: true,
            auth_token: Some("secret".to_string()),
            rate_limit_rps: None,
            max_body_size_kb: default_max_body_size_kb(),
            advertised_address: None,
        };
        assert!(validate_http_config(&http).is_ok());
    }

    #[test]
    fn tls_bind_requires_certificate_material() {
        let mut config = ConferenceServerConfig::default();
        config.sip.tls_bind = Some("0.0.0.0:5061".to_string());
        assert!(
            validate_config(&config).is_err(),
            "tls_bind without cert/key"
        );

        config.sip.tls_cert_path = Some("/etc/fcp/node.crt".to_string());
        config.sip.tls_key_path = Some("/etc/fcp/node.key".to_string());
        assert!(validate_config(&config).is_ok());

        config.sip.tls_bind = Some("not-an-address".to_string());
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn tls_client_auth_mode_and_identity_pairing_are_checked() {
        let mut config = ConferenceServerConfig::default();
        config.sip.tls_client_auth = "sometimes".to_string();
        assert!(validate_config(&config).is_err());
        config.sip.tls_client_auth = "optional".to_string();
        assert!(validate_config(&config).is_ok());

        config.sip.tls_client_cert_path = Some("/etc/fcp/client.crt".to_string());
        assert!(validate_config(&config).is_err(), "cert without key");
        config.sip.tls_client_key_path = Some("/etc/fcp/client.key".to_string());
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn tls_client_identity_falls_back_to_listener_certificate() {
        let mut sip = SipSection::default();
        assert!(sip.tls_client_identity().is_none());
        sip.tls_cert_path = Some("node.crt".to_string());
        sip.tls_key_path = Some("node.key".to_string());
        assert_eq!(sip.tls_client_identity(), Some(("node.crt", "node.key")));
        sip.tls_client_cert_path = Some("client.crt".to_string());
        sip.tls_client_key_path = Some("client.key".to_string());
        assert_eq!(
            sip.tls_client_identity(),
            Some(("client.crt", "client.key"))
        );
    }

    #[test]
    fn advertised_tls_address_substitutes_local_ip_for_unspecified_bind() {
        let mut sip = SipSection::default();
        assert!(sip.advertised_tls_address().is_none());
        sip.tls_bind = Some("0.0.0.0:5061".to_string());
        assert!(
            sip.advertised_tls_address().is_none(),
            "nothing routable to advertise"
        );
        sip.local_ip = Some("10.0.0.20".to_string());
        assert_eq!(
            sip.advertised_tls_address(),
            Some("10.0.0.20:5061".parse().unwrap())
        );
        sip.tls_bind = Some("10.0.0.21:5062".to_string());
        assert_eq!(
            sip.advertised_tls_address(),
            Some("10.0.0.21:5062".parse().unwrap())
        );
    }

    #[test]
    fn advertised_http_address_prefers_explicit_then_routable_bind() {
        let mut config = ConferenceServerConfig::default();
        // Default bind is loopback: routable as-is.
        assert_eq!(config.advertised_http_address(), config.http.bind_address);

        config.http.bind_address = "0.0.0.0:8090".parse().unwrap();
        // Nothing better known: the bind is all there is.
        assert_eq!(
            config.advertised_http_address(),
            "0.0.0.0:8090".parse().unwrap()
        );
        config.sip.local_ip = Some("10.0.0.20".to_string());
        assert_eq!(
            config.advertised_http_address(),
            "10.0.0.20:8090".parse().unwrap()
        );
        config.http.advertised_address = Some("192.0.2.5:9000".parse().unwrap());
        assert_eq!(
            config.advertised_http_address(),
            "192.0.2.5:9000".parse().unwrap()
        );
    }

    #[test]
    fn resilience_settings_have_safe_defaults_and_parse() {
        let defaults = ConferenceServerConfig::default();
        assert_eq!(
            defaults.service.drain_wait_secs, 0,
            "hang up at once, as before"
        );
        assert_eq!(defaults.cluster.trunk_rtp_timeout_secs, 30);
        assert!(defaults.cluster.trunk_srtp);

        let toml_str = r#"
[service]
drain_wait_secs = 120

[cluster]
trunk_rtp_timeout_secs = 0
trunk_srtp = false
"#;
        let config: ConferenceServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.drain_wait_secs, 120);
        assert_eq!(config.cluster.trunk_rtp_timeout_secs, 0);
        assert!(!config.cluster.trunk_srtp);
    }

    #[test]
    fn cluster_secret_and_advertised_address_parse() {
        let toml_str = r#"
[http]
bind_address = "0.0.0.0:8090"
advertised_address = "10.0.0.20:8090"

[cluster]
enabled = true
secret = "s3cret"
"#;
        let config: ConferenceServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.cluster.secret.as_deref(), Some("s3cret"));
        assert_eq!(
            config.http.advertised_address,
            Some("10.0.0.20:8090".parse().unwrap())
        );
        let defaults = ConferenceServerConfig::default();
        assert!(defaults.cluster.secret.is_none());
        assert!(defaults.http.advertised_address.is_none());
    }

    #[test]
    fn trunk_reconcile_interval_must_be_positive_in_cluster_mode() {
        let mut config = ConferenceServerConfig::default();
        assert_eq!(config.cluster.trunk_reconcile_interval_secs, 2);
        config.cluster.enabled = true;
        config.cluster.trunk_reconcile_interval_secs = 0;
        assert!(validate_config(&config).is_err());
    }
}
