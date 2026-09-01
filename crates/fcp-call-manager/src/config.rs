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

    /// Routing configuration
    #[serde(default)]
    pub routing: RoutingSection,

    /// Events configuration
    #[serde(default)]
    pub events: EventsSection,

    /// B2BUA behavior configuration (extension threshold, topology hiding, etc.)
    #[serde(default)]
    pub b2bua: fcp_b2bua_core::B2buaConfig,

    /// SIP trunk configurations (session-manager mode).
    /// When non-empty and registrar.enabled=false, operates as session manager.
    #[serde(default)]
    pub trunks: Vec<fcp_sbc_core::TrunkConfig>,

    /// PBX features (hunt groups, parking, pickup, intercom, CMC/FAC).
    #[serde(default)]
    pub pbx: fcp_pbx::PbxConfig,

    /// ACD/Queue configuration.
    #[serde(default)]
    pub queue: fcp_queue::QueueSystemConfig,

    /// Presence service configuration.
    #[serde(default)]
    pub presence: fcp_presence::PresenceConfig,

    /// Cluster configuration.
    #[serde(default)]
    pub cluster: ClusterSection,
}

/// Service identification.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,

    #[serde(default = "default_tenant_id")]
    pub default_tenant_id: fcp_core::TenantId,
}

fn default_service_type() -> String {
    "call-manager".to_string()
}

fn default_node_id() -> String {
    format!("cm-{}", uuid::Uuid::new_v4())
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

    /// TLS/SIPS bind address (default: 0.0.0.0:5061)
    /// Only active if tls_cert_path and tls_key_path are set.
    #[serde(default = "default_sips_bind")]
    pub sips_bind: String,

    /// Path to TLS certificate file (PEM format)
    pub tls_cert_path: Option<String>,

    /// Path to TLS private key file (PEM format)
    pub tls_key_path: Option<String>,

    /// Optional SIP-over-WebSocket bind address.
    #[serde(default)]
    pub ws_bind: Option<String>,

    /// Optional secure SIP-over-WebSocket bind address.
    #[serde(default)]
    pub wss_bind: Option<String>,

    /// Local IP for SDP/Contact (auto-detect if not set)
    pub local_ip: Option<String>,

    /// Local SIP URI (default: sip:call-manager@<local_ip>)
    pub local_uri: Option<String>,
}

fn default_sip_bind() -> String {
    "0.0.0.0:5060".to_string()
}

fn default_sips_bind() -> String {
    "0.0.0.0:5061".to_string()
}

impl Default for SipSection {
    fn default() -> Self {
        Self {
            udp_bind: default_sip_bind(),
            tcp_bind: default_sip_bind(),
            sips_bind: default_sips_bind(),
            tls_cert_path: None,
            tls_key_path: None,
            ws_bind: None,
            wss_bind: None,
            local_ip: None,
            local_uri: None,
        }
    }
}

/// Registrar behavior configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegistrarSection {
    /// Enable the SIP registrar. Set to false for session-manager mode.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Enable SIP Digest authentication
    #[serde(default = "default_true")]
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

    /// Domain aliases - list of domains that should be treated as equivalent.
    /// All domains in the same list are considered aliases of each other.
    /// Example: ["fcp.local", "192.168.1.225", "pbx.example.com"]
    /// When looking up registrations for alice@192.168.1.225, we also check
    /// alice@fcp.local and alice@pbx.example.com.
    #[serde(default)]
    pub domain_aliases: Vec<String>,
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
            enabled: true,
            auth_enabled: true,
            realm: default_realm(),
            default_expires: default_expires(),
            min_expires: default_min_expires(),
            max_expires: default_max_expires(),
            domain_aliases: Vec::new(),
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

    /// Redis connection URL for registration store and cluster state.
    /// When set alongside `registration_backend = "redis"`, registrations
    /// are stored in Redis while other data remains in PostgreSQL.
    /// Example: "redis://redis:6379"
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Override the registration storage backend.
    /// Set to "redis" to use Redis for registrations (requires `redis_url`).
    /// When unset or null, registrations use the primary backend.
    #[serde(default)]
    pub registration_backend: Option<String>,
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
            redis_url: None,
            registration_backend: None,
        }
    }
}

/// HTTP health/API endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpSection {
    /// Bind address for health/status/calls endpoint
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,

    /// Optional bearer token for authenticating the HTTP control API.
    /// Also supports `?token=<token>` for the wallboard WebSocket endpoint.
    /// Public binds require this to be set.
    #[serde(default)]
    pub api_token: Option<String>,
}

