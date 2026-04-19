//! Configuration loading and management for fcp-admin.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Admin API configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AdminConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceConfig,

    /// API configuration
    #[serde(default)]
    pub api: ApiConfig,

    /// Storage configuration
    #[serde(default)]
    pub storage: StorageConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// Events configuration
    #[serde(default)]
    pub events: EventsConfig,

    /// Cluster monitoring configuration
    #[serde(default)]
    pub cluster: ClusterConfig,

    /// Directory for voicemail greetings (used by greeting upload/download endpoints)
    #[serde(default)]
    pub voicemail_greetings_dir: Option<String>,

    /// Directory for voicemail recordings (used by message audio playback)
    #[serde(default)]
    pub voicemail_recordings_dir: Option<String>,

    /// Directory for user profile pictures
    #[serde(default)]
    pub profile_pictures_dir: Option<String>,

    /// Directory for audio prompt files (IVR, voicemail, system prompts)
    #[serde(default)]
    pub prompts_dir: Option<String>,

    /// Security policy configuration (account lockout, password expiration)
    #[serde(default)]
    pub security: SecurityConfig,

    /// TTS configuration for prompt audio synthesis
    #[serde(default)]
    pub tts: Option<TtsSection>,
}

/// TTS configuration for synthesizing prompt audio.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsSection {
    /// TTS engine name (e.g., "kokoro")
    pub engine: String,

    /// Path to Python interpreter (e.g., ".venv/bin/python3")
    #[serde(default)]
    pub python_path: Option<String>,

    /// TTS cache directory
    #[serde(default = "default_tts_cache_dir")]
    pub cache_dir: String,

    /// Default voice
    #[serde(default)]
    pub voice: Option<String>,
}

fn default_tts_cache_dir() -> String {
    "/tmp/fcp/tts_cache".to_string()
}

/// Security policy configuration for login protection.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    /// Maximum failed login attempts before lockout (default: 5)
    #[serde(default = "default_max_failed_attempts")]
    pub max_failed_attempts: u32,

    /// Lockout duration in seconds (default: 1800 = 30 minutes)
    #[serde(default = "default_lockout_duration_secs")]
    pub lockout_duration_secs: u64,

    /// Password expiration in days (None/0 = disabled)
    #[serde(default)]
    pub password_expiration_days: Option<u32>,
}

fn default_max_failed_attempts() -> u32 {
    5
}

fn default_lockout_duration_secs() -> u64 {
    1800
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            max_failed_attempts: default_max_failed_attempts(),
            lockout_duration_secs: default_lockout_duration_secs(),
            password_expiration_days: None,
        }
    }
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            service: ServiceConfig::default(),
            api: ApiConfig::default(),
            storage: StorageConfig::default(),
            logging: LoggingConfig::default(),
            events: EventsConfig::default(),
            cluster: ClusterConfig::default(),
            voicemail_greetings_dir: None,
            voicemail_recordings_dir: None,
            profile_pictures_dir: None,
            prompts_dir: None,
            security: SecurityConfig::default(),
            tts: None,
        }
    }
}

/// Service configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    /// Service type (always "admin")
    #[serde(default = "default_service_type")]
    pub service_type: String,

    /// Node identifier
    #[serde(default = "default_node_id")]
    pub node_id: String,

    /// Default tenant scope for tenant-scoped admin operations.
    #[serde(default = "default_tenant_id")]
    pub default_tenant_id: fcp_core::TenantId,
}

fn default_service_type() -> String {
    "admin".to_string()
}

fn default_node_id() -> String {
    format!("admin-{}", uuid::Uuid::new_v4())
}

fn default_tenant_id() -> fcp_core::TenantId {
    fcp_core::TenantId::nil()
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
            default_tenant_id: default_tenant_id(),
        }
    }
}

/// API configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiConfig {
    /// Bind address (internal only, behind gateway)
    #[serde(default = "default_bind_address")]
    pub bind_address: SocketAddr,

    /// Enable authentication
    #[serde(default = "default_true")]
    pub auth_enabled: bool,

    /// Bearer authentication token
    #[serde(default)]
    pub auth_token: Option<String>,

    /// Shared token accepted only by the device auto-provisioning endpoint.
    ///
    /// Keep this distinct from `auth_token`; provisioning tokens may be placed
    /// in device bootstrap URLs and must not grant admin API access.
    #[serde(default)]
    pub provisioning_auth_token: Option<String>,

    /// Require HTTPS for the provisioning endpoint (checked via
    /// `X-Forwarded-Proto`).  Defaults to `true` because provisioning
    /// responses may contain SIP digest credentials.
    #[serde(default = "default_true")]
    pub provisioning_require_https: bool,

    /// Allow unauthenticated tenant self-service signup.
    #[serde(default)]
    pub allow_public_signup: bool,

    /// Deployment mode for permission evaluation
    #[serde(default)]
    pub deployment_mode: fcp_core::permissions::DeploymentMode,
}

fn default_bind_address() -> SocketAddr {
    "127.0.0.1:8080".parse().unwrap()
}

fn default_true() -> bool {
    true
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            auth_enabled: true,
            auth_token: Some(uuid::Uuid::new_v4().to_string()),
            provisioning_auth_token: None,
            provisioning_require_https: true,
            allow_public_signup: false,
            deployment_mode: fcp_core::permissions::DeploymentMode::default(),
        }
    }
}

