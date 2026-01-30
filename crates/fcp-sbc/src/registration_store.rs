//! In-memory registration store for the SBC.
//!
//! Lightweight `RegistrationStore` backed by a `DashMap` — no file I/O,
//! no persistence.  Suitable for an SBC whose registrar exists only to
//! track concurrent registrations and publish lifecycle events.

use async_trait::async_trait;
use dashmap::DashMap;
use fcp_core::storage::{Registration, RegistrationStore};
use std::time::SystemTime;

/// In-memory registration store using `DashMap` for lock-free concurrent access.
pub struct InMemoryRegistrationStore {
    /// Keyed by AOR → list of active bindings.
    registrations: DashMap<String, Vec<Registration>>,
}

impl InMemoryRegistrationStore {
    pub fn new() -> Self {
        Self {
            registrations: DashMap::new(),
        }
    }
}

#[async_trait]
impl RegistrationStore for InMemoryRegistrationStore {
    async fn get_registrations(&self, aor: &str) -> anyhow::Result<Vec<Registration>> {
        Ok(self
            .registrations
            .get(aor)
            .map(|r| r.value().clone())
            .unwrap_or_default())
    }

    async fn put_registration(&self, registration: Registration) -> anyhow::Result<()> {
        let mut entry = self
            .registrations
            .entry(registration.aor.clone())
            .or_default();
        // Replace existing binding for the same contact
        entry.retain(|r| r.contact != registration.contact);
        entry.push(registration);
        Ok(())
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> anyhow::Result<()> {
        if let Some(mut entry) = self.registrations.get_mut(aor) {
            entry.retain(|r| r.contact != contact);
            if entry.is_empty() {
                drop(entry);
                self.registrations.remove(aor);
            }
        }
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> anyhow::Result<()> {
        self.registrations.remove(aor);
        Ok(())
    }

    async fn cleanup_expired(&self) -> anyhow::Result<usize> {
        let now = SystemTime::now();
        let mut removed = 0;

        // Collect keys first to avoid holding refs during mutation
        let keys: Vec<String> = self
            .registrations
            .iter()
            .map(|r| r.key().clone())
            .collect();

        for key in keys {
            if let Some(mut entry) = self.registrations.get_mut(&key) {
                let before = entry.len();
                entry.retain(|r| r.expires_at > now);
                removed += before - entry.len();
                if entry.is_empty() {
                    drop(entry);
                    self.registrations.remove(&key);
                }
            }
        }

        Ok(removed)
    }
}
