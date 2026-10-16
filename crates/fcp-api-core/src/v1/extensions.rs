//! Extension management API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::storage::{
    CredentialStore, DeviceStore, ExtensionStore, PhoneNumberStore, SiteLocationStore,
};
use fcp_core::{
    DeviceId, Extension, ExtensionTarget, LocationId, PhoneNumberId, ServiceId, TenantId, UserId,
};
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
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ExtensionResponse {
    pub id: String,
    pub number: String,
    pub target: ExtensionTargetResponse,
    pub tenant_id: String,
    pub description: Option<String>,
    pub location_id: Option<String>,
    pub enabled: bool,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: SystemTime,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
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
            location_id: ext.location_id.map(|id| id.to_string()),
            enabled: ext.enabled,
            created_at: ext.created_at,
            updated_at: ext.updated_at,
        }
    }
}

/// Create extension request.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateExtensionRequest {
    pub number: String,
    pub target: ExtensionTargetResponse,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: TenantId,
    pub description: Option<String>,
    pub location_id: Option<String>,
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
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateExtensionRequest {
    pub target: Option<ExtensionTargetResponse>,
    pub description: Option<String>,
    pub location_id: Option<String>,
    pub enabled: Option<bool>,
}

/// An extension dials only what is the tenant's: the call manager rings
/// whatever user, device or number the target names, in any tenant, so a
/// target from elsewhere would ring another tenant's phones. A service has
/// no store to check against and names nothing a call reaches today.
async fn check_target(
    storage: &fcp_core::DynStorage,
    tenant: &TenantId,
    target: &ExtensionTarget,
) -> Result<(), ApiError> {
    let owner = match target {
        ExtensionTarget::User(id) => storage.get_user_by_id(id).await?.map(|u| u.tenant_id),
        ExtensionTarget::Device(id) => storage.get_device(id).await?.map(|d| d.tenant_id),
        ExtensionTarget::PhoneNumber(id) => {
            storage.get_phone_number(id).await?.map(|n| n.tenant_id)
        }
        ExtensionTarget::Service(_) => return Ok(()),
    };
    if owner.as_ref() != Some(tenant) {
        return Err(ApiError::BadRequest(format!("Target {target} not found")));
    }
    Ok(())
}

/// The extension's location, checked to be the tenant's; empty is none.
async fn location_for(
    storage: &fcp_core::DynStorage,
    tenant: &TenantId,
    location: &str,
) -> Result<Option<LocationId>, ApiError> {
    if location.is_empty() {
        return Ok(None);
    }
    let id = LocationId::from_str(location)
        .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {}", e)))?;
    match storage.get_location(&id).await? {
        Some(found) if &found.tenant_id == tenant => Ok(Some(id)),
        _ => Err(ApiError::BadRequest(format!("Location {id} not found"))),
    }
}

/// List extensions.
#[utoipa::path(
    get,
    path = "/api/v1/extensions",
    tag = "extensions",
    operation_id = "extensions_list_extensions",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<ExtensionResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_extensions(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let extensions = storage
        .list_extensions(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list extensions: {}", e)))?;

    let response: Vec<ExtensionResponse> = extensions.into_iter().map(|e| e.into()).collect();

    info!("Listed {} extensions", response.len());
    Ok(Json(crate::page::paginate(
        response,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{:>20}/{}", r.number, r.id),
    )?))
}

/// Get extension by number.
#[utoipa::path(
    get,
    path = "/api/v1/extensions/{number}",
    tag = "extensions",
    operation_id = "extensions_get_extension",
    params(("number" = String, Path)),
    responses(
        (status = 200, description = "OK", body = ExtensionResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
#[utoipa::path(
    post,
    path = "/api/v1/extensions",
    tag = "extensions",
    operation_id = "extensions_create_extension",
    request_body = CreateExtensionRequest,
    responses(
        (status = 201, description = "OK", body = ExtensionResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_extension(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateExtensionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let target = req.target.to_extension_target()?;
    check_target(storage, &tenant.0, &target).await?;

    // SECURITY: Use authenticated tenant, not request body tenant_id
    let mut extension = Extension::new(req.number.clone(), target, tenant.0)
        .map_err(|e| ApiError::BadRequest(format!("Invalid extension: {}", e)))?;

    if let Some(desc) = req.description {
        extension.set_description(desc);
    }

    if let Some(location) = req.location_id {
        extension.location_id = location_for(storage, &tenant.0, &location).await?;
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
#[utoipa::path(
    patch,
    path = "/api/v1/extensions/{number}",
    tag = "extensions",
    operation_id = "extensions_update_extension",
    params(("number" = String, Path)),
    request_body = UpdateExtensionRequest,
    responses(
        (status = 200, description = "OK", body = ExtensionResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
        let target = target_resp.to_extension_target()?;
        check_target(storage, &tenant_id, &target).await?;
        extension.target = target;
    }

    if req.description.is_some() {
        if let Some(desc) = req.description {
            extension.set_description(desc);
        }
    }

    if let Some(location) = req.location_id {
        extension.location_id = location_for(storage, &tenant_id, &location).await?;
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
#[utoipa::path(
    delete,
    path = "/api/v1/extensions/{number}",
    tag = "extensions",
    operation_id = "extensions_delete_extension",
    params(("number" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
