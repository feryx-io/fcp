//! Configuration loading and management for fcp-voicemail-server.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Voicemail server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct VoicemailServerConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceSection,

    /// SIP transport configuration
    #[serde(default)]
    pub sip: SipSection,

    /// Storage configuration
    #[serde(default)]
    pub storage: StorageSection,

    /// HTTP health/API endpoint configuration
    #[serde(default)]
    pub http: HttpSection,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingSection,

    /// Media/RTP configuration
    #[serde(default)]
    pub media: MediaSection,

    /// Events configuration
    #[serde(default)]
    pub events: EventsSection,

    /// B2BUA behavior configuration (extension threshold, topology hiding, etc.)
    #[serde(default)]
    pub b2bua: fcp_b2bua_core::B2buaConfig,

    /// Voicemail-specific configuration
    #[serde(default)]
    pub voicemail: VoicemailSection,

    /// SIP trunk configurations (optional — for outbound MWI NOTIFY or transfers).
    #[serde(default)]
    pub trunks: Vec<fcp_sbc_core::TrunkConfig>,
}

/// Service identification.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,
}

fn default_service_type() -> String {
    "voicemail".to_string()
}

fn default_node_id() -> String {
    format!("voicemail-{}", uuid::Uuid::new_v4())
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
        }
    }
}

/// SIP transport configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SipSection {
    /// UDP bind address
    #[serde(default = "default_sip_bind")]
    pub udp_bind: String,

    /// TCP bind address
    #[serde(default = "default_sip_bind")]
    pub tcp_bind: String,

    /// Local IP for SDP/Contact (auto-detect if not set)
    pub local_ip: Option<String>,

    /// Local SIP URI (default: sip:voicemail@<local_ip>)
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

/// Storage backend configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageSection {
    /// PostgreSQL connection URL
    #[serde(default = "default_database_url")]
    pub database_url: String,

    /// Maximum database connections
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
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
            max_connections: default_max_connections(),
        }
    }
}

/// HTTP health/API endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpSection {
    /// Bind address for health/status/voicemail endpoint
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,
}

fn default_http_bind() -> SocketAddr {
    "0.0.0.0:8087".parse().unwrap()
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
        }
    }
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingSection {
    /// Log level
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log format (json, pretty, compact)
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

/// Media/RTP configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaSection {
    /// RTP port range start
    #[serde(default = "default_rtp_port_start")]
    pub rtp_port_start: u16,

    /// RTP port range end
    #[serde(default = "default_rtp_port_end")]
    pub rtp_port_end: u16,

    /// Supported codecs
    #[serde(default = "default_codecs")]
    pub codecs: Vec<String>,

    /// Public IP for SDP/RTP (required when behind NAT or in Docker)
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

/// Events configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventsSection {
    /// URL of fcp-events-server ingest endpoint (optional)
    pub server_url: Option<String>,
}

impl Default for EventsSection {
    fn default() -> Self {
        Self { server_url: None }
    }
}

/// Voicemail-specific configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoicemailSection {
    /// Directory for voicemail recordings
    #[serde(default = "default_recordings_dir")]
    pub recordings_dir: String,

    /// Directory for voicemail greetings
    #[serde(default = "default_greetings_dir")]
    pub greetings_dir: String,

    /// Maximum message duration in seconds
    #[serde(default = "default_max_message_duration")]
    pub max_message_duration: u32,

    /// Maximum messages per mailbox
    #[serde(default = "default_max_messages_per_mailbox")]
    pub max_messages_per_mailbox: u32,

    /// Play beep before recording
    #[serde(default = "default_play_beep")]
    pub play_beep: bool,

    /// Path to default system greeting audio file
    pub default_greeting_path: Option<String>,

    /// Path to beep audio file
    pub beep_audio_path: Option<String>,

    /// Static DID/extension → mailbox_id routing table
    #[serde(default)]
    pub routes: Vec<VoicemailRoute>,

    /// Default mailbox ID when no route matches
    pub default_mailbox_id: Option<String>,
}

fn default_recordings_dir() -> String {
    "/var/fcp/voicemail/recordings".to_string()
}

fn default_greetings_dir() -> String {
    "/var/fcp/voicemail/greetings".to_string()
}

fn default_max_message_duration() -> u32 {
    300
}

fn default_max_messages_per_mailbox() -> u32 {
    100
}

fn default_play_beep() -> bool {
    true
}