fn default_http_bind() -> SocketAddr {
    "127.0.0.1:8084".parse().unwrap()
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

    /// Public IP for SDP/RTP (required when behind NAT or in Docker)
    pub public_ip: Option<String>,

    /// TTS engine name (e.g., "kokoro"). Enables text-to-speech when set.
    #[serde(default)]
    pub tts_engine: Option<String>,

    /// Default TTS voice (e.g., "af_heart" for Kokoro)
    #[serde(default)]
    pub tts_voice: Option<String>,

    /// TTS cache directory for generated audio files
    #[serde(default = "default_tts_cache_dir")]
    pub tts_cache_dir: String,

    /// Path to Python interpreter for local TTS (e.g., ".venv/bin/python3")
    #[serde(default)]
    pub tts_python: Option<String>,

    /// URL of shared fcp-tts service.  When set, uses Remote provider
    /// instead of spawning a local TTS subprocess.
    #[serde(default)]
    pub tts_remote_url: Option<String>,

    /// Bearer token for the shared fcp-tts service.
    #[serde(default)]
    pub tts_remote_token: Option<String>,

    /// Directory for pre-generated prompt audio files
    #[serde(default = "default_prompts_dir")]
    pub prompts_dir: String,
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

fn default_tts_cache_dir() -> String {
    "/tmp/fcp/tts_cache".to_string()
}

fn default_prompts_dir() -> String {
    "/tmp/fcp/audio/prompts".to_string()
}

impl Default for MediaSection {
    fn default() -> Self {
        Self {
            rtp_port_start: default_rtp_port_start(),
            rtp_port_end: default_rtp_port_end(),
            codecs: default_codecs(),
            public_ip: None,
            tts_engine: None,
            tts_voice: None,
            tts_cache_dir: default_tts_cache_dir(),
            tts_python: None,
            tts_remote_url: None,
            tts_remote_token: None,
            prompts_dir: default_prompts_dir(),
        }
    }
}

/// Routing configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RoutingSection {
    /// Path to routing rules YAML file (optional)
    pub rules_file: Option<String>,

    /// Enable hot-reload of rules file
    #[serde(default)]
    pub hot_reload: bool,
}

/// Events configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EventsSection {
    /// URL of fcp-events-server ingest endpoint (optional).
    /// Used for HTTP-based event forwarding (fallback when Redis is not configured).
    /// Example: "http://fcp-events-server:8081/ingest"
    pub server_url: Option<String>,

    /// Optional bearer token for authenticating to fcp-events-server.
    #[serde(default)]
    pub auth_token: Option<String>,

    /// Use Redis Pub/Sub + Streams for distributed event bus.
    /// Requires `storage.redis_url` to be set.
    #[serde(default)]
    pub use_redis: bool,

    /// Redis Pub/Sub channel name for events (default: "{prefix}events").
    #[serde(default)]
    pub redis_channel: Option<String>,

    /// Redis Stream key for durable events (default: "{prefix}event_stream").
    #[serde(default)]
    pub redis_stream_key: Option<String>,

    /// External webhook endpoints to receive event notifications.
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClusterSection {
    /// Enable cluster mode. Requires `storage.redis_url`.
    #[serde(default)]
    pub enabled: bool,

    /// Shared secret for inter-node HTTP authentication.
    #[serde(default)]
    pub cluster_secret: Option<String>,

    /// Heartbeat interval in seconds (default: 5).
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,

    /// Heartbeat TTL in seconds — if a node misses this many seconds of
    /// heartbeats, it is considered dead (default: 15).
    #[serde(default = "default_heartbeat_ttl")]
    pub heartbeat_ttl_secs: u64,

    /// Call registry TTL in seconds — maximum time a call ownership key
    /// persists in Redis before expiring (default: 7200 = 2 hours).
    #[serde(default = "default_call_ttl")]
    pub call_ttl_secs: u64,

    /// Circuit breaker: number of consecutive Redis failures before opening.
    #[serde(default = "default_cb_threshold")]
    pub redis_circuit_breaker_threshold: u32,

    /// Circuit breaker: cooldown in seconds before allowing a probe request.
    #[serde(default = "default_cb_cooldown")]
    pub redis_circuit_breaker_cooldown_secs: u64,
}

fn default_heartbeat_interval() -> u64 {
    5
}

