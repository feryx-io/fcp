//! Configuration loading and management for fcp-registrar.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Registrar server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RegistrarConfig {
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

    /// HTTP health endpoint configuration
    #[serde(default)]
    pub http: HttpSection,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingSection,
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
    "registrar".to_string()
}

fn default_node_id() -> String {
    format!("registrar-{}", uuid::Uuid::new_v4())
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
}

fn default_sip_bind() -> String {
    "0.0.0.0:5060".to_string()
}

impl Default for SipSection {
    fn default() -> Self {
        Self {
            udp_bind: default_sip_bind(),
            tcp_bind: default_sip_bind(),
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

/// HTTP health endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpSection {
    /// Bind address for health/status endpoint
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,
}

fn default_http_bind() -> SocketAddr {
    "0.0.0.0:8083".parse().unwrap()
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

/// Load configuration from file or use defaults.
///
/// Priority:
/// 1. File specified via --config or FCP_REGISTRAR_CONFIG
/// 2. ./fcp-registrar.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<RegistrarConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-registrar.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            RegistrarConfig::default()
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
        "Configuration loaded"
    );

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<RegistrarConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: RegistrarConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RegistrarConfig::default();
        assert_eq!(config.service.service_type, "registrar");
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.sip.tcp_bind, "0.0.0.0:5060");
        assert!(!config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "fcp.local");
        assert_eq!(config.registrar.default_expires, 3600);
        assert_eq!(config.storage.backend, "sqlite");
        assert_eq!(config.http.bind_address.port(), 8083);
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "registrar"
node_id = "reg-test-01"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"

[registrar]
auth_enabled = true
realm = "test.local"
default_expires = 1800

[storage]
backend = "sqlite"
connection_string = "sqlite:///app/data/fcp.db"

[http]
bind_address = "0.0.0.0:9090"

[logging]
level = "debug"
format = "pretty"
"#;
        let config: RegistrarConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "reg-test-01");
        assert!(config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "test.local");
        assert_eq!(config.registrar.default_expires, 1800);
        assert_eq!(config.http.bind_address.port(), 9090);
    }
}
