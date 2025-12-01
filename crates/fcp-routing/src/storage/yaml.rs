//! YAML file-based storage implementation.
//!
//! This module provides a file-based storage backend using YAML files.
//! It supports hot-reloading of configuration when the file changes.

use super::RoutingStore;
use crate::models::{PolicyRule, User};
use anyhow::{Context, Result};
use async_trait::async_trait;
use fcp_core::TenantId;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// YAML-backed routing store with hot-reload support.
///
/// This implementation stores all routing data in a single YAML file.
/// When `watch` is enabled, it automatically reloads the configuration
/// when the file changes.
pub struct YamlRoutingStore {
    /// Path to the YAML configuration file
    config_path: PathBuf,

    /// Cached routing data (protected by RwLock for concurrent access)
    data: Arc<RwLock<RoutingData>>,

    /// File watcher for hot-reload (kept alive for the duration of the store)
    _watcher: Option<RecommendedWatcher>,
}

/// Internal structure matching the YAML file format.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RoutingData {
    /// List of users
    #[serde(default)]
    users: Vec<User>,

    /// List of policy rules
    #[serde(default)]
    policies: Vec<PolicyRule>,

    // Future: destination_classes, routes, transforms
}

impl YamlRoutingStore {
    /// Create a new YAML routing store.
    ///
    /// # Arguments
    ///
    /// * `config_path` - Path to the routing.yaml file
    /// * `watch` - Enable hot-reloading when file changes
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub async fn new(config_path: PathBuf, watch: bool) -> Result<Self> {
        // Load initial data
        let data = Self::load_file(&config_path).await?;
        let data = Arc::new(RwLock::new(data));

        // Set up file watcher if requested
        let watcher = if watch {
            match Self::setup_watcher(config_path.clone(), data.clone()) {
                Ok(w) => {
                    info!(path = ?config_path, "Hot-reload enabled for routing config");
                    Some(w)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to setup file watcher, hot-reload disabled");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            config_path,
            data,
            _watcher: watcher,
        })
    }

    /// Load and parse the YAML file.
    async fn load_file(path: &Path) -> Result<RoutingData> {
        let contents = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read routing config from {:?}", path))?;

        let data: RoutingData = serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse routing config from {:?}", path))?;

        info!(
            path = ?path,
            users = data.users.len(),
            policies = data.policies.len(),
            "Loaded routing configuration"
        );

        Ok(data)
    }

    /// Set up file watcher for hot-reload.
    fn setup_watcher(
        path: PathBuf,
        data: Arc<RwLock<RoutingData>>,
    ) -> Result<RecommendedWatcher> {
        // Clone for the closure
        let watch_path = path.clone();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            match res {
                Ok(event) => {
                    // Only reload on modify events
                    if matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_)
                    ) {
                        info!(path = ?watch_path, "Routing config file changed, reloading");

                        // Attempt to reload the file
                        match std::fs::read_to_string(&watch_path) {
                            Ok(contents) => match serde_yaml::from_str::<RoutingData>(&contents) {
                                Ok(new_data) => {
                                    // Try to acquire write lock (non-blocking)
                                    if let Ok(mut guard) = data.try_write() {
                                        *guard = new_data;
                                        info!(
                                            path = ?watch_path,
                                            users = guard.users.len(),
                                            policies = guard.policies.len(),
                                            "Routing config reloaded successfully"
                                        );
                                    } else {
                                        warn!("Could not acquire lock to reload config");
                                    }
                                }
                                Err(e) => {
                                    error!(error = %e, "Failed to parse updated routing config, keeping old config");
                                }
                            },
                            Err(e) => {
                                error!(error = %e, "Failed to read updated routing config file");
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "File watch error");
                }
            }
        })?;

        // Watch the file (not recursive)
        watcher.watch(&path, RecursiveMode::NonRecursive)?;

        Ok(watcher)
    }
}

#[async_trait]
impl RoutingStore for YamlRoutingStore {
    async fn get_user(&self, user_id: &str, tenant_id: &TenantId) -> Result<Option<User>> {
        let data = self.data.read().await;
        Ok(data
            .users
            .iter()
            .find(|u| u.id == user_id && &u.tenant_id == tenant_id)
            .cloned())
    }

    async fn list_users(&self, tenant_id: &TenantId) -> Result<Vec<User>> {
        let data = self.data.read().await;
        Ok(data
            .users
            .iter()
            .filter(|u| &u.tenant_id == tenant_id)
            .cloned()
            .collect())
    }

    async fn list_policy_rules(&self, tenant_id: &TenantId) -> Result<Vec<PolicyRule>> {
        let data = self.data.read().await;
        Ok(data
            .policies
            .iter()
            .filter(|p| &p.tenant_id == tenant_id)
            .cloned()
            .collect())
    }

    async fn get_policy_rule(
        &self,
        name: &str,
        tenant_id: &TenantId,
    ) -> Result<Option<PolicyRule>> {
        let data = self.data.read().await;
        Ok(data
            .policies
            .iter()
            .find(|p| p.name == name && &p.tenant_id == tenant_id)
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_yaml_store_load() {
        // Create a temporary YAML file
        let mut file = NamedTempFile::new().unwrap();
        let tenant_id = TenantId::nil();
        writeln!(
            file,
            r#"
users:
  - id: "alice@test.com"
    tenant_id: "{}"
    display_name: "Alice"
    roles: ["employee"]
    groups: []
    endpoints: []
    enabled: true

policies:
  - name: "test-rule"
    tenant_id: "{}"
    priority: 100
    enabled: true
    applies_to:
      roles: ["employee"]
    destination_match:
      pattern: "^[0-9]+$"
    time_windows: []
    source_ip_ranges: []
    action: Allow
    reason: "Test policy"
"#,
            tenant_id.as_uuid(),
            tenant_id.as_uuid()
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let store = YamlRoutingStore::new(path, false).await.unwrap();

        // Test user retrieval
        let user = store
            .get_user("alice@test.com", &tenant_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user.id, "alice@test.com");
        assert!(user.roles.contains("employee"));

        // Test policy retrieval
        let policies = store.list_policy_rules(&tenant_id).await.unwrap();
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].name, "test-rule");
    }

    #[tokio::test]
    async fn test_yaml_store_empty_file() {
        // Create empty YAML file
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "users: []\npolicies: []").unwrap();

        let path = file.path().to_path_buf();
        let store = YamlRoutingStore::new(path, false).await.unwrap();

        let tenant_id = TenantId::nil();
        let users = store.list_users(&tenant_id).await.unwrap();
        assert_eq!(users.len(), 0);

        let policies = store.list_policy_rules(&tenant_id).await.unwrap();
        assert_eq!(policies.len(), 0);
    }
}
