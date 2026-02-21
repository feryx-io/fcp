//! Thin caching wrappers for hot-path storage traits.
//!
//! Each wrapper uses a `DashMap` with TTL-based expiry to avoid hitting the
//! underlying database on every SIP INVITE. Cache entries are evicted either
//! by TTL expiry or by explicit invalidation via `ConfigChanged` events from
//! the distributed event bus.

use crate::storage::{CredentialStore, User};
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::debug;

/// A cache entry with a timestamp for TTL expiry.
#[derive(Clone)]
struct CacheEntry<T: Clone> {
    value: T,
    inserted_at: Instant,
}

impl<T: Clone> CacheEntry<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            inserted_at: Instant::now(),
        }
    }

    fn is_expired(&self, ttl: Duration) -> bool {
        self.inserted_at.elapsed() >= ttl
    }
}

// ---------------------------------------------------------------------------
// CachedCredentialStore
// ---------------------------------------------------------------------------

/// Caching wrapper around any `CredentialStore`.
///
/// Caches `get_user()` results by username. Write-through on `put_user()` and
/// `delete_user()` — the cache entry is evicted immediately.
pub struct CachedCredentialStore {
    inner: Arc<dyn CredentialStore>,
    cache: DashMap<String, CacheEntry<Option<User>>>,
    ttl: Duration,
}

impl CachedCredentialStore {
    /// Create a new cached credential store.
    pub fn new(inner: Arc<dyn CredentialStore>, ttl: Duration) -> Self {
        Self {
            inner,
            cache: DashMap::new(),
            ttl,
        }
    }

    /// Evict a specific user from the cache.
    pub fn evict(&self, username: &str) {
        self.cache.remove(username);
        debug!(username = %username, "Evicted credential cache entry");
    }

    /// Evict all entries from the cache.
    pub fn evict_all(&self) {
        self.cache.clear();
        debug!("Cleared entire credential cache");
    }

    /// Get cache statistics.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

#[async_trait]
impl CredentialStore for CachedCredentialStore {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        // Check cache
        if let Some(entry) = self.cache.get(username) {
            if !entry.is_expired(self.ttl) {
                debug!(username = %username, "Credential cache HIT");
                return Ok(entry.value.clone());
            }
            // Expired — drop ref before removing
            drop(entry);
            self.cache.remove(username);
        }

        // Cache miss — query inner store
        debug!(username = %username, "Credential cache MISS");
        let result = self.inner.get_user(username).await?;

        // Cache the result (including None for negative caching)
        self.cache
            .insert(username.to_string(), CacheEntry::new(result.clone()));

        Ok(result)
    }

    async fn put_user(&self, user: User) -> Result<()> {
        let username = user.username.clone();
        self.inner.put_user(user).await?;
        // Evict to force re-fetch on next access
        self.cache.remove(&username);
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        self.inner.delete_user(username).await?;
        self.cache.remove(username);
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        // list_users is not a hot path, always go to inner
        self.inner.list_users().await
    }
}

// ---------------------------------------------------------------------------
// CachedExtensionStore
// ---------------------------------------------------------------------------

use crate::storage::ExtensionStore;

/// Caching wrapper around any `ExtensionStore`.
///
/// Caches `get_extension_by_number()` by (tenant_id, number) key since
/// that's the hot-path lookup during call routing.
pub struct CachedExtensionStore {
    inner: Arc<dyn ExtensionStore>,
    // Key: "tenant_id:number"
    cache: DashMap<String, CacheEntry<Option<crate::Extension>>>,
    ttl: Duration,
}

impl CachedExtensionStore {
    pub fn new(inner: Arc<dyn ExtensionStore>, ttl: Duration) -> Self {
        Self {
            inner,
            cache: DashMap::new(),
            ttl,
        }
    }

    fn cache_key(tenant_id: &crate::TenantId, number: &str) -> String {
        format!("{}:{}", tenant_id, number)
    }

    /// Evict a specific extension from the cache by ID.
    /// Since we cache by (tenant, number), we clear the entire cache on ID-based eviction.
    pub fn evict_by_id(&self, _id: &crate::ExtensionId) {
        self.cache.clear();
        debug!("Cleared extension cache (evict by ID)");
    }

    /// Evict all entries.
    pub fn evict_all(&self) {
        self.cache.clear();
    }

    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

#[async_trait]
impl ExtensionStore for CachedExtensionStore {
    async fn get_extension(
        &self,
        id: &crate::ExtensionId,
    ) -> Result<Option<crate::Extension>> {
        // get_extension by ID is not the hot path — skip cache
        self.inner.get_extension(id).await
    }

    async fn get_extension_by_number(
        &self,
        tenant_id: &crate::TenantId,
        number: &str,
    ) -> Result<Option<crate::Extension>> {
        let key = Self::cache_key(tenant_id, number);

        if let Some(entry) = self.cache.get(&key) {
            if !entry.is_expired(self.ttl) {
                debug!(tenant_id = %tenant_id, number = %number, "Extension cache HIT");
                return Ok(entry.value.clone());
            }
            drop(entry);
            self.cache.remove(&key);
        }

        debug!(tenant_id = %tenant_id, number = %number, "Extension cache MISS");
        let result = self.inner.get_extension_by_number(tenant_id, number).await?;

        self.cache.insert(key, CacheEntry::new(result.clone()));
        Ok(result)
    }

