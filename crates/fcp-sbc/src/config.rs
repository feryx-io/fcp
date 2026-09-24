//! Configuration loading for fcp-sbc.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

/// Deployment-level wrapper around SbcConfig that adds cluster membership fields.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SbcDeployConfig {
    /// How this node names itself and how it leaves service.
    #[serde(default)]
    pub service: ServiceSection,

    /// Cluster membership configuration.
    #[serde(default)]
    pub cluster: ClusterSection,

    /// Redis URL for cluster membership.
    #[serde(default)]
    pub redis_url: Option<String>,

    /// All other SBC configuration.
    /// SBC engine configuration, under the `[core]` table (see
    /// `fcp-sbc-local.toml` and `docs/SINGLE-NODE-SETUP.md`). Every field
    /// falls back to `SbcConfig::default()` so partial files work.
    #[serde(default)]
    pub core: fcp_sbc_core::SbcConfig,
}

/// What this node calls itself, and how long it waits for the calls it is
/// carrying when it leaves service
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSection {
    /// This node's id: in cluster membership, in the drain's events, on
    /// the calls it carries and on the trunks it registers — one id, the
    /// same everywhere (`docs/DEPLOYMENT_GAPS.md` §1.2). `sbc-<uuid>`,
    /// fresh per run, when unset.
    #[serde(default = "default_node_id")]
    pub node_id: String,

    /// Seconds a drain waits for the calls this node is carrying before it
    /// ends what is left with a BYE that says why. Zero — the default — is
    /// what the SBC always did: it stops where it stands.
    #[serde(default)]
    pub drain_wait_secs: u64,
}

fn default_node_id() -> String {
    format!("sbc-{}", uuid::Uuid::new_v4())
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            node_id: default_node_id(),
            drain_wait_secs: 0,
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
