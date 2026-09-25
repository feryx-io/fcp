//! fcp-link's configuration: where the cloud is, what to enroll with,
//! where to keep what enrollment gave, and what to look at here.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LinkFileConfig {
    #[serde(default)]
    pub service: ServiceConfig,
    #[serde(default)]
    pub link: LinkConfig,
    #[serde(default)]
    pub site: SiteConfig,
    #[serde(default)]
    pub health: HealthConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    #[serde(default = "default_node_id")]
    pub node_id: String,
}

fn default_node_id() -> String {
    "link-01".to_string()
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            node_id: default_node_id(),
        }
    }
}

/// `[link]`: the cloud, and the token and state of this site's enrollment.
///
/// ```toml
/// [link]
/// cloud_url = "https://cloud.example.com:8446"   # the enroll listener
/// link_url = "https://cloud.example.com:8447"    # the heartbeat listener (mutual TLS)
/// enrollment_token_file = "/run/secrets/fcp-enrollment-token"
/// state_dir = "/var/lib/fcp/link"                # the key, the certificate, the secret
/// cloud_ca = "/var/lib/fcp/certs/cloud-ca.crt"   # what signs the cloud's listeners, if not a public CA
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LinkConfig {
    #[serde(default)]
    pub cloud_url: String,
    #[serde(default)]
    pub link_url: String,
    /// The one-time token, in the file or as `${NAME}` from the
    /// environment; needed only until the state directory holds a
    /// certificate.
    #[serde(default)]
    pub enrollment_token: Option<String>,
    #[serde(default)]
    pub enrollment_token_file: Option<String>,
    #[serde(default = "default_state_dir")]
    pub state_dir: String,
    /// A CA to trust for the cloud's listeners, beside the system's.
    #[serde(default)]
    pub cloud_ca: Option<String>,
    /// Seconds between heartbeats until the cloud says otherwise.
    #[serde(default = "default_heartbeat_interval_secs")]
    pub heartbeat_interval_secs: u64,
    /// Seconds a request to the cloud may take.
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
}

fn default_state_dir() -> String {
    "/var/lib/fcp/link".to_string()
}
fn default_heartbeat_interval_secs() -> u64 {
    30
}
fn default_request_timeout_secs() -> u64 {
    10
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            cloud_url: String::new(),
            link_url: String::new(),
            enrollment_token: None,
            enrollment_token_file: None,
            state_dir: default_state_dir(),
            cloud_ca: None,
            heartbeat_interval_secs: default_heartbeat_interval_secs(),
            request_timeout_secs: default_request_timeout_secs(),
        }
    }
}

/// `[site]`: what the link looks at here. With the site's Redis, the
/// heartbeat carries every node of the site's cluster and its release.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SiteConfig {
    #[serde(default)]
    pub redis_url: Option<String>,
    #[serde(default = "default_heartbeat_ttl_secs")]
    pub heartbeat_ttl_secs: u64,
}

fn default_heartbeat_ttl_secs() -> u64 {
    15
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthConfig {
    #[serde(default = "default_health_bind")]
    pub bind_address: SocketAddr,
}

fn default_health_bind() -> SocketAddr {
    "127.0.0.1:8096".parse().unwrap()
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            bind_address: default_health_bind(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
}

fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_format() -> String {
    "compact".to_string()
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

pub fn load_config(path: Option<&Path>) -> Result<LinkFileConfig> {
    let Some(path) = path else {
        info!("No configuration file given; using defaults");
        return Ok(LinkFileConfig::default());
    };
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let config: LinkFileConfig = fcp_core::config::parse_toml(&contents)
        .with_context(|| format!("parsing {}", path.display()))?;
    info!(path = %path.display(), "Configuration loaded");
    Ok(config)
}

/// What must hold before the link runs.
pub fn validate_config(config: &LinkFileConfig) -> Result<()> {
    for (name, value) in [
        ("link.cloud_url", &config.link.cloud_url),
        ("link.link_url", &config.link.link_url),
    ] {
        let value = value.trim();
        if value.is_empty() {
            anyhow::bail!("{name} must be set (or FCP_LINK_CLOUD_URL / FCP_LINK_LINK_URL)");
        }
        if !value.starts_with("https://") {
            anyhow::bail!("{name} must be an https:// URL; the link never speaks plain HTTP");
        }
    }
    if config.link.heartbeat_interval_secs == 0 {
        anyhow::bail!("link.heartbeat_interval_secs must be greater than zero");
    }
    Ok(())
}