    async fn list_extensions(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::Extension>> {
        self.inner.list_extensions(tenant_id).await
    }

    async fn put_extension(&self, extension: crate::Extension) -> Result<()> {
        self.inner.put_extension(extension).await?;
        // Evict all — we don't know the old number if it changed
        self.cache.clear();
        Ok(())
    }

    async fn delete_extension(&self, id: &crate::ExtensionId) -> Result<()> {
        self.inner.delete_extension(id).await?;
        self.cache.clear();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// CachedDeviceStore
// ---------------------------------------------------------------------------

use crate::storage::DeviceStore;

/// Caching wrapper around any `DeviceStore`.
///
/// Caches `get_device_by_sip_uri()` since that's the hot-path lookup
/// during call routing (mapping SIP URI to device config).
pub struct CachedDeviceStore {
    inner: Arc<dyn DeviceStore>,
    // Key: sip_uri
    cache: DashMap<String, CacheEntry<Option<crate::Device>>>,
    ttl: Duration,
}

impl CachedDeviceStore {
    pub fn new(inner: Arc<dyn DeviceStore>, ttl: Duration) -> Self {
        Self {
            inner,
            cache: DashMap::new(),
            ttl,
        }
    }

    /// Evict a specific device from the cache by SIP URI.
    pub fn evict(&self, sip_uri: &str) {
        self.cache.remove(sip_uri);
        debug!(sip_uri = %sip_uri, "Evicted device cache entry");
    }

    /// Evict all entries.
    pub fn evict_all(&self) {
        self.cache.clear();
    }

    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

#[async_trait]
impl DeviceStore for CachedDeviceStore {
    async fn get_device(
        &self,
        id: &crate::DeviceId,
    ) -> Result<Option<crate::Device>> {
        // get by ID is not the hot path
        self.inner.get_device(id).await
    }

    async fn get_device_by_sip_uri(
        &self,
        sip_uri: &str,
    ) -> Result<Option<crate::Device>> {
        if let Some(entry) = self.cache.get(sip_uri) {
            if !entry.is_expired(self.ttl) {
                debug!(sip_uri = %sip_uri, "Device cache HIT");
                return Ok(entry.value.clone());
            }
            drop(entry);
            self.cache.remove(sip_uri);
        }

        debug!(sip_uri = %sip_uri, "Device cache MISS");
        let result = self.inner.get_device_by_sip_uri(sip_uri).await?;

        self.cache
            .insert(sip_uri.to_string(), CacheEntry::new(result.clone()));
        Ok(result)
    }

    async fn list_devices(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::Device>> {
        self.inner.list_devices(tenant_id).await
    }

    async fn list_devices_by_user(
        &self,
        user_id: &crate::UserId,
    ) -> Result<Vec<crate::Device>> {
        self.inner.list_devices_by_user(user_id).await
    }

    async fn put_device(&self, device: crate::Device) -> Result<()> {
        self.inner.put_device(device).await?;
        self.cache.clear();
        Ok(())
    }

    async fn delete_device(&self, id: &crate::DeviceId) -> Result<()> {
        self.inner.delete_device(id).await?;
        self.cache.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::User;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock credential store that counts calls.
    struct MockCredentialStore {
        call_count: AtomicUsize,
    }

    impl MockCredentialStore {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl CredentialStore for MockCredentialStore {
        async fn get_user(&self, _username: &str) -> Result<Option<User>> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        }

        async fn put_user(&self, _user: User) -> Result<()> {
            Ok(())
        }

        async fn delete_user(&self, _username: &str) -> Result<()> {
            Ok(())
        }

        async fn list_users(&self) -> Result<Vec<User>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn test_cached_credential_store_hit() {
        let mock = Arc::new(MockCredentialStore::new());
        let cached = CachedCredentialStore::new(mock.clone(), Duration::from_secs(60));

        // First call — miss
        let _ = cached.get_user("alice").await.unwrap();
        assert_eq!(mock.calls(), 1);

        // Second call — hit (should not increment inner calls)
        let _ = cached.get_user("alice").await.unwrap();
        assert_eq!(mock.calls(), 1);
    }

    #[tokio::test]
    async fn test_cached_credential_store_evict() {
        let mock = Arc::new(MockCredentialStore::new());
        let cached = CachedCredentialStore::new(mock.clone(), Duration::from_secs(60));

        let _ = cached.get_user("alice").await.unwrap();
        assert_eq!(mock.calls(), 1);

        // Evict
        cached.evict("alice");

        // Should miss again
        let _ = cached.get_user("alice").await.unwrap();
        assert_eq!(mock.calls(), 2);
    }

    #[tokio::test]
    async fn test_cached_credential_store_ttl_expiry() {
        let mock = Arc::new(MockCredentialStore::new());
        // Very short TTL
        let cached = CachedCredentialStore::new(mock.clone(), Duration::from_millis(10));

        let _ = cached.get_user("alice").await.unwrap();
        assert_eq!(mock.calls(), 1);

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Should miss again
        let _ = cached.get_user("alice").await.unwrap();
        assert_eq!(mock.calls(), 2);
    }
}
