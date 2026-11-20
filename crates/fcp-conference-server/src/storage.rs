//! Storage backend initialization for fcp-conference-server.

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

    // Create prompt store from the same pool
    let prompt_store: Arc<dyn fcp_core::prompt::PromptStore> = Arc::new(
        fcp_storage_postgres::prompt::PostgresPromptStore::new(storage.pool().clone()),
    );

    // The recording registry: a recording is a row first and a file
    // second (docs/VIDEO_CONFERENCING.md §11).
    let recording_store: Arc<dyn fcp_core::recording::RecordingStore> = Arc::new(
        fcp_storage_postgres::recording::PostgresRecordingStore::new(storage.pool().clone()),
    );

    // The tenant's retention and recording-consent policy.
    let compliance_store: Arc<dyn fcp_core::compliance::ComplianceStore> = Arc::new(
        fcp_storage_postgres::compliance::PostgresComplianceStore::new(storage.pool().clone()),
    );

    // Questions asked in a room (`docs/WEBINARS.md` §2.5).
    let question_store: Arc<dyn fcp_core::qa::QuestionStore> = Arc::new(
        fcp_storage_postgres::questions::PostgresQuestionStore::new(storage.pool().clone()),
    );

    // Chat in a room (`docs/WEBINARS.md` §2.5).
    let chat_store: Arc<dyn fcp_core::chat::ChatStore> = Arc::new(
        fcp_storage_postgres::chat::PostgresChatStore::new(storage.pool().clone()),
    );

    // A room's sittings (`docs/WEBINARS.md` §2.6).
    let sitting_store: Arc<dyn fcp_core::webinar::SittingStore> = Arc::new(
        fcp_storage_postgres::sittings::PostgresSittingStore::new(storage.pool().clone()),
    );

    info!("PostgreSQL storage initialized successfully");
    Ok(StorageResult {
        replica: fcp_storage_postgres::ReplicaHealth::of(storage.clone()),
        dyn_storage: Arc::new(
            DynStorage::from_all_in_one(storage.clone())
                .with_prompt_store(prompt_store)
                .with_recording_store(recording_store)
                .with_compliance_store(compliance_store)
                .with_question_store(question_store)
                .with_chat_store(chat_store)
                .with_sitting_store(sitting_store),
        ),
        config_store: storage,
    })
}
