//! Storage backend initialization for fcp-admin.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::config::StorageConfig;

/// Storage initialization result with both DynStorage and ConfigStore.
pub struct StorageResult {
    pub dyn_storage: Arc<DynStorage>,
    pub config_store: Arc<dyn fcp_core::config_store::ConfigStore>,
}

/// Initialize PostgreSQL storage backend from configuration.
pub async fn initialize_storage(config: &StorageConfig) -> Result<StorageResult> {
    let pg_config = PostgresConfig {
        connection_string: config.database_url.clone(),
        max_connections: config.max_connections,
        connection_timeout: Duration::from_secs(30),
    };

    let storage = PostgresStorage::new(pg_config)
        .await
        .context("Failed to initialize PostgreSQL storage")?;

    let storage = Arc::new(storage);

    info!("PostgreSQL storage initialized successfully");
    Ok(StorageResult {
        dyn_storage: Arc::new(DynStorage::from_all_in_one(storage.clone())),
        config_store: storage,
    })
}
