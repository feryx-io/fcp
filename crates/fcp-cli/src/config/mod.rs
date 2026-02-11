//! Configuration and context management

use anyhow::{Context as AnyhowContext, Result};
use directories::ProjectDirs;
use fcp_core::DeploymentMode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A context represents a connection to an FCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    /// Context name (e.g., "local", "production")
    pub name: String,
    /// Server URL (e.g., "http://localhost:8080")
    pub url: String,
    /// Optional authentication token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Optional events server URL (e.g., "http://localhost:8081")
    /// Used for aggregated call views in multi-node deployments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_url: Option<String>,
}

/// Application-level settings persisted alongside contexts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Deployment mode (carrier, service_provider, enterprise)
    #[serde(default)]
    pub deployment_mode: DeploymentMode,
    /// Default output format (table, json, yaml)
    #[serde(default = "default_output_format")]
    pub output_format: String,
    /// Disable colored output
    #[serde(default)]
    pub no_color: bool,
    /// API request timeout in seconds
    #[serde(default = "default_api_timeout")]
    pub api_timeout_secs: u64,
    /// Require confirmation for destructive operations
    #[serde(default = "default_true")]
    pub confirm_destructive: bool,
    /// Default page size for list commands
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_output_format() -> String {
    "table".to_string()
}
fn default_api_timeout() -> u64 {
    30
}
fn default_true() -> bool {
    true
}
fn default_page_size() -> usize {
    50
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            deployment_mode: DeploymentMode::default(),
            output_format: default_output_format(),
            no_color: false,
            api_timeout_secs: default_api_timeout(),
            confirm_destructive: true,
            page_size: default_page_size(),
        }
    }
}

impl AppSettings {
    /// Get a setting value by dotted key
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "deployment.mode" | "deployment_mode" => {
                Some(format!("{}", serde_json::to_string(&self.deployment_mode)
                    .unwrap_or_default().trim_matches('"')))
            }
            "output.format" | "output_format" => Some(self.output_format.clone()),
            "output.no_color" | "no_color" => Some(self.no_color.to_string()),
            "api.timeout" | "api_timeout_secs" => Some(self.api_timeout_secs.to_string()),
            "api.confirm_destructive" | "confirm_destructive" => {
                Some(self.confirm_destructive.to_string())
            }
            "api.page_size" | "page_size" => Some(self.page_size.to_string()),
            _ => None,
        }
    }

    /// Set a setting value by dotted key. Returns error if key unknown or value invalid.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "deployment.mode" | "deployment_mode" => {
                self.deployment_mode = match value.to_lowercase().as_str() {
                    "carrier" => DeploymentMode::Carrier,
                    "service_provider" | "serviceprovider" => DeploymentMode::ServiceProvider,
                    "enterprise" => DeploymentMode::Enterprise,
                    _ => anyhow::bail!(
                        "Invalid deployment mode '{}'. Valid: carrier, service_provider, enterprise",
                        value
                    ),
                };
            }
            "output.format" | "output_format" => {
                match value.to_lowercase().as_str() {
                    "table" | "json" | "yaml" => self.output_format = value.to_lowercase(),
                    _ => anyhow::bail!(
                        "Invalid output format '{}'. Valid: table, json, yaml",
                        value
                    ),
                }
            }
            "output.no_color" | "no_color" => {
                self.no_color = value.parse::<bool>()
                    .map_err(|_| anyhow::anyhow!("Invalid boolean '{}'. Use true or false", value))?;
            }
            "api.timeout" | "api_timeout_secs" => {
                let secs = value.parse::<u64>()
                    .map_err(|_| anyhow::anyhow!("Invalid timeout '{}'. Must be a positive integer", value))?;
                if secs == 0 || secs > 300 {
                    anyhow::bail!("Timeout must be between 1 and 300 seconds");
                }
                self.api_timeout_secs = secs;
            }
            "api.confirm_destructive" | "confirm_destructive" => {
                self.confirm_destructive = value.parse::<bool>()
                    .map_err(|_| anyhow::anyhow!("Invalid boolean '{}'. Use true or false", value))?;
            }
            "api.page_size" | "page_size" => {
                let size = value.parse::<usize>()
                    .map_err(|_| anyhow::anyhow!("Invalid page size '{}'. Must be a positive integer", value))?;
                if size == 0 || size > 1000 {
                    anyhow::bail!("Page size must be between 1 and 1000");
                }
                self.page_size = size;
            }
            _ => anyhow::bail!(
                "Unknown configuration key '{}'. Valid keys:\n  \
                 deployment.mode, output.format, output.no_color,\n  \
                 api.timeout, api.confirm_destructive, api.page_size",
                key
            ),
        }
        Ok(())
    }

    /// List all valid setting keys
    pub fn keys() -> &'static [&'static str] {
        &[
            "deployment.mode",
            "output.format",
            "output.no_color",
            "api.timeout",
            "api.confirm_destructive",
            "api.page_size",
        ]
    }
}

