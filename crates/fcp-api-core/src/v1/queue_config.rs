//! Queue configuration CRUD API endpoints.

use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::storage::QueueConfigStore;
use fcp_core::{QueueConfigId, QueueConfigRecord};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use tracing::info;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// Queue config response.
#[derive(Debug, Serialize)]
pub struct QueueConfigResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub extension: String,
    pub strategy: String,
    pub max_wait_time_secs: i32,
    pub max_queue_depth: i32,
    pub overflow_destination: Option<String>,
    pub moh_class: Option<String>,
    pub announce_position: bool,
    pub announce_interval_secs: i32,
    pub retry_on_no_answer: bool,
    pub agents: Vec<String>,
    pub priority: i32,
    pub required_skills: Vec<String>,
    pub record_calls: bool,
    pub callback_enabled: bool,
    pub timezone: String,
    pub enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<QueueConfigRecord> for QueueConfigResponse {
    fn from(c: QueueConfigRecord) -> Self {
        Self {
            id: c.id.to_string(),
            tenant_id: c.tenant_id.to_string(),
            name: c.name,
            extension: c.extension,
            strategy: c.strategy,
            max_wait_time_secs: c.max_wait_time_secs,
            max_queue_depth: c.max_queue_depth,
            overflow_destination: c.overflow_destination,
            moh_class: c.moh_class,
            announce_position: c.announce_position,
            announce_interval_secs: c.announce_interval_secs,
            retry_on_no_answer: c.retry_on_no_answer,
            agents: c.agents,
            priority: c.priority,
            required_skills: c.required_skills,
            record_calls: c.record_calls,
            callback_enabled: c.callback_enabled,
            timezone: c.timezone,
            enabled: c.enabled,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

/// Create queue config request.
#[derive(Debug, Deserialize)]
pub struct CreateQueueConfigRequest {
    pub name: String,
    pub extension: String,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub max_wait_time_secs: Option<i32>,
    #[serde(default)]
    pub max_queue_depth: Option<i32>,
    pub overflow_destination: Option<String>,
    pub moh_class: Option<String>,
    #[serde(default)]
    pub announce_position: Option<bool>,
    #[serde(default)]
    pub announce_interval_secs: Option<i32>,
    #[serde(default)]
    pub retry_on_no_answer: Option<bool>,
    #[serde(default)]
    pub agents: Option<Vec<String>>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub required_skills: Option<Vec<String>>,
    #[serde(default)]
    pub record_calls: Option<bool>,
    #[serde(default)]
    pub callback_enabled: Option<bool>,
    pub timezone: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Update queue config request.
#[derive(Debug, Deserialize)]
pub struct UpdateQueueConfigRequest {
    pub name: Option<String>,
    pub extension: Option<String>,
    pub strategy: Option<String>,
    pub max_wait_time_secs: Option<i32>,
    pub max_queue_depth: Option<i32>,
    pub overflow_destination: Option<String>,
    pub moh_class: Option<String>,
    pub announce_position: Option<bool>,
    pub announce_interval_secs: Option<i32>,
    pub retry_on_no_answer: Option<bool>,
    pub agents: Option<Vec<String>>,
    pub priority: Option<i32>,
    pub required_skills: Option<Vec<String>>,
    pub record_calls: Option<bool>,
    pub callback_enabled: Option<bool>,
    pub timezone: Option<String>,
    pub enabled: Option<bool>,
}

/// List queue configs.
pub async fn list_queue_configs(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = tenant.0;

    let configs = storage
        .list_queue_configs(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list queue configs: {}", e)))?;

    let response: Vec<QueueConfigResponse> = configs
        .into_iter()
        .map(QueueConfigResponse::from)
        .collect();
    Ok(Json(response))
}

/// Get a single queue config.
pub async fn get_queue_config(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let config_id = QueueConfigId::from_string(id);

    let config = storage
        .get_queue_config(&config_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get queue config: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Queue config not found".to_string()))?;

    Ok(Json(QueueConfigResponse::from(config)))
}

/// Create a new queue config.
pub async fn create_queue_config(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateQueueConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = tenant.0;

    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("Queue name is required".to_string()));
    }
    if req.extension.trim().is_empty() {
        return Err(ApiError::BadRequest("Queue extension is required".to_string()));
    }

    let mut config = QueueConfigRecord::new(req.name, req.extension, tenant_id);
    if let Some(strategy) = req.strategy { config.strategy = strategy; }
    if let Some(v) = req.max_wait_time_secs { config.max_wait_time_secs = v; }
    if let Some(v) = req.max_queue_depth { config.max_queue_depth = v; }
    config.overflow_destination = req.overflow_destination;
    config.moh_class = req.moh_class;
    if let Some(v) = req.announce_position { config.announce_position = v; }
    if let Some(v) = req.announce_interval_secs { config.announce_interval_secs = v; }
    if let Some(v) = req.retry_on_no_answer { config.retry_on_no_answer = v; }
    if let Some(agents) = req.agents { config.agents = agents; }
    if let Some(v) = req.priority { config.priority = v; }
    if let Some(skills) = req.required_skills { config.required_skills = skills; }
    if let Some(v) = req.record_calls { config.record_calls = v; }
    if let Some(v) = req.callback_enabled { config.callback_enabled = v; }
    if let Some(tz) = req.timezone { config.timezone = tz; }
    if let Some(v) = req.enabled { config.enabled = v; }

    info!(queue_name = %config.name, queue_id = %config.id, "Creating queue config");

    storage
        .put_queue_config(config.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create queue config: {}", e)))?;

    Ok((StatusCode::CREATED, Json(QueueConfigResponse::from(config))))
}

/// Update an existing queue config.
pub async fn update_queue_config(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateQueueConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let config_id = QueueConfigId::from_string(id);

    let mut config = storage
        .get_queue_config(&config_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get queue config: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Queue config not found".to_string()))?;

    if let Some(name) = req.name { config.name = name; }
    if let Some(ext) = req.extension { config.extension = ext; }
    if let Some(strategy) = req.strategy { config.strategy = strategy; }
    if let Some(v) = req.max_wait_time_secs { config.max_wait_time_secs = v; }
    if let Some(v) = req.max_queue_depth { config.max_queue_depth = v; }
    if let Some(dest) = req.overflow_destination { config.overflow_destination = Some(dest); }
    if let Some(moh) = req.moh_class { config.moh_class = Some(moh); }
    if let Some(v) = req.announce_position { config.announce_position = v; }
    if let Some(v) = req.announce_interval_secs { config.announce_interval_secs = v; }
    if let Some(v) = req.retry_on_no_answer { config.retry_on_no_answer = v; }
    if let Some(agents) = req.agents { config.agents = agents; }
    if let Some(v) = req.priority { config.priority = v; }
    if let Some(skills) = req.required_skills { config.required_skills = skills; }
    if let Some(v) = req.record_calls { config.record_calls = v; }
    if let Some(v) = req.callback_enabled { config.callback_enabled = v; }
    if let Some(tz) = req.timezone { config.timezone = tz; }
    if let Some(v) = req.enabled { config.enabled = v; }

    config.updated_at = SystemTime::now();

    info!(queue_name = %config.name, queue_id = %config.id, "Updating queue config");

    storage
        .put_queue_config(config.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update queue config: {}", e)))?;

    Ok(Json(QueueConfigResponse::from(config)))
}

/// Delete a queue config.
pub async fn delete_queue_config(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let config_id = QueueConfigId::from_string(id);

    // Verify it exists
    let config = storage
        .get_queue_config(&config_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get queue config: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Queue config not found".to_string()))?;

    info!(queue_name = %config.name, queue_id = %config.id, "Deleting queue config");

    storage
        .delete_queue_config(&config_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete queue config: {}", e)))?;

    Ok(StatusCode::NO_CONTENT)
}
