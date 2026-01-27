//! Storage backend initialization for fcp-registrar.
//!
//! Supports SQLite, PostgreSQL, MySQL, and MongoDB through conditional compilation.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use std::sync::Arc;
use tracing::info;

use crate::config::StorageSection;

/// Initialize a storage backend from configuration.
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

async fn init_sqlite(config: &StorageSection) -> Result<Arc<DynStorage>> {
    #[cfg(feature = "storage-sqlite")]
    {
        use fcp_storage_sqlite::{SqliteConfig, SqliteStorage};

        let db_path = config
            .connection_string
            .strip_prefix("sqlite:///")
            .or_else(|| config.connection_string.strip_prefix("sqlite://"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid SQLite connection string format. Expected: sqlite://path/to/db.sqlite"
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
        anyhow::bail!("SQLite storage backend not available. Compile with --features storage-sqlite")
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
        anyhow::bail!(
            "MySQL storage backend not available. Compile with --features storage-mysql"
        )
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
