//! Configuration loading and management for fcp-jobs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tracing::info;

/// Jobs service configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JobsConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceConfig,

    /// Redis configuration
    #[serde(default)]
    pub redis: RedisConfig,

    /// Worker configuration
    #[serde(default)]
    pub workers: WorkerConfig,

    /// Job retry configuration
    #[serde(default)]
    pub retry: RetryConfig,

    /// Health check configuration
    #[serde(default)]
    pub health: HealthConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            service: ServiceConfig::default(),
            redis: RedisConfig::default(),
            workers: WorkerConfig::default(),
            retry: RetryConfig::default(),
            health: HealthConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

/// Service configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    /// Service type (always "jobs")
    #[serde(default = "default_service_type")]
    pub service_type: String,

    /// Node identifier
    #[serde(default = "default_node_id")]
    pub node_id: String,
}

fn default_service_type() -> String {
    "jobs".to_string()
}

fn default_node_id() -> String {
    format!("jobs-{}", uuid::Uuid::new_v4())
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
        }
    }
}

/// Redis configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RedisConfig {
    /// Redis connection URL
    #[serde(default = "default_redis_url")]
    pub url: String,

    /// Connection pool size
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,

    /// Connection timeout (seconds)
    #[serde(default = "default_connection_timeout_secs", with = "serde_duration")]
    pub connection_timeout: Duration,

    /// Queue key prefix
    #[serde(default = "default_queue_prefix")]
    pub queue_prefix: String,
}

fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}

fn default_pool_size() -> usize {
    20
}

fn default_connection_timeout_secs() -> Duration {
    Duration::from_secs(5)
}

fn default_queue_prefix() -> String {
    "fcp:jobs".to_string()
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: default_redis_url(),
            pool_size: default_pool_size(),
            connection_timeout: default_connection_timeout_secs(),
            queue_prefix: default_queue_prefix(),
        }
    }
}

/// Worker configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkerConfig {
    /// Number of concurrent workers
    #[serde(default = "default_worker_count")]
    pub count: usize,

    /// Poll interval when queue is empty (seconds)
    #[serde(default = "default_poll_interval_secs", with = "serde_duration")]
    pub poll_interval: Duration,

    /// Job processing timeout (seconds)
    #[serde(default = "default_job_timeout_secs", with = "serde_duration")]
    pub job_timeout: Duration,

    /// Graceful shutdown timeout (seconds)
    #[serde(default = "default_shutdown_timeout_secs", with = "serde_duration")]
    pub shutdown_timeout: Duration,
}

fn default_worker_count() -> usize {
    num_cpus::get()
}

fn default_poll_interval_secs() -> Duration {
    Duration::from_secs(5)
}

fn default_job_timeout_secs() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

fn default_shutdown_timeout_secs() -> Duration {
    Duration::from_secs(30)
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            count: default_worker_count(),
            poll_interval: default_poll_interval_secs(),
            job_timeout: default_job_timeout_secs(),
            shutdown_timeout: default_shutdown_timeout_secs(),
        }
    }
}

/// Retry configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RetryConfig {
    /// Maximum retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Initial retry delay (seconds)
    #[serde(default = "default_initial_delay_secs", with = "serde_duration")]
    pub initial_delay: Duration,

    /// Maximum retry delay (seconds)
    #[serde(default = "default_max_delay_secs", with = "serde_duration")]
    pub max_delay: Duration,

    /// Backoff multiplier
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

fn default_max_retries() -> u32 {
    5
}

fn default_initial_delay_secs() -> Duration {
    Duration::from_secs(10)
}

fn default_max_delay_secs() -> Duration {
    Duration::from_secs(3600) // 1 hour
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_delay: default_initial_delay_secs(),
            max_delay: default_max_delay_secs(),
            backoff_multiplier: default_backoff_multiplier(),
        }
    }
}

/// Health check configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthConfig {
    /// Enable health check endpoint
    #[serde(default = "default_health_enabled")]
    pub enabled: bool,

    /// Health check bind address
    #[serde(default = "default_health_bind")]
    pub bind_address: SocketAddr,
}

fn default_health_enabled() -> bool {
    true
}

fn default_health_bind() -> SocketAddr {
    "0.0.0.0:8082".parse().unwrap()
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            enabled: default_health_enabled(),
            bind_address: default_health_bind(),
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

/// Load configuration from file or use defaults.
///
/// Configuration priority:
/// 1. File specified via --config or FCP_JOBS_CONFIG
/// 2. ./fcp-jobs.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<JobsConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-jobs.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            JobsConfig::default()
        }
    };

    info!(
        redis_url = %config.redis.url,
        worker_count = config.workers.count,
        max_retries = config.retry.max_retries,
        "Configuration loaded successfully"
    );

    Ok(config)
}

/// Load configuration from a specific file.
fn load_config_from_file(path: &Path) -> Result<JobsConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: JobsConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = JobsConfig::default();
        assert_eq!(config.service.service_type, "jobs");
        assert_eq!(config.redis.url, "redis://127.0.0.1:6379");
        assert_eq!(config.workers.count, num_cpus::get());
        assert_eq!(config.retry.max_retries, 5);
    }

    #[test]
    fn test_load_default_config() {
        // Should not fail when no config file exists
        let result = load_config(None);
        assert!(result.is_ok());
    }
}
