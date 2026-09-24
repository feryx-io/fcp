//! Configuration loading and management for fcp-ivr.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// IVR server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IvrConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceSection,

    /// SIP transport configuration
    #[serde(default)]
    pub sip: SipSection,

    /// What this node accepts, and what it refuses
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). Every limit is optional;
    /// unset is unlimited, as it was before the section existed.
    #[serde(default)]
    pub admission: fcp_sbc_core::AdmissionSection,

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

    /// Cluster configuration
    #[serde(default)]
    pub cluster: ClusterSection,

    /// B2BUA behavior configuration (extension threshold, topology hiding, etc.)
    #[serde(default)]
    pub b2bua: fcp_b2bua_core::B2buaConfig,

    /// SIP trunk configurations (for Dial node outbound calls).
    #[serde(default)]
    pub trunks: Vec<fcp_sbc_core::TrunkConfig>,
}

/// Service identification.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,

    #[serde(default = "default_tenant_id")]
    pub default_tenant_id: fcp_core::TenantId,

    /// Seconds a drain waits for the callers in a flow to finish before
    /// the node ends what is left
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). Zero — the default — is
    /// what this binary always did: it stops where it stands.
    #[serde(default)]
    pub drain_wait_secs: u64,
}

fn default_service_type() -> String {
    "ivr".to_string()
}

fn default_node_id() -> String {
    format!("ivr-{}", uuid::Uuid::new_v4())
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
            drain_wait_secs: 0,
        }
    }
}

/// SIP transport configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SipSection {
    /// UDP bind address
    #[serde(default = "default_sip_bind")]
    pub udp_bind: String,

    /// TCP bind address
    #[serde(default = "default_sip_bind")]
    pub tcp_bind: String,

    /// Optional SIP-over-WebSocket bind address.
    #[serde(default)]
    pub ws_bind: Option<String>,

    /// Optional secure SIP-over-WebSocket bind address.
    #[serde(default)]
    pub wss_bind: Option<String>,

    /// TLS certificate path for secure SIP listeners (WSS).
    #[serde(default)]
    pub tls_cert_path: Option<String>,

    /// TLS private key path for secure SIP listeners (WSS).
    #[serde(default)]
    pub tls_key_path: Option<String>,

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
            ws_bind: None,
            wss_bind: None,
            tls_cert_path: None,
            tls_key_path: None,
            local_ip: None,
            local_uri: None,
        }
    }
}

/// Storage backend configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// HTTP health/API endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    /// Bind address for health/status/flows endpoint
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,

    /// Optional bearer token for the HTTP control API.
    /// Public binds require this to be set.
    #[serde(default)]
    pub api_token: Option<String>,
}

fn default_http_bind() -> SocketAddr {
    "127.0.0.1:8086".parse().unwrap()
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
            api_token: None,
        }
    }
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EventsSection {
    /// URL of fcp-events-server ingest endpoint (optional)
    /// Example: "http://fcp-events-server:8081/ingest"
    pub server_url: Option<String>,

    /// Optional bearer token for authenticating to fcp-events-server.
    #[serde(default)]
    pub auth_token: Option<String>,
}

/// IVR flow configuration (IVR-specific).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

    /// URL of shared fcp-tts service (used when tts_provider = "remote")
    #[serde(default)]
    pub tts_remote_url: Option<String>,

    /// Bearer token for shared fcp-tts service
    #[serde(default)]
    pub tts_remote_token: Option<String>,

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
            tts_remote_url: None,
            tts_remote_token: None,
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
#[serde(deny_unknown_fields)]
pub struct DidRoute {
    /// Phone number / DID (e.g. "+15551234567")
    pub did: String,

    /// Flow ID to execute (e.g. "main-menu")
    pub flow_id: String,
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
}

fn default_heartbeat_interval() -> u64 {
    5
}
fn default_heartbeat_ttl() -> u64 {
    15
}

impl Default for ClusterSection {
    fn default() -> Self {
        Self {
            enabled: false,
            region: None,
            heartbeat_interval_secs: default_heartbeat_interval(),
            heartbeat_ttl_secs: default_heartbeat_ttl(),
        }
    }
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

    validate_http_config(&config.http)?;
    validate_sip_config(&config.sip)?;

    info!(
        node_id = %config.service.node_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        database_url = %config.storage.database_url,
        http_bind = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        did_routes = config.flows.routes.len(),
        default_flow = config.flows.default_flow_id.as_deref().unwrap_or("none"),
        trunks = config.trunks.len(),
        "Configuration loaded"
    );

    Ok(config)
}

fn validate_http_config(http: &HttpSection) -> Result<()> {
    let public_bind = !http.bind_address.ip().is_loopback();
    let has_token = http
        .api_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());

    if public_bind && !has_token {
        anyhow::bail!(
            "http.api_token must be set when binding the IVR HTTP API to a non-loopback address ({})",
            http.bind_address
        );
    }

    Ok(())
}

fn validate_sip_config(sip: &SipSection) -> Result<()> {
    fcp_sip::validate_sip_binds(
        &sip.udp_bind,
        &sip.tcp_bind,
        sip.ws_bind.as_deref(),
        sip.wss_bind.as_deref(),
        sip.tls_cert_path.as_deref(),
        sip.tls_key_path.as_deref(),
    )
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
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.http.bind_address, "127.0.0.1:8086".parse().unwrap());
        assert!(config.http.api_token.is_none());
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
database_url = "postgresql://localhost/fcp"

[http]
bind_address = "0.0.0.0:8086"
api_token = "secret"

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

    #[test]
    fn test_public_http_bind_requires_token() {
        let http = HttpSection {
            bind_address: "0.0.0.0:8086".parse().unwrap(),
            api_token: None,
        };
        assert!(validate_http_config(&http).is_err());

        let http = HttpSection {
            bind_address: "0.0.0.0:8086".parse().unwrap(),
            api_token: Some("secret".to_string()),
        };
        assert!(validate_http_config(&http).is_ok());
    }
}
