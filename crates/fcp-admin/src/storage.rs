//! Storage backend initialization for fcp-admin.
//!
//! This module handles dynamic initialization of storage backends based on configuration.
//! It supports SQLite, PostgreSQL, MySQL, and MongoDB backends through conditional compilation.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use std::sync::Arc;
use tracing::info;

use crate::config::StorageConfig;

/// Initialize a storage backend from configuration.
///
/// This function matches on the configured backend type and initializes the appropriate
/// storage implementation. The backend is wrapped in a DynStorage trait object to allow
/// runtime polymorphism.
///
/// # Conditional Compilation
///
/// Storage backends are conditionally compiled based on cargo features:
/// - `storage-sqlite`: SQLite support (default)
/// - `storage-postgres`: PostgreSQL support
/// - `storage-mysql`: MySQL support
/// - `storage-mongodb`: MongoDB support
///
/// If a backend is configured but not compiled in, this function returns an error.
pub async fn initialize_storage(config: &StorageConfig) -> Result<Arc<DynStorage>> {
    info!(
        backend = %config.storage_type,
        "Initializing storage backend for admin API"
    );

    match config.storage_type.as_str() {
        "sqlite" => {
            #[cfg(feature = "storage-sqlite")]
            {
                use fcp_storage_sqlite::{SqliteConfig, SqliteStorage};

                let sqlite_config =
                    SqliteConfig::new(std::path::PathBuf::from(&config.sqlite_path));
                let storage = SqliteStorage::new(sqlite_config)
                    .await
                    .context("Failed to initialize SQLite storage")?;

                info!(path = %config.sqlite_path, "SQLite storage initialized successfully");
                Ok(Arc::new(DynStorage::from_all_in_one(Arc::new(storage))))
            }

            #[cfg(not(feature = "storage-sqlite"))]
            {
                anyhow::bail!(
                    "SQLite storage backend not available. Compile with --features storage-sqlite"
                )
            }
        }

        "postgres" => {
            #[cfg(feature = "storage-postgres")]
            {
                use fcp_storage_postgres::{PostgresConfig, PostgresStorage};

                let postgres_url = config
                    .postgres_url
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("PostgreSQL URL not configured"))?;

                let postgres_config = PostgresConfig {
                    connection_string: postgres_url.clone(),
                    max_connections: config.max_connections,
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
                anyhow::bail!(
                    "PostgreSQL storage backend not available. Compile with --features storage-postgres"
                )
            }
        }

        "mysql" => {
            #[cfg(feature = "storage-mysql")]
            {
                use fcp_storage_mysql::{MysqlConfig, MysqlStorage};

                let mysql_url = config
                    .mysql_url
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("MySQL URL not configured"))?;

                let mysql_config = MysqlConfig {
                    connection_string: mysql_url.clone(),
                    max_connections: config.max_connections,
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
                anyhow::bail!(
                    "MySQL storage backend not available. Compile with --features storage-mysql"
                )
            }
        }

        "mongodb" => {
            #[cfg(feature = "storage-mongodb")]
            {
                use fcp_storage_mongodb::{MongodbConfig, MongodbStorage};

                let mongodb_url = config
                    .mongodb_url
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("MongoDB URL not configured"))?;

                // Extract database name from connection string (format: mongodb://host:port/dbname)
                let database_name = mongodb_url
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
                    .unwrap_or_else(|| "fcp_admin".to_string());

                let mongodb_config = MongodbConfig {
                    connection_string: mongodb_url.clone(),
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
                anyhow::bail!(
                    "MongoDB storage backend not available. Compile with --features storage-mongodb"
                )
            }
        }

        other => anyhow::bail!("Unknown storage backend type: {}", other),
    }
}
