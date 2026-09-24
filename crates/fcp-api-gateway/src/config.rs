//! Configuration loading and management for fcp-api-gateway.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tracing::info;

/// API Gateway configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceConfig,

    /// Gateway configuration
    #[serde(default)]
    pub gateway: GatewaySettings,

    /// Backend services
    #[serde(default)]
    pub backends: BackendsConfig,

    /// Redis configuration for caching
    #[serde(default)]
    pub redis: RedisConfig,

    /// Rate limiting configuration
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Cache configuration
    #[serde(default)]
    pub cache: CacheConfig,

    /// CORS configuration
    #[serde(default)]
    pub cors: CorsConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,
}

/// Service configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// Service type (always "gateway")
    #[serde(default = "default_service_type")]
    pub service_type: String,

    /// Node identifier
    #[serde(default = "default_node_id")]
    pub node_id: String,
}

fn default_service_type() -> String {
    "gateway".to_string()
}

fn default_node_id() -> String {
    format!("gateway-{}", uuid::Uuid::new_v4())
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
        }
    }
}

/// Gateway settings
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewaySettings {
    /// Bind address (public-facing)
    #[serde(default = "default_bind_address")]
    pub bind_address: SocketAddr,

    /// Request timeout
    #[serde(default = "default_request_timeout_secs", with = "serde_duration")]
    pub request_timeout: Duration,

    /// Enable compression
    #[serde(default = "default_true")]
    pub compression_enabled: bool,
}

fn default_bind_address() -> SocketAddr {
    "0.0.0.0:8000".parse().unwrap()
}

fn default_request_timeout_secs() -> Duration {
    Duration::from_secs(30)
}

fn default_true() -> bool {
    true
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            request_timeout: default_request_timeout_secs(),
            compression_enabled: true,
        }
    }
}

/// Backend services configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BackendsConfig {
    /// Admin API backend
    #[serde(default)]
    pub admin: BackendService,

    /// Events API backend
    #[serde(default)]
    pub events: Option<BackendService>,

    /// Jobs API backend
    #[serde(default)]
    pub jobs: Option<BackendService>,
}

/// Which backend a `BackendService` entry is: each kind serves its
/// health at a different path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// `fcp-admin`, which mounts `fcp-api-core` at `/api/v1`.
    Admin,
    /// `fcp-events-server`.
    Events,
    /// `fcp-jobs`.
    Jobs,
}

impl BackendKind {
    /// The health route the backend of this kind actually serves
    /// (`docs/DEPLOYMENT_GAPS.md` §1.4).
    pub fn default_health_check_path(self) -> &'static str {
        match self {
            BackendKind::Admin => "/api/v1/health",
            BackendKind::Events | BackendKind::Jobs => "/health",
        }
    }
}

/// Backend service configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackendService {
    /// Service URL
    pub url: String,

    /// Health check path. Unset, the kind's own: `/api/v1/health` for the
    /// admin backend, `/health` for the events server and jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_path: Option<String>,

    /// How often the gateway probes this backend in the background; the
    /// last probe is what `GET /gateway/status` reports (seconds).
    #[serde(
        default = "default_health_check_interval_secs",
        with = "serde_duration"
    )]
    pub health_check_interval: Duration,

    /// Connection timeout
    #[serde(default = "default_connection_timeout_secs", with = "serde_duration")]
    pub connection_timeout: Duration,
}

fn default_health_check_interval_secs() -> Duration {
    Duration::from_secs(10)
}

fn default_connection_timeout_secs() -> Duration {
    Duration::from_secs(5)
}

impl BackendService {
    /// The path this backend is probed at: `health_check_path` if set,
    /// else the kind's own.
    pub fn health_check_path(&self, kind: BackendKind) -> String {
        self.health_check_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| kind.default_health_check_path().to_string())
    }
}

impl Default for BackendService {
    fn default() -> Self {
        Self {
            url: "http://localhost:8080".to_string(),
            health_check_path: None,
            health_check_interval: default_health_check_interval_secs(),
            connection_timeout: default_connection_timeout_secs(),
        }
    }
}

/// Redis configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedisConfig {
    /// Redis connection URL
    #[serde(default = "default_redis_url")]
    pub url: String,

    /// Connection pool size
    #[serde(default = "default_redis_pool_size")]
    pub pool_size: usize,

    /// Connection timeout
    #[serde(default = "default_redis_timeout_secs", with = "serde_duration")]
    pub connection_timeout: Duration,
}

fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}

fn default_redis_pool_size() -> usize {
    100
}

fn default_redis_timeout_secs() -> Duration {
    Duration::from_secs(5)
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: default_redis_url(),
            pool_size: default_redis_pool_size(),
            connection_timeout: default_redis_timeout_secs(),
        }
    }
}

