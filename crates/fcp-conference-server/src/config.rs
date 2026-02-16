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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,

    /// Maximum time (seconds) to wait for graceful shutdown before force-closing sessions.
    /// Default: 30. Set 0 for unlimited (not recommended).
    #[serde(default = "default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,
}

fn default_service_type() -> String {
    "conference".to_string()
}

fn default_shutdown_timeout_secs() -> u64 {
    30
}

fn default_node_id() -> String {
    format!("conference-{}", uuid::Uuid::new_v4())
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
            shutdown_timeout_secs: default_shutdown_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SipSection {
    #[serde(default = "default_sip_bind")]
    pub udp_bind: String,

    #[serde(default = "default_sip_bind")]
    pub tcp_bind: String,

    pub local_ip: Option<String>,

    pub local_uri: Option<String>,
}

fn default_sip_bind() -> String {
    "0.0.0.0:5060".to_string()
}

impl Default for SipSection {
    fn default() -> Self {
        Self {
            udp_bind: default_sip_bind(),
            tcp_bind: default_sip_bind(),
            local_ip: None,
            local_uri: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageSection {
    #[serde(default = "default_backend")]
    pub backend: String,

    #[serde(default = "default_connection_string")]
    pub connection_string: String,

    #[serde(default)]
    pub max_connections: Option<u32>,
}

fn default_backend() -> String {
    "sqlite".to_string()
}

fn default_connection_string() -> String {
    "sqlite://data/fcp-conference.db".to_string()
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            connection_string: default_connection_string(),
            max_connections: None,
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
}

fn default_http_bind() -> SocketAddr {
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 8090)
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
            auth_enabled: false,
            auth_token: None,
            rate_limit_rps: None,
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
}

impl Default for EventsSection {
    fn default() -> Self {
        Self { server_url: None }
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

    /// Directory for pre-recorded announcement WAV files.
    #[serde(default = "default_announcements_dir")]
    pub announcements_dir: String,

    /// Maximum time (seconds) a participant waits for a moderator before being disconnected.
    /// Default: 300 (5 minutes). Set 0 for unlimited.
    #[serde(default = "default_max_wait_for_moderator_secs")]
    pub max_wait_for_moderator_secs: u64,

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
    60
}

fn default_max_wait_for_moderator_secs() -> u64 {
    300
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
            announcements_dir: default_announcements_dir(),
            max_wait_for_moderator_secs: default_max_wait_for_moderator_secs(),
            max_invite_rate_per_ip: default_max_invite_rate(),
            session_expires_secs: default_session_expires_secs(),
            max_recording_size_mb: default_max_recording_size_mb(),
            max_participants_per_node: default_max_participants_per_node(),
        }
    }
}

/// Validate configuration for logical consistency.
/// Called at startup to fail fast on misconfigured values.
pub fn validate_config(config: &ConferenceServerConfig) -> Result<()> {
    // RTP port range
    if config.media.rtp_port_start >= config.media.rtp_port_end {
        anyhow::bail!(
            "media.rtp_port_start ({}) must be less than rtp_port_end ({})",
            config.media.rtp_port_start,
            config.media.rtp_port_end
        );
    }

    // SIP bind addresses must be parseable as SocketAddr
    config
        .sip
        .udp_bind
        .parse::<std::net::SocketAddr>()
        .with_context(|| format!("Invalid sip.udp_bind: '{}'", config.sip.udp_bind))?;
    config
        .sip
        .tcp_bind
        .parse::<std::net::SocketAddr>()
        .with_context(|| format!("Invalid sip.tcp_bind: '{}'", config.sip.tcp_bind))?;

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

    // Auth enabled requires a token
    if config.http.auth_enabled && config.http.auth_token.is_none() {
        tracing::warn!("http.auth_enabled is true but no auth_token is configured");
    }

    // Storage backend must be recognized
    match config.storage.backend.as_str() {
        "sqlite" | "postgres" | "mysql" | "mongodb" => {}
        other => anyhow::bail!(
            "storage.backend '{}' is not supported (sqlite, postgres, mysql, mongodb)",
            other
        ),
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
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        storage_backend = %config.storage.backend,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ConferenceServerConfig::default();
        assert_eq!(config.service.service_type, "conference");
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.storage.backend, "sqlite");
        assert_eq!(
            config.storage.connection_string,
            "sqlite://data/fcp-conference.db"
        );
        assert_eq!(config.http.bind_address.port(), 8090);
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

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
local_ip = "192.168.1.100"

[storage]
backend = "sqlite"
connection_string = "sqlite:///app/data/fcp-conference.db"

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
        assert_eq!(config.conference.default_max_participants, 100);
        assert_eq!(config.conference.pin_timeout_secs, 15);
        assert_eq!(config.conference.max_pin_attempts, 5);
    }
}
