//! Storage backend initialization for fcp-voicemail-server.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::config::StorageSection;

/// Storage initialization result with both DynStorage and ConfigStore.
pub struct StorageResult {
    /// What `/status` says about the read replica.
    pub replica: fcp_storage_postgres::ReplicaHealth,
    pub dyn_storage: Arc<DynStorage>,
    pub config_store: Arc<dyn fcp_core::config_store::ConfigStore>,
    pub flow_store: Arc<dyn fcp_flow::storage::FlowStore>,
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

    info!("PostgreSQL storage initialized successfully");
    Ok(StorageResult {
        replica: fcp_storage_postgres::ReplicaHealth::of(storage.clone()),
        dyn_storage: Arc::new(DynStorage::from_all_in_one(storage.clone())),
        flow_store: storage.clone() as Arc<dyn fcp_flow::storage::FlowStore>,
        config_store: storage,
    })
}
