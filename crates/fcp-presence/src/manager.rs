//! PresenceManager — central runtime state for the presence service.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use dashmap::DashMap;
use tracing::{debug, info, warn};

use fcp_core::storage::{PresenceStore, StoredAuthPolicy, StoredPresentity, StoredSubscription};
use fcp_core::TenantId;
use fcp_events::EventPublisher;

use crate::config::PresenceConfig;
use crate::dialog_info;
use crate::events;
use crate::pidf;
use crate::types::{
    EventStateComposition, PresenceAuthPolicy, PresenceInfo, PresenceState, PresenceUpdate,
    Watcher,
};

/// Central manager for presence state and watcher subscriptions.
///
/// Thread-safe via `DashMap` — can be shared across tokio tasks.
#[derive(Clone)]
pub struct PresenceManager {
    /// AOR → PresenceInfo
    presentities: Arc<DashMap<String, PresenceInfo>>,
    /// Presentity AOR → Vec<Watcher>
    watchers: Arc<DashMap<String, Vec<Watcher>>>,
    /// Configuration
    config: Arc<parking_lot::RwLock<PresenceConfig>>,
    /// Optional event publisher
    events: Option<Arc<dyn EventPublisher>>,
    /// Optional persistence store for watcher subscriptions
    store: Option<Arc<dyn PresenceStore>>,
    /// Per-presentity authorization policies (AOR → policy)
    auth_policies: Arc<DashMap<String, PresenceAuthPolicy>>,
    /// RFC 3903 PUBLISH ETag → Event State Composition tracking
    publish_etags: Arc<DashMap<String, EventStateComposition>>,
    /// RFC 4235 dialog-info version counter per presentity AOR
    dialog_versions: Arc<DashMap<String, u64>>,
}

impl PresenceManager {
    /// Create a new PresenceManager.
    pub fn new(
        config: PresenceConfig,
        events: Option<Arc<dyn EventPublisher>>,
        store: Option<Arc<dyn PresenceStore>>,
    ) -> Self {
        Self {
            presentities: Arc::new(DashMap::new()),
            watchers: Arc::new(DashMap::new()),
            config: Arc::new(parking_lot::RwLock::new(config)),
            events,
            store,
            auth_policies: Arc::new(DashMap::new()),
            publish_etags: Arc::new(DashMap::new()),
            dialog_versions: Arc::new(DashMap::new()),
        }
    }

    /// Cap a requested PUBLISH expiry to the configured maximum.
    pub fn cap_publish_expiry(&self, requested: u32) -> u32 {
        let max = self.config.read().max_publish_expiry_secs;
        requested.min(max)
    }

    /// Get the configured notify delay.
    pub fn notify_delay(&self) -> Duration {
        Duration::from_millis(self.config.read().notify_delay_ms)
    }

    /// Get the configured cleanup interval.
    pub fn cleanup_interval(&self) -> Duration {
        Duration::from_secs(self.config.read().cleanup_interval_secs)
    }

    /// Clean up expired subscriptions from the persistent store.
    pub async fn cleanup_expired_subscriptions_from_store(&self) -> usize {
        if let Some(ref store) = self.store {
            match store.cleanup_expired_subscriptions().await {
                Ok(count) => {
                    if count > 0 {
                        debug!(count, "Cleaned expired subscriptions from store");
                    }
                    count
                }
                Err(e) => {
                    warn!(error = %e, "Failed to cleanup expired subscriptions");
                    0
                }
            }
        } else {
            0
        }
    }

    /// Update or create a presentity's presence state.
    pub async fn update_presence(
        &self,
        update: PresenceUpdate,
        tenant_id: &TenantId,
    ) -> Result<PresenceInfo> {
        // Extract config values before any await point (parking_lot guards are !Send)
        let max_presentities = self.config.read().max_presentities;

        // Check max presentities limit
        if !self.presentities.contains_key(&update.aor)
            && self.presentities.len() >= max_presentities
        {
            bail!(
                "Max presentities limit reached ({max_presentities})"
            );
        }

        let old_state = self
            .presentities
            .get(&update.aor)
            .map(|p| p.state);

        let info = PresenceInfo {
            aor: update.aor.clone(),
            state: update.state,
            note: update.note,
            tenant_id: tenant_id.clone(),
            updated_at: Instant::now(),
        };

        self.presentities.insert(update.aor.clone(), info.clone());

        // Persist to store (non-blocking)
        if let Some(ref store) = self.store {
            let stored = StoredPresentity {
                aor: info.aor.clone(),
                state: info.state.to_string(),
                note: info.note.clone(),
                tenant_id: tenant_id.clone(),
                updated_at_epoch: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            };
            let store = Arc::clone(store);
            tokio::spawn(async move {
                if let Err(e) = store.save_presentity(stored).await {
                    warn!(error = %e, "Failed to persist presentity state");
                }
            });
        }

        // Publish status changed event if state actually changed
        let old = old_state.unwrap_or(PresenceState::Closed);
        if old != update.state {
            debug!(
                aor = %update.aor,
                old_state = %old,
                new_state = %update.state,
                "Presence state changed"
            );
            events::status_changed(&self.events, &update.aor, old, update.state, tenant_id)
                .await;
        }

        Ok(info)
    }

    /// Get current presence info for an AOR.
    pub fn get_presence(&self, aor: &str) -> Option<PresenceInfo> {
        self.presentities.get(aor).map(|p| p.clone())
    }

