//! Database-backed configuration store with hot-reload support.
//!
//! This module implements a "DB primary, TOML seed" configuration pattern:
//! on first boot, TOML values seed the database. After that, the database
//! is the source of truth. Bootstrap config (DB URL, bind addresses, log level)
//! stays in minimal TOML files.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::Result;

/// A single configuration entry stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    /// Service name (e.g. "call-manager", "admin", "_global").
    pub service: String,
    /// Section name within the service (e.g. "registrar", "media", "sip").
    pub section: String,
    /// Tenant ID. "_system" for global/default config.
    pub tenant_id: String,
    /// The configuration value as JSON.
    pub value: serde_json::Value,
    /// Monotonically increasing version, incremented on each update.
    pub version: i64,
    /// When this entry was last updated.
    pub updated_at: DateTime<Utc>,
    /// Who made the last update (e.g. "admin:alice", "seed", "api").
    pub updated_by: Option<String>,
}

/// An audit log entry recording a configuration change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unique audit log ID.
    pub id: i64,
    /// Service name.
    pub service: String,
    /// Section name.
    pub section: String,
    /// Tenant ID.
    pub tenant_id: String,
    /// Previous value (None for first insert).
    pub old_value: Option<serde_json::Value>,
    /// New value.
    pub new_value: serde_json::Value,
    /// Who made the change.
    pub changed_by: Option<String>,
    /// When the change was made.
    pub changed_at: DateTime<Utc>,
    /// Version after the change.
    pub version: i64,
}

/// Trait for storing and retrieving configuration entries.
///
/// Implementations must be thread-safe and async-friendly.
/// The primary implementation is backed by PostgreSQL.
#[async_trait]
pub trait ConfigStore: Send + Sync {
    /// Get a specific config entry by service, section, and tenant.
    async fn get_config(
        &self,
        service: &str,
        section: &str,
        tenant_id: &str,
    ) -> Result<Option<ConfigEntry>>;

    /// Get all config sections for a given service and tenant.
    async fn get_service_config(&self, service: &str, tenant_id: &str) -> Result<Vec<ConfigEntry>>;

    /// Upsert a config entry. Increments version on update.
    /// Inserts an audit log entry in the same transaction.
    /// Returns the new version number.
    async fn set_config(
        &self,
        service: &str,
        section: &str,
        tenant_id: &str,
        value: serde_json::Value,
        updated_by: Option<&str>,
    ) -> Result<i64>;

    /// Delete a config entry. Returns true if a row was actually deleted.
    async fn delete_config(&self, service: &str, section: &str, tenant_id: &str) -> Result<bool>;

    /// List config entries with optional filters.
    async fn list_configs(
        &self,
        service: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Result<Vec<ConfigEntry>>;

    /// Get the latest version number for a service+tenant combination.
    /// Returns 0 if no entries exist. Used for poll-based change detection.
    async fn get_latest_version(&self, service: &str, tenant_id: &str) -> Result<i64>;

    /// Get audit log entries, optionally filtered by section.
    async fn get_audit_log(
        &self,
        service: &str,
        tenant_id: &str,
        section: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AuditEntry>>;
}

/// A watch-channel wrapper for hot-reloading a typed config section.
///
/// Components hold a `ConfigWatcher<T>` and call `borrow()` to get
/// the current config. The poll loop calls `update()` when a new
/// version is detected.
pub struct ConfigWatcher<T: Clone + Send + Sync + 'static> {
    tx: watch::Sender<Arc<T>>,
    rx: watch::Receiver<Arc<T>>,
    service: String,
    section: String,
    tenant_id: String,
    last_version: std::sync::atomic::AtomicI64,
}

impl<T: Clone + Send + Sync + 'static> ConfigWatcher<T> {
    /// Create a new watcher with an initial value.
    pub fn new(initial: T, service: &str, section: &str, tenant_id: &str) -> Self {
        let (tx, rx) = watch::channel(Arc::new(initial));
        Self {
            tx,
            rx,
            service: service.to_string(),
            section: section.to_string(),
            tenant_id: tenant_id.to_string(),
            last_version: std::sync::atomic::AtomicI64::new(0),
        }
    }

    /// Get the current config value.
    pub fn borrow(&self) -> watch::Ref<'_, Arc<T>> {
        self.rx.borrow()
    }

    /// Get a clone of the current config Arc.
    pub fn get(&self) -> Arc<T> {
        self.rx.borrow().clone()
    }

    /// Subscribe to config changes. Returns a new receiver.
    pub fn subscribe(&self) -> watch::Receiver<Arc<T>> {
        self.tx.subscribe()
    }

    /// Update the config value. Notifies all subscribers.
    pub fn update(&self, value: T, version: i64) {
        self.last_version
            .store(version, std::sync::atomic::Ordering::Relaxed);
        let _ = self.tx.send(Arc::new(value));
    }

    /// Get the last known version.
    pub fn last_version(&self) -> i64 {
        self.last_version.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get the service name this watcher is tracking.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Get the section name this watcher is tracking.
    pub fn section(&self) -> &str {
        &self.section
    }

    /// Get the tenant ID this watcher is tracking.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
}

