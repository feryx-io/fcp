//! Storage backend initialization for fcp-registrar.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::config::StorageSection;

/// Initialize PostgreSQL storage backend from configuration.
pub async fn initialize_storage(config: &StorageSection) -> Result<Arc<DynStorage>> {
    let pg_config = PostgresConfig {
        connection_string: config.database_url.clone(),
        max_connections: config.max_connections,
        connection_timeout: Duration::from_secs(30),
    };

    let storage = PostgresStorage::new(pg_config)
        .await
        .context("Failed to initialize PostgreSQL storage")?;

    info!("PostgreSQL storage initialized successfully");
    Ok(Arc::new(DynStorage::from_all_in_one(Arc::new(storage))))
}
