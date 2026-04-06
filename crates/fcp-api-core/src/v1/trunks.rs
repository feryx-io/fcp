//! SIP Trunk management API endpoints.

use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::storage::TrunkStore;
use fcp_core::{Trunk, TrunkDirection, TrunkId};
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

/// Trunk response.
#[derive(Debug, Serialize)]
pub struct TrunkResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub enabled: bool,
    pub direction: String,
    pub priority: i32,
    pub weight: i32,
    pub hosts: Vec<String>,
    pub transport_protocol: String,
    pub transport_port: i32,
    pub verify_tls: bool,
    pub has_credentials: bool,
    pub auth_username: Option<String>,
    pub auth_realm: Option<String>,
    pub register: bool,
    pub register_expiry: i32,
    pub max_concurrent_calls: i32,
    pub max_failures: i32,
    pub recovery_interval_secs: i32,
    pub health_check_enabled: bool,
    pub health_check_interval_secs: i32,
    pub health_status: String,
    pub consecutive_failures: u32,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<Trunk> for TrunkResponse {
    fn from(t: Trunk) -> Self {
        Self {
            id: t.id.to_string(),
            tenant_id: t.tenant_id.to_string(),
            name: t.name,
            enabled: t.enabled,
            direction: t.direction.to_string(),
            priority: t.priority,
            weight: t.weight,
            hosts: t.hosts,
            transport_protocol: t.transport_protocol,
            transport_port: t.transport_port,
            verify_tls: t.verify_tls,
            has_credentials: t.auth_username.is_some(),
            auth_username: t.auth_username,
            auth_realm: t.auth_realm,
            register: t.register,
            register_expiry: t.register_expiry,
            max_concurrent_calls: t.max_concurrent_calls,
            max_failures: t.max_failures,
            recovery_interval_secs: t.recovery_interval_secs,
            health_check_enabled: t.health_check_enabled,
            health_check_interval_secs: t.health_check_interval_secs,
            health_status: "unknown".to_string(),
            consecutive_failures: 0,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

impl TrunkResponse {
    /// Enrich with health data from the health map.
    fn with_health(mut self, health_map: &Option<crate::trunk_health::TrunkHealthMap>) -> Self {
        if let Some(map) = health_map {
            if let Some(entry) = map.get(&self.id) {
                self.health_status = entry.status.to_string();
                self.consecutive_failures = entry.consecutive_failures;
            }
        }
        self
    }
}

/// Create trunk request.
#[derive(Debug, Deserialize)]
pub struct CreateTrunkRequest {
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub weight: Option<i32>,
    #[serde(default)]
    pub hosts: Option<Vec<String>>,
    #[serde(default)]
    pub transport_protocol: Option<String>,
    #[serde(default)]
    pub transport_port: Option<i32>,
    #[serde(default)]
    pub verify_tls: Option<bool>,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    pub auth_realm: Option<String>,
    #[serde(default)]
    pub register: Option<bool>,
    #[serde(default)]
    pub register_expiry: Option<i32>,
    #[serde(default)]
    pub max_concurrent_calls: Option<i32>,
    #[serde(default)]
    pub max_failures: Option<i32>,
    #[serde(default)]
    pub recovery_interval_secs: Option<i32>,
    #[serde(default)]
    pub health_check_enabled: Option<bool>,
    #[serde(default)]
    pub health_check_interval_secs: Option<i32>,
}

/// Update trunk request.
#[derive(Debug, Deserialize)]
pub struct UpdateTrunkRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub direction: Option<String>,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
    pub hosts: Option<Vec<String>>,
    pub transport_protocol: Option<String>,
    pub transport_port: Option<i32>,
    pub verify_tls: Option<bool>,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    pub auth_realm: Option<String>,
    pub register: Option<bool>,
    pub register_expiry: Option<i32>,
    pub max_concurrent_calls: Option<i32>,
    pub max_failures: Option<i32>,
    pub recovery_interval_secs: Option<i32>,
    pub health_check_enabled: Option<bool>,
    pub health_check_interval_secs: Option<i32>,
}

/// List trunks.
pub async fn list_trunks(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = tenant.0;

    let trunks = storage
        .list_trunks(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list trunks: {}", e)))?;

    let health_map = &state.trunk_health;
    let response: Vec<TrunkResponse> = trunks
        .into_iter()
        .map(|t| TrunkResponse::from(t).with_health(health_map))
        .collect();
    Ok(Json(response))
}

/// Get a single trunk.
pub async fn get_trunk(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    let trunk = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;

    Ok(Json(TrunkResponse::from(trunk).with_health(&state.trunk_health)))
}

/// Create a new trunk.
pub async fn create_trunk(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateTrunkRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = tenant.0;

    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("Trunk name is required".to_string()));
    }

    let direction = if let Some(ref dir) = req.direction {
        dir.parse::<TrunkDirection>()
            .map_err(|e| ApiError::BadRequest(e))?
    } else {
        TrunkDirection::Upstream
    };

    let mut trunk = Trunk::new(req.name, tenant_id);
    trunk.direction = direction;
    if let Some(enabled) = req.enabled { trunk.enabled = enabled; }
    if let Some(priority) = req.priority { trunk.priority = priority; }
    if let Some(weight) = req.weight { trunk.weight = weight; }
    if let Some(hosts) = req.hosts { trunk.hosts = hosts; }
    if let Some(ref proto) = req.transport_protocol { trunk.transport_protocol = proto.clone(); }
    if let Some(port) = req.transport_port { trunk.transport_port = port; }
    if let Some(verify) = req.verify_tls { trunk.verify_tls = verify; }
    trunk.auth_username = req.auth_username;
    trunk.auth_password = req.auth_password;
    trunk.auth_realm = req.auth_realm;
    if let Some(register) = req.register { trunk.register = register; }
    if let Some(expiry) = req.register_expiry { trunk.register_expiry = expiry; }
    if let Some(max) = req.max_concurrent_calls { trunk.max_concurrent_calls = max; }
    if let Some(max) = req.max_failures { trunk.max_failures = max; }
    if let Some(secs) = req.recovery_interval_secs { trunk.recovery_interval_secs = secs; }
    if let Some(enabled) = req.health_check_enabled { trunk.health_check_enabled = enabled; }
    if let Some(secs) = req.health_check_interval_secs { trunk.health_check_interval_secs = secs; }

    info!(trunk_name = %trunk.name, trunk_id = %trunk.id, "Creating SIP trunk");

    storage
        .put_trunk(trunk.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create trunk: {}", e)))?;

    Ok((StatusCode::CREATED, Json(TrunkResponse::from(trunk))))
}

/// Update an existing trunk.
pub async fn update_trunk(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTrunkRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    let mut trunk = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;

    if let Some(name) = req.name { trunk.name = name; }
    if let Some(enabled) = req.enabled { trunk.enabled = enabled; }
    if let Some(ref dir) = req.direction {
        trunk.direction = dir.parse::<TrunkDirection>()
            .map_err(|e| ApiError::BadRequest(e))?;
    }
    if let Some(priority) = req.priority { trunk.priority = priority; }
    if let Some(weight) = req.weight { trunk.weight = weight; }
    if let Some(hosts) = req.hosts { trunk.hosts = hosts; }
    if let Some(ref proto) = req.transport_protocol { trunk.transport_protocol = proto.clone(); }
    if let Some(port) = req.transport_port { trunk.transport_port = port; }
    if let Some(verify) = req.verify_tls { trunk.verify_tls = verify; }
    if let Some(username) = req.auth_username { trunk.auth_username = Some(username); }
    if let Some(password) = req.auth_password { trunk.auth_password = Some(password); }
    if let Some(realm) = req.auth_realm { trunk.auth_realm = Some(realm); }
    if let Some(register) = req.register { trunk.register = register; }
    if let Some(expiry) = req.register_expiry { trunk.register_expiry = expiry; }
    if let Some(max) = req.max_concurrent_calls { trunk.max_concurrent_calls = max; }
    if let Some(max) = req.max_failures { trunk.max_failures = max; }
    if let Some(secs) = req.recovery_interval_secs { trunk.recovery_interval_secs = secs; }
    if let Some(enabled) = req.health_check_enabled { trunk.health_check_enabled = enabled; }
    if let Some(secs) = req.health_check_interval_secs { trunk.health_check_interval_secs = secs; }

    trunk.updated_at = SystemTime::now();

    info!(trunk_name = %trunk.name, trunk_id = %trunk.id, "Updating SIP trunk");

    storage
        .put_trunk(trunk.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update trunk: {}", e)))?;

    Ok(Json(TrunkResponse::from(trunk)))
}

/// Delete a trunk.
pub async fn delete_trunk(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let trunk_id = TrunkId::from_string(id);

    // Verify it exists
    let trunk = storage
        .get_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get trunk: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Trunk not found".to_string()))?;

    info!(trunk_name = %trunk.name, trunk_id = %trunk.id, "Deleting SIP trunk");

    storage
        .delete_trunk(&trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete trunk: {}", e)))?;

    Ok(StatusCode::NO_CONTENT)
}
