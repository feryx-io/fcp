//! Simple in-memory RegistrationStore for the SBC.
//!
//! Keeps per-AOR bindings in a DashMap and cleans up expired
//! registrations on access. Intended for lightweight SBC deployments
//! that don't need persistent registrar storage.

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use fcp_core::{Registration, RegistrationStore};
use std::time::SystemTime;

#[derive(Default)]
pub struct InMemoryRegistrationStore {
    inner: DashMap<String, Vec<Registration>>,
}

impl InMemoryRegistrationStore {
    /// Remove expired bindings from the entire store.
    /// Returns the number of individual registrations removed.
    fn remove_expired(&self) -> usize {
        let now = SystemTime::now();
        let keys: Vec<String> = self.inner.iter().map(|e| e.key().clone()).collect();
        let mut removed = 0;
        for key in keys {
            if let Some(mut entry) = self.inner.get_mut(&key) {
                let before = entry.len();
                entry.retain(|reg| reg.expires_at > now);
                removed += before - entry.len();
                if entry.is_empty() {
                    drop(entry);
                    self.inner.remove(&key);
                }
            }
        }
        removed
    }
}

#[async_trait]
impl RegistrationStore for InMemoryRegistrationStore {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        self.remove_expired();
        Ok(self
            .inner
            .get(aor)
            .map(|v| v.value().clone())
            .unwrap_or_default())
    }

    async fn get_registrations_in_tenant(
        &self,
        tenant_id: &fcp_core::TenantId,
        aor: &str,
    ) -> Result<Vec<Registration>> {
        let registrations = self.get_registrations(aor).await?;
        Ok(registrations
            .into_iter()
            .filter(|registration| registration.tenant_id == *tenant_id)
            .collect())
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let mut entry = self.inner.entry(registration.aor.clone()).or_default();
        entry.retain(|r| r.contact != registration.contact);
        entry.push(registration);
        Ok(())
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        if let Some(mut entry) = self.inner.get_mut(aor) {
            entry.retain(|r| r.contact != contact);
            if entry.is_empty() {
                drop(entry);
                self.inner.remove(aor);
            }
        }
        Ok(())
    }

    async fn delete_registration_in_tenant(
        &self,
        tenant_id: &fcp_core::TenantId,
        aor: &str,
        contact: &str,
    ) -> Result<()> {
        if let Some(mut entry) = self.inner.get_mut(aor) {
            entry.retain(|registration| {
                !(registration.contact == contact && registration.tenant_id == *tenant_id)
            });
            if entry.is_empty() {
                drop(entry);
                self.inner.remove(aor);
            }
        }
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        self.inner.remove(aor);
        Ok(())
    }

    async fn delete_all_registrations_in_tenant(
        &self,
        tenant_id: &fcp_core::TenantId,
        aor: &str,
    ) -> Result<()> {
        if let Some(mut entry) = self.inner.get_mut(aor) {
            entry.retain(|registration| registration.tenant_id != *tenant_id);
            if entry.is_empty() {
                drop(entry);
                self.inner.remove(aor);
            }
        }
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        Ok(self.remove_expired())
    }

    async fn get_all_registrations_in_tenant(
        &self,
        tenant_id: &fcp_core::TenantId,
    ) -> Result<Vec<Registration>> {
        self.remove_expired();
        Ok(self
            .inner
            .iter()
            .flat_map(|entry| entry.value().clone())
            .filter(|registration| registration.tenant_id == *tenant_id)
            .collect())
    }
}
