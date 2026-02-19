//! Configuration for fcp-presence-server.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Top-level presence server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PresenceServerConfig {
    #[serde(default)]
    pub service: ServiceSection,

    #[serde(default)]
    pub storage: StorageSection,

    #[serde(default)]
    pub presence: PresenceSection,

    #[serde(default)]
    pub http: HttpSection,

    #[serde(default)]
    pub logging: LoggingSection,

    #[serde(default)]
    pub redis: Option<RedisSection>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceSection {
    #[serde(default = "default_node_id")]
    pub node_id: String,

    #[serde(default = "default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,
}

fn default_node_id() -> String {
    format!("presence-{}", uuid::Uuid::new_v4())
}

fn default_shutdown_timeout_secs() -> u64 {
    10
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            node_id: default_node_id(),
            shutdown_timeout_secs: default_shutdown_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageSection {
    #[serde(default = "default_backend")]
    pub backend: String,

    #[serde(default = "default_connection_string")]
    pub connection_string: String,
}

fn default_backend() -> String {
    "sqlite".to_string()
}

fn default_connection_string() -> String {
    "sqlite://data/fcp-presence.db".to_string()
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            connection_string: default_connection_string(),
        }
    }
}

/// Presence-specific settings (mapped to PresenceConfig).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
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
pub struct HttpSection {
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,
}

fn default_http_bind() -> SocketAddr {
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 8080)
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
pub struct RedisSection {
    pub url: String,
    pub node_id: Option<String>,
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
        storage_backend = %config.storage.backend,
        http_bind = %config.http.bind_address,
        "Configuration loaded"
    );

    Ok(config)
}

/// Validate configuration for logical consistency.
pub fn validate_config(config: &PresenceServerConfig) -> Result<()> {
    match config.storage.backend.as_str() {
        "sqlite" | "postgres" | "mysql" | "mongodb" => {}
        other => anyhow::bail!(
            "storage.backend '{}' is not supported (sqlite, postgres, mysql, mongodb)",
            other
        ),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PresenceServerConfig::default();
        assert_eq!(config.storage.backend, "sqlite");
        assert_eq!(config.http.bind_address.port(), 8080);
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
node_id = "pres-01"

[storage]
backend = "sqlite"
connection_string = "sqlite:///data/presence.db"

[http]
bind_address = "0.0.0.0:9090"

[presence]
max_subscriptions = 5000
max_presentities = 5000

[logging]
level = "debug"
format = "compact"
"#;
        let config: PresenceServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "pres-01");
        assert_eq!(config.presence.max_subscriptions, 5000);
        assert_eq!(config.http.bind_address.port(), 9090);
    }
}
