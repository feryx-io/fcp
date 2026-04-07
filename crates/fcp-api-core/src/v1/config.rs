//! Configuration management API endpoints.
//!
//! Provides REST endpoints for CRUD operations on database-backed config
//! entries, plus audit log access and validation.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::ApiState;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn get_store(
    state: &ApiState,
) -> Result<&std::sync::Arc<dyn fcp_core::config_store::ConfigStore>, ApiError> {
    state
        .config_store
        .as_ref()
        .ok_or_else(|| ApiError::Internal("config store not available".into()))
}

// ── Request/Response Types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListConfigsQuery {
    pub service: Option<String>,
    pub tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub service: Option<String>,
    pub section: Option<String>,
    #[serde(default = "default_audit_limit")]
    pub limit: i64,
}

fn default_audit_limit() -> i64 {
    50
}

#[derive(Debug, Serialize)]
pub struct ConfigEntryResponse {
    pub service: String,
    pub section: String,
    pub tenant_id: String,
    pub value: serde_json::Value,
    pub version: i64,
    pub updated_at: String,
    pub updated_by: Option<String>,
}

impl From<fcp_core::config_store::ConfigEntry> for ConfigEntryResponse {
    fn from(e: fcp_core::config_store::ConfigEntry) -> Self {
        Self {
            service: e.service,
            section: e.section,
            tenant_id: e.tenant_id,
            value: e.value,
            version: e.version,
            updated_at: e.updated_at.to_rfc3339(),
            updated_by: e.updated_by,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AuditEntryResponse {
    pub id: i64,
    pub service: String,
    pub section: String,
    pub tenant_id: String,
    pub old_value: Option<serde_json::Value>,
    pub new_value: serde_json::Value,
    pub changed_by: Option<String>,
    pub changed_at: String,
    pub version: i64,
}

impl From<fcp_core::config_store::AuditEntry> for AuditEntryResponse {
    fn from(e: fcp_core::config_store::AuditEntry) -> Self {
        Self {
            id: e.id,
            service: e.service,
            section: e.section,
            tenant_id: e.tenant_id,
            old_value: e.old_value,
            new_value: e.new_value,
            changed_by: e.changed_by,
            changed_at: e.changed_at.to_rfc3339(),
            version: e.version,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub value: serde_json::Value,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
    pub updated_by: Option<String>,
}

fn default_tenant_id() -> String {
    "_system".to_string()
}

#[derive(Debug, Serialize)]
pub struct UpdateConfigResponse {
    pub version: i64,
    pub service: String,
    pub section: String,
    pub tenant_id: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteConfigResponse {
    pub deleted: bool,
}

#[derive(Debug, Serialize)]
pub struct ValidateConfigResponse {
    pub valid: bool,
    pub errors: Vec<String>,
}

// ── Query params for section-level endpoints ─────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SectionQuery {
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/v1/config -- list all config entries
pub async fn list_configs(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Query(params): Query<ListConfigsQuery>,
) -> Result<Json<Vec<ConfigEntryResponse>>, ApiError> {
    let store = get_store(&state)?;
    let tenant_id = tenant.0.to_string();
    let entries = store
        .list_configs(params.service.as_deref(), Some(tenant_id.as_str()))
        .await
        .map_err(|e| ApiError::Internal(format!("failed to list configs: {e}")))?;

    Ok(Json(entries.into_iter().map(Into::into).collect()))
}

/// GET /api/v1/config/:service -- list sections for a service
pub async fn list_service_config(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(service): Path<String>,
    Query(_params): Query<SectionQuery>,
) -> Result<Json<Vec<ConfigEntryResponse>>, ApiError> {
    let store = get_store(&state)?;
    let tenant_id = tenant.0.to_string();
    let entries = store
        .get_service_config(&service, tenant_id.as_str())
        .await
        .map_err(|e| ApiError::Internal(format!("failed to get service config: {e}")))?;

    Ok(Json(entries.into_iter().map(Into::into).collect()))
}

/// GET /api/v1/config/:service/:section -- get specific section
pub async fn get_config(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path((service, section)): Path<(String, String)>,
    Query(_params): Query<SectionQuery>,
) -> Result<Json<ConfigEntryResponse>, ApiError> {
    let store = get_store(&state)?;
    let tenant_id = tenant.0.to_string();
    let entry = store
        .get_config(&service, &section, tenant_id.as_str())
        .await
        .map_err(|e| ApiError::Internal(format!("failed to get config: {e}")))?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "config not found: {service}/{section} (tenant: {})",
                tenant_id
            ))
        })?;

    Ok(Json(entry.into()))
}

/// PUT /api/v1/config/:service/:section -- update/create section
pub async fn update_config(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path((service, section)): Path<(String, String)>,
    Json(req): Json<UpdateConfigRequest>,
) -> Result<Json<UpdateConfigResponse>, ApiError> {
    let store = get_store(&state)?;
    let tenant_id = tenant.0.to_string();

    // Validate that value is a JSON object
    if !req.value.is_object() {
        return Err(ApiError::BadRequest(
            "config value must be a JSON object".into(),
        ));
    }

    let version = store
        .set_config(
            &service,
            &section,
            tenant_id.as_str(),
            req.value,
            req.updated_by.as_deref(),
        )
        .await
        .map_err(|e| ApiError::Internal(format!("failed to set config: {e}")))?;

    Ok(Json(UpdateConfigResponse {
        version,
        service,
        section,
        tenant_id,
    }))
}

/// DELETE /api/v1/config/:service/:section -- delete config entry
pub async fn delete_config(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path((service, section)): Path<(String, String)>,
    Query(_params): Query<SectionQuery>,
) -> Result<Json<DeleteConfigResponse>, ApiError> {
    let store = get_store(&state)?;
    let tenant_id = tenant.0.to_string();
    let deleted = store
        .delete_config(&service, &section, tenant_id.as_str())
        .await
        .map_err(|e| ApiError::Internal(format!("failed to delete config: {e}")))?;

    Ok(Json(DeleteConfigResponse { deleted }))
}

/// GET /api/v1/config/audit -- audit log
pub async fn get_audit_log(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Query(params): Query<AuditLogQuery>,
) -> Result<Json<Vec<AuditEntryResponse>>, ApiError> {
    let store = get_store(&state)?;

    let service = params.service.as_deref().unwrap_or("_global");

    let limit = params.limit.min(500).max(1);

    let entries = store
        .get_audit_log(service, params.section.as_deref(), limit)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to get audit log: {e}")))?;
    let tenant_id = tenant.0.to_string();

    Ok(Json(
        entries
            .into_iter()
            .filter(|entry| entry.tenant_id == tenant_id)
            .map(Into::into)
            .collect(),
    ))
}

/// POST /api/v1/config/:service/:section/validate -- validate without saving
pub async fn validate_config(
    Path((_service, _section)): Path<(String, String)>,
    Json(req): Json<UpdateConfigRequest>,
) -> Result<Json<ValidateConfigResponse>, ApiError> {
    let mut errors = Vec::new();

    // Basic structural validation
    if !req.value.is_object() {
        errors.push("config value must be a JSON object".to_string());
    }

    // Check for empty objects
    if let Some(obj) = req.value.as_object() {
        if obj.is_empty() {
            errors.push("config value must not be empty".to_string());
        }
    }

    Ok(Json(ValidateConfigResponse {
        valid: errors.is_empty(),
        errors,
    }))
}
