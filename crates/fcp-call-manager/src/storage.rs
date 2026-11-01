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

/// Storage initialization result with DynStorage, ConfigStore, and FlowStore.
pub struct StorageResult {
    /// What `/status` says about the read replica.
    pub replica: fcp_storage_postgres::ReplicaHealth,
    /// Whether the primary answers, probed every second: what the SIP
    /// path reads before work that needs the store (`docs/DEEP_TESTING.md` D5).
    pub primary: Arc<fcp_storage_postgres::primary_health::PrimaryHealth>,
    pub dyn_storage: Arc<DynStorage>,
    pub config_store: Arc<dyn fcp_core::config_store::ConfigStore>,
    pub flow_store: Arc<dyn fcp_flow::FlowStore>,
}

/// Initialize PostgreSQL storage backend from configuration.
pub async fn initialize_storage(config: &StorageSection) -> Result<StorageResult> {
    let pg_config = PostgresConfig {
        connection_string: config.database_url.clone(),
        read_replica_url: config.read_replica_url.clone(),
        max_replica_lag: Duration::from_secs(config.max_replica_lag_secs),
        max_connections: config.max_connections,
        // A connection the call path waits for longer than this is one a
        // phone has given up on (`docs/DEEP_TESTING.md` D5).
        connection_timeout: Duration::from_secs(config.acquire_timeout_secs.max(1)),
        encryption: None,
    };

    let storage = PostgresStorage::new(pg_config)
        .await
        .context("Failed to initialize PostgreSQL storage")?;

    let storage = Arc::new(storage);

    // Create SIP trace store backed by the same PostgreSQL pool
    let sip_trace_store = Arc::new(fcp_storage_postgres::sip_trace::PostgresSipTraceStore::new(
        storage.pool().clone(),
    ));

    let prompt_store = Arc::new(fcp_storage_postgres::prompt::PostgresPromptStore::new(
        storage.pool().clone(),
    ));

    let flow_trace_store = Arc::new(
        fcp_storage_postgres::flow_trace::PostgresFlowTraceStore::new(storage.pool().clone()),
    );

    let emergency_store = Arc::new(
        fcp_storage_postgres::emergency::PostgresEmergencyStore::new(storage.pool().clone()),
    );
    let compliance_store = Arc::new(
        fcp_storage_postgres::compliance::PostgresComplianceStore::new(storage.pool().clone()),
    );
    let dyn_st = DynStorage::from_all_in_one(storage.clone())
        .with_emergency_store(emergency_store as Arc<dyn fcp_core::e911::EmergencyStore>)
        .with_compliance_store(compliance_store as Arc<dyn fcp_core::compliance::ComplianceStore>)
        .with_call_recording_store(Arc::new(
            fcp_storage_postgres::recording::PostgresRecordingStore::new(storage.pool().clone()),
        )
            as Arc<dyn fcp_core::recording::CallRecordingStore>)
        .with_encryption_store(storage.clone() as Arc<dyn fcp_core::encryption::EncryptionStore>)
        .with_sip_trace_store(sip_trace_store as Arc<dyn fcp_core::sip_trace::SipTraceStore>)
        .with_prompt_store(prompt_store as Arc<dyn fcp_core::prompt::PromptStore>)
        .with_flow_trace_store(flow_trace_store as Arc<dyn fcp_core::flow_trace::FlowTraceStore>)
        // A queue's agents may be a group's members (docs/USERS.md §5).
        .with_group_store(storage.clone() as Arc<dyn fcp_core::group::GroupStore>)
        // A registration is matched to the device that fetched from its
        // address (docs/PROVISIONING.md §8).
        .with_provisioning_store(
            storage.clone() as Arc<dyn fcp_core::provisioning::ProvisioningStore>
        )
        // The lots a call parks in (docs/PARKING.md).
        .with_parking_lot_store(storage.clone() as Arc<dyn fcp_core::parking::ParkingLotStore>)
        .with_line_label_store(storage.clone() as Arc<dyn fcp_core::line_label::LineLabelStore>)
        // A watched person's chosen status, read when nobody has announced
        // it since the node started (docs/PRESENCE.md §3).
        .with_presence_status_store(
            storage.clone() as Arc<dyn fcp_core::presence_status::PresenceStatusStore>
        );

    info!(
        "PostgreSQL storage initialized successfully (with SIP trace + prompt + flow trace stores)"
    );
    Ok(StorageResult {
        replica: fcp_storage_postgres::ReplicaHealth::of(storage.clone()),
        primary: fcp_storage_postgres::primary_health::PrimaryHealth::watch(storage.pool().clone()),
        dyn_storage: Arc::new(dyn_st),
        flow_store: storage.clone() as Arc<dyn fcp_flow::FlowStore>,
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
    let new_storage =
        inner
            .with_credential_store(Arc::clone(&cached_creds) as Arc<dyn fcp_core::CredentialStore>)
            .with_extension_store(
                Arc::clone(&cached_exts) as Arc<dyn fcp_core::storage::ExtensionStore>
            )
            .with_device_store(Arc::clone(&cached_devs) as Arc<dyn fcp_core::storage::DeviceStore>);

    info!(
        "Hot-path stores wrapped with in-memory cache (TTL={}s)",
        cache_ttl.as_secs()
    );
    (
        Arc::new(new_storage),
        cached_creds,
        cached_exts,
        cached_devs,
    )
}