/// Load a config section from the database, or seed it from the provided TOML value.
///
/// This implements the "DB primary, TOML seed" pattern:
/// 1. Try to load from DB for the given service/section/tenant
/// 2. If found, deserialize and return it
/// 3. If not found, serialize the `toml_value` into the DB as the seed, then return it
///
/// Returns the deserialized config and the version number.
pub async fn load_or_seed_config<T>(
    store: &dyn ConfigStore,
    service: &str,
    section: &str,
    tenant_id: &str,
    toml_value: &T,
) -> Result<(T, i64)>
where
    T: Serialize + DeserializeOwned + Clone,
{
    // Try loading from DB first
    if let Some(entry) = store.get_config(service, section, tenant_id).await? {
        match serde_json::from_value::<T>(entry.value.clone()) {
            Ok(config) => {
                debug!(
                    service,
                    section,
                    tenant_id,
                    version = entry.version,
                    "loaded config from database"
                );
                return Ok((config, entry.version));
            }
            Err(e) => {
                warn!(
                    service,
                    section,
                    tenant_id,
                    error = %e,
                    "failed to deserialize DB config, re-seeding from TOML"
                );
                // Fall through to seed
            }
        }
    }

    // Seed from TOML value
    let json_value = serde_json::to_value(toml_value).map_err(|e| {
        crate::Error::Config(format!("failed to serialize config for seeding: {e}"))
    })?;

    let version = store
        .set_config(service, section, tenant_id, json_value, Some("seed"))
        .await?;

    info!(
        service,
        section, tenant_id, version, "seeded config into database from TOML"
    );

    Ok((toml_value.clone(), version))
}

/// Try to load config with tenant fallback: tenant-specific -> _system -> default.
///
/// Lookup order:
/// 1. Tenant-specific config (service, section, tenant_id)
/// 2. System-wide config (service, section, "_system")
/// 3. `T::default()`
pub async fn get_config_with_tenant_fallback<T>(
    store: &dyn ConfigStore,
    service: &str,
    section: &str,
    tenant_id: &str,
) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    // Try tenant-specific first
    if tenant_id != "_system" {
        if let Some(entry) = store.get_config(service, section, tenant_id).await? {
            if let Ok(config) = serde_json::from_value::<T>(entry.value) {
                debug!(service, section, tenant_id, "loaded tenant-specific config");
                return Ok(config);
            }
        }
    }

    // Fall back to _system
    if let Some(entry) = store.get_config(service, section, "_system").await? {
        if let Ok(config) = serde_json::from_value::<T>(entry.value) {
            debug!(
                service,
                section, "loaded system-wide config (fallback from tenant {tenant_id})"
            );
            return Ok(config);
        }
    }

    // Fall back to default
    debug!(
        service,
        section, tenant_id, "using default config (no DB entry found)"
    );
    Ok(T::default())
}

/// Spawn a background task that polls the config store for changes
/// and updates the provided watchers.
///
/// The poll loop checks version numbers every `poll_interval` and
/// only deserializes + updates when a change is detected.
pub fn spawn_config_poll_loop<T>(
    store: Arc<dyn ConfigStore>,
    watcher: Arc<ConfigWatcher<T>>,
    poll_interval: std::time::Duration,
) -> tokio::task::JoinHandle<()>
where
    T: DeserializeOwned + Clone + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(poll_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;

            let service = watcher.service();
            let section = watcher.section();
            let tenant_id = watcher.tenant_id();

            // Check if version has changed
            match store.get_latest_version(service, tenant_id).await {
                Ok(db_version) => {
                    if db_version <= watcher.last_version() {
                        continue; // No change
                    }
                }
                Err(e) => {
                    warn!(
                        service,
                        section,
                        error = %e,
                        "config poll: failed to check version"
                    );
                    continue;
                }
            }

            // Version changed — reload this section
            match store
                .get_config(watcher.service(), watcher.section(), watcher.tenant_id())
                .await
            {
                Ok(Some(entry)) => match serde_json::from_value::<T>(entry.value) {
                    Ok(config) => {
                        info!(
                            service,
                            section,
                            version = entry.version,
                            "config updated from database"
                        );
                        watcher.update(config, entry.version);
                    }
                    Err(e) => {
                        warn!(
                            service,
                            section,
                            error = %e,
                            "config poll: failed to deserialize updated config"
                        );
                    }
                },
                Ok(None) => {
                    debug!(service, section, "config poll: entry was deleted");
                }
                Err(e) => {
                    warn!(
                        service,
                        section,
                        error = %e,
                        "config poll: failed to load config"
                    );
                }
            }
        }
    })
}