fn default_heartbeat_ttl() -> u64 {
    15
}

fn default_call_ttl() -> u64 {
    7200
}

fn default_cb_threshold() -> u32 {
    5
}

fn default_cb_cooldown() -> u64 {
    30
}

impl Default for ClusterSection {
    fn default() -> Self {
        Self {
            enabled: false,
            cluster_secret: None,
            heartbeat_interval_secs: default_heartbeat_interval(),
            heartbeat_ttl_secs: default_heartbeat_ttl(),
            call_ttl_secs: default_call_ttl(),
            redis_circuit_breaker_threshold: default_cb_threshold(),
            redis_circuit_breaker_cooldown_secs: default_cb_cooldown(),
        }
    }
}

/// Webhook endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebhookConfig {
    /// HTTP(S) URL to POST events to.
    pub url: String,

    /// Event type filter — only forward events of these types.
    /// Valid values: "call", "media", "queue", "recording", "system", etc.
    /// If empty/unset, all events are forwarded.
    #[serde(default)]
    pub event_types: Vec<String>,

    /// Optional secret for HMAC-SHA256 signatures in X-FCP-Signature.
    /// X-Webhook-Signature is also emitted for backward compatibility.
    pub secret: Option<String>,
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

    validate_config(&config)?;

    info!(
        node_id = %config.service.node_id,
        default_tenant_id = %config.service.default_tenant_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        registrar_enabled = config.registrar.enabled,
        auth_enabled = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        http_bind = %config.http.bind_address,
        max_calls = config.calls.max_concurrent_calls,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        trunks = config.trunks.len(),
        "Configuration loaded"
    );

    Ok(config)
}

pub fn validate_config(config: &CallManagerConfig) -> Result<()> {
    validate_http_config(&config.http)?;
    validate_registrar_config(&config.sip, &config.registrar)?;
    fcp_pbx::config::validate_config(&config.pbx).map_err(anyhow::Error::msg)?;
    Ok(())
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
            "http.api_token must be set when binding the call-manager HTTP API to a non-loopback address ({})",
            http.bind_address
        );
    }

    Ok(())
}

fn validate_registrar_config(sip: &SipSection, registrar: &RegistrarSection) -> Result<()> {
    if !registrar.enabled {
        return Ok(());
    }

    let public_udp = bind_is_public(&sip.udp_bind)
        .with_context(|| format!("Invalid SIP UDP bind address '{}'", sip.udp_bind))?;
    let public_tcp = bind_is_public(&sip.tcp_bind)
        .with_context(|| format!("Invalid SIP TCP bind address '{}'", sip.tcp_bind))?;
    let public_ws = sip
        .ws_bind
        .as_deref()
        .map(bind_is_public)
        .transpose()
        .with_context(|| format!("Invalid SIP WS bind address '{:?}'", sip.ws_bind))?
        .unwrap_or(false);
    let public_wss = sip
        .wss_bind
        .as_deref()
        .map(bind_is_public)
        .transpose()
        .with_context(|| format!("Invalid SIP WSS bind address '{:?}'", sip.wss_bind))?
        .unwrap_or(false);

    if (public_udp || public_tcp || public_ws || public_wss) && !registrar.auth_enabled {
        anyhow::bail!(
            "registrar.auth_enabled must be true when the call-manager registrar binds to a non-loopback SIP address (udp={}, tcp={}, ws={:?}, wss={:?})",
            sip.udp_bind,
            sip.tcp_bind,
            sip.ws_bind,
            sip.wss_bind
        );
    }

    if sip.wss_bind.is_some() && (sip.tls_cert_path.is_none() || sip.tls_key_path.is_none()) {
        anyhow::bail!("sip.wss_bind requires sip.tls_cert_path and sip.tls_key_path");
    }

    Ok(())
}

