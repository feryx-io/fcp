use crate::Error;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

const MAX_SIP_MESSAGE_SIZE: usize = 65_535;
const MAX_CONNECTION_POOL_SIZE: usize = 10_000;
const MAX_RTP_PORT_SPAN: u16 = 10_000;

/// Complete runtime configuration for FCP.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FcpConfig {
    /// Deployment mode (carrier, service_provider, enterprise)
    #[serde(default)]
    pub deployment_mode: crate::DeploymentMode,

    /// Node configuration
    pub node: NodeConfig,

    /// Transport configuration
    pub transport: TransportConfig,

    /// Media configuration
    pub media: MediaConfig,

    /// Storage configuration
    pub storage: StorageConfig,

    /// Registrar configuration
    pub registrar: RegistrarConfig,

    /// API configuration
    pub api: ApiConfig,

    /// Observability configuration
    pub observability: ObservabilityConfig,

    /// Routing configuration
    pub routing: RoutingConfig,
}

impl FcpConfig {
    /// Validate configuration values for common bounds and CRLF safety.
    pub fn validate(&self) -> Result<(), Error> {
        self.node.validate().map_err(Error::Config)?;
        self.transport.validate().map_err(Error::Config)?;
        self.media.validate().map_err(Error::Config)?;
        self.storage.validate().map_err(Error::Config)?;
        self.registrar.validate().map_err(Error::Config)?;
        self.api.validate().map_err(Error::Config)?;
        self.observability.validate().map_err(Error::Config)?;
        self.routing.validate().map_err(Error::Config)?;
        Ok(())
    }
}

/// Node-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique node identifier (auto-generated if not provided)
    pub node_id: Option<String>,

    /// Human-friendly node name
    pub name: String,

    /// Local domain for SIP URIs (e.g., "pbx.example.com")
    pub local_domain: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            node_id: None,
            name: "fcp-node".to_string(),
            local_domain: "fcp.local".to_string(),
        }
    }
}

fn reject_crlf(field: &str, value: &str) -> Result<(), String> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n')) {
        Err(format!("{field} must not contain CR or LF characters"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        let config = FcpConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_bad_values() {
        let mut config = FcpConfig::default();
        config.transport.max_message_size = MAX_SIP_MESSAGE_SIZE + 1;
        assert!(matches!(config.validate(), Err(Error::Config(_))));

        config.transport.max_message_size = MAX_SIP_MESSAGE_SIZE;
        config.node.name = "bad\r\nname".to_string();
        assert!(matches!(config.validate(), Err(Error::Config(_))));
    }
}
impl NodeConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("node.name must not be empty".to_string());
        }
        if self.local_domain.is_empty() {
            return Err("node.local_domain must not be empty".to_string());
        }
        reject_crlf("node.name", &self.name)?;
        reject_crlf("node.local_domain", &self.local_domain)?;
        Ok(())
    }
}

/// Transport layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// UDP bind address
    pub udp_bind: Option<SocketAddr>,

    /// TCP bind address
    pub tcp_bind: Option<SocketAddr>,

    /// TLS bind address (SIPS)
    pub tls_bind: Option<SocketAddr>,

    /// TLS certificate path (PEM format)
    pub tls_cert: Option<PathBuf>,

    /// TLS private key path (PEM format)
    pub tls_key: Option<PathBuf>,

    /// Maximum message size (bytes)
    pub max_message_size: usize,

    /// Connection pool size for TCP/TLS
    pub connection_pool_size: usize,

    /// Connection idle timeout
    pub connection_timeout: Duration,

    /// Local IP address for SDP (if not set, derived from bind address)
    pub local_ip: Option<String>,

    /// RTP port for media
    pub rtp_port: u16,

    /// Enable SDP support for calls
    pub enable_sdp: bool,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            udp_bind: Some("0.0.0.0:5060".parse().unwrap()),
            tcp_bind: Some("0.0.0.0:5060".parse().unwrap()),
            tls_bind: None,
            tls_cert: None,
            tls_key: None,
            max_message_size: 65535,
            connection_pool_size: 1000,
            connection_timeout: Duration::from_secs(300),
            local_ip: None,
            rtp_port: 10000,
            enable_sdp: true,
        }
    }
}