/// Storage configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    /// PostgreSQL connection URL
    #[serde(default = "default_database_url")]
    pub database_url: String,

    /// Maximum database connections
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Minimum database connections
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,
}

fn default_database_url() -> String {
    "postgresql://localhost/fcp".to_string()
}

fn default_max_connections() -> u32 {
    50
}

fn default_min_connections() -> u32 {
    5
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            database_url: default_database_url(),
            max_connections: default_max_connections(),
            min_connections: default_min_connections(),
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

/// Events configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventsConfig {
    /// URL of fcp-events-server ingest endpoint (optional)
    /// Example: "http://fcp-events-server:8081/ingest"
    #[serde(default)]
    pub server_url: Option<String>,

    /// Optional bearer token for authenticating to fcp-events-server.
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            server_url: None,
            auth_token: None,
        }
    }
}

/// Cluster monitoring configuration.
///
/// When `redis_url` is set, the admin API connects to Redis to read
/// cluster heartbeat keys written by other FCP services. When absent,
/// cluster monitoring is disabled and the UI shows setup instructions.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClusterConfig {
    /// Redis connection URL (e.g. "redis://localhost:6379").
    /// When None, cluster monitoring is disabled.
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Key prefix for Redis keys (default: "fcp:").
    #[serde(default = "default_key_prefix")]
    pub key_prefix: String,
}

fn default_key_prefix() -> String {
    "fcp:".to_string()
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            redis_url: None,
            key_prefix: default_key_prefix(),
        }
    }
}

/// Load configuration from file or use defaults.
///
/// Configuration priority:
/// 1. File specified via --config or FCP_ADMIN_CONFIG
/// 2. ./fcp-admin.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<AdminConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-admin.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            AdminConfig::default()
        }
    };

    info!(
        bind_address = %config.api.bind_address,
        auth_enabled = config.api.auth_enabled,
        "Configuration loaded successfully; validation deferred until after overrides"
    );

    Ok(config)
}

pub fn validate_config(config: &AdminConfig) -> Result<()> {
    let public_bind = !config.api.bind_address.ip().is_loopback();
    let has_token = config
        .api
        .auth_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());

    if config.api.auth_enabled && !has_token {
        anyhow::bail!("api.auth_token must be set when api.auth_enabled = true");
    }

    let has_provisioning_token = config
        .api
        .provisioning_auth_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());

    if let Some(token) = config.api.provisioning_auth_token.as_deref() {
        if token.contains('\r') || token.contains('\n') {
            anyhow::bail!("api.provisioning_auth_token must not contain CR/LF characters");
        }
    }

    if has_token && has_provisioning_token {
        let auth_token = config.api.auth_token.as_deref().unwrap_or_default().trim();
        let provisioning_token = config
            .api
            .provisioning_auth_token
            .as_deref()
            .unwrap_or_default()
            .trim();
        if auth_token == provisioning_token {
            anyhow::bail!("api.provisioning_auth_token must be distinct from api.auth_token");
        }
    }

    if config.api.allow_public_signup && !has_token {
        anyhow::bail!("api.auth_token must be set when api.allow_public_signup = true");
    }

    if public_bind && !config.api.auth_enabled {
        anyhow::bail!(
            "api.auth_enabled must be true when binding the admin API to a non-loopback address ({})",
            config.api.bind_address
        );
    }

    if public_bind && !has_token {
        anyhow::bail!(
            "api.auth_token must be set when binding the admin API to a non-loopback address ({})",
            config.api.bind_address
        );
    }

    Ok(())
}

/// Load configuration from a specific file.
fn load_config_from_file(path: &Path) -> Result<AdminConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: AdminConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AdminConfig::default();
        assert_eq!(config.service.service_type, "admin");
        assert_eq!(config.service.default_tenant_id, fcp_core::TenantId::nil());
        assert_eq!(config.api.bind_address.port(), 8080);
        assert!(config.api.bind_address.ip().is_loopback());
        assert!(config.api.auth_enabled);
        assert!(config
            .api
            .auth_token
            .as_deref()
            .is_some_and(|token| !token.is_empty()));
        assert!(config.api.provisioning_auth_token.is_none());
        assert!(!config.api.allow_public_signup);
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn test_load_default_config() {
        // Should not fail when no config file exists
        let result = load_config(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_public_bind_requires_auth() {
        let mut config = AdminConfig::default();
        config.api.bind_address = "0.0.0.0:8080".parse().unwrap();
        config.api.auth_enabled = false;
        assert!(validate_config(&config).is_err());

        config.api.auth_enabled = true;
        config.api.auth_token = None;
        assert!(validate_config(&config).is_err());

        config.api.auth_token = Some("secret".to_string());
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_public_signup_requires_token() {
        let mut config = AdminConfig::default();
        config.api.allow_public_signup = true;
        config.api.auth_token = None;
        assert!(validate_config(&config).is_err());

        config.api.auth_token = Some("secret".to_string());
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_provisioning_token_must_be_distinct() {
        let mut config = AdminConfig::default();
        config.api.auth_token = Some("secret".to_string());
        config.api.provisioning_auth_token = Some("secret".to_string());
        assert!(validate_config(&config).is_err());

        config.api.provisioning_auth_token = Some("provisioning-secret".to_string());
        assert!(validate_config(&config).is_ok());
    }
}