fn bind_is_public(bind: &str) -> Result<bool> {
    let addr: std::net::SocketAddr = bind.parse()?;
    Ok(!addr.ip().is_loopback())
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
        assert_eq!(config.service.default_tenant_id, fcp_core::TenantId::nil());
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.sip.tcp_bind, "0.0.0.0:5060");
        assert!(config.registrar.enabled);
        assert!(config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "fcp.local");
        assert_eq!(config.registrar.default_expires, 3600);
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.http.bind_address, "127.0.0.1:8084".parse().unwrap());
        assert!(config.http.api_token.is_none());
        assert_eq!(config.calls.max_concurrent_calls, 1000);
        assert_eq!(config.calls.call_timeout_seconds, 60);
        assert!(config.calls.enable_sdp);
        assert!(config.calls.enable_refer);
        assert_eq!(config.media.rtp_port_start, 10000);
        assert_eq!(config.media.rtp_port_end, 20000);
        // New sections
        assert!(config.routing.rules_file.is_none());
        assert!(!config.routing.hot_reload);
        assert!(config.events.server_url.is_none());
        assert!(config.events.webhooks.is_empty());
        assert!(config.trunks.is_empty());
        // PBX defaults (enabled by default)
        assert!(config.pbx.enabled);
        assert!(config.pbx.hunt_groups.is_empty());
        // Queue defaults
        assert!(!config.queue.enabled);
        assert!(config.queue.queues.is_empty());
    }

    #[test]
    fn test_public_http_bind_requires_token() {
        let http = HttpSection {
            bind_address: "0.0.0.0:8084".parse().unwrap(),
            api_token: None,
        };
        assert!(validate_http_config(&http).is_err());

        let http = HttpSection {
            bind_address: "0.0.0.0:8084".parse().unwrap(),
            api_token: Some("secret".to_string()),
        };
        assert!(validate_http_config(&http).is_ok());
    }

    #[test]
    fn test_public_registrar_bind_requires_auth() {
        let sip = SipSection {
            udp_bind: "0.0.0.0:5060".to_string(),
            tcp_bind: "0.0.0.0:5060".to_string(),
            local_ip: None,
            sips_bind: default_sips_bind(),
            tls_cert_path: None,
            tls_key_path: None,
            ws_bind: None,
            wss_bind: None,
            local_uri: None,
        };
        let registrar = RegistrarSection {
            enabled: true,
            auth_enabled: false,
            realm: default_realm(),
            default_expires: default_expires(),
            min_expires: default_min_expires(),
            max_expires: default_max_expires(),
            domain_aliases: Vec::new(),
        };
        assert!(validate_registrar_config(&sip, &registrar).is_err());

        let registrar = RegistrarSection {
            auth_enabled: true,
            ..registrar
        };
        assert!(validate_registrar_config(&sip, &registrar).is_ok());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "call-manager"
node_id = "cm-test-01"
default_tenant_id = "00000000-0000-0000-0000-000000000123"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
local_ip = "192.168.1.100"

[registrar]
auth_enabled = true
realm = "test.local"
default_expires = 1800

[storage]
database_url = "postgresql://localhost/fcp"

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

[routing]
rules_file = "/app/routing.yaml"
hot_reload = true

[events]
server_url = "http://fcp-events-server:8081/ingest"

[[events.webhooks]]
url = "https://hooks.example.com/queue-events"
event_types = ["queue"]
secret = "whsec_test123"

[[events.webhooks]]
url = "https://hooks.example.com/all-events"
"#;
        let config: CallManagerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "cm-test-01");
        assert_eq!(
            config.service.default_tenant_id,
            "00000000-0000-0000-0000-000000000123".parse().unwrap()
        );
        assert!(config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "test.local");
        assert_eq!(config.registrar.default_expires, 1800);
        assert_eq!(config.http.bind_address.port(), 9090);
        assert_eq!(config.calls.max_concurrent_calls, 500);
        assert!(!config.calls.enable_refer);
        assert_eq!(config.media.rtp_port_start, 20000);
        assert_eq!(config.media.codecs.len(), 3);
        assert_eq!(config.sip.local_ip.as_deref(), Some("192.168.1.100"));
        // New sections
        assert_eq!(
            config.routing.rules_file.as_deref(),
            Some("/app/routing.yaml")
        );
        assert!(config.routing.hot_reload);
        assert_eq!(
            config.events.server_url.as_deref(),
            Some("http://fcp-events-server:8081/ingest")
        );
        // Webhook config
        assert_eq!(config.events.webhooks.len(), 2);
        assert_eq!(
            config.events.webhooks[0].url,
            "https://hooks.example.com/queue-events"
        );
        assert_eq!(config.events.webhooks[0].event_types, vec!["queue"]);
        assert_eq!(
            config.events.webhooks[0].secret.as_deref(),
            Some("whsec_test123")
        );
        assert_eq!(
            config.events.webhooks[1].url,
            "https://hooks.example.com/all-events"
        );
        assert!(config.events.webhooks[1].event_types.is_empty());
        assert!(config.events.webhooks[1].secret.is_none());
    }
}
