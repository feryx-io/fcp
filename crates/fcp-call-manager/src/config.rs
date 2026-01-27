//! Configuration loading and management for fcp-call-manager.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Call manager server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CallManagerConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceSection,

    /// SIP transport configuration
    #[serde(default)]
    pub sip: SipSection,

    /// Registrar behavior configuration
    #[serde(default)]
    pub registrar: RegistrarSection,

    /// Storage configuration
    #[serde(default)]
    pub storage: StorageSection,

    /// HTTP health/API endpoint configuration
    #[serde(default)]
    pub http: HttpSection,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingSection,

    /// Call handling configuration
    #[serde(default)]
    pub calls: CallsSection,

    /// Media/RTP configuration
    #[serde(default)]
    pub media: MediaSection,
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
    "call-manager".to_string()
}

fn default_node_id() -> String {
    format!("cm-{}", uuid::Uuid::new_v4())
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

    /// Local SIP URI (default: sip:call-manager@<local_ip>)
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

/// Registrar behavior configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegistrarSection {
    /// Enable SIP Digest authentication
    #[serde(default)]
    pub auth_enabled: bool,

    /// Authentication realm
    #[serde(default = "default_realm")]
    pub realm: String,

    /// Default registration expiry in seconds
    #[serde(default = "default_expires")]
    pub default_expires: u32,

    /// Minimum registration expiry in seconds
    #[serde(default = "default_min_expires")]
    pub min_expires: u32,

    /// Maximum registration expiry in seconds
    #[serde(default = "default_max_expires")]
    pub max_expires: u32,
}

fn default_realm() -> String {
    "fcp.local".to_string()
}

fn default_expires() -> u32 {
    3600
}

fn default_min_expires() -> u32 {
    60
}

fn default_max_expires() -> u32 {
    86400
}

impl Default for RegistrarSection {
    fn default() -> Self {
        Self {
            auth_enabled: false,
            realm: default_realm(),
            default_expires: default_expires(),
            min_expires: default_min_expires(),
            max_expires: default_max_expires(),
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
    "sqlite://data/fcp.db".to_string()
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
    /// Bind address for health/status/calls endpoint
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,
}

fn default_http_bind() -> SocketAddr {
    "0.0.0.0:8084".parse().unwrap()
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

/// Call handling configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CallsSection {
    /// Maximum concurrent calls
    #[serde(default = "default_max_concurrent_calls")]
    pub max_concurrent_calls: u32,

    /// Max ring time before 408 timeout (seconds)
    #[serde(default = "default_call_timeout")]
    pub call_timeout_seconds: u32,

    /// Generate SDP offers in INVITE
    #[serde(default = "default_true")]
    pub enable_sdp: bool,

    /// Support REFER call transfer
    #[serde(default = "default_true")]
    pub enable_refer: bool,
}

fn default_max_concurrent_calls() -> u32 {
    1000
}

fn default_call_timeout() -> u32 {
    60
}

fn default_true() -> bool {
    true
}

impl Default for CallsSection {
    fn default() -> Self {
        Self {
            max_concurrent_calls: default_max_concurrent_calls(),
            call_timeout_seconds: default_call_timeout(),
            enable_sdp: true,
            enable_refer: true,
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
        }
    }
}

/// Load configuration from file or use defaults.
///
/// Priority:
/// 1. File specified via --config or FCP_CALL_MANAGER_CONFIG
/// 2. ./fcp-call-manager.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<CallManagerConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-call-manager.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            CallManagerConfig::default()
        }
    };

    info!(
        node_id = %config.service.node_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        auth_enabled = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        storage_backend = %config.storage.backend,
        http_bind = %config.http.bind_address,
        max_calls = config.calls.max_concurrent_calls,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        "Configuration loaded"
    );

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<CallManagerConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: CallManagerConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CallManagerConfig::default();
        assert_eq!(config.service.service_type, "call-manager");
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.sip.tcp_bind, "0.0.0.0:5060");
        assert!(!config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "fcp.local");
        assert_eq!(config.registrar.default_expires, 3600);
        assert_eq!(config.storage.backend, "sqlite");
        assert_eq!(config.http.bind_address.port(), 8084);
        assert_eq!(config.calls.max_concurrent_calls, 1000);
        assert_eq!(config.calls.call_timeout_seconds, 60);
        assert!(config.calls.enable_sdp);
        assert!(config.calls.enable_refer);
        assert_eq!(config.media.rtp_port_start, 10000);
        assert_eq!(config.media.rtp_port_end, 20000);
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "call-manager"
node_id = "cm-test-01"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
local_ip = "192.168.1.100"

[registrar]
auth_enabled = true
realm = "test.local"
default_expires = 1800

[storage]
backend = "sqlite"
connection_string = "sqlite:///app/data/fcp.db"

[http]
bind_address = "0.0.0.0:9090"

[calls]
max_concurrent_calls = 500
call_timeout_seconds = 30
enable_sdp = true
enable_refer = false

[media]
rtp_port_start = 20000
rtp_port_end = 30000
codecs = ["PCMU", "PCMA", "G722"]

[logging]
level = "debug"
format = "pretty"
"#;
        let config: CallManagerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "cm-test-01");
        assert!(config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "test.local");
        assert_eq!(config.registrar.default_expires, 1800);
        assert_eq!(config.http.bind_address.port(), 9090);
        assert_eq!(config.calls.max_concurrent_calls, 500);
        assert!(!config.calls.enable_refer);
        assert_eq!(config.media.rtp_port_start, 20000);
        assert_eq!(config.media.codecs.len(), 3);
        assert_eq!(config.sip.local_ip.as_deref(), Some("192.168.1.100"));
    }
}
