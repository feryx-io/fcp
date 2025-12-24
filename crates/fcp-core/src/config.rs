use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Complete runtime configuration for FCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct FcpConfig {
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

/// Storage backend types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    /// SQLite (for development and single-node)
    Sqlite,

    /// PostgreSQL (for production)
    Postgres,

    /// CockroachDB (for distributed deployments)
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
        }
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
