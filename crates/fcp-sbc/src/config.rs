//! Configuration loading for fcp-sbc.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

/// Deployment-level wrapper around SbcConfig that adds cluster membership fields.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SbcDeployConfig {
    /// Cluster membership configuration.
    #[serde(default)]
    pub cluster: ClusterSection,

    /// Redis URL for cluster membership.
    #[serde(default)]
    pub redis_url: Option<String>,

    /// All other SBC configuration.
    #[serde(flatten)]
    pub core: fcp_sbc_core::SbcConfig,
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClusterSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,
    #[serde(default = "default_heartbeat_ttl")]
    pub heartbeat_ttl_secs: u64,
}

fn default_heartbeat_interval() -> u64 { 5 }
fn default_heartbeat_ttl() -> u64 { 15 }

impl Default for ClusterSection {
    fn default() -> Self {
        Self {
            enabled: false,
            heartbeat_interval_secs: default_heartbeat_interval(),
            heartbeat_ttl_secs: default_heartbeat_ttl(),
        }
    }
}

/// Load SBC configuration from a TOML file or use defaults.
///
/// If no path is provided, tries `fcp-sbc.toml` in the current directory,
/// then falls back to `SbcDeployConfig::default()`.
pub fn load_config(config_path: Option<&Path>) -> Result<SbcDeployConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-sbc.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            SbcDeployConfig::default()
        }
    };

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<SbcDeployConfig> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: SbcDeployConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}
