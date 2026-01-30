//! Configuration loading for fcp-sbc.

use anyhow::{Context, Result};
use fcp_sbc_core::SbcConfig;
use std::path::Path;
use tracing::info;

/// Load SBC configuration from a TOML file or use defaults.
///
/// If no path is provided, tries `fcp-sbc.toml` in the current directory,
/// then falls back to `SbcConfig::default()`.
pub fn load_config(config_path: Option<&Path>) -> Result<SbcConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-sbc.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            SbcConfig::default()
        }
    };

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<SbcConfig> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: SbcConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}