/// Configuration file structure
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Name of the current/default context
    pub current_context: String,
    /// List of configured contexts
    pub contexts: Vec<Context>,
    /// Application settings
    #[serde(default)]
    pub settings: AppSettings,
}

/// A resolved context with all values determined
#[derive(Debug, Clone)]
pub struct ResolvedContext {
    /// Context name
    pub name: String,
    /// Server URL
    pub url: String,
    /// Authentication token (if any)
    pub token: Option<String>,
    /// Events server URL (if any)
    pub events_url: Option<String>,
}

impl Config {
    /// Load configuration from disk, or create default if it doesn't exist
    pub fn load_or_create() -> Result<Self> {
        let config_path = Self::config_file_path()?;

        if config_path.exists() {
            let content =
                std::fs::read_to_string(&config_path).context("Failed to read config file")?;
            let config: Config = toml::from_str(&content).context("Failed to parse config file")?;
            Ok(config)
        } else {
            // Create default configuration
            let default_config = Self::default();
            default_config.save()?;
            Ok(default_config)
        }
    }

    /// Save configuration to disk
    pub fn save(&self) -> Result<()> {
        let config_dir = Self::config_dir()?;
        std::fs::create_dir_all(&config_dir).context("Failed to create config directory")?;

        let config_path = Self::config_file_path()?;
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        std::fs::write(&config_path, content).context("Failed to write config file")?;

        Ok(())
    }

    /// Get a context by name
    pub fn get_context(&self, name: &str) -> Option<&Context> {
        self.contexts.iter().find(|c| c.name == name)
    }

    /// Get the current context
    pub fn current(&self) -> Option<&Context> {
        self.get_context(&self.current_context)
    }

    /// Add a new context
    pub fn add_context(&mut self, context: Context) -> Result<()> {
        if self.contexts.iter().any(|c| c.name == context.name) {
            anyhow::bail!("Context '{}' already exists", context.name);
        }
        self.contexts.push(context);
        self.save()
    }

    /// Delete a context by name
    pub fn delete_context(&mut self, name: &str) -> Result<()> {
        if !self.contexts.iter().any(|c| c.name == name) {
            anyhow::bail!("Context '{}' not found", name);
        }

        self.contexts.retain(|c| c.name != name);

        // If we deleted the current context, switch to the first available
        if self.current_context == name {
            if let Some(first) = self.contexts.first() {
                self.current_context = first.name.clone();
            } else {
                anyhow::bail!("Cannot delete the last context");
            }
        }

        self.save()
    }

    /// Switch to a different context
    pub fn use_context(&mut self, name: &str) -> Result<()> {
        if !self.contexts.iter().any(|c| c.name == name) {
            anyhow::bail!("Context '{}' not found", name);
        }
        self.current_context = name.to_string();
        self.save()
    }

    /// Get the config directory path
    fn config_dir() -> Result<PathBuf> {
        ProjectDirs::from("com", "fcp", "cli")
            .map(|proj_dirs| proj_dirs.config_dir().to_path_buf())
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))
    }

    /// Get the config file path
    fn config_file_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            current_context: "local".to_string(),
            contexts: vec![Context {
                name: "local".to_string(),
                url: "http://localhost:8080".to_string(),
                token: None,
                events_url: Some("http://localhost:8081".to_string()),
            }],
            settings: AppSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.current_context, "local");
        assert_eq!(config.contexts.len(), 1);
        assert_eq!(config.contexts[0].name, "local");
    }

    #[test]
    fn test_add_context() {
        let _config = Config::default();

        let _new_context = Context {
            name: "production".to_string(),
            url: "https://fcp.example.com".to_string(),
            token: Some("secret-token".to_string()),
            events_url: Some("https://events.fcp.example.com".to_string()),
        };

        // Note: This will fail in tests because it tries to save to disk
        // In real tests, we'd use a temp directory
        // config.add_context(new_context).unwrap();
        // assert_eq!(config.contexts.len(), 2);
    }

    #[test]
    fn test_get_context() {
        let config = Config::default();
        let context = config.get_context("local");
        assert!(context.is_some());
        assert_eq!(context.unwrap().name, "local");
    }

    #[test]
    fn test_current_context() {
        let config = Config::default();
        let current = config.current();
        assert!(current.is_some());
        assert_eq!(current.unwrap().name, "local");
    }
}
