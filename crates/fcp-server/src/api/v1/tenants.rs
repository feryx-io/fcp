//! API endpoints for tenant management

use crate::api::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::{Tenant, TenantId, TenantStatus, TenantStore};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::SystemTime;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Tenant management requires registrar to be enabled".to_string())
        })?
    };
}

/// Response for tenant API calls
#[derive(Debug, Serialize)]
pub struct TenantResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
}

impl From<Tenant> for TenantResponse {
    fn from(tenant: Tenant) -> Self {
        Self {
            id: tenant.id.to_string(),
            name: tenant.name,
            description: tenant.description,
            status: tenant.status.to_string().to_lowercase(),
            contact_email: tenant.contact_email,
            contact_phone: tenant.contact_phone,
            max_users: tenant.max_users,
            max_phone_numbers: tenant.max_phone_numbers,
            max_concurrent_calls: tenant.max_concurrent_calls,
            billing_id: tenant.billing_id,
            created_at: tenant
                .created_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            updated_at: tenant
                .updated_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            trial_expires_at: tenant.trial_expires_at.and_then(|t| {
                t.duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs())
            }),
            metadata: tenant.metadata,
        }
    }
}

/// Request to create a new tenant
#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
}

/// Request to update a tenant
#[derive(Debug, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
}

/// List all tenants
pub async fn list_tenants(
    State(state): State<ApiState>,
) -> Result<Json<Vec<TenantResponse>>, ApiError> {
    let storage = require_storage!(state);
    let tenants = storage.list_tenants().await?;
    let response: Vec<TenantResponse> = tenants.into_iter().map(|t| t.into()).collect();
    Ok(Json(response))
}

/// Get a specific tenant by ID
pub async fn get_tenant(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<TenantResponse>, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = TenantId::from_str(&id)
        .map_err(|_| ApiError::BadRequest("Invalid tenant ID format".to_string()))?;
    let tenant = storage
        .get_tenant(&tenant_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Tenant {} not found", id)))?;
    Ok(Json(tenant.into()))
}

/// Create a new tenant
pub async fn create_tenant(
    State(state): State<ApiState>,
    Json(req): Json<CreateTenantRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    // Check if tenant name already exists
    if let Some(_) = storage.get_tenant_by_name(&req.name).await? {
        return Err(ApiError::Conflict(format!(
            "Tenant with name '{}' already exists",
            req.name
        )));
    }

    let now = SystemTime::now();
    let status = if let Some(status_str) = &req.status {
        match status_str.to_lowercase().as_str() {
            "active" => TenantStatus::Active,
            "suspended" => TenantStatus::Suspended,
            "trial" => TenantStatus::Trial,
            "closed" => TenantStatus::Closed,
            _ => {
                return Err(ApiError::BadRequest(format!(
                    "Invalid status '{}'. Must be one of: active, suspended, trial, closed",
                    status_str
                )))
            }
        }
    } else {
        TenantStatus::Active
    };

    let trial_expires_at = req.trial_expires_at.map(|secs| {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs)
    });

    let tenant = Tenant {
        id: TenantId::new(),
        name: req.name,
        description: req.description,
        status,
        contact_email: req.contact_email,
        contact_phone: req.contact_phone,
        max_users: req.max_users,
        max_phone_numbers: req.max_phone_numbers,
        max_concurrent_calls: req.max_concurrent_calls,
        billing_id: req.billing_id,
        created_at: now,
        updated_at: now,
        trial_expires_at,
        metadata: req.metadata,
    };

    storage.put_tenant(tenant.clone()).await?;

    let response: TenantResponse = tenant.into();
    Ok((StatusCode::CREATED, Json(response)))
}

/// Update an existing tenant
pub async fn update_tenant(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTenantRequest>,
) -> Result<Json<TenantResponse>, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = TenantId::from_str(&id)
        .map_err(|_| ApiError::BadRequest("Invalid tenant ID format".to_string()))?;

    // Get existing tenant
    let mut tenant = storage
        .get_tenant(&tenant_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Tenant {} not found", id)))?;

    // Update fields if provided
    if let Some(name) = req.name {
        // Check if new name conflicts with another tenant
        if let Some(existing) = storage.get_tenant_by_name(&name).await? {
            if existing.id != tenant.id {
                return Err(ApiError::Conflict(format!(
                    "Tenant with name '{}' already exists",
                    name
                )));
            }
        }
        tenant.name = name;
    }

    if let Some(description) = req.description {
        tenant.description = Some(description);
    }

    if let Some(status_str) = req.status {
        tenant.status = match status_str.to_lowercase().as_str() {
            "active" => TenantStatus::Active,
            "suspended" => TenantStatus::Suspended,
            "trial" => TenantStatus::Trial,
            "closed" => TenantStatus::Closed,
            _ => {
                return Err(ApiError::BadRequest(format!(
                    "Invalid status '{}'. Must be one of: active, suspended, trial, closed",
                    status_str
                )))
            }
        };
    }

    if let Some(contact_email) = req.contact_email {
        tenant.contact_email = Some(contact_email);
    }

    if let Some(contact_phone) = req.contact_phone {
        tenant.contact_phone = Some(contact_phone);
    }

    if let Some(max_users) = req.max_users {
        tenant.max_users = Some(max_users);
    }

    if let Some(max_phone_numbers) = req.max_phone_numbers {
        tenant.max_phone_numbers = Some(max_phone_numbers);
    }

    if let Some(max_concurrent_calls) = req.max_concurrent_calls {
        tenant.max_concurrent_calls = Some(max_concurrent_calls);
    }

    if let Some(billing_id) = req.billing_id {
        tenant.billing_id = Some(billing_id);
    }

    if let Some(trial_expires_at_secs) = req.trial_expires_at {
        tenant.trial_expires_at = Some(
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(trial_expires_at_secs),
        );
    }

    if let Some(metadata) = req.metadata {
        tenant.metadata = Some(metadata);
    }

    tenant.updated_at = SystemTime::now();

    storage.put_tenant(tenant.clone()).await?;

    Ok(Json(tenant.into()))
}

/// Delete a tenant
pub async fn delete_tenant(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = TenantId::from_str(&id)
        .map_err(|_| ApiError::BadRequest("Invalid tenant ID format".to_string()))?;

    // Check if tenant exists
    if storage.get_tenant(&tenant_id).await?.is_none() {
        return Err(ApiError::NotFound(format!("Tenant {} not found", id)));
    }

    storage.delete_tenant(&tenant_id).await?;

    Ok(StatusCode::NO_CONTENT)
}