impl Default for VoicemailSection {
    fn default() -> Self {
        Self {
            recordings_dir: default_recordings_dir(),
            greetings_dir: default_greetings_dir(),
            max_message_duration: default_max_message_duration(),
            max_messages_per_mailbox: default_max_messages_per_mailbox(),
            play_beep: default_play_beep(),
            default_greeting_path: None,
            beep_audio_path: None,
            routes: Vec::new(),
            default_mailbox_id: None,
        }
    }
}

/// A static DID/extension-to-mailbox mapping.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoicemailRoute {
    /// Phone number / DID / extension (e.g. "+15551234567" or "5000")
    pub did: String,

    /// Mailbox ID to route to
    pub mailbox_id: String,
}

/// Load configuration from file or use defaults.
///
/// Priority:
/// 1. File specified via --config or FCP_VOICEMAIL_CONFIG
/// 2. ./fcp-voicemail-server.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<VoicemailServerConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-voicemail-server.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            VoicemailServerConfig::default()
        }
    };

    info!(
        node_id = %config.service.node_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        database_url = %config.storage.database_url,
        http_bind = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        voicemail_routes = config.voicemail.routes.len(),
        default_mailbox = config.voicemail.default_mailbox_id.as_deref().unwrap_or("none"),
        trunks = config.trunks.len(),
        "Configuration loaded"
    );

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<VoicemailServerConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: VoicemailServerConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = VoicemailServerConfig::default();
        assert_eq!(config.service.service_type, "voicemail");
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.sip.tcp_bind, "0.0.0.0:5060");
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.http.bind_address.port(), 8087);
        assert_eq!(config.media.rtp_port_start, 10000);
        assert_eq!(config.media.rtp_port_end, 20000);
        assert!(config.events.server_url.is_none());
        assert!(config.trunks.is_empty());
        // Voicemail-specific defaults
        assert_eq!(config.voicemail.recordings_dir, "/var/fcp/voicemail/recordings");
        assert_eq!(config.voicemail.greetings_dir, "/var/fcp/voicemail/greetings");
        assert_eq!(config.voicemail.max_message_duration, 300);
        assert_eq!(config.voicemail.max_messages_per_mailbox, 100);
        assert!(config.voicemail.play_beep);
        assert!(config.voicemail.routes.is_empty());
        assert!(config.voicemail.default_mailbox_id.is_none());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "voicemail"
node_id = "vm-test-01"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
local_ip = "192.168.1.100"

[storage]
database_url = "postgresql://localhost/fcp"

[http]
bind_address = "0.0.0.0:8087"

[media]
rtp_port_start = 20000
rtp_port_end = 30000
codecs = ["PCMU", "PCMA"]

[logging]
level = "debug"
format = "pretty"

[events]
server_url = "http://fcp-events-server:8081/ingest"

[voicemail]
recordings_dir = "/data/recordings"
greetings_dir = "/data/greetings"
max_message_duration = 120
max_messages_per_mailbox = 50
play_beep = true
default_greeting_path = "/var/fcp/audio/default-greeting.wav"
beep_audio_path = "/var/fcp/audio/beep.wav"
default_mailbox_id = "general"

[[voicemail.routes]]
did = "+15551234567"
mailbox_id = "sales"

[[voicemail.routes]]
did = "5000"
mailbox_id = "support"
"#;
        let config: VoicemailServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "vm-test-01");
        assert_eq!(config.http.bind_address.port(), 8087);
        assert_eq!(config.media.rtp_port_start, 20000);
        assert_eq!(config.sip.local_ip.as_deref(), Some("192.168.1.100"));
        assert_eq!(
            config.events.server_url.as_deref(),
            Some("http://fcp-events-server:8081/ingest")
        );
        // Voicemail-specific
        assert_eq!(config.voicemail.recordings_dir, "/data/recordings");
        assert_eq!(config.voicemail.greetings_dir, "/data/greetings");
        assert_eq!(config.voicemail.max_message_duration, 120);
        assert_eq!(config.voicemail.max_messages_per_mailbox, 50);
        assert!(config.voicemail.play_beep);
        assert_eq!(config.voicemail.routes.len(), 2);
        assert_eq!(config.voicemail.routes[0].did, "+15551234567");
        assert_eq!(config.voicemail.routes[0].mailbox_id, "sales");
        assert_eq!(config.voicemail.routes[1].did, "5000");
        assert_eq!(config.voicemail.routes[1].mailbox_id, "support");
        assert_eq!(
            config.voicemail.default_mailbox_id.as_deref(),
            Some("general")
        );
    }
}
