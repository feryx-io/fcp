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
}

fn default_service_type() -> String {
    "admin".to_string()
}

fn default_node_id() -> String {
    format!("admin-{}", uuid::Uuid::new_v4())
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
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
}

fn default_bind_address() -> SocketAddr {
    "0.0.0.0:8080".parse().unwrap()
}

fn default_true() -> bool {
    true
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            auth_enabled: true,
            auth_token: None,
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
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self { server_url: None }
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
        "Configuration loaded successfully"
    );

    Ok(config)
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
        assert_eq!(config.api.bind_address.port(), 8080);
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn test_load_default_config() {
        // Should not fail when no config file exists
        let result = load_config(None);
        assert!(result.is_ok());
    }
}
