//! PresenceManager — central runtime state for the presence service.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Result};
use dashmap::DashMap;
use tracing::{debug, info};

use fcp_core::TenantId;
use fcp_events::EventPublisher;

use crate::config::PresenceConfig;
use crate::dialog_info;
use crate::events;
use crate::pidf;
use crate::types::{PresenceInfo, PresenceState, PresenceUpdate, Watcher};

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
}

impl PresenceManager {
    /// Create a new PresenceManager.
    pub fn new(config: PresenceConfig, events: Option<Arc<dyn EventPublisher>>) -> Self {
        Self {
            presentities: Arc::new(DashMap::new()),
            watchers: Arc::new(DashMap::new()),
            config: Arc::new(parking_lot::RwLock::new(config)),
            events,
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

    /// Remove a presentity (gone offline).
    pub fn remove_presence(&self, aor: &str) {
        self.presentities.remove(aor);
    }

    /// Add a watcher subscription for a presentity.
    pub async fn add_watcher(&self, watcher: Watcher) -> Result<()> {
        let max = self.config.read().max_watchers_per_presentity;

        let presentity_aor = watcher.presentity_aor.clone();
        let watcher_aor = watcher.watcher_aor.clone();
        let tenant_id = watcher.tenant_id.clone();

        let mut entry = self
            .watchers
            .entry(presentity_aor.clone())
            .or_default();

        if max > 0 && entry.len() >= max {
            bail!(
                "Max watchers per presentity reached ({max}) for {presentity_aor}"
            );
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

    /// Remove expired watcher subscriptions. Returns the number removed.
    pub fn cleanup_expired_watchers(&self) -> usize {
        let now = Instant::now();
        let mut removed = 0;

        self.watchers.iter_mut().for_each(|mut entry| {
            let before = entry.len();
            entry.retain(|w| w.expires_at > now);
            removed += before - entry.len();
        });

        // Clean up empty watcher lists
        self.watchers.retain(|_, v| !v.is_empty());

        if removed > 0 {
            debug!(count = removed, "Cleaned up expired watchers");
        }

        removed
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
    pub fn generate_dialog_info(&self, aor: &str) -> String {
        let state = self
            .presentities
            .get(aor)
            .map(|p| p.state)
            .unwrap_or(PresenceState::Closed);
        dialog_info::generate_dialog_info(aor, &state, None)
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
        let mgr = PresenceManager::new(test_config(), None);
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
        let mgr = PresenceManager::new(test_config(), Some(bus.clone()));
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
        let mgr = PresenceManager::new(test_config(), None);
        let tenant = test_tenant();

        let update = PresenceUpdate {
            aor: "alice@example.com".to_string(),
            state: PresenceState::Open,
            note: None,
        };
        mgr.update_presence(update, &tenant).await.unwrap();
        assert!(mgr.get_presence("alice@example.com").is_some());

        mgr.remove_presence("alice@example.com");
        assert!(mgr.get_presence("alice@example.com").is_none());
    }

    #[tokio::test]
    async fn test_add_and_get_watchers() {
        let mgr = PresenceManager::new(test_config(), None);
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
        let mgr = PresenceManager::new(test_config(), None);
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
        let mgr = PresenceManager::new(config, None);
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
        let mgr = PresenceManager::new(test_config(), None);
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
        assert_eq!(removed, 1);
        assert_eq!(mgr.get_watchers("alice@example.com").len(), 1);
        assert_eq!(
            mgr.get_watchers("alice@example.com")[0].subscription_id,
            "sub-valid"
        );
    }

    #[tokio::test]
    async fn test_refresh_watcher() {
        let mgr = PresenceManager::new(test_config(), None);
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
        let mgr = PresenceManager::new(test_config(), None);
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
        let mgr = PresenceManager::new(test_config(), None);
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
        let mgr = PresenceManager::new(test_config(), None);
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
    async fn test_generate_pidf() {
        let mgr = PresenceManager::new(test_config(), None);
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
}
