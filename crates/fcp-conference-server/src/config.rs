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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,
}

fn default_service_type() -> String {
    "conference".to_string()
}

fn default_node_id() -> String {
    format!("conference-{}", uuid::Uuid::new_v4())
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
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
}

fn default_http_bind() -> SocketAddr {
    "0.0.0.0:8090".parse().unwrap()
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
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
        }
    }
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
