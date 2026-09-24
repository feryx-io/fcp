//! Configuration for fcp-presence-server.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Top-level presence server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceServerConfig {
    #[serde(default)]
    pub service: ServiceSection,

    #[serde(default)]
    pub sip: SipSection,

    #[serde(default)]
    pub storage: StorageSection,

    #[serde(default)]
    pub presence: PresenceSection,

    /// What this node accepts, and what it refuses
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). Every limit is optional;
    /// unset is unlimited, as it was before the section existed. A
    /// draining node grants subscriptions no longer than
    /// `drain_register_expiry_secs`, so its watchers come back somewhere
    /// that is serving.
    #[serde(default)]
    pub admission: fcp_sbc_core::AdmissionSection,

    #[serde(default)]
    pub http: HttpSection,

    #[serde(default)]
    pub logging: LoggingSection,

    #[serde(default)]
    pub redis: Option<RedisSection>,

    #[serde(default)]
    pub xcap: Option<XcapSection>,

    #[serde(default)]
    pub webhooks: Option<WebhookSection>,

    #[serde(default)]
    pub cluster: ClusterSection,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,

    #[serde(default = "default_tenant_id")]
    pub default_tenant_id: fcp_core::TenantId,

    #[serde(default = "default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,

    /// Seconds a drain waits for this node's watchers to subscribe
    /// somewhere else before it ends the subscriptions it still holds
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). A draining node grants
    /// short subscriptions, so they come back within the minute. Zero —
    /// the default — is what this binary always did: it stops where it
    /// stands, telling every watcher as it goes.
    #[serde(default)]
    pub drain_wait_secs: u64,
}

fn default_service_type() -> String {
    "presence".to_string()
}

fn default_node_id() -> String {
    format!("presence-{}", uuid::Uuid::new_v4())
}

fn default_tenant_id() -> fcp_core::TenantId {
    fcp_core::TenantId::nil()
}

fn default_shutdown_timeout_secs() -> u64 {
    10
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
            default_tenant_id: default_tenant_id(),
            shutdown_timeout_secs: default_shutdown_timeout_secs(),
            drain_wait_secs: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SipSection {
    /// UDP bind address.
    #[serde(default = "default_sip_bind")]
    pub udp_bind: String,

    /// TCP bind address.
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

    /// Advertised local IP for Contact/Via when the bind address is unspecified.
    pub local_ip: Option<String>,

    /// Explicit local SIP URI override.
    pub local_uri: Option<String>,

    /// Enable SIP Digest authentication for SUBSCRIBE/PUBLISH.
    #[serde(default = "default_true")]
    pub auth_enabled: bool,

    /// Authentication realm for SIP Digest.
    #[serde(default = "default_realm")]
    pub realm: String,
}

fn default_sip_bind() -> String {
    "127.0.0.1:5060".to_string()
}

fn default_true() -> bool {
    true
}

fn default_realm() -> String {
    "fcp.local".to_string()
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
            auth_enabled: default_true(),
            realm: default_realm(),
        }
    }
}

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

/// Presence-specific settings (mapped to PresenceConfig).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceSection {
    #[serde(default = "default_max_subscriptions")]
    pub max_subscriptions: usize,

    #[serde(default = "default_max_presentities")]
    pub max_presentities: usize,

    #[serde(default = "default_default_subscription_expiry")]
    pub default_subscription_expiry_secs: u32,

    #[serde(default = "default_cleanup_interval")]
    pub cleanup_interval_secs: u64,
}

fn default_max_subscriptions() -> usize {
    10000
}

fn default_max_presentities() -> usize {
    10000
}

fn default_default_subscription_expiry() -> u32 {
    3600
}

fn default_cleanup_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,

    /// Bearer token required for non-health HTTP API endpoints.
    pub api_token: Option<String>,
}

fn default_http_bind() -> SocketAddr {
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 8080)
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
            api_token: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct RedisSection {
    pub url: String,
    pub node_id: Option<String>,
}

/// XCAP server configuration (RFC 4825).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct XcapSection {
    #[serde(default = "default_xcap_enabled")]
    pub enabled: bool,
    #[serde(default = "default_xcap_root")]
    pub root_path: String,
}

fn default_xcap_enabled() -> bool {
    true
}

fn default_xcap_root() -> String {
    "/xcap-root".to_string()
}

impl Default for XcapSection {
    fn default() -> Self {
        Self {
            enabled: default_xcap_enabled(),
            root_path: default_xcap_root(),
        }
    }
}

/// Webhook delivery configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookSection {
    /// Whether webhook delivery is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Maximum number of delivery retries. Default: 3.
    #[serde(default = "default_webhook_max_retries")]
    pub max_retries: u32,

    /// Base delay in ms for exponential backoff. Default: 1000.
    #[serde(default = "default_webhook_base_delay_ms")]
    pub base_delay_ms: u64,

    /// Maximum delay in ms for exponential backoff. Default: 30000.
    #[serde(default = "default_webhook_max_delay_ms")]
    pub max_delay_ms: u64,
}

fn default_webhook_max_retries() -> u32 {
    3
}

fn default_webhook_base_delay_ms() -> u64 {
    1000
}

fn default_webhook_max_delay_ms() -> u64 {
    30000
}

