//! Configuration loading and management for fcp-ivr.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// IVR server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IvrConfig {
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

    /// IVR flow configuration
    #[serde(default)]
    pub flows: FlowsSection,

    /// B2BUA behavior configuration (extension threshold, topology hiding, etc.)
    #[serde(default)]
    pub b2bua: fcp_b2bua_core::B2buaConfig,

    /// SIP trunk configurations (for Dial node outbound calls).
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
    "ivr".to_string()
}

fn default_node_id() -> String {
    format!("ivr-{}", uuid::Uuid::new_v4())
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

    /// Local SIP URI (default: sip:ivr@<local_ip>)
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
    /// Backend type: "sqlite", "postgres", "mysql", "mongodb"
    #[serde(default = "default_backend")]
    pub backend: String,

    /// Backend-specific connection string
    #[serde(default = "default_connection_string")]
    pub connection_string: String,

    /// Connection pool size (for PG/MySQL/Mongo)
    #[serde(default)]
    pub max_connections: Option<u32>,
}

fn default_backend() -> String {
    "sqlite".to_string()
}

fn default_connection_string() -> String {
    "sqlite://data/fcp-ivr.db".to_string()
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

/// HTTP health/API endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpSection {
    /// Bind address for health/status/flows endpoint
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,
}

fn default_http_bind() -> SocketAddr {
    "0.0.0.0:8086".parse().unwrap()
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
    /// Example: "http://fcp-events-server:8081/ingest"
    pub server_url: Option<String>,
}

impl Default for EventsSection {
    fn default() -> Self {
        Self { server_url: None }
    }
}

/// IVR flow configuration (IVR-specific).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlowsSection {
    /// Default flow ID to use when no DID route matches.
    pub default_flow_id: Option<String>,

    /// Static DID-to-flow-ID routing table.
    #[serde(default)]
    pub routes: Vec<DidRoute>,

    /// TTS provider ("none", future: "google", "aws")
    #[serde(default = "default_tts_provider")]
    pub tts_provider: String,

    /// TTS API key (provider-specific)
    pub tts_api_key: Option<String>,

    /// TTS language code (e.g. "en-US")
    #[serde(default = "default_tts_language")]
    pub tts_language: String,

    /// TTS voice name (provider-specific)
    pub tts_voice: Option<String>,

    /// Search path for Play node audio files
    pub audio_path: Option<String>,
}

impl Default for FlowsSection {
    fn default() -> Self {
        Self {
            default_flow_id: None,
            routes: Vec::new(),
            tts_provider: default_tts_provider(),
            tts_api_key: None,
            tts_language: default_tts_language(),
            tts_voice: None,
            audio_path: None,
        }
    }
}

fn default_tts_provider() -> String {
    "none".to_string()
}

fn default_tts_language() -> String {
    "en-US".to_string()
}

/// A static DID-to-flow mapping.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DidRoute {
    /// Phone number / DID (e.g. "+15551234567")
    pub did: String,

    /// Flow ID to execute (e.g. "main-menu")
    pub flow_id: String,
}

/// Load configuration from file or use defaults.
///
/// Priority:
/// 1. File specified via --config or FCP_IVR_CONFIG
/// 2. ./fcp-ivr.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<IvrConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-ivr.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            IvrConfig::default()
        }
    };

    info!(
        node_id = %config.service.node_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        storage_backend = %config.storage.backend,
        http_bind = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        did_routes = config.flows.routes.len(),
        default_flow = config.flows.default_flow_id.as_deref().unwrap_or("none"),
        trunks = config.trunks.len(),
        "Configuration loaded"
    );

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<IvrConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: IvrConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = IvrConfig::default();
        assert_eq!(config.service.service_type, "ivr");
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.sip.tcp_bind, "0.0.0.0:5060");
        assert_eq!(config.storage.backend, "sqlite");
        assert_eq!(config.http.bind_address.port(), 8086);
        assert_eq!(config.media.rtp_port_start, 10000);
        assert_eq!(config.media.rtp_port_end, 20000);
        assert!(config.events.server_url.is_none());
        assert!(config.trunks.is_empty());
        // IVR-specific defaults
        assert!(config.flows.default_flow_id.is_none());
        assert!(config.flows.routes.is_empty());
        assert_eq!(config.flows.tts_provider, "none");
        assert_eq!(config.flows.tts_language, "en-US");
        assert!(config.flows.tts_api_key.is_none());
        assert!(config.flows.tts_voice.is_none());
        assert!(config.flows.audio_path.is_none());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "ivr"
node_id = "ivr-test-01"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
local_ip = "192.168.1.100"

[storage]
backend = "sqlite"
connection_string = "sqlite:///app/data/fcp-ivr.db"

[http]
bind_address = "0.0.0.0:8086"

[media]
rtp_port_start = 20000
rtp_port_end = 30000
codecs = ["PCMU", "PCMA", "G722"]

[logging]
level = "debug"
format = "pretty"

[events]
server_url = "http://fcp-events-server:8081/ingest"

[flows]
default_flow_id = "fallback-menu"
tts_provider = "none"
tts_language = "en-US"
audio_path = "/var/lib/fcp/audio"

[[flows.routes]]
did = "+15551234567"
flow_id = "main-menu"

[[flows.routes]]
did = "+15559876543"
flow_id = "support-queue"
"#;
        let config: IvrConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "ivr-test-01");
        assert_eq!(config.http.bind_address.port(), 8086);
        assert_eq!(config.media.rtp_port_start, 20000);
        assert_eq!(config.media.codecs.len(), 3);
        assert_eq!(config.sip.local_ip.as_deref(), Some("192.168.1.100"));
        assert_eq!(
            config.events.server_url.as_deref(),
            Some("http://fcp-events-server:8081/ingest")
        );
        // IVR-specific
        assert_eq!(
            config.flows.default_flow_id.as_deref(),
            Some("fallback-menu")
        );
        assert_eq!(config.flows.routes.len(), 2);
        assert_eq!(config.flows.routes[0].did, "+15551234567");
        assert_eq!(config.flows.routes[0].flow_id, "main-menu");
        assert_eq!(config.flows.routes[1].did, "+15559876543");
        assert_eq!(config.flows.routes[1].flow_id, "support-queue");
        assert_eq!(config.flows.tts_language, "en-US");
        assert_eq!(
            config.flows.audio_path.as_deref(),
            Some("/var/lib/fcp/audio")
        );
    }
}