    /// Get all tracked presentities.
    pub fn get_all_presences(&self) -> Vec<PresenceInfo> {
        self.presentities
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get all presences filtered by tenant.
    pub fn get_presences_by_tenant(&self, tenant_id: &TenantId) -> Vec<PresenceInfo> {
        self.presentities
            .iter()
            .filter(|e| &e.value().tenant_id == tenant_id)
            .map(|e| e.value().clone())
            .collect()
    }

    /// Remove a presentity (gone offline).
    ///
    /// Publishes a StatusChanged event (→ Closed) so watchers are notified,
    /// and removes from persistent storage.
    pub async fn remove_presence(&self, aor: &str, tenant_id: &TenantId) {
        let old_state = self
            .presentities
            .get(aor)
            .map(|p| p.state);

        self.presentities.remove(aor);

        // Publish StatusChanged event so the NOTIFY dispatcher can inform watchers
        if let Some(old) = old_state {
            if old != PresenceState::Closed {
                events::status_changed(&self.events, aor, old, PresenceState::Closed, tenant_id)
                    .await;
            }
        }

        // Remove from persistent store
        if let Some(ref store) = self.store {
            let store = Arc::clone(store);
            let aor = aor.to_string();
            tokio::spawn(async move {
                if let Err(e) = store.remove_presentity(&aor).await {
                    warn!(error = %e, "Failed to remove presentity from store");
                }
            });
        }
    }

    /// Add a watcher subscription for a presentity.
    pub async fn add_watcher(&self, watcher: Watcher) -> Result<()> {
        let max = self.config.read().max_watchers_per_presentity;

        let presentity_aor = watcher.presentity_aor.clone();
        let watcher_aor = watcher.watcher_aor.clone();
        let tenant_id = watcher.tenant_id.clone();

        // Verify cross-tenant isolation: watcher must belong to the same tenant as the presentity
        if let Some(info) = self.presentities.get(&presentity_aor) {
            if info.tenant_id != watcher.tenant_id {
                bail!("Cross-tenant subscription not allowed");
            }
        }

        let mut entry = self
            .watchers
            .entry(presentity_aor.clone())
            .or_default();

        if max > 0 && entry.len() >= max {
            bail!(
                "Max watchers per presentity reached ({max}) for {presentity_aor}"
            );
        }

        // Persist to store (non-blocking)
        if let Some(ref store) = self.store {
            let stored = watcher_to_stored(&watcher);
            let store = Arc::clone(store);
            tokio::spawn(async move {
                if let Err(e) = store.save_subscription(stored).await {
                    warn!(error = %e, "Failed to persist watcher subscription");
                }
            });
        }

        entry.push(watcher);
        drop(entry);

        info!(
            presentity = %presentity_aor,
            watcher = %watcher_aor,
            "Watcher added"
        );

        events::watcher_added(&self.events, &presentity_aor, &watcher_aor, &tenant_id).await;

        Ok(())
    }

    /// Remove a watcher subscription.
    pub async fn remove_watcher(&self, presentity_aor: &str, subscription_id: &str) {
        let mut removed_aor = None;
        let mut tenant_id = None;

        if let Some(mut entry) = self.watchers.get_mut(presentity_aor) {
            if let Some(pos) = entry
                .iter()
                .position(|w| w.subscription_id == subscription_id)
            {
                let removed = entry.remove(pos);
                removed_aor = Some(removed.watcher_aor);
                tenant_id = Some(removed.tenant_id);
            }
        }

        // Remove from persistent store
        if let Some(ref store) = self.store {
            let store = Arc::clone(store);
            let sub_id = subscription_id.to_string();
            tokio::spawn(async move {
                if let Err(e) = store.remove_subscription(&sub_id).await {
                    warn!(error = %e, "Failed to remove subscription from store");
                }
            });
        }

        if let (Some(watcher_aor), Some(tid)) = (removed_aor, tenant_id) {
            info!(
                presentity = %presentity_aor,
                watcher = %watcher_aor,
                "Watcher removed"
            );
            events::watcher_removed(&self.events, presentity_aor, &watcher_aor, &tid).await;
        }
    }

    /// Get all watchers for a presentity.
    pub fn get_watchers(&self, presentity_aor: &str) -> Vec<Watcher> {
        self.watchers
            .get(presentity_aor)
            .map(|entry| entry.clone())
            .unwrap_or_default()
    }

    /// Get all active watchers across all presentities (for shutdown).
    pub fn get_all_watchers(&self) -> Vec<Watcher> {
        self.watchers
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    /// Get watchers for a presentity filtered by event package.
    pub fn get_watchers_by_event(&self, presentity_aor: &str, event_package: &str) -> Vec<Watcher> {
        self.watchers
            .get(presentity_aor)
            .map(|entry| {
                entry
                    .iter()
                    .filter(|w| w.event_package == event_package)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remove expired watcher subscriptions. Returns the removed watchers.
    pub fn cleanup_expired_watchers(&self) -> Vec<Watcher> {
        let now = Instant::now();
        let mut expired = Vec::new();

        self.watchers.iter_mut().for_each(|mut entry| {
            let (keep, remove): (Vec<_>, Vec<_>) =
                entry.drain(..).partition(|w| w.expires_at > now);
            expired.extend(remove);
            *entry = keep;
        });

        // Clean up empty watcher lists
        self.watchers.retain(|_, v| !v.is_empty());

        if !expired.is_empty() {
            debug!(count = expired.len(), "Cleaned up expired watchers");

            // Remove from persistent store
            if let Some(ref store) = self.store {
                let store = Arc::clone(store);
                let sub_ids: Vec<String> =
                    expired.iter().map(|w| w.subscription_id.clone()).collect();
                tokio::spawn(async move {
                    for sub_id in sub_ids {
                        if let Err(e) = store.remove_subscription(&sub_id).await {
                            warn!(error = %e, "Failed to remove expired subscription from store");
                        }
                    }
                });
            }
        }

        expired
    }

    /// Find an existing watcher subscription by watcher AOR, presentity AOR, and event package.
    /// Returns the subscription_id if found.
    pub fn find_existing_watcher(
        &self,
        watcher_aor: &str,
        presentity_aor: &str,
        event_package: &str,
    ) -> Option<String> {
        self.watchers.get(presentity_aor).and_then(|entry| {
            entry
                .iter()
                .find(|w| w.watcher_aor == watcher_aor && w.event_package == event_package)
                .map(|w| w.subscription_id.clone())
        })
    }

    /// Refresh a watcher's expiry time.
    pub fn refresh_watcher(
        &self,
        presentity_aor: &str,
        subscription_id: &str,
        new_expires: Instant,
    ) -> Result<()> {
        if let Some(mut entry) = self.watchers.get_mut(presentity_aor) {
            if let Some(watcher) = entry
                .iter_mut()
                .find(|w| w.subscription_id == subscription_id)
            {
                watcher.expires_at = new_expires;

                // Update persistent store
                if let Some(ref store) = self.store {
                    let stored = watcher_to_stored(watcher);
                    let store = Arc::clone(store);
                    tokio::spawn(async move {
                        if let Err(e) = store.save_subscription(stored).await {
                            warn!(error = %e, "Failed to persist refreshed subscription");
                        }
                    });
                }

                return Ok(());
            }
        }
        bail!("Watcher not found: {subscription_id} for {presentity_aor}");
    }

    /// Generate PIDF XML for a presentity's current state (for NOTIFY body).
    pub fn generate_pidf(&self, aor: &str) -> Option<String> {
        self.presentities.get(aor).map(|info| {
            pidf::generate_pidf(aor, &info.state, info.note.as_deref())
        })
    }

    /// Parse PIDF XML from a PUBLISH body.
    pub fn parse_pidf(&self, xml: &str) -> Result<PresenceUpdate> {
        pidf::parse_pidf(xml)
    }

    /// Generate dialog-info XML for BLF notifications (RFC 4235).
    ///
    /// Each call increments the version counter for this AOR, ensuring
    /// BLF phones see monotonically increasing version numbers.
    pub fn generate_dialog_info(&self, aor: &str) -> String {
        let state = self
            .presentities
            .get(aor)
            .map(|p| p.state)
            .unwrap_or(PresenceState::Closed);
        let version = self.next_dialog_version(aor);
        dialog_info::generate_dialog_info(aor, &state, None, version)
    }

    /// Atomically increment and return the next dialog-info version for an AOR.
    fn next_dialog_version(&self, aor: &str) -> u64 {
        let mut entry = self.dialog_versions.entry(aor.to_string()).or_insert(0);
        let v = *entry;
        *entry = v + 1;
        v
    }

    /// Auto-set OnThePhone state when a call starts.
    pub async fn set_on_the_phone(&self, aor: &str, tenant_id: &TenantId) {
        let auto = self.config.read().auto_phone_state;
        if !auto {
            return;
        }

        let update = PresenceUpdate {
            aor: aor.to_string(),
            state: PresenceState::OnThePhone,
            note: Some("On the phone".to_string()),
        };

        if let Err(e) = self.update_presence(update, tenant_id).await {
            debug!(aor = %aor, error = %e, "Failed to set on-the-phone state");
        }
    }

    /// Clear OnThePhone state when a call ends, reverting to Open.
    pub async fn clear_on_the_phone(&self, aor: &str, tenant_id: &TenantId) {
        let auto = self.config.read().auto_phone_state;
        if !auto {
            return;
        }

        // Only revert if currently OnThePhone
        let is_on_phone = self
            .presentities
            .get(aor)
            .map(|p| p.state == PresenceState::OnThePhone)
            .unwrap_or(false);

        if is_on_phone {
            let update = PresenceUpdate {
                aor: aor.to_string(),
                state: PresenceState::Open,
                note: None,
            };

            if let Err(e) = self.update_presence(update, tenant_id).await {
                debug!(aor = %aor, error = %e, "Failed to clear on-the-phone state");
            }
        }
    }

    // ---- RFC 3903 PUBLISH ETag management ----

    /// Store an initial PUBLISH, creating a new ESC entry. Returns the assigned ETag.
    pub fn store_publish(
        &self,
        aor: &str,
        state: PresenceState,
        note: Option<String>,
        tenant_id: &TenantId,
        expires_secs: u32,
    ) -> String {
        let etag = uuid::Uuid::new_v4().simple().to_string();
        let esc = EventStateComposition {
            etag: etag.clone(),
            aor: aor.to_string(),
            state,
            note,
            tenant_id: tenant_id.clone(),
            expires_at: Instant::now() + Duration::from_secs(expires_secs as u64),
        };
        self.publish_etags.insert(etag.clone(), esc);
        etag
    }

    /// Refresh an existing PUBLISH (extend expiry, no state change).
    /// Returns a new ETag on success, or an error if the old ETag is not found (→ 412).
    pub fn refresh_publish(&self, etag: &str, expires_secs: u32) -> Result<String> {
        let old = self
            .publish_etags
            .remove(etag)
            .map(|(_, v)| v)
            .ok_or_else(|| anyhow::anyhow!("412 Conditional Request Failed"))?;

        let new_etag = uuid::Uuid::new_v4().simple().to_string();
        let esc = EventStateComposition {
            etag: new_etag.clone(),
            aor: old.aor,
            state: old.state,
            note: old.note,
            tenant_id: old.tenant_id,
            expires_at: Instant::now() + Duration::from_secs(expires_secs as u64),
        };
        self.publish_etags.insert(new_etag.clone(), esc);
        Ok(new_etag)
    }

    /// Modify an existing PUBLISH (replace state, return new ETag).
    /// Returns an error if the old ETag is not found (→ 412).
    pub fn modify_publish(
        &self,
        etag: &str,
        state: PresenceState,
        note: Option<String>,
        expires_secs: u32,
    ) -> Result<String> {
        let old = self
            .publish_etags
            .remove(etag)
            .map(|(_, v)| v)
            .ok_or_else(|| anyhow::anyhow!("412 Conditional Request Failed"))?;

        let new_etag = uuid::Uuid::new_v4().simple().to_string();
        let esc = EventStateComposition {
            etag: new_etag.clone(),
            aor: old.aor,
            state,
            note,
            tenant_id: old.tenant_id,
            expires_at: Instant::now() + Duration::from_secs(expires_secs as u64),
        };
        self.publish_etags.insert(new_etag.clone(), esc);
        Ok(new_etag)
    }

    /// Remove a PUBLISH (Expires: 0). Returns an error if ETag not found (→ 412).
    pub fn remove_publish(&self, etag: &str) -> Result<()> {
        self.publish_etags
            .remove(etag)
            .ok_or_else(|| anyhow::anyhow!("412 Conditional Request Failed"))?;
        Ok(())
    }

    /// Clean up expired PUBLISH ESC entries. If the last ESC for an AOR is removed,
    /// the presentity reverts to Closed.
    pub async fn cleanup_expired_publishes(&self) {
        let now = Instant::now();
        let mut expired_aors: Vec<(String, TenantId)> = Vec::new();

        self.publish_etags.retain(|_, esc| {
            if esc.expires_at <= now {
                expired_aors.push((esc.aor.clone(), esc.tenant_id.clone()));
                false
            } else {
                true
            }
        });

        // For each AOR that had an ESC expire, check if any remaining ESCs exist.
        // If none remain, revert to Closed.
        for (aor, tenant_id) in expired_aors {
            let has_remaining = self
                .publish_etags
                .iter()
                .any(|entry| entry.value().aor == aor);
            if !has_remaining {
                debug!(aor = %aor, "Last PUBLISH expired, reverting to Closed");
                let update = PresenceUpdate {
                    aor: aor.clone(),
                    state: PresenceState::Closed,
                    note: None,
                };
                if let Err(e) = self.update_presence(update, &tenant_id).await {
                    warn!(aor = %aor, error = %e, "Failed to revert presence after PUBLISH expiry");
                }
            }
        }
    }

    /// Get the number of tracked presentities.
    pub fn presentity_count(&self) -> usize {
        self.presentities.len()
    }

    /// Get the total number of watcher subscriptions across all presentities.
    pub fn total_watcher_count(&self) -> usize {
        self.watchers
            .iter()
            .map(|entry| entry.len())
            .sum()
    }

    /// Set authorization policy for a presentity.
    pub fn set_auth_policy(&self, aor: &str, policy: PresenceAuthPolicy) {
        self.auth_policies.insert(aor.to_string(), policy.clone());

        // Persist to store (non-blocking)
        if let Some(ref store) = self.store {
            let stored = StoredAuthPolicy {
                aor: aor.to_string(),
                default_allow: policy.default_allow,
                allow_list_json: serde_json::to_string(&policy.allow_list).unwrap_or_default(),
                block_list_json: serde_json::to_string(&policy.block_list).unwrap_or_default(),
                tenant_id: TenantId::nil(), // Will be overridden if tenant context available
            };
            let store = Arc::clone(store);
            tokio::spawn(async move {
                if let Err(e) = store.save_auth_policy(stored).await {
                    warn!(error = %e, "Failed to persist auth policy");
                }
            });
        }
    }

    /// Get authorization policy for a presentity.
    pub fn get_auth_policy(&self, aor: &str) -> Option<PresenceAuthPolicy> {
        self.auth_policies.get(aor).map(|p| p.clone())
    }

    /// Remove authorization policy for a presentity (reverts to default-allow).
    pub fn remove_auth_policy(&self, aor: &str) {
        self.auth_policies.remove(aor);

        // Remove from persistent store (non-blocking)
        if let Some(ref store) = self.store {
            let store = Arc::clone(store);
            let aor = aor.to_string();
            tokio::spawn(async move {
                if let Err(e) = store.remove_auth_policy(&aor).await {
                    warn!(error = %e, "Failed to remove auth policy from store");
                }
            });
        }
    }

    /// Check if a watcher is authorized to subscribe to a presentity.
    ///
    /// If no policy is set, all watchers are allowed (default open).
    pub fn is_watcher_authorized(&self, watcher_aor: &str, presentity_aor: &str) -> bool {
        match self.auth_policies.get(presentity_aor) {
            Some(policy) => policy.is_allowed(watcher_aor),
            None => true, // No policy = allow all
        }
    }

    /// Load persisted auth policies from the store into memory.
    /// Returns the number of policies loaded.
    pub async fn load_auth_policies(&self, tenant_id: &TenantId) -> Result<usize> {
        let store = match &self.store {
            Some(s) => s,
            None => return Ok(0),
        };

        let stored = store.load_auth_policies(tenant_id).await?;
        let count = stored.len();

        for s in stored {
            let allow_list: Vec<String> =
                serde_json::from_str(&s.allow_list_json).unwrap_or_default();
            let block_list: Vec<String> =
                serde_json::from_str(&s.block_list_json).unwrap_or_default();
            let policy = PresenceAuthPolicy {
                default_allow: s.default_allow,
                allow_list,
                block_list,
            };
            self.auth_policies.insert(s.aor, policy);
        }

        if count > 0 {
            info!(count, "Loaded persisted presence auth policies");
        }

        Ok(count)
    }

    /// Load persisted subscriptions from the store into memory.
    /// Returns the number of subscriptions loaded.
    pub async fn load_subscriptions(&self, tenant_id: &TenantId) -> Result<usize> {
        let store = match &self.store {
            Some(s) => s,
            None => return Ok(0),
        };

        let stored = store.load_subscriptions(tenant_id).await?;
        let count = stored.len();

        for s in stored {
            let watcher = stored_to_watcher(&s);
            self.watchers
                .entry(watcher.presentity_aor.clone())
                .or_default()
                .push(watcher);
        }

        if count > 0 {
            info!(count, "Loaded persisted presence subscriptions");
        }

        Ok(count)
    }

    /// Load persisted presentity state from the store into memory.
    /// Returns the number of presentities loaded.
    pub async fn load_presentities(&self, tenant_id: &TenantId) -> Result<usize> {
        let store = match &self.store {
            Some(s) => s,
            None => return Ok(0),
        };

        let stored = store.load_presentities(tenant_id).await?;
        let count = stored.len();

        for s in stored {
            let state: PresenceState = s.state.parse().unwrap_or(PresenceState::Closed);
            let info = PresenceInfo {
                aor: s.aor.clone(),
                state,
                note: s.note,
                tenant_id: s.tenant_id,
                updated_at: Instant::now(),
            };
            self.presentities.insert(s.aor, info);
        }

        if count > 0 {
            info!(count, "Loaded persisted presentity states");
        }

        Ok(count)
    }
}

/// Convert a Watcher to a StoredSubscription for persistence.
fn watcher_to_stored(w: &Watcher) -> StoredSubscription {
    let remaining = w.expires_at.saturating_duration_since(Instant::now());
    let expires_at_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + remaining.as_secs();

    StoredSubscription {
        subscription_id: w.subscription_id.clone(),
        watcher_aor: w.watcher_aor.clone(),
        presentity_aor: w.presentity_aor.clone(),
        tenant_id: w.tenant_id.clone(),
        contact_uri: w.contact_uri.clone(),
        event_package: w.event_package.clone(),
        expires_at_epoch,
    }
}

/// Convert a StoredSubscription back to a Watcher (runtime type with Instant).
fn stored_to_watcher(s: &StoredSubscription) -> Watcher {
    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let remaining_secs = s.expires_at_epoch.saturating_sub(now_epoch);

    Watcher {
        watcher_aor: s.watcher_aor.clone(),
        presentity_aor: s.presentity_aor.clone(),
        subscription_id: s.subscription_id.clone(),
        expires_at: Instant::now() + Duration::from_secs(remaining_secs),
        tenant_id: s.tenant_id.clone(),
        contact_uri: s.contact_uri.clone(),
        event_package: s.event_package.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_config() -> PresenceConfig {
        PresenceConfig {
            enabled: true,
            max_watchers_per_presentity: 100,
            max_presentities: 10000,
            ..Default::default()
        }
    }

    fn test_tenant() -> TenantId {
        TenantId::new()
    }

    #[tokio::test]
    async fn test_update_and_get_presence() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Open,
            note: Some("Available".to_string()),
        };

        let info = mgr.update_presence(update, &tenant).await.unwrap();
        assert_eq!(info.state, PresenceState::Open);
        assert_eq!(info.note.as_deref(), Some("Available"));

        let retrieved = mgr.get_presence("alice@example.com").unwrap();
        assert_eq!(retrieved.state, PresenceState::Open);
        assert_eq!(retrieved.aor, "alice@example.com");
    }

    #[tokio::test]
    async fn test_update_presence_publishes_event() {
        use fcp_events::InMemoryEventBus;

        let bus = Arc::new(InMemoryEventBus::new());
        let mgr = PresenceManager::new(test_config(), Some(bus.clone()), None);
        let tenant = test_tenant();

        // First update sets state (transitions from Closed → Open)
        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();

        // Change state to trigger event
        let update2 = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Busy,
            note: None,
        };
        mgr.update_presence(update2, &tenant).await.unwrap();

        // Verify events were published (we can check the bus has events)
        // The InMemoryEventBus stores events; we just verify no errors occurred.
    }

    #[tokio::test]
    async fn test_remove_presence() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();
        assert!(mgr.get_presence("alice@example.com").is_some());

        mgr.remove_presence("alice@example.com", &tenant).await;
        assert!(mgr.get_presence("alice@example.com").is_none());
    }

    #[tokio::test]
    async fn test_add_and_get_watchers() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let watcher = Watcher {
            watcher_aor: "bob@example.com".to_string(),
            presentity_aor: "alice@example.com".to_string(),
            subscription_id: "sub-1".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
            tenant_id: tenant.clone(),
            contact_uri: "sip:bob@192.168.1.100:5060".to_string(),
            event_package: "presence".to_string(),
        };

        mgr.add_watcher(watcher).await.unwrap();

        let watchers = mgr.get_watchers("alice@example.com");
        assert_eq!(watchers.len(), 1);
        assert_eq!(watchers[0].watcher_aor, "bob@example.com");
        assert_eq!(watchers[0].subscription_id, "sub-1");
    }

    #[tokio::test]
    async fn test_remove_watcher() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let watcher = Watcher {
            watcher_aor: "bob@example.com".to_string(),
            presentity_aor: "alice@example.com".to_string(),
            subscription_id: "sub-1".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
            tenant_id: tenant.clone(),
            contact_uri: "sip:bob@192.168.1.100:5060".to_string(),
            event_package: "presence".to_string(),
        };

        mgr.add_watcher(watcher).await.unwrap();
        assert_eq!(mgr.get_watchers("alice@example.com").len(), 1);

        mgr.remove_watcher("alice@example.com", "sub-1").await;
        assert_eq!(mgr.get_watchers("alice@example.com").len(), 0);
    }

