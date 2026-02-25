//! Storage backend initialization for fcp-call-manager.
//!
//! Uses PostgreSQL as the primary storage backend. When `storage.redis_url`
//! and `storage.registration_backend = "redis"` are configured, registrations
//! are stored in Redis while other data remains in PostgreSQL.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::config::StorageSection;

/// Storage initialization result with both DynStorage and ConfigStore.
pub struct StorageResult {
    pub dyn_storage: Arc<DynStorage>,
    pub config_store: Arc<dyn fcp_core::config_store::ConfigStore>,
}

/// Initialize PostgreSQL storage backend from configuration.
pub async fn initialize_storage(config: &StorageSection) -> Result<StorageResult> {
    let pg_config = PostgresConfig {
        connection_string: config.database_url.clone(),
        max_connections: config.max_connections,
        connection_timeout: Duration::from_secs(30),
    };

    let storage = PostgresStorage::new(pg_config)
        .await
        .context("Failed to initialize PostgreSQL storage")?;

    let storage = Arc::new(storage);

    // Create SIP trace store backed by the same PostgreSQL pool
    let sip_trace_store = Arc::new(
        fcp_storage_postgres::sip_trace::PostgresSipTraceStore::new(storage.pool().clone()),
    );

    let dyn_st = DynStorage::from_all_in_one(storage.clone())
        .with_sip_trace_store(sip_trace_store as Arc<dyn fcp_core::sip_trace::SipTraceStore>);

    info!("PostgreSQL storage initialized successfully (with SIP trace store)");
    Ok(StorageResult {
        dyn_storage: Arc::new(dyn_st),
        config_store: storage,
    })
}

/// Initialize Redis storage if configured.
///
/// Returns None if `redis_url` is not set. When the cluster section has
/// circuit breaker settings, attaches a `RedisCircuitBreaker` to the storage.
#[cfg(feature = "storage-redis")]
pub async fn initialize_redis(
    config: &StorageSection,
    cluster_config: &crate::config::ClusterSection,
) -> Result<Option<fcp_storage_redis::RedisStorage>> {
    if let Some(ref redis_url) = config.redis_url {
        let redis_config = fcp_storage_redis::RedisConfig {
            url: redis_url.clone(),
            key_prefix: "fcp:".to_string(),
        };
        let mut storage = fcp_storage_redis::RedisStorage::new(redis_config)
            .await
            .context("Failed to initialize Redis storage")?;

        // Attach circuit breaker if cluster is enabled
        if cluster_config.enabled {
            storage = storage.with_circuit_breaker(
                cluster_config.redis_circuit_breaker_threshold,
                Duration::from_secs(cluster_config.redis_circuit_breaker_cooldown_secs),
            );
            info!(
                threshold = cluster_config.redis_circuit_breaker_threshold,
                cooldown_secs = cluster_config.redis_circuit_breaker_cooldown_secs,
                "Redis circuit breaker enabled"
            );
        }

        Ok(Some(storage))
    } else {
        Ok(None)
    }
}

/// If registration_backend is "redis", swap the registration store in DynStorage
/// to use Redis. Returns the modified DynStorage.
#[cfg(feature = "storage-redis")]
pub fn apply_redis_registration_store(
    dyn_storage: Arc<DynStorage>,
    redis_storage: &fcp_storage_redis::RedisStorage,
) -> Arc<DynStorage> {
    let redis_reg_store = Arc::new(
        fcp_storage_redis::registration::RedisRegistrationStore::new(redis_storage.clone()),
    );

    let inner = Arc::try_unwrap(dyn_storage).unwrap_or_else(|arc| (*arc).clone());
    let split = inner.with_registration_store(redis_reg_store);

    info!("Registration store swapped to Redis (split backend)");
    Arc::new(split)
}

/// Wrap the hot-path stores (credentials, extensions, devices) in DashMap caches.
///
/// Returns the new DynStorage with cached stores, plus handles to the three
/// cache wrappers so callers can evict entries on ConfigChanged events.
pub fn apply_cached_stores(
    dyn_storage: Arc<DynStorage>,
    cache_ttl: Duration,
) -> (
    Arc<DynStorage>,
    Arc<fcp_core::CachedCredentialStore>,
    Arc<fcp_core::CachedExtensionStore>,
    Arc<fcp_core::CachedDeviceStore>,
) {
    let cached_creds = Arc::new(fcp_core::CachedCredentialStore::new(
        Arc::clone(&dyn_storage) as Arc<dyn fcp_core::CredentialStore>,
        cache_ttl,
        10_000,
    ));
    let cached_exts = Arc::new(fcp_core::CachedExtensionStore::new(
        Arc::clone(&dyn_storage) as Arc<dyn fcp_core::storage::ExtensionStore>,
        cache_ttl,
        10_000,
    ));
    let cached_devs = Arc::new(fcp_core::CachedDeviceStore::new(
        Arc::clone(&dyn_storage) as Arc<dyn fcp_core::storage::DeviceStore>,
        cache_ttl,
        10_000,
    ));

    let inner = Arc::try_unwrap(dyn_storage).unwrap_or_else(|arc| (*arc).clone());
    let new_storage = inner
        .with_credential_store(Arc::clone(&cached_creds) as Arc<dyn fcp_core::CredentialStore>)
        .with_extension_store(Arc::clone(&cached_exts) as Arc<dyn fcp_core::storage::ExtensionStore>)
        .with_device_store(Arc::clone(&cached_devs) as Arc<dyn fcp_core::storage::DeviceStore>);

    info!("Hot-path stores wrapped with in-memory cache (TTL={}s)", cache_ttl.as_secs());
    (Arc::new(new_storage), cached_creds, cached_exts, cached_devs)
}
