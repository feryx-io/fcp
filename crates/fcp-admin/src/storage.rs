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
    pub flow_store: Arc<dyn fcp_flow::storage::FlowStore>,
}

/// Initialize PostgreSQL storage backend from configuration.
pub async fn initialize_storage(config: &StorageConfig) -> Result<StorageResult> {
    let pg_config = PostgresConfig {
        connection_string: config.database_url.clone(),
        max_connections: config.max_connections,
        connection_timeout: Duration::from_secs(30),
        encryption: None,
    };

    let storage = PostgresStorage::new(pg_config)
        .await
        .context("Failed to initialize PostgreSQL storage")?;

    let storage = Arc::new(storage);

    // Create SIP trace store backed by the same PostgreSQL pool
    let sip_trace_store = Arc::new(fcp_storage_postgres::sip_trace::PostgresSipTraceStore::new(
        storage.pool().clone(),
    ));

    // Create API token store backed by the same PostgreSQL pool
    let api_token_store = Arc::new(fcp_storage_postgres::api_token::PostgresApiTokenStore::new(
        storage.pool().clone(),
    ));

    // Create prompt store backed by the same PostgreSQL pool
    let prompt_store = Arc::new(fcp_storage_postgres::prompt::PostgresPromptStore::new(
        storage.pool().clone(),
    ));

    // Create flow trace store backed by the same PostgreSQL pool
    let flow_trace_store = Arc::new(
        fcp_storage_postgres::flow_trace::PostgresFlowTraceStore::new(storage.pool().clone()),
    );

    let emergency_store = Arc::new(
        fcp_storage_postgres::emergency::PostgresEmergencyStore::new(storage.pool().clone()),
    );
    let dyn_st = DynStorage::from_all_in_one(storage.clone())
        .with_emergency_store(emergency_store as Arc<dyn fcp_core::e911::EmergencyStore>)
        .with_encryption_store(storage.clone() as Arc<dyn fcp_core::encryption::EncryptionStore>)
        .with_sip_trace_store(sip_trace_store as Arc<dyn fcp_core::sip_trace::SipTraceStore>)
        .with_api_token_store(api_token_store as Arc<dyn fcp_core::storage::ApiTokenStore>)
        .with_prompt_store(prompt_store as Arc<dyn fcp_core::prompt::PromptStore>)
        .with_flow_trace_store(flow_trace_store as Arc<dyn fcp_core::flow_trace::FlowTraceStore>)
        .with_config_template_store(
            storage.clone() as Arc<dyn fcp_core::storage::ConfigTemplateStore>
        );

    info!("PostgreSQL storage initialized successfully (with SIP trace, API token, prompt, flow trace, and config template stores)");
    Ok(StorageResult {
        dyn_storage: Arc::new(dyn_st),
        flow_store: storage.clone() as Arc<dyn fcp_flow::storage::FlowStore>,
        config_store: storage,
    })
}
