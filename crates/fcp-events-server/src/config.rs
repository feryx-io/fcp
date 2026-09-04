//! Configuration loading and management for fcp-events-server.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tracing::info;

/// Events server configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EventsConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceConfig,

    /// WebSocket configuration
    #[serde(default)]
    pub websocket: WebSocketConfig,

    /// Event bus configuration
    #[serde(default)]
    pub event_bus: EventBusConfig,

    /// Connection limits
    #[serde(default)]
    pub limits: LimitsConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// Upstream services configuration
    #[serde(default)]
    pub upstream: UpstreamConfig,

    /// Webhook dispatcher configuration
    #[serde(default)]
    pub webhook: WebhookConfig,

    /// Cluster membership configuration
    #[serde(default)]
    pub cluster: ClusterConfig,

    /// Service authentication configuration
    #[serde(default)]
    pub auth: AuthConfig,
}

/// Upstream services configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpstreamConfig {
    /// Call manager URL for registration data
    #[serde(default = "default_call_manager_url")]
    pub call_manager_url: String,
}

fn default_call_manager_url() -> String {
    "http://localhost:8084".to_string()
}

impl Default for UpstreamConfig {
    fn default() -> Self {
        Self {
            call_manager_url: default_call_manager_url(),
        }
    }
}

/// Webhook dispatcher configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebhookConfig {
    /// Whether webhook dispatching is enabled
    #[serde(default)]
    pub enabled: bool,

    /// HTTP request timeout in seconds
    #[serde(default = "default_webhook_request_timeout")]
    pub request_timeout_secs: u64,

    /// Maximum number of delivery retries
    #[serde(default = "default_webhook_max_retries")]
    pub max_retries: u32,

    /// Base retry delay in seconds (doubles with each attempt)
    #[serde(default = "default_webhook_base_retry_delay")]
    pub base_retry_delay_secs: u64,

    /// Maximum retry delay in seconds
    #[serde(default = "default_webhook_max_retry_delay")]
    pub max_retry_delay_secs: u64,

    /// Maximum concurrent webhook deliveries
    #[serde(default = "default_webhook_max_concurrent")]
    pub max_concurrent: usize,

    /// How often to poll for pending retries (seconds)
    #[serde(default = "default_webhook_retry_interval")]
    pub retry_interval_secs: u64,

    /// PostgreSQL connection URL for webhook storage
    #[serde(default = "default_webhook_database_url")]
    pub database_url: String,
}

fn default_webhook_request_timeout() -> u64 {
    10
}
fn default_webhook_max_retries() -> u32 {
    5
}
fn default_webhook_base_retry_delay() -> u64 {
    30
}
fn default_webhook_max_retry_delay() -> u64 {
    3600
}
fn default_webhook_max_concurrent() -> usize {
    50
}
fn default_webhook_retry_interval() -> u64 {
    60
}
fn default_webhook_database_url() -> String {
    "postgresql://localhost/fcp".to_string()
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            request_timeout_secs: default_webhook_request_timeout(),
            max_retries: default_webhook_max_retries(),
            base_retry_delay_secs: default_webhook_base_retry_delay(),
            max_retry_delay_secs: default_webhook_max_retry_delay(),
            max_concurrent: default_webhook_max_concurrent(),
            retry_interval_secs: default_webhook_retry_interval(),
            database_url: default_webhook_database_url(),
        }
    }
}

/// Authentication configuration for the events server.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AuthConfig {
    /// Shared bearer token required for ingest, query, and webhook management APIs.
    /// WebSocket clients may provide the same token via `?token=...`.
    #[serde(default)]
    pub token: Option<String>,
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClusterConfig {
    /// Enable cluster membership heartbeats.
    #[serde(default)]
    pub enabled: bool,

    /// Deployment region this node advertises in cluster membership, e.g.
    /// `us-east`; falls back to the `FCP_REGION` environment variable.
    /// Placement prefers nodes in the caller's region. See
    /// `docs/MULTI_REGION.md`.
    #[serde(default)]
    pub region: Option<String>,

    /// Redis URL for cluster membership (if different from event_bus.redis_url).
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Heartbeat interval in seconds (default: 5).
    #[serde(default = "default_cluster_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,

    /// Heartbeat TTL in seconds (default: 15).
    #[serde(default = "default_cluster_heartbeat_ttl")]
    pub heartbeat_ttl_secs: u64,
}

fn default_cluster_heartbeat_interval() -> u64 {
    5
}
fn default_cluster_heartbeat_ttl() -> u64 {
    15
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            region: None,
            redis_url: None,
            heartbeat_interval_secs: default_cluster_heartbeat_interval(),
            heartbeat_ttl_secs: default_cluster_heartbeat_ttl(),
        }
    }
}

/// Service configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    /// Service type (always "events")
    #[serde(default = "default_service_type")]
    pub service_type: String,

    /// Node identifier
    #[serde(default = "default_node_id")]
    pub node_id: String,
}

fn default_service_type() -> String {
    "events".to_string()
}

fn default_node_id() -> String {
    format!("events-{}", uuid::Uuid::new_v4())
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
        }
    }
}

/// WebSocket configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebSocketConfig {
    /// Bind address
    #[serde(default = "default_bind_address")]
    pub bind_address: SocketAddr,

    /// Heartbeat interval (seconds)
    #[serde(default = "default_heartbeat_interval_secs", with = "serde_duration")]
    pub heartbeat_interval: Duration,

    /// Client timeout (seconds)
    #[serde(default = "default_client_timeout_secs", with = "serde_duration")]
    pub client_timeout: Duration,

    /// Max message size (bytes)
    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,

    /// Event batch size (max events per message)
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Event batch timeout (milliseconds)
    #[serde(default = "default_batch_timeout_ms", with = "serde_duration_millis")]
    pub batch_timeout: Duration,
}