/// Rate limiting configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitConfig {
    /// Enable rate limiting
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Default requests per minute per tenant
    #[serde(default = "default_requests_per_minute")]
    pub default_requests_per_minute: u32,

    /// Burst size (max requests in burst)
    #[serde(default = "default_burst_size")]
    pub burst_size: u32,
}

fn default_requests_per_minute() -> u32 {
    1000
}

fn default_burst_size() -> u32 {
    50
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_requests_per_minute: default_requests_per_minute(),
            burst_size: default_burst_size(),
        }
    }
}

/// Cache configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    /// Enable caching (disabled by default for security — enable explicitly)
    #[serde(default)]
    pub enabled: bool,

    /// Default TTL for cached responses (seconds)
    #[serde(default = "default_cache_ttl_secs", with = "serde_duration")]
    pub default_ttl: Duration,

    /// Cache GET requests only
    #[serde(default = "default_true")]
    pub get_only: bool,
}

fn default_cache_ttl_secs() -> Duration {
    Duration::from_secs(60)
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default — must be explicitly enabled
            default_ttl: default_cache_ttl_secs(),
            get_only: true,
        }
    }
}

/// CORS configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorsConfig {
    /// Enable CORS
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Allow all origins
    #[serde(default = "default_true")]
    pub allow_all_origins: bool,

    /// Allowed origins (if not allow_all)
    #[serde(default)]
    pub allowed_origins: Vec<String>,

    /// Allow credentials
    #[serde(default)]
    pub allow_credentials: bool,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_all_origins: true,
            allowed_origins: Vec::new(),
            allow_credentials: false,
        }
    }
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// Load configuration from file or use defaults.
///
/// Configuration priority:
/// 1. File specified via --config or FCP_GATEWAY_CONFIG
/// 2. ./fcp-api-gateway.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<GatewayConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-api-gateway.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            GatewayConfig::default()
        }
    };

    info!(
        bind_address = %config.gateway.bind_address,
        redis_url = %mask_redis_url(&config.redis.url),
        cache_enabled = config.cache.enabled,
        rate_limit_enabled = config.rate_limit.enabled,
        "Configuration loaded successfully"
    );

    Ok(config)
}

/// Load configuration from a specific file.
fn load_config_from_file(path: &Path) -> Result<GatewayConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: GatewayConfig = fcp_core::config::parse_toml(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

/// Mask sensitive information in Redis URL for logging.
fn mask_redis_url(url: &str) -> String {
    if let Some(at_pos) = url.find('@') {
        if let Some(scheme_end) = url.find("://") {
            let scheme = &url[..scheme_end + 3];
            let after_at = &url[at_pos..];
            return format!("{}***{}", scheme, after_at);
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GatewayConfig::default();
        assert_eq!(config.service.service_type, "gateway");
        assert_eq!(config.gateway.bind_address.port(), 8000);
        assert_eq!(config.redis.url, "redis://127.0.0.1:6379");
        assert!(config.rate_limit.enabled);
        assert!(!config.cache.enabled); // Cache disabled by default for security
    }

    #[test]
    fn test_load_default_config() {
        // Should not fail when no config file exists
        let result = load_config(None);
        assert!(result.is_ok());
    }

    #[test]
    fn a_backend_is_probed_at_its_kinds_health_route_unless_told_otherwise() {
        let backend = BackendService::default();
        assert_eq!(
            backend.health_check_path(BackendKind::Admin),
            "/api/v1/health"
        );
        assert_eq!(backend.health_check_path(BackendKind::Events), "/health");
        assert_eq!(backend.health_check_path(BackendKind::Jobs), "/health");

        let explicit = BackendService {
            health_check_path: Some("/healthz".to_string()),
            ..BackendService::default()
        };
        assert_eq!(explicit.health_check_path(BackendKind::Admin), "/healthz");
        let blank = BackendService {
            health_check_path: Some("  ".to_string()),
            ..BackendService::default()
        };
        assert_eq!(blank.health_check_path(BackendKind::Jobs), "/health");

        let parsed: GatewayConfig = toml::from_str(
            r#"
[backends.admin]
url = "http://admin:8080"
[backends.events]
url = "http://events:8081"
health_check_path = "/ping"
"#,
        )
        .unwrap();
        assert_eq!(parsed.backends.admin.health_check_path, None);
        assert_eq!(
            parsed
                .backends
                .events
                .unwrap()
                .health_check_path(BackendKind::Events),
            "/ping"
        );
    }

    #[test]
    fn test_mask_redis_url() {
        let url = "redis://:password@localhost:6379";
        let masked = mask_redis_url(url);
        assert_eq!(masked, "redis://***@localhost:6379");
        assert!(!masked.contains("password"));
    }
}
