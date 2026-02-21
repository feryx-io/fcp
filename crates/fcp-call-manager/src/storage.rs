//! Storage backend initialization for fcp-call-manager.
//!
//! Supports SQLite, PostgreSQL, MySQL, and MongoDB through conditional compilation.
//! When `storage.redis_url` and `storage.registration_backend = "redis"` are configured,
//! registrations are stored in Redis while other data remains in the primary backend.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use std::sync::Arc;
use tracing::info;

use crate::config::StorageSection;

/// Initialize a storage backend from configuration.
///
/// Returns the primary DynStorage. For Redis integration, use `initialize_redis()`
/// separately and then call `apply_redis_registration_store()`.
pub async fn initialize_storage(config: &StorageSection) -> Result<Arc<DynStorage>> {
    info!(
        backend = %config.backend,
        connection_string = %mask_connection_string(&config.connection_string),
        "Initializing storage backend"
    );

    match config.backend.as_str() {
        "sqlite" => init_sqlite(config).await,
        "postgres" | "postgresql" => init_postgres(config).await,
        "mysql" => init_mysql(config).await,
        "mongodb" | "mongo" => init_mongodb(config).await,
        other => anyhow::bail!("Unknown storage backend: {}", other),
    }
}

/// Initialize Redis storage if configured.
///
/// Returns None if `redis_url` is not set.
#[cfg(feature = "storage-redis")]
pub async fn initialize_redis(
    config: &StorageSection,
) -> Result<Option<fcp_storage_redis::RedisStorage>> {
    if let Some(ref redis_url) = config.redis_url {
        let redis_config = fcp_storage_redis::RedisConfig {
            url: redis_url.clone(),
            key_prefix: "fcp:".to_string(),
        };
        let storage = fcp_storage_redis::RedisStorage::new(redis_config)
            .await
            .context("Failed to initialize Redis storage")?;
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

async fn init_sqlite(config: &StorageSection) -> Result<Arc<DynStorage>> {
    #[cfg(feature = "storage-sqlite")]
    {
        use fcp_storage_sqlite::{SqliteConfig, SqliteStorage};

        let db_path = config
            .connection_string
            .strip_prefix("sqlite://")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid SQLite connection string format. Expected: sqlite:///absolute/path or sqlite://relative/path"
                )
            })?;

        let sqlite_config = SqliteConfig::new(std::path::PathBuf::from(db_path));
        let storage = SqliteStorage::new(sqlite_config)
            .await
            .context("Failed to initialize SQLite storage")?;

        info!("SQLite storage initialized successfully");
        Ok(Arc::new(DynStorage::from_all_in_one(Arc::new(storage))))
    }

    #[cfg(not(feature = "storage-sqlite"))]
    {
        let _ = config;
        anyhow::bail!(
            "SQLite storage backend not available. Compile with --features storage-sqlite"
        )
    }
}

async fn init_postgres(config: &StorageSection) -> Result<Arc<DynStorage>> {
    #[cfg(feature = "storage-postgres")]
    {
        use fcp_storage_postgres::{PostgresConfig, PostgresStorage};

        let postgres_config = PostgresConfig {
            connection_string: config.connection_string.clone(),
            max_connections: config.max_connections.unwrap_or(20),
            connection_timeout: std::time::Duration::from_secs(30),
        };

        let storage = PostgresStorage::new(postgres_config)
            .await
            .context("Failed to initialize PostgreSQL storage")?;

        info!("PostgreSQL storage initialized successfully");
        Ok(Arc::new(DynStorage::from_all_in_one(Arc::new(storage))))
    }

    #[cfg(not(feature = "storage-postgres"))]
    {
        let _ = config;
        anyhow::bail!(
            "PostgreSQL storage backend not available. Compile with --features storage-postgres"
        )
    }
}

async fn init_mysql(config: &StorageSection) -> Result<Arc<DynStorage>> {
    #[cfg(feature = "storage-mysql")]
    {
        use fcp_storage_mysql::{MysqlConfig, MysqlStorage};

        let mysql_config = MysqlConfig {
            connection_string: config.connection_string.clone(),
            max_connections: config.max_connections.unwrap_or(20),
            connection_timeout: std::time::Duration::from_secs(30),
        };

        let storage = MysqlStorage::new(mysql_config)
            .await
            .context("Failed to initialize MySQL storage")?;

        info!("MySQL storage initialized successfully");
        Ok(Arc::new(DynStorage::from_all_in_one(Arc::new(storage))))
    }

    #[cfg(not(feature = "storage-mysql"))]
    {
        let _ = config;
        anyhow::bail!("MySQL storage backend not available. Compile with --features storage-mysql")
    }
}

async fn init_mongodb(config: &StorageSection) -> Result<Arc<DynStorage>> {
    #[cfg(feature = "storage-mongodb")]
    {
        use fcp_storage_mongodb::{MongodbConfig, MongodbStorage};

        let database_name = config
            .connection_string
            .rsplit('/')
            .next()
            .and_then(|s| {
                let name = s.split('?').next()?;
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                }
            })
            .unwrap_or_else(|| "fcp".to_string());

        let mongodb_config = MongodbConfig {
            connection_string: config.connection_string.clone(),
            database_name,
            timeout: std::time::Duration::from_secs(30),
        };

        let storage = MongodbStorage::new(mongodb_config)
            .await
            .context("Failed to initialize MongoDB storage")?;

        info!("MongoDB storage initialized successfully");
        Ok(Arc::new(DynStorage::from_all_in_one(Arc::new(storage))))
    }

    #[cfg(not(feature = "storage-mongodb"))]
    {
        let _ = config;
        anyhow::bail!(
            "MongoDB storage backend not available. Compile with --features storage-mongodb"
        )
    }
}

/// Wrap the hot-path stores (credentials, extensions, devices) in DashMap caches.
///
/// Returns the new DynStorage with cached stores, plus handles to the three
/// cache wrappers so callers can evict entries on ConfigChanged events.
pub fn apply_cached_stores(
    dyn_storage: Arc<DynStorage>,
    cache_ttl: std::time::Duration,
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

/// Mask sensitive information in connection strings for logging.
fn mask_connection_string(conn_str: &str) -> String {
    if conn_str.starts_with("sqlite:") {
        return conn_str.to_string();
    }

    if let Some(at_pos) = conn_str.rfind('@') {
        if let Some(scheme_end) = conn_str.find("://") {
            let scheme = &conn_str[..scheme_end + 3];
            let after_at = &conn_str[at_pos..];
            let middle_part = &conn_str[scheme_end + 3..at_pos];
            if let Some(colon_pos) = middle_part.find(':') {
                let username = &middle_part[..colon_pos];
                return format!("{}{}:***{}", scheme, username, after_at);
            }
        }
    }

    if let Some(scheme_end) = conn_str.find("://") {
        format!("{}://***", &conn_str[..scheme_end])
    } else {
        "***".to_string()
    }
}
