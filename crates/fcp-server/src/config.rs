//! Configuration loading and management.

use anyhow::{Context, Result};
use fcp_core::FcpConfig;
use std::fs;
use std::path::Path;
use tracing::{info, warn};

/// Load configuration from file or use defaults.
///
/// Configuration priority:
/// 1. File specified via --config or FCP_CONFIG
/// 2. ./fcp.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<FcpConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            FcpConfig::default()
        }
    };

    match config.validate() {
        Ok(_) => {
            info!(
                event = "config_validated",
                "Configuration validated successfully"
            );
            Ok(config)
        }
        Err(e) => {
            warn!(
                event = "config_validation_failed",
                error = %e,
                "Configuration validation failed"
            );
            Err(e).context("invalid configuration")
        }
    }
}

/// Load configuration from a specific file.
fn load_config_from_file(path: &Path) -> Result<FcpConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: FcpConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_config() {
        // Should not fail when no config file exists
        let result = load_config(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_nonexistent_file() {
        let path = Path::new("/nonexistent/config.toml");
        let result = load_config_from_file(path);
        assert!(result.is_err());
    }
}
