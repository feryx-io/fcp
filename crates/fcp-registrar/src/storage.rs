//! Storage backend initialization for fcp-registrar.

use anyhow::{Context, Result};
use fcp_core::DynStorage;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::config::StorageSection;

/// Initialize PostgreSQL storage backend from configuration.
/// The stores, and a way to ask about the replica after the backend has
/// been dropped into them.
pub struct StorageResult {
    pub dyn_storage: Arc<DynStorage>,
    pub replica: fcp_storage_postgres::ReplicaHealth,
}

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

    info!("PostgreSQL storage initialized successfully");
    let storage = Arc::new(storage);
    Ok(StorageResult {
        dyn_storage: Arc::new(DynStorage::from_all_in_one(storage.clone())),
        replica: fcp_storage_postgres::ReplicaHealth::of(storage),
    })
}
