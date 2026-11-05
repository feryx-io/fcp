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

    /// The TOML value a section was last seeded from, if one is recorded.
    async fn get_seed(
        &self,
        service: &str,
        section: &str,
        tenant_id: &str,
    ) -> Result<Option<serde_json::Value>>;

    /// Record the TOML value a section was seeded from. The row's value is
    /// not touched, and no version is spent.
    async fn set_seed(
        &self,
        service: &str,
        section: &str,
        tenant_id: &str,
        seed: serde_json::Value,
    ) -> Result<()>;

    /// Get audit log entries, newest first: one service's, or every
    /// service's when `service` is `None`, optionally one section's.
    async fn get_audit_log(
        &self,
        service: Option<&str>,
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

/// Who wrote a seed: the TOML's values, as opposed to an edit through the API.
pub const SEEDED_BY: &str = "seed";

/// A section's stored entry, brought up to date with the TOML it was seeded
/// from (`docs/DEEP_TESTING.md` D8).
///
/// The database is the source of truth after the first boot, but that must
/// not mean a TOML edited since is never read: a key the release added, or a
/// value the operator changed in the file, would be silently ignored. So each
/// key is settled three ways against the recorded seed: a key whose TOML
/// value differs from the seed takes the TOML's value; any other key keeps
/// the stored one, so an edit made through the API stands until the file
/// changes that key. With no seed recorded, only keys the row lacks or holds
/// as null are taken from the TOML. The TOML is then recorded as the seed.
///
/// Returns `None` when the section has no row; the caller seeds it, and
/// the next boot, finding the row equal to the file, records the seed.
pub async fn reconcile_with_seed(
    store: &dyn ConfigStore,
    service: &str,
    section: &str,
    tenant_id: &str,
    toml_value: &serde_json::Value,
) -> Result<Option<ConfigEntry>> {
    let Some(mut entry) = store.get_config(service, section, tenant_id).await? else {
        return Ok(None);
    };
    let seed = store.get_seed(service, section, tenant_id).await?;
    // A section the caller could not serialize says nothing about the file.
    if !toml_value.is_object() || seed.as_ref() == Some(toml_value) {
        return Ok(Some(entry));
    }
    let merged = match &seed {
        Some(seed) => merge_toml_changes(&entry.value, seed, toml_value),
        None => fill_unset(&entry.value, toml_value),
    };
    if merged != entry.value {
        entry.version = store
            .set_config(service, section, tenant_id, merged.clone(), Some(SEEDED_BY))
            .await?;
        info!(
            service,
            section,
            tenant_id,
            version = entry.version,
            "config brought up to date with its TOML"
        );
        entry.value = merged;
    }
    store
        .set_seed(service, section, tenant_id, toml_value.clone())
        .await?;
    Ok(Some(entry))
}

/// `stored`, with every key whose value moved from `seed` to `toml` taken
/// from `toml`. Objects are settled key by key; anything else whole.
fn merge_toml_changes(
    stored: &serde_json::Value,
    seed: &serde_json::Value,
    toml: &serde_json::Value,
) -> serde_json::Value {
    use serde_json::Value;
    match (stored, toml) {
        (Value::Object(stored), Value::Object(toml)) => {
            let empty = serde_json::Map::new();
            let seed = seed.as_object().unwrap_or(&empty);
            let mut out = stored.clone();
            for (key, new) in toml {
                let old = seed.get(key).unwrap_or(&Value::Null);
                if new == old {
                    continue;
                }
                let merged = match stored.get(key) {
                    Some(current) => merge_toml_changes(current, old, new),
                    None => new.clone(),
                };
                out.insert(key.clone(), merged);
            }
            Value::Object(out)
        }
        _ if toml != seed => toml.clone(),
        _ => stored.clone(),
    }
}

/// `stored`, with keys it lacks or holds as null taken from `toml`.
fn fill_unset(stored: &serde_json::Value, toml: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match (stored, toml) {
        (Value::Object(stored), Value::Object(toml)) => {
            let mut out = stored.clone();
            for (key, new) in toml {
                let filled = match stored.get(key) {
                    None | Some(Value::Null) => new.clone(),
                    Some(current) => fill_unset(current, new),
                };
                out.insert(key.clone(), filled);
            }
            Value::Object(out)
        }
        _ => stored.clone(),
    }
}

/// Load a config section from the database, or seed it from the provided TOML value.
///
/// This implements the "DB primary, TOML seed" pattern:
/// 1. Try to load from DB for the given service/section/tenant, brought up
///    to date with the TOML by [`reconcile_with_seed`]
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
    let toml_json = serde_json::to_value(toml_value).map_err(|e| {
        crate::Error::Config(format!("failed to serialize config for seeding: {e}"))
    })?;
    // Try loading from DB first
    if let Some(entry) = reconcile_with_seed(store, service, section, tenant_id, &toml_json).await?
    {
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
    let version = store
        .set_config(
            service,
            section,
            tenant_id,
            toml_json.clone(),
            Some(SEEDED_BY),
        )
        .await?;
    store
        .set_seed(service, section, tenant_id, toml_json)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// One section's row and its audit log, in memory.
    #[derive(Default)]
    struct OneSection {
        row: Mutex<Option<ConfigEntry>>,
        seed: Mutex<Option<serde_json::Value>>,
    }

    #[async_trait]
    impl ConfigStore for OneSection {
        async fn get_config(&self, _: &str, _: &str, _: &str) -> Result<Option<ConfigEntry>> {
            Ok(self.row.lock().unwrap().clone())
        }
        async fn get_service_config(&self, _: &str, _: &str) -> Result<Vec<ConfigEntry>> {
            unimplemented!()
        }
        async fn set_config(
            &self,
            service: &str,
            section: &str,
            tenant_id: &str,
            value: serde_json::Value,
            updated_by: Option<&str>,
        ) -> Result<i64> {
            let mut row = self.row.lock().unwrap();
            let version = row.as_ref().map_or(1, |r| r.version + 1);
            *row = Some(ConfigEntry {
                service: service.into(),
                section: section.into(),
                tenant_id: tenant_id.into(),
                value,
                version,
                updated_at: Utc::now(),
                updated_by: updated_by.map(str::to_string),
            });
            Ok(version)
        }
        async fn delete_config(&self, _: &str, _: &str, _: &str) -> Result<bool> {
            unimplemented!()
        }
        async fn list_configs(&self, _: Option<&str>, _: Option<&str>) -> Result<Vec<ConfigEntry>> {
            unimplemented!()
        }
        async fn get_latest_version(&self, _: &str, _: &str) -> Result<i64> {
            unimplemented!()
        }
        async fn get_seed(&self, _: &str, _: &str, _: &str) -> Result<Option<serde_json::Value>> {
            Ok(self.seed.lock().unwrap().clone())
        }
        async fn set_seed(&self, _: &str, _: &str, _: &str, seed: serde_json::Value) -> Result<()> {
            *self.seed.lock().unwrap() = Some(seed);
            Ok(())
        }
        async fn get_audit_log(
            &self,
            _: Option<&str>,
            _: &str,
            _: Option<&str>,
            _: i64,
        ) -> Result<Vec<AuditEntry>> {
            unimplemented!()
        }
    }

    /// A section as the first boot leaves it: `value` stored and seeded.
    async fn seeded(value: serde_json::Value) -> OneSection {
        let store = OneSection::default();
        store
            .set_config("vm", "vm", "_system", value.clone(), Some(SEEDED_BY))
            .await
            .unwrap();
        store.set_seed("vm", "vm", "_system", value).await.unwrap();
        store
    }

    async fn boot(store: &OneSection, toml: serde_json::Value) -> serde_json::Value {
        reconcile_with_seed(store, "vm", "vm", "_system", &toml)
            .await
            .unwrap()
            .expect("a row")
            .value
    }

    fn version(store: &OneSection) -> i64 {
        store.row.lock().unwrap().as_ref().unwrap().version
    }

    #[tokio::test]
    async fn a_key_set_in_the_toml_after_the_first_boot_is_read() {
        let store = seeded(json!({"play_beep": true, "default_greeting_path": null})).await;
        let toml = json!({"play_beep": true, "default_greeting_path": "/greeting.wav"});
        assert_eq!(boot(&store, toml.clone()).await, toml);
        assert_eq!(version(&store), 2);
        // The next boot with the same file changes nothing.
        assert_eq!(
            boot(&store, toml).await["default_greeting_path"],
            "/greeting.wav"
        );
        assert_eq!(version(&store), 2);
    }

    #[tokio::test]
    async fn an_api_edit_stands_until_the_toml_changes_that_key() {
        let seed = json!({"max": 100, "media": {"public_ip": "1.1.1.1", "port": 1}});
        let store = seeded(seed.clone()).await;
        let edited = json!({"max": 50, "media": {"public_ip": "1.1.1.1", "port": 9}});
        store
            .set_config("vm", "vm", "_system", edited.clone(), Some("api"))
            .await
            .unwrap();
        // The file unchanged: the edit stands, nothing is written.
        assert_eq!(boot(&store, seed).await, edited);
        assert_eq!(version(&store), 2);
        // The file changes one nested key: that key moves, the edits stay,
        // and stay on the boot after.
        let toml = json!({"max": 100, "media": {"public_ip": "2.2.2.2", "port": 1}});
        let moved = json!({"max": 50, "media": {"public_ip": "2.2.2.2", "port": 9}});
        assert_eq!(boot(&store, toml.clone()).await, moved);
        assert_eq!(boot(&store, toml).await, moved);
        assert_eq!(version(&store), 3);
    }

    #[tokio::test]
    async fn with_no_seed_on_record_only_unset_keys_are_filled() {
        let store = OneSection::default();
        store
            .set_config(
                "vm",
                "vm",
                "_system",
                json!({"max": 50, "path": null}),
                Some("api"),
            )
            .await
            .unwrap();
        let toml = json!({"max": 100, "path": "/p", "new": 1});
        let filled = json!({"max": 50, "path": "/p", "new": 1});
        assert_eq!(boot(&store, toml.clone()).await, filled);
        // The file is now the seed: the edit to max still stands.
        assert_eq!(boot(&store, toml).await, filled);
    }

    #[tokio::test]
    async fn no_row_is_left_for_the_caller_to_seed() {
        let store = OneSection::default();
        let got = reconcile_with_seed(&store, "vm", "vm", "_system", &json!({}))
            .await
            .unwrap();
        assert!(got.is_none());
    }
}