impl TransportConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_message_size == 0 || self.max_message_size > MAX_SIP_MESSAGE_SIZE {
            return Err(format!(
                "transport.max_message_size must be between 1 and {} bytes",
                MAX_SIP_MESSAGE_SIZE
            ));
        }
        if self.connection_pool_size == 0 || self.connection_pool_size > MAX_CONNECTION_POOL_SIZE {
            return Err(format!(
                "transport.connection_pool_size must be between 1 and {}",
                MAX_CONNECTION_POOL_SIZE
            ));
        }
        if self.connection_timeout.is_zero() {
            return Err("transport.connection_timeout must be greater than zero".to_string());
        }
        if self.rtp_port == 0 {
            return Err("transport.rtp_port must be greater than zero".to_string());
        }
        if let Some(ip) = &self.local_ip {
            reject_crlf("transport.local_ip", ip)?;
        }

        // Validate TLS configuration if TLS is enabled
        if self.tls_bind.is_some() {
            // If TLS is enabled, both cert and key must be provided
            if self.tls_cert.is_none() || self.tls_key.is_none() {
                return Err(
                    "transport.tls_cert and transport.tls_key must be provided when TLS is enabled"
                        .to_string(),
                );
            }

            // Verify certificate file exists
            if let Some(cert_path) = &self.tls_cert {
                if !cert_path.exists() {
                    return Err(format!(
                        "TLS certificate file not found: {}",
                        cert_path.display()
                    ));
                }
                if !cert_path.is_file() {
                    return Err(format!(
                        "TLS certificate path is not a file: {}",
                        cert_path.display()
                    ));
                }
            }

            // Verify key file exists
            if let Some(key_path) = &self.tls_key {
                if !key_path.exists() {
                    return Err(format!(
                        "TLS private key file not found: {}",
                        key_path.display()
                    ));
                }
                if !key_path.is_file() {
                    return Err(format!(
                        "TLS private key path is not a file: {}",
                        key_path.display()
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Registrar configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrarConfig {
    /// Enable REGISTER request handling
    pub enabled: bool,

    /// Enable authentication for REGISTER requests
    pub auth_enabled: bool,

    /// Authentication realm
    pub realm: String,

    /// Data directory for file-based storage
    pub data_dir: PathBuf,

    /// Path to users file (JSON format, optional)
    /// If specified, users will be loaded from this file on startup.
    /// Example format: [{"username": "alice", "password": "secret123", "display_name": "Alice", "enabled": true}]
    pub users_file: Option<PathBuf>,

    /// Default registration expiry (seconds)
    pub default_expires: u32,

    /// Minimum registration expiry (seconds)
    pub min_expires: u32,

    /// Maximum registration expiry (seconds)
    pub max_expires: u32,
}

impl Default for RegistrarConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auth_enabled: false,
            realm: "fcp.local".to_string(),
            data_dir: PathBuf::from("./data"),
            users_file: None,
            default_expires: 3600,
            min_expires: 60,
            max_expires: 86400,
        }
    }
}

impl RegistrarConfig {
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("registrar.realm", &self.realm)?;
        if self.min_expires == 0 {
            return Err("registrar.min_expires must be greater than zero".to_string());
        }
        if self.min_expires > self.max_expires {
            return Err("registrar.min_expires cannot exceed registrar.max_expires".to_string());
        }
        if self.default_expires < self.min_expires || self.default_expires > self.max_expires {
            return Err(
                "registrar.default_expires must fall between min_expires and max_expires"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// Storage configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Storage backend type
    pub backend: StorageBackend,

    /// Database connection string
    pub connection_string: String,

    /// Maximum number of database connections
    pub max_connections: u32,

    /// Connection timeout
    pub connection_timeout: Duration,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::Sqlite,
            connection_string: "sqlite://fcp.db".to_string(),
            max_connections: 10,
            connection_timeout: Duration::from_secs(30),
        }
    }
}

impl StorageConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_connections == 0 {
            return Err("storage.max_connections must be greater than zero".to_string());
        }
        if self.connection_timeout.is_zero() {
            return Err("storage.connection_timeout must be greater than zero".to_string());
        }
        Ok(())
    }
}

/// Storage backend types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    /// SQLite (for development and single-node)
    Sqlite,

    /// PostgreSQL (for production)
    Postgres,

    /// MySQL (for existing MySQL infrastructure)
    Mysql,

    /// MongoDB (for large-scale NoSQL)
    Mongodb,

    /// CockroachDB (for distributed deployments) - FUTURE
    Cockroach,
}

/// API server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Enable REST API
    pub enabled: bool,

    /// REST API bind address
    pub bind: SocketAddr,

    /// Enable Prometheus metrics endpoint
    pub metrics_enabled: bool,

    /// Metrics endpoint path
    pub metrics_path: String,

    /// Enable API authentication
    pub auth_enabled: bool,

    /// API authentication token (if auth enabled)
    pub auth_token: Option<String>,

    /// Deployment mode for permission evaluation.
    /// Controls which permissions are available (e.g., trunk management
    /// is only enabled in Carrier/ServiceProvider modes).
    #[serde(default)]
    pub deployment_mode: crate::permissions::DeploymentMode,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: "127.0.0.1:8080".parse().unwrap(),
            metrics_enabled: true,
            metrics_path: "/metrics".to_string(),
            auth_enabled: false,
            auth_token: None,
            deployment_mode: crate::permissions::DeploymentMode::default(),
        }
    }
}