impl Default for WebhookSection {
    fn default() -> Self {
        Self {
            enabled: false,
            max_retries: default_webhook_max_retries(),
            base_delay_ms: default_webhook_base_delay_ms(),
            max_delay_ms: default_webhook_max_delay_ms(),
        }
    }
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterSection {
    #[serde(default)]
    pub enabled: bool,

    /// Deployment region this node advertises in cluster membership, e.g.
    /// `us-east`; falls back to the `FCP_REGION` environment variable.
    /// Placement prefers nodes in the caller's region. See
    /// `docs/MULTI_REGION.md`.
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,
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
pub fn load_config(config_path: Option<&Path>) -> Result<PresenceServerConfig> {
    let config = if let Some(path) = config_path {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?
    } else {
        let default_path = Path::new("fcp-presence-server.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            let contents = std::fs::read_to_string(default_path)
                .context("Failed to read default config file")?;
            toml::from_str(&contents).context("Failed to parse default config file")?
        } else {
            info!("No configuration file found, using defaults");
            PresenceServerConfig::default()
        }
    };

    info!(
        node_id = %config.service.node_id,
        default_tenant_id = %config.service.default_tenant_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        database_url = %config.storage.database_url,
        http_bind = %config.http.bind_address,
        "Configuration loaded"
    );

    Ok(config)
}

/// Validate configuration for logical consistency.
pub fn validate_config(config: &PresenceServerConfig) -> Result<()> {
    validate_http_config(&config.http)?;
    validate_sip_config(&config.sip)?;
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
            "http.api_token must be set when binding the presence HTTP API to a non-loopback address ({})",
            http.bind_address
        );
    }

    Ok(())
}

fn validate_sip_config(sip: &SipSection) -> Result<()> {
    // Shared bind address parsing and WSS TLS prerequisite check.
    fcp_sip::validate_sip_binds(
        &sip.udp_bind,
        &sip.tcp_bind,
        sip.ws_bind.as_deref(),
        sip.wss_bind.as_deref(),
        sip.tls_cert_path.as_deref(),
        sip.tls_key_path.as_deref(),
    )?;

    // Presence-specific: public binds require SIP digest auth.
    let udp_bind: SocketAddr = sip.udp_bind.parse().expect("already validated above");
    let tcp_bind: SocketAddr = sip.tcp_bind.parse().expect("already validated above");
    let ws_bind = sip
        .ws_bind
        .as_deref()
        .map(|b| b.parse::<SocketAddr>().expect("already validated above"));
    let wss_bind = sip
        .wss_bind
        .as_deref()
        .map(|b| b.parse::<SocketAddr>().expect("already validated above"));

    let public_bind = !udp_bind.ip().is_loopback()
        || !tcp_bind.ip().is_loopback()
        || ws_bind.as_ref().is_some_and(|a| !a.ip().is_loopback())
        || wss_bind.as_ref().is_some_and(|a| !a.ip().is_loopback());

    if public_bind && !sip.auth_enabled {
        anyhow::bail!(
            "sip.auth_enabled must be true when binding the presence SIP server to a non-loopback address (udp={}, tcp={}, ws={:?}, wss={:?})",
            udp_bind,
            tcp_bind,
            sip.ws_bind,
            sip.wss_bind
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PresenceServerConfig::default();
        assert_eq!(config.service.default_tenant_id, fcp_core::TenantId::nil());
        assert_eq!(config.sip.udp_bind, "127.0.0.1:5060");
        assert_eq!(config.sip.tcp_bind, "127.0.0.1:5060");
        assert!(config.sip.auth_enabled);
        assert_eq!(config.sip.realm, "fcp.local");
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.http.bind_address, "127.0.0.1:8080".parse().unwrap());
        assert!(config.http.api_token.is_none());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
node_id = "pres-01"
default_tenant_id = "00000000-0000-0000-0000-000000000456"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
local_ip = "192.168.1.100"
auth_enabled = true
realm = "presence.local"

[storage]
database_url = "postgresql://localhost/fcp"

[http]
bind_address = "0.0.0.0:9090"
api_token = "secret"

[presence]
max_subscriptions = 5000
max_presentities = 5000

[logging]
level = "debug"
format = "compact"
"#;
        let config: PresenceServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "pres-01");
        assert_eq!(
            config.service.default_tenant_id,
            "00000000-0000-0000-0000-000000000456".parse().unwrap()
        );
        assert_eq!(config.sip.local_ip.as_deref(), Some("192.168.1.100"));
        assert!(config.sip.auth_enabled);
        assert_eq!(config.sip.realm, "presence.local");
        assert_eq!(config.presence.max_subscriptions, 5000);
        assert_eq!(config.http.bind_address.port(), 9090);
        assert_eq!(config.http.api_token.as_deref(), Some("secret"));
    }

    #[test]
    fn test_public_http_bind_requires_token() {
        let http = HttpSection {
            bind_address: "0.0.0.0:8080".parse().unwrap(),
            api_token: None,
        };
        assert!(validate_http_config(&http).is_err());

        let http = HttpSection {
            bind_address: "0.0.0.0:8080".parse().unwrap(),
            api_token: Some("secret".to_string()),
        };
        assert!(validate_http_config(&http).is_ok());
    }

    #[test]
    fn test_public_sip_bind_requires_auth() {
        let sip = SipSection {
            udp_bind: "0.0.0.0:5060".to_string(),
            tcp_bind: "0.0.0.0:5060".to_string(),
            auth_enabled: false,
            ..Default::default()
        };
        assert!(validate_sip_config(&sip).is_err());

        let sip = SipSection {
            udp_bind: "0.0.0.0:5060".to_string(),
            tcp_bind: "0.0.0.0:5060".to_string(),
            auth_enabled: true,
            ..Default::default()
        };
        assert!(validate_sip_config(&sip).is_ok());
    }
}
