//! Config template management API endpoints.

use crate::{auth::AuthIdentity, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::config_template::{ConfigTemplate, PhoneVendor};
use fcp_core::permissions::Permission;
use fcp_core::ConfigTemplateId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
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

/// Helper to get config template store or return error.
fn require_template_store(
    storage: &fcp_core::DynStorage,
) -> Result<&std::sync::Arc<dyn fcp_core::storage::ConfigTemplateStore>, ApiError> {
    storage
        .config_template_store()
        .ok_or_else(|| ApiError::Internal("Config template store not configured".to_string()))
}

/// Config template response.
#[derive(Debug, Serialize)]
pub struct ConfigTemplateResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub vendor: String,
    pub model_pattern: Option<String>,
    pub template_body: String,
    pub description: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<ConfigTemplate> for ConfigTemplateResponse {
    fn from(t: ConfigTemplate) -> Self {
        Self {
            id: t.id.to_string(),
            tenant_id: t.tenant_id.to_string(),
            name: t.name,
            vendor: t.vendor.to_string(),
            model_pattern: t.model_pattern,
            template_body: t.template_body,
            description: t.description,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

/// Create config template request.
#[derive(Debug, Deserialize)]
pub struct CreateConfigTemplateRequest {
    pub name: String,
    pub vendor: String,
    pub model_pattern: Option<String>,
    pub template_body: String,
    pub description: Option<String>,
}

/// Update config template request.
#[derive(Debug, Deserialize)]
pub struct UpdateConfigTemplateRequest {
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub model_pattern: Option<String>,
    pub template_body: Option<String>,
    pub description: Option<String>,
}

/// List config templates for the authenticated tenant.
pub async fn list_config_templates(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let template_store = require_template_store(storage)?;

    let templates = template_store
        .list_config_templates(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list config templates: {}", e)))?;

    let response: Vec<ConfigTemplateResponse> = templates.into_iter().map(Into::into).collect();

    info!("Listed {} config templates", response.len());
    Ok(Json(response))
}

/// Get a config template by ID.
pub async fn get_config_template(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<ConfigTemplateResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let template_store = require_template_store(storage)?;

    let id = ConfigTemplateId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid config template ID: {}", e)))?;

    let template = template_store
        .get_config_template(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get config template: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Config template {} not found", id)))?;

    if template.tenant_id != tenant.0 {
        return Err(ApiError::NotFound(format!(
            "Config template {} not found",
            id
        )));
    }

    Ok(Json(template.into()))
}

/// Create a new config template.
pub async fn create_config_template(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateConfigTemplateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceCreate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let template_store = require_template_store(storage)?;

    let vendor: PhoneVendor = req
        .vendor
        .parse()
        .map_err(|e: String| ApiError::BadRequest(e))?;

    let now = SystemTime::now();
    let template = ConfigTemplate {
        id: ConfigTemplateId::new(),
        tenant_id: tenant.0,
        name: req.name,
        vendor,
        model_pattern: req.model_pattern,
        template_body: req.template_body,
        description: req.description,
        created_at: now,
        updated_at: now,
    };

    template_store
        .put_config_template(template.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create config template: {}", e)))?;

    info!(
        "Created config template {} ({})",
        template.id, template.name
    );

    Ok((
        StatusCode::CREATED,
        Json(ConfigTemplateResponse::from(template)),
    ))
}

/// Update a config template.
pub async fn update_config_template(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateConfigTemplateRequest>,
) -> Result<Json<ConfigTemplateResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let template_store = require_template_store(storage)?;

    let id = ConfigTemplateId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid config template ID: {}", e)))?;

    let mut template = template_store
        .get_config_template(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get config template: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Config template {} not found", id)))?;

    if template.tenant_id != tenant.0 {
        return Err(ApiError::NotFound(format!(
            "Config template {} not found",
            id
        )));
    }

    if let Some(name) = req.name {
        template.name = name;
    }
    if let Some(vendor_str) = req.vendor {
        template.vendor = vendor_str
            .parse()
            .map_err(|e: String| ApiError::BadRequest(e))?;
    }
    if req.model_pattern.is_some() {
        template.model_pattern = req.model_pattern;
    }
    if let Some(body) = req.template_body {
        template.template_body = body;
    }
    if req.description.is_some() {
        template.description = req.description;
    }
    template.updated_at = SystemTime::now();

    template_store
        .put_config_template(template.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update config template: {}", e)))?;

    info!(
        "Updated config template {} ({})",
        template.id, template.name
    );

    Ok(Json(template.into()))
}

/// Delete a config template.
pub async fn delete_config_template(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceDelete, state.deployment_mode)?;
    let storage = require_storage!(state);
    let template_store = require_template_store(storage)?;

    let id = ConfigTemplateId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid config template ID: {}", e)))?;

    // Verify the template exists and belongs to the tenant
    let template = template_store
        .get_config_template(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get config template: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Config template {} not found", id)))?;

    if template.tenant_id != tenant.0 {
        return Err(ApiError::NotFound(format!(
            "Config template {} not found",
            id
        )));
    }

    template_store
        .delete_config_template(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete config template: {}", e)))?;

    info!("Deleted config template {}", id);
    Ok(StatusCode::NO_CONTENT)
}