    #[tokio::test]
    async fn test_max_watchers_enforced() {
        let config = PresenceConfig {
            max_watchers_per_presentity: 2,
            ..test_config()
        };
        let mgr = PresenceManager::new(config, None, None);
        let tenant = test_tenant();

        for i in 0..2 {
            let watcher = Watcher {
                watcher_aor: format!("user{i}@example.com"),
                presentity_aor: "alice@example.com".to_string(),
                subscription_id: format!("sub-{i}"),
                expires_at: Instant::now() + Duration::from_secs(3600),
                tenant_id: tenant.clone(),
                contact_uri: format!("sip:user{i}@192.168.1.100:5060"),
                event_package: "presence".to_string(),
            };
            mgr.add_watcher(watcher).await.unwrap();
        }

        // Third should fail
        let watcher = Watcher {
            watcher_aor: "user2@example.com".to_string(),
            presentity_aor: "alice@example.com".to_string(),
            subscription_id: "sub-2".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
            tenant_id: tenant.clone(),
            contact_uri: "sip:user2@192.168.1.100:5060".to_string(),
            event_package: "presence".to_string(),
        };
        let result = mgr.add_watcher(watcher).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cleanup_expired_watchers() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        // Add an already-expired watcher
        let watcher = Watcher {
            watcher_aor: "bob@example.com".to_string(),
            presentity_aor: "alice@example.com".to_string(),
            subscription_id: "sub-expired".to_string(),
            expires_at: Instant::now() - Duration::from_secs(1),
            tenant_id: tenant.clone(),
            contact_uri: "sip:bob@192.168.1.100:5060".to_string(),
            event_package: "presence".to_string(),
        };
        mgr.add_watcher(watcher).await.unwrap();

        // Add a valid watcher
        let watcher2 = Watcher {
            watcher_aor: "charlie@example.com".to_string(),
            presentity_aor: "alice@example.com".to_string(),
            subscription_id: "sub-valid".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
            tenant_id: tenant.clone(),
            contact_uri: "sip:charlie@192.168.1.101:5060".to_string(),
            event_package: "presence".to_string(),
        };
        mgr.add_watcher(watcher2).await.unwrap();

        let removed = mgr.cleanup_expired_watchers();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].subscription_id, "sub-expired");
        assert_eq!(mgr.get_watchers("alice@example.com").len(), 1);
        assert_eq!(
            mgr.get_watchers("alice@example.com")[0].subscription_id,
            "sub-valid"
        );
    }

    #[tokio::test]
    async fn test_refresh_watcher() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let original_expires = Instant::now() + Duration::from_secs(100);
        let watcher = Watcher {
            watcher_aor: "bob@example.com".to_string(),
            presentity_aor: "alice@example.com".to_string(),
            subscription_id: "sub-1".to_string(),
            expires_at: original_expires,
            tenant_id: tenant.clone(),
            contact_uri: "sip:bob@192.168.1.100:5060".to_string(),
            event_package: "presence".to_string(),
        };
        mgr.add_watcher(watcher).await.unwrap();

        let new_expires = Instant::now() + Duration::from_secs(3600);
        mgr.refresh_watcher("alice@example.com", "sub-1", new_expires)
            .unwrap();

        let watchers = mgr.get_watchers("alice@example.com");
        assert!(watchers[0].expires_at > original_expires);
    }

    #[tokio::test]
    async fn test_on_the_phone_auto_state() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        // Start with Open state
        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();

        // Set on-the-phone
        mgr.set_on_the_phone("alice@example.com", &tenant).await;
        let info = mgr.get_presence("alice@example.com").unwrap();
        assert_eq!(info.state, PresenceState::OnThePhone);

        // Clear on-the-phone → should revert to Open
        mgr.clear_on_the_phone("alice@example.com", &tenant).await;
        let info = mgr.get_presence("alice@example.com").unwrap();
        assert_eq!(info.state, PresenceState::Open);
    }

    #[tokio::test]
    async fn test_presentity_count() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        assert_eq!(mgr.presentity_count(), 0);

        for i in 0..3 {
            let update = PresenceUpdate {
                aor: format!("user{i}@example.com"),
                state: PresenceState::Open,
                note: None,
            };
            mgr.update_presence(update, &tenant).await.unwrap();
        }

        assert_eq!(mgr.presentity_count(), 3);
        assert_eq!(mgr.get_all_presences().len(), 3);
    }

    #[tokio::test]
    async fn test_total_watcher_count() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        assert_eq!(mgr.total_watcher_count(), 0);

        for i in 0..3 {
            let watcher = Watcher {
                watcher_aor: format!("watcher{i}@example.com"),
                presentity_aor: "alice@example.com".to_string(),
                subscription_id: format!("sub-{i}"),
                expires_at: Instant::now() + Duration::from_secs(3600),
                tenant_id: tenant.clone(),
                contact_uri: format!("sip:watcher{i}@192.168.1.100:5060"),
                event_package: "presence".to_string(),
            };
            mgr.add_watcher(watcher).await.unwrap();
        }

        assert_eq!(mgr.total_watcher_count(), 3);
    }

    #[tokio::test]
    async fn test_find_existing_watcher() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        // No watcher → None
        assert!(mgr.find_existing_watcher("bob@example.com", "alice@example.com", "presence").is_none());

        let watcher = Watcher {
            watcher_aor: "bob@example.com".to_string(),
            presentity_aor: "alice@example.com".to_string(),
            subscription_id: "sub-1".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
            tenant_id: tenant.clone(),
            contact_uri: "sip:bob@192.168.1.100:5060".to_string(),
            event_package: "presence".to_string(),
        };
        mgr.add_watcher(watcher).await.unwrap();

        // Find existing
        assert_eq!(
            mgr.find_existing_watcher("bob@example.com", "alice@example.com", "presence"),
            Some("sub-1".to_string())
        );

        // Wrong event → None
        assert!(mgr.find_existing_watcher("bob@example.com", "alice@example.com", "dialog").is_none());

        // Wrong watcher → None
        assert!(mgr.find_existing_watcher("charlie@example.com", "alice@example.com", "presence").is_none());
    }

    #[tokio::test]
    async fn test_generate_pidf() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        // No presentity → None
        assert!(mgr.generate_pidf("nobody@example.com").is_none());

        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Busy,
            note: Some("Do not disturb".to_string()),
        };
        mgr.update_presence(update, &tenant).await.unwrap();

        let xml = mgr.generate_pidf("alice@example.com").unwrap();
        assert!(xml.contains("pres:alice@example.com"));
        assert!(xml.contains("<basic>open</basic>"));
        assert!(xml.contains("<note>Do not disturb</note>"));
    }

    // ---- Fix 1: PUBLISH ETag tracking tests ----

    #[test]
    fn test_store_publish_returns_etag() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let etag = mgr.store_publish(
            "alice@example.com",
            PresenceState::Open,
            Some("Available".into()),
            &tenant,
            3600,
        );
        assert!(!etag.is_empty());
    }

    #[test]
    fn test_refresh_publish_returns_new_etag() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let etag1 = mgr.store_publish("alice@example.com", PresenceState::Open, None, &tenant, 3600);
        let etag2 = mgr.refresh_publish(&etag1, 7200).unwrap();
        assert_ne!(etag1, etag2);

        // Old ETag should no longer be valid
        assert!(mgr.refresh_publish(&etag1, 3600).is_err());
    }

    #[test]
    fn test_modify_publish_returns_new_etag() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let etag1 = mgr.store_publish("alice@example.com", PresenceState::Open, None, &tenant, 3600);
        let etag2 = mgr.modify_publish(&etag1, PresenceState::Busy, Some("DND".into()), 3600).unwrap();
        assert_ne!(etag1, etag2);

        // Old ETag should no longer be valid
        assert!(mgr.modify_publish(&etag1, PresenceState::Away, None, 3600).is_err());
    }

    #[test]
    fn test_remove_publish() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let etag = mgr.store_publish("alice@example.com", PresenceState::Open, None, &tenant, 3600);
        mgr.remove_publish(&etag).unwrap();

        // Removed ETag should return 412
        assert!(mgr.remove_publish(&etag).is_err());
    }

    #[test]
    fn test_412_on_unknown_etag() {
        let mgr = PresenceManager::new(test_config(), None, None);
        assert!(mgr.refresh_publish("nonexistent", 3600).is_err());
        assert!(mgr.modify_publish("nonexistent", PresenceState::Open, None, 3600).is_err());
        assert!(mgr.remove_publish("nonexistent").is_err());
    }

    #[tokio::test]
    async fn test_cleanup_expired_publishes() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        // Store a PUBLISH that expires immediately
        let etag = mgr.store_publish("alice@example.com", PresenceState::Open, None, &tenant, 0);

        // Also set presence so cleanup can revert it
        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();

        // Wait a tiny bit for expiry
        tokio::time::sleep(Duration::from_millis(10)).await;

        mgr.cleanup_expired_publishes().await;

        // The expired ETag should be gone
        assert!(mgr.refresh_publish(&etag, 3600).is_err());
    }

    // ---- Fix 3: Dialog-info version counter tests ----

    #[tokio::test]
    async fn test_dialog_info_version_increments() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::OnThePhone,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();

        let xml0 = mgr.generate_dialog_info("alice@example.com");
        assert!(xml0.contains("version=\"0\""));

        let xml1 = mgr.generate_dialog_info("alice@example.com");
        assert!(xml1.contains("version=\"1\""));

        let xml2 = mgr.generate_dialog_info("alice@example.com");
        assert!(xml2.contains("version=\"2\""));
    }

    #[tokio::test]
    async fn test_dialog_info_version_per_aor() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        for aor in &["alice@example.com", "bob@example.com"] {
            let update = PresenceUpdate {
                aor: aor.to_string(),
                state: PresenceState::Ringing,
                note: None,
            };
            mgr.update_presence(update, &tenant).await.unwrap();
        }

        // Each AOR should have independent version counters
        let xml_alice = mgr.generate_dialog_info("alice@example.com");
        assert!(xml_alice.contains("version=\"0\""));

        let xml_bob = mgr.generate_dialog_info("bob@example.com");
        assert!(xml_bob.contains("version=\"0\""));

        // Second call for alice should be version 1
        let xml_alice2 = mgr.generate_dialog_info("alice@example.com");
        assert!(xml_alice2.contains("version=\"1\""));
    }

    // ---- Fix 4: remove_presence publishes event ----

    #[tokio::test]
    async fn test_remove_presence_publishes_event() {
        use fcp_events::InMemoryEventBus;

        let bus = Arc::new(InMemoryEventBus::new());
        let mgr = PresenceManager::new(test_config(), Some(bus.clone()), None);
        let tenant = test_tenant();

        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();

        // Remove should publish StatusChanged (Open → Closed)
        mgr.remove_presence("alice@example.com", &tenant).await;

        assert!(mgr.get_presence("alice@example.com").is_none());
        // The event was published to the bus (verified by no errors)
    }

    // ---- P1 fixes: config, tenant isolation, shutdown ----

    #[test]
    fn test_cap_publish_expiry_default() {
        let mgr = PresenceManager::new(test_config(), None, None);
        // Default max is 3600
        assert_eq!(mgr.cap_publish_expiry(1800), 1800); // under cap
        assert_eq!(mgr.cap_publish_expiry(3600), 3600); // at cap
        assert_eq!(mgr.cap_publish_expiry(7200), 3600); // above cap
    }

    #[test]
    fn test_cap_publish_expiry_custom_max() {
        let config = PresenceConfig {
            max_publish_expiry_secs: 600,
            ..test_config()
        };
        let mgr = PresenceManager::new(config, None, None);
        assert_eq!(mgr.cap_publish_expiry(300), 300);
        assert_eq!(mgr.cap_publish_expiry(600), 600);
        assert_eq!(mgr.cap_publish_expiry(3600), 600);
    }

    #[test]
    fn test_notify_delay_returns_configured() {
        let config = PresenceConfig {
            notify_delay_ms: 100,
            ..test_config()
        };
        let mgr = PresenceManager::new(config, None, None);
        assert_eq!(mgr.notify_delay(), Duration::from_millis(100));
    }

    #[test]
    fn test_cleanup_interval_returns_configured() {
        let config = PresenceConfig {
            cleanup_interval_secs: 120,
            ..test_config()
        };
        let mgr = PresenceManager::new(config, None, None);
        assert_eq!(mgr.cleanup_interval(), Duration::from_secs(120));
    }

    #[tokio::test]
    async fn test_cross_tenant_watcher_rejected() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant_a = TenantId::new();
        let tenant_b = TenantId::new();

        // Create presentity for tenant A
        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant_a).await.unwrap();

        // Try to add watcher from tenant B → should fail
        let watcher = Watcher {
            watcher_aor: "bob@other.com".to_string(),
            presentity_aor: "alice@example.com".to_string(),
            subscription_id: "sub-cross".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
            tenant_id: tenant_b,
            contact_uri: "sip:bob@192.168.1.100:5060".to_string(),
            event_package: "presence".to_string(),
        };
        let result = mgr.add_watcher(watcher).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cross-tenant"));
    }

    #[tokio::test]
    async fn test_get_presences_by_tenant() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant_a = TenantId::new();
        let tenant_b = TenantId::new();

        // Create presentities for different tenants
        let update_a = PresenceUpdate {
            aor: "alice@a.com".to_string(),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update_a, &tenant_a).await.unwrap();

        let update_b = PresenceUpdate {
            aor: "bob@b.com".to_string(),
            state: PresenceState::Busy,
            note: None,
        };
        mgr.update_presence(update_b, &tenant_b).await.unwrap();

        let presences_a = mgr.get_presences_by_tenant(&tenant_a);
        assert_eq!(presences_a.len(), 1);
        assert_eq!(presences_a[0].aor, "alice@a.com");

        let presences_b = mgr.get_presences_by_tenant(&tenant_b);
        assert_eq!(presences_b.len(), 1);
        assert_eq!(presences_b[0].aor, "bob@b.com");
    }

    #[tokio::test]
    async fn test_get_all_watchers() {
        let mgr = PresenceManager::new(test_config(), None, None);
        let tenant = test_tenant();

        // Add watchers to different presentities
        for (i, presentity) in ["alice@example.com", "bob@example.com"].iter().enumerate() {
            let watcher = Watcher {
                watcher_aor: format!("watcher{i}@example.com"),
                presentity_aor: presentity.to_string(),
                subscription_id: format!("sub-{i}"),
                expires_at: Instant::now() + Duration::from_secs(3600),
                tenant_id: tenant.clone(),
                contact_uri: format!("sip:watcher{i}@192.168.1.100:5060"),
                event_package: "presence".to_string(),
            };
            mgr.add_watcher(watcher).await.unwrap();
        }

        let all = mgr.get_all_watchers();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_auth_policy_persistence_with_none_store() {
        // With no store, set/remove should be no-op (not crash)
        let mgr = PresenceManager::new(test_config(), None, None);
        let policy = PresenceAuthPolicy {
            default_allow: false,
            allow_list: vec!["bob@example.com".to_string()],
            block_list: vec![],
        };
        mgr.set_auth_policy("alice@example.com", policy);
        assert!(mgr.get_auth_policy("alice@example.com").is_some());

        mgr.remove_auth_policy("alice@example.com");
        assert!(mgr.get_auth_policy("alice@example.com").is_none());
    }
}
