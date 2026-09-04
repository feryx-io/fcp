//! Storage backend initialization for fcp-ivr.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::config::StorageSection;

/// Storage initialization result with DynStorage, ConfigStore, and FlowStore.
pub struct StorageResult {
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
        connection_timeout: Duration::from_secs(30),
        encryption: None,
    };

    let storage = PostgresStorage::new(pg_config)
        .await
        .context("Failed to initialize PostgreSQL storage")?;

    let storage = Arc::new(storage);

    let prompt_store = Arc::new(fcp_storage_postgres::prompt::PostgresPromptStore::new(
        storage.pool().clone(),
    ));

    let dyn_st = DynStorage::from_all_in_one(storage.clone())
        .with_prompt_store(prompt_store as Arc<dyn fcp_core::prompt::PromptStore>);

    info!("PostgreSQL storage initialized successfully (with prompt store)");
    Ok(StorageResult {
        dyn_storage: Arc::new(dyn_st),
        config_store: storage.clone(),
        flow_store: storage,
    })
}