fn default_bind_address() -> SocketAddr {
    "127.0.0.1:8081".parse().unwrap()
}

fn default_heartbeat_interval_secs() -> Duration {
    Duration::from_secs(30)
}

fn default_client_timeout_secs() -> Duration {
    Duration::from_secs(60)
}

fn default_max_message_size() -> usize {
    1024 * 1024 // 1MB
}

fn default_batch_size() -> usize {
    10
}

fn default_batch_timeout_ms() -> Duration {
    Duration::from_millis(100)
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            heartbeat_interval: default_heartbeat_interval_secs(),
            client_timeout: default_client_timeout_secs(),
            max_message_size: default_max_message_size(),
            batch_size: default_batch_size(),
            batch_timeout: default_batch_timeout_ms(),
        }
    }
}

/// Event bus configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventBusConfig {
    /// Event bus type: "in-memory" or "redis"
    #[serde(default = "default_event_bus_type")]
    pub bus_type: String,

    /// Buffer size for event streams
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    /// Redis URL for distributed event bus (required when bus_type = "redis")
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Redis key prefix (default: "fcp:")
    #[serde(default = "default_redis_key_prefix")]
    pub redis_key_prefix: String,

    /// Redis consumer group name for this events-server instance
    #[serde(default = "default_consumer_group")]
    pub consumer_group: String,

    /// Redis consumer name (unique per instance within a consumer group)
    #[serde(default = "default_consumer_name")]
    pub consumer_name: String,

    /// Other regions' Redis instances to forward this region's events to
    /// (cross-region fan-out, `docs/MULTI_REGION.md`). Needs
    /// `bus_type = "redis"`. Every region should list every other region.
    #[serde(default)]
    pub peer_regions: Vec<PeerRegionConfig>,
}

/// One peer region for the event bridge.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeerRegionConfig {
    /// The peer's `[cluster] region` label.
    pub region: String,
    /// The peer region's Redis.
    pub redis_url: String,
    /// Key prefix the peer's nodes use; defaults to this server's
    /// `redis_key_prefix`.
    #[serde(default)]
    pub key_prefix: Option<String>,
}

fn default_event_bus_type() -> String {
    "in-memory".to_string()
}

fn default_buffer_size() -> usize {
    1000
}

fn default_redis_key_prefix() -> String {
    "fcp:".to_string()
}

fn default_consumer_group() -> String {
    "fcp-events-server".to_string()
}

fn default_consumer_name() -> String {
    format!("consumer-{}", uuid::Uuid::new_v4())
}

impl Default for EventBusConfig {
    fn default() -> Self {
        Self {
            bus_type: default_event_bus_type(),
            buffer_size: default_buffer_size(),
            redis_url: None,
            redis_key_prefix: default_redis_key_prefix(),
            consumer_group: default_consumer_group(),
            consumer_name: default_consumer_name(),
            peer_regions: Vec::new(),
        }
    }
}

/// Connection limits configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LimitsConfig {
    /// Maximum concurrent connections
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,

    /// Maximum connections per tenant
    #[serde(default = "default_max_connections_per_tenant")]
    pub max_connections_per_tenant: usize,
}

fn default_max_connections() -> usize {
    10000
}

fn default_max_connections_per_tenant() -> usize {
    100
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_connections: default_max_connections(),
            max_connections_per_tenant: default_max_connections_per_tenant(),
        }
    }
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
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

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

/// Helper module for serializing/deserializing Duration as seconds
mod serde_duration {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

/// Helper module for serializing/deserializing Duration as milliseconds
mod serde_duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_millis() as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

/// Load configuration from file or use defaults.
///
/// Configuration priority:
/// 1. File specified via --config or FCP_EVENTS_CONFIG
/// 2. ./fcp-events-server.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<EventsConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-events-server.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            EventsConfig::default()
        }
    };

    info!(
        bind_address = %config.websocket.bind_address,
        max_connections = config.limits.max_connections,
        event_bus_type = %config.event_bus.bus_type,
        "Configuration loaded successfully"
    );

    Ok(config)
}

/// Load configuration from a specific file.
fn load_config_from_file(path: &Path) -> Result<EventsConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: EventsConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = EventsConfig::default();
        assert_eq!(config.service.service_type, "events");
        assert!(config.websocket.bind_address.ip().is_loopback());
        assert_eq!(config.websocket.bind_address.port(), 8081);
        assert_eq!(config.event_bus.bus_type, "in-memory");
        assert_eq!(config.limits.max_connections, 10000);
        assert!(!config.webhook.enabled);
        assert_eq!(config.webhook.max_retries, 5);
        assert!(config.auth.token.is_none());
    }

    #[test]
    fn peer_regions_parse_with_optional_prefix() {
        let toml_src = r#"
[event_bus]
bus_type = "redis"
redis_url = "redis://us:6379"

[[event_bus.peer_regions]]
region = "eu-west"
redis_url = "redis://eu:6379"

[[event_bus.peer_regions]]
region = "ap-south"
redis_url = "redis://ap:6379"
key_prefix = "fcp-ap:"
"#;
        let config: EventsConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(config.event_bus.peer_regions.len(), 2);
        assert_eq!(config.event_bus.peer_regions[0].region, "eu-west");
        assert_eq!(config.event_bus.peer_regions[0].key_prefix, None);
        assert_eq!(
            config.event_bus.peer_regions[1].key_prefix.as_deref(),
            Some("fcp-ap:")
        );
        assert!(EventsConfig::default().event_bus.peer_regions.is_empty());
    }

    #[test]
    fn test_load_default_config() {
        // Should not fail when no config file exists
        let result = load_config(None);
        assert!(result.is_ok());
    }
}
