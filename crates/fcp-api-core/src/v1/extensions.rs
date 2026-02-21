//! Extension management API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::storage::ExtensionStore;
use fcp_core::{DeviceId, Extension, ExtensionTarget, PhoneNumberId, ServiceId, TenantId, UserId};
use fcp_events::{PlatformEvent, SystemEvent};
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

/// Extension response.
#[derive(Debug, Serialize)]
pub struct ExtensionResponse {
    pub id: String,
    pub number: String,
    pub target: ExtensionTargetResponse,
    pub tenant_id: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "id")]
pub enum ExtensionTargetResponse {
    #[serde(rename = "user")]
    User(String),
    #[serde(rename = "phone_number")]
    PhoneNumber(String),
    #[serde(rename = "device")]
    Device(String),
    #[serde(rename = "service")]
    Service(String),
}

impl From<ExtensionTarget> for ExtensionTargetResponse {
    fn from(target: ExtensionTarget) -> Self {
        match target {
            ExtensionTarget::User(id) => ExtensionTargetResponse::User(id.to_string()),
            ExtensionTarget::PhoneNumber(id) => {
                ExtensionTargetResponse::PhoneNumber(id.to_string())
            }
            ExtensionTarget::Device(id) => ExtensionTargetResponse::Device(id.to_string()),
            ExtensionTarget::Service(id) => ExtensionTargetResponse::Service(id.to_string()),
        }
    }
}

impl ExtensionTargetResponse {
    fn to_extension_target(&self) -> Result<ExtensionTarget, ApiError> {
        match self {
            ExtensionTargetResponse::User(id) => {
                let user_id = UserId::from_str(id)
                    .map_err(|e| ApiError::BadRequest(format!("Invalid user ID: {}", e)))?;
                Ok(ExtensionTarget::User(user_id))
            }
            ExtensionTargetResponse::PhoneNumber(id) => {
                let phone_id = PhoneNumberId::from_str(id)
                    .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;
                Ok(ExtensionTarget::PhoneNumber(phone_id))
            }
            ExtensionTargetResponse::Device(id) => {
                let device_id = DeviceId::from_str(id)
                    .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;
                Ok(ExtensionTarget::Device(device_id))
            }
            ExtensionTargetResponse::Service(id) => {
                let service_id = ServiceId::from_str(id)
                    .map_err(|e| ApiError::BadRequest(format!("Invalid service ID: {}", e)))?;
                Ok(ExtensionTarget::Service(service_id))
            }
        }
    }
}

impl From<Extension> for ExtensionResponse {
    fn from(ext: Extension) -> Self {
        Self {
            id: ext.id.to_string(),
            number: ext.number,
            target: ext.target.into(),
            tenant_id: ext.tenant_id.to_string(),
            description: ext.description,
            enabled: ext.enabled,
            created_at: ext.created_at,
            updated_at: ext.updated_at,
        }
    }
}

/// Create extension request.
#[derive(Debug, Deserialize)]
pub struct CreateExtensionRequest {
    pub number: String,
    pub target: ExtensionTargetResponse,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: TenantId,
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_tenant_id() -> TenantId {
    TenantId::nil()
}

fn default_true() -> bool {
    true
}

/// Update extension request.
#[derive(Debug, Deserialize)]
pub struct UpdateExtensionRequest {
    pub target: Option<ExtensionTargetResponse>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

/// List extensions.
pub async fn list_extensions(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let extensions = storage
        .list_extensions(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list extensions: {}", e)))?;

    let response: Vec<ExtensionResponse> = extensions.into_iter().map(|e| e.into()).collect();

    info!("Listed {} extensions", response.len());
    Ok(Json(response))
}

/// Get extension by number.
pub async fn get_extension(
    State(state): State<ApiState>,
    Path(number): Path<String>,
    tenant: crate::auth::TenantContext,
) -> Result<Json<ExtensionResponse>, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let extension = storage
        .get_extension_by_number(&tenant_id, &number)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get extension: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Extension {} not found", number)))?;

    Ok(Json(extension.into()))
}

/// Create extension.
pub async fn create_extension(
    State(state): State<ApiState>,
    Json(req): Json<CreateExtensionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let target = req.target.to_extension_target()?;

    let mut extension = Extension::new(req.number.clone(), target, req.tenant_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid extension: {}", e)))?;

    if let Some(desc) = req.description {
        extension.set_description(desc);
    }

    extension.set_enabled(req.enabled);

    // Store the extension
    storage
        .put_extension(extension.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create extension: {}", e)))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "extension".to_string(),
            entity_id: extension.id.to_string(),
            tenant_id: extension.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

    info!(
        "Created extension {} -> {}",
        extension.number, extension.target
    );

    Ok((
        StatusCode::CREATED,
        Json(ExtensionResponse::from(extension)),
    ))
}

/// Update extension.
pub async fn update_extension(
    State(state): State<ApiState>,
    Path(number): Path<String>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<UpdateExtensionRequest>,
) -> Result<Json<ExtensionResponse>, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    // Get existing extension
    let mut extension = storage
        .get_extension_by_number(&tenant_id, &number)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get extension: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Extension {} not found", number)))?;

    // Update fields if provided
    if let Some(target_resp) = req.target {
        extension.target = target_resp.to_extension_target()?;
    }

    if req.description.is_some() {
        if let Some(desc) = req.description {
            extension.set_description(desc);
        }
    }

    if let Some(enabled) = req.enabled {
        extension.set_enabled(enabled);
    }

    // Update timestamp
    extension.updated_at = std::time::SystemTime::now();

    // Store updated extension
    storage
        .put_extension(extension.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update extension: {}", e)))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "extension".to_string(),
            entity_id: extension.id.to_string(),
            tenant_id: extension.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

    info!(
        "Updated extension {} -> {}",
        extension.number, extension.target
    );

    Ok(Json(extension.into()))
}

/// Delete extension.
pub async fn delete_extension(
    State(state): State<ApiState>,
    Path(number): Path<String>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    // Get the extension to find its ID
    let extension = storage
        .get_extension_by_number(&tenant_id, &number)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get extension: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Extension {} not found", number)))?;

    // Delete by ID
    storage
        .delete_extension(&extension.id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete extension: {}", e)))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "extension".to_string(),
            entity_id: extension.id.to_string(),
            tenant_id: extension.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

    info!("Deleted extension {}", number);
    Ok(StatusCode::NO_CONTENT)
}
