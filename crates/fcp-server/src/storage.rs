//! Storage backend initialization for fcp-server.
//!
//! This module handles dynamic initialization of storage backends based on configuration.
//! It supports SQLite, PostgreSQL, MySQL, and MongoDB backends through conditional compilation.

use anyhow::{Context, Result};
use fcp_core::{DynStorage, StorageBackend, StorageConfig};
use std::sync::Arc;
use tracing::info;

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
        backend = ?config.backend,
        connection_string = %mask_connection_string(&config.connection_string),
        "Initializing storage backend"
    );

    match config.backend {
        StorageBackend::Sqlite => {
            #[cfg(feature = "storage-sqlite")]
            {
                use fcp_core::storage::sqlite::SqliteStorage;

                // Parse connection string (format: sqlite://path/to/db.sqlite or sqlite:///path/to/db.sqlite)
                let db_path = config.connection_string
                    .strip_prefix("sqlite://")
                    .or_else(|| config.connection_string.strip_prefix("sqlite:///"))
                    .ok_or_else(|| anyhow::anyhow!("Invalid SQLite connection string format. Expected: sqlite://path/to/db.sqlite or sqlite:///path/to/db.sqlite"))?;

                // fcp-core's SqliteStorage::new takes just a path string
                let storage = SqliteStorage::new(db_path)
                    .await
                    .context("Failed to initialize SQLite storage")?;

                info!("SQLite storage initialized successfully");
                Ok(Arc::new(DynStorage::from_all_in_one(Arc::new(storage))))
            }

            #[cfg(not(feature = "storage-sqlite"))]
            {
                anyhow::bail!(
                    "SQLite storage backend not available. Compile with --features storage-sqlite"
                )
            }
        }

        StorageBackend::Postgres => {
            #[cfg(feature = "storage-postgres")]
            {
                use fcp_storage_postgres::{PostgresConfig, PostgresStorage};

                let postgres_config = PostgresConfig {
                    connection_string: config.connection_string.clone(),
                    max_connections: config.max_connections as u32,
                    connection_timeout: config.connection_timeout,
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

        StorageBackend::Mysql => {
            #[cfg(feature = "storage-mysql")]
            {
                use fcp_storage_mysql::{MysqlConfig, MysqlStorage};

                let mysql_config = MysqlConfig {
                    connection_string: config.connection_string.clone(),
                    max_connections: config.max_connections as u32,
                    connection_timeout: config.connection_timeout,
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

        StorageBackend::Mongodb => {
            #[cfg(feature = "storage-mongodb")]
            {
                use fcp_storage_mongodb::{MongodbConfig, MongodbStorage};

                // Extract database name from connection string (format: mongodb://host:port/dbname)
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
                    timeout: config.connection_timeout,
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

        StorageBackend::Cockroach => {
            anyhow::bail!("CockroachDB storage backend not yet implemented")
        }
    }
}

/// Mask sensitive information in connection strings for logging.
///
/// This function replaces passwords in connection strings with asterisks to prevent
/// leaking credentials in logs.
fn mask_connection_string(conn_str: &str) -> String {
    if conn_str.starts_with("sqlite:") {
        // SQLite connection strings don't contain credentials
        return conn_str.to_string();
    }

    // For database URLs like: postgres://user:password@host/db
    if let Some(at_pos) = conn_str.rfind('@') {
        if let Some(scheme_end) = conn_str.find("://") {
            let scheme = &conn_str[..scheme_end + 3];
            let after_at = &conn_str[at_pos..];

            // Check if there's a colon (indicating user:password format)
            let middle_part = &conn_str[scheme_end + 3..at_pos];
            if middle_part.contains(':') {
                // Split on the first colon to get username
                if let Some(colon_pos) = middle_part.find(':') {
                    let username = &middle_part[..colon_pos];
                    return format!("{}{}:***{}", scheme, username, after_at);
                }
            }
        }
    }

    // For MongoDB URIs like: mongodb://user:password@host/db
    if conn_str.starts_with("mongodb://") || conn_str.starts_with("mongodb+srv://") {
        let scheme = if conn_str.starts_with("mongodb+srv://") {
            "mongodb+srv://"
        } else {
            "mongodb://"
        };

        if let Some(at_pos) = conn_str.rfind('@') {
            let after_scheme = &conn_str[scheme.len()..at_pos];
            let after_at = &conn_str[at_pos..];

            if after_scheme.contains(':') {
                if let Some(colon_pos) = after_scheme.find(':') {
                    let username = &after_scheme[..colon_pos];
                    return format!("{}{}:***{}", scheme, username, after_at);
                }
            }
        }
    }

    // If we can't parse it, mask the entire string after the scheme
    if let Some(scheme_end) = conn_str.find("://") {
        format!("{}://***", &conn_str[..scheme_end])
    } else {
        "***".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_sqlite_connection_string() {
        let conn_str = "sqlite://fcp.db";
        assert_eq!(mask_connection_string(conn_str), conn_str);
    }

    #[test]
    fn test_mask_postgres_connection_string() {
        let conn_str = "postgres://username:secret123@localhost:5432/fcp";
        let masked = mask_connection_string(conn_str);
        assert_eq!(masked, "postgres://username:***@localhost:5432/fcp");
        assert!(!masked.contains("secret123"));
    }

    #[test]
    fn test_mask_mysql_connection_string() {
        let conn_str = "mysql://root:mypassword@127.0.0.1:3306/fcp_db";
        let masked = mask_connection_string(conn_str);
        assert_eq!(masked, "mysql://root:***@127.0.0.1:3306/fcp_db");
        assert!(!masked.contains("mypassword"));
    }

    #[test]
    fn test_mask_mongodb_connection_string() {
        let conn_str = "mongodb://admin:supersecret@mongo.example.com:27017/fcp?authSource=admin";
        let masked = mask_connection_string(conn_str);
        assert_eq!(
            masked,
            "mongodb://admin:***@mongo.example.com:27017/fcp?authSource=admin"
        );
        assert!(!masked.contains("supersecret"));
    }

    #[test]
    fn test_mask_mongodb_srv_connection_string() {
        let conn_str = "mongodb+srv://user:pass123@cluster0.mongodb.net/fcp";
        let masked = mask_connection_string(conn_str);
        assert_eq!(masked, "mongodb+srv://user:***@cluster0.mongodb.net/fcp");
        assert!(!masked.contains("pass123"));
    }

    #[test]
    fn test_mask_unknown_format() {
        let conn_str = "unknown://something";
        let masked = mask_connection_string(conn_str);
        assert_eq!(masked, "unknown://***");
    }
}