impl ApiConfig {
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("api.metrics_path", &self.metrics_path)?;
        if !self.metrics_path.starts_with('/') {
            return Err("api.metrics_path must start with '/'".to_string());
        }
        if let Some(token) = &self.auth_token {
            reject_crlf("api.auth_token", token)?;
        }
        Ok(())
    }
}

/// Observability configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Log level (trace, debug, info, warn, error)
    pub log_level: String,

    /// Log format (json, pretty, compact)
    pub log_format: LogFormat,

    /// Enable OpenTelemetry tracing
    pub tracing_enabled: bool,

    /// OpenTelemetry collector endpoint
    pub tracing_endpoint: Option<String>,

    /// Service name for tracing
    pub service_name: String,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            log_format: LogFormat::Pretty,
            tracing_enabled: false,
            tracing_endpoint: None,
            service_name: "fcp".to_string(),
        }
    }
}

impl ObservabilityConfig {
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("observability.log_level", &self.log_level)?;
        reject_crlf("observability.service_name", &self.service_name)?;
        if let Some(endpoint) = &self.tracing_endpoint {
            reject_crlf("observability.tracing_endpoint", endpoint)?;
        }
        Ok(())
    }
}

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable format
    Pretty,

    /// JSON format (for log aggregation)
    Json,

    /// Compact format
    Compact,
}

/// Routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Enable routing policy enforcement
    pub enabled: bool,

    /// Routing backend (YAML file or database)
    pub backend: RoutingBackend,

    /// Default tenant ID for single-tenant deployments
    pub default_tenant_id: String,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: RoutingBackend::Yaml {
                path: PathBuf::from("routing.yaml"),
                watch: true,
            },
            default_tenant_id: "default".to_string(),
        }
    }
}

impl RoutingConfig {
    pub fn validate(&self) -> Result<(), String> {
        reject_crlf("routing.default_tenant_id", &self.default_tenant_id)?;
        match &self.backend {
            RoutingBackend::Yaml { path, .. } => {
                if let Some(path_str) = path.to_str() {
                    reject_crlf("routing.backend.yaml.path", path_str)?;
                }
            }
            RoutingBackend::Database { connection_string } => {
                reject_crlf(
                    "routing.backend.database.connection_string",
                    connection_string,
                )?;
            }
        }
        Ok(())
    }
}

/// Routing backend types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RoutingBackend {
    /// YAML file-based storage (for small deployments)
    Yaml {
        /// Path to routing.yaml file
        path: PathBuf,
        /// Watch for file changes and hot-reload
        watch: bool,
    },
    /// Database-backed storage (for large deployments) - FUTURE
    Database {
        /// Database connection string
        connection_string: String,
    },
}

/// Media configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Enable media anchoring (RTP flows through FCP server)
    /// If false, all calls use pass-through mode (RTP flows direct between endpoints)
    pub enabled: bool,

    /// RTP port range start
    pub rtp_port_start: u16,

    /// RTP port range end
    pub rtp_port_end: u16,

    /// Enable SRTP encryption
    pub enable_srtp: bool,

    /// Enable transcoding between different codecs
    pub enable_transcoding: bool,

    /// Enable call recording
    pub enable_recording: bool,

    /// Enable DTMF detection and relay
    pub enable_dtmf: bool,

    /// Public IP address for RTP (if behind NAT)
    pub public_ip: Option<String>,

    /// STUN server for NAT traversal (optional)
    pub stun_server: Option<String>,

    /// TURN server for NAT traversal (optional)
    pub turn_server: Option<String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default - use pass-through mode
            rtp_port_start: 10000,
            rtp_port_end: 20000,
            enable_srtp: false,
            enable_transcoding: false,
            enable_recording: false,
            enable_dtmf: true,
            public_ip: None,
            stun_server: None,
            turn_server: None,
        }
    }
}

impl MediaConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.rtp_port_start == 0 || self.rtp_port_end == 0 {
            return Err(
                "media.rtp_port_start and media.rtp_port_end must be greater than zero".to_string(),
            );
        }
        if self.rtp_port_start > self.rtp_port_end {
            return Err("media.rtp_port_start cannot exceed media.rtp_port_end".to_string());
        }
        if self.rtp_port_end - self.rtp_port_start > MAX_RTP_PORT_SPAN {
            return Err(format!(
                "media RTP port span is too large (max {} ports)",
                MAX_RTP_PORT_SPAN
            ));
        }
        if let Some(ip) = &self.public_ip {
            reject_crlf("media.public_ip", ip)?;
        }
        if let Some(stun) = &self.stun_server {
            reject_crlf("media.stun_server", stun)?;
        }
        if let Some(turn) = &self.turn_server {
            reject_crlf("media.turn_server", turn)?;
        }
        Ok(())
    }
}
