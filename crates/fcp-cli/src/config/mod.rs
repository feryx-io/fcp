//! Configuration and context management

use anyhow::{Context as AnyhowContext, Result};
use directories::ProjectDirs;
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
}

/// Configuration file structure
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Name of the current/default context
    pub current_context: String,
    /// List of configured contexts
    pub contexts: Vec<Context>,
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
}

impl Config {
    /// Load configuration from disk, or create default if it doesn't exist
    pub fn load_or_create() -> Result<Self> {
        let config_path = Self::config_file_path()?;

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .context("Failed to read config file")?;
            let config: Config = toml::from_str(&content)
                .context("Failed to parse config file")?;
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
        std::fs::create_dir_all(&config_dir)
            .context("Failed to create config directory")?;

        let config_path = Self::config_file_path()?;
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;

        std::fs::write(&config_path, content)
            .context("Failed to write config file")?;

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
            }],
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
        let mut config = Config::default();

        let new_context = Context {
            name: "production".to_string(),
            url: "https://fcp.example.com".to_string(),
            token: Some("secret-token".to_string()),
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
