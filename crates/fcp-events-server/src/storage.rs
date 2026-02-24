//! Storage backend initialization for fcp-events-server.

use anyhow::{Context, Result};
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use std::sync::Arc;
use tracing::info;

/// Storage initialization result with both PostgresStorage and ConfigStore.
pub struct StorageResult {
    pub pg_storage: Arc<PostgresStorage>,
    pub config_store: Arc<dyn fcp_core::config_store::ConfigStore>,
}

/// Initialize PostgreSQL storage backend from a database URL.
pub async fn initialize_storage(database_url: &str) -> Result<StorageResult> {
    let pg_config = PostgresConfig::new(database_url);

    let storage = PostgresStorage::new(pg_config)
        .await
        .context("Failed to initialize PostgreSQL storage")?;

    let storage = Arc::new(storage);

    info!("PostgreSQL storage initialized successfully");
    Ok(StorageResult {
        pg_storage: storage.clone(),
        config_store: storage,
    })
}
