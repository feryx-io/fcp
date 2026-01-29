//! Phone number management API endpoints.

use crate::{error::ApiError, state::ApiState};
use fcp_core::storage::PhoneNumberStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::{E911Address, PhoneNumber, PhoneNumberId, TenantId};
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

/// Phone number response.
#[derive(Debug, Serialize)]
pub struct PhoneNumberResponse {
    pub id: String,
    pub e164: String,
    pub country_code: String,
    pub npa: String,
    pub nxx: String,
    pub line: String,
    pub state: String,
    pub tenant_id: String,

    // Regulatory
    pub e911_address: Option<E911AddressResponse>,
    pub e911_callback: Option<String>,
    pub cnam: Option<String>,
    pub spam_score: Option<f32>,

    // Carrier metadata
    pub carrier: Option<String>,
    pub lrn: Option<String>,
    pub rate_center: Option<String>,

    // Lifecycle
    pub acquired_at: SystemTime,
    pub activated_at: Option<SystemTime>,

    // Features
    pub recording_enabled: bool,
    pub block_anonymous: bool,
    pub tags: Vec<String>,

    // Audit
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct E911AddressResponse {
    pub address1: String,
    pub address2: Option<String>,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
    pub validated: bool,
    pub validated_at: Option<SystemTime>,
}

impl From<E911Address> for E911AddressResponse {
    fn from(addr: E911Address) -> Self {
        Self {
            address1: addr.address1,
            address2: addr.address2,
            city: addr.city,
            state: addr.state,
            postal_code: addr.postal_code,
            country: addr.country,
            validated: addr.validated,
            validated_at: addr.validated_at,
        }
    }
}

impl From<E911AddressResponse> for E911Address {
    fn from(resp: E911AddressResponse) -> Self {
        E911Address {
            address1: resp.address1,
            address2: resp.address2,
            city: resp.city,
            state: resp.state,
            postal_code: resp.postal_code,
            country: resp.country,
            validated: resp.validated,
            validated_at: resp.validated_at,
        }
    }
}

impl From<PhoneNumber> for PhoneNumberResponse {
    fn from(number: PhoneNumber) -> Self {
        Self {
            id: number.id.to_string(),
            e164: number.e164,
            country_code: number.country_code,
            npa: number.npa,
            nxx: number.nxx,
            line: number.line,
            state: number.state.to_string(),
            tenant_id: number.tenant_id.to_string(),
            e911_address: number.e911_address.map(|a| a.into()),
            e911_callback: number.e911_callback,
            cnam: number.cnam,
            spam_score: number.spam_score,
            carrier: number.carrier,
            lrn: number.lrn,
            rate_center: number.rate_center,
            acquired_at: number.acquired_at,
            activated_at: number.activated_at,
            recording_enabled: number.recording_enabled,
            block_anonymous: number.block_anonymous,
            tags: number.tags,
            created_at: number.created_at,
            updated_at: number.updated_at,
        }
    }
}

/// Create phone number request.
#[derive(Debug, Deserialize)]
pub struct CreatePhoneNumberRequest {
    pub e164: String,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: TenantId,
    pub e911_address: Option<E911AddressResponse>,
    pub e911_callback: Option<String>,
    pub cnam: Option<String>,
    pub carrier: Option<String>,
    pub tags: Option<Vec<String>>,
}

fn default_tenant_id() -> TenantId {
    TenantId::nil()
}

/// Update phone number request.
#[derive(Debug, Deserialize)]
pub struct UpdatePhoneNumberRequest {
    pub state: Option<String>,
    pub e911_address: Option<E911AddressResponse>,
    pub e911_callback: Option<String>,
    pub cnam: Option<String>,
    pub recording_enabled: Option<bool>,
    pub block_anonymous: Option<bool>,
    pub allow_list: Option<Vec<String>>,
    pub block_list: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
}

/// List phone numbers.
pub async fn list_phone_numbers(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let numbers = storage
        .list_phone_numbers(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list phone numbers: {}", e)))?;

    let response: Vec<PhoneNumberResponse> = numbers.into_iter().map(|n| n.into()).collect();

    info!("Listed {} phone numbers", response.len());
    Ok(Json(response))
}

/// Get phone number by ID.
pub async fn get_phone_number(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<PhoneNumberResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    let number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get phone number: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Phone number {} not found", id)))?;

    Ok(Json(number.into()))
}

/// Create phone number.
pub async fn create_phone_number(
    State(state): State<ApiState>,
    Json(req): Json<CreatePhoneNumberRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    // Parse and validate E.164
    let mut number = PhoneNumber::new(req.e164.clone(), req.tenant_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number: {}", e)))?;

    // Set optional fields
    if let Some(e911) = req.e911_address {
        number.e911_address = Some(e911.into());
    }
    number.e911_callback = req.e911_callback;

    if let Some(cnam) = req.cnam {
        number
            .set_cnam(cnam)
            .map_err(|e| ApiError::BadRequest(format!("Invalid CNAM: {}", e)))?;
    }

    number.carrier = req.carrier;
    if let Some(tags) = req.tags {
        number.tags = tags;
    }

    // Store the phone number
    storage
        .put_phone_number(number.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create phone number: {}", e)))?;

    info!(
        "Created phone number {} ({})",
        number.id, number.e164
    );

    Ok((StatusCode::CREATED, Json(PhoneNumberResponse::from(number))))
}

/// Update phone number.
pub async fn update_phone_number(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePhoneNumberRequest>,
) -> Result<Json<PhoneNumberResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    // Get existing number
    let mut number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get phone number: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Phone number {} not found", id)))?;

    // Update fields if provided
    // NOTE: State transitions are NOT allowed via PATCH to prevent bypassing lifecycle validation.
    // Use dedicated lifecycle endpoints instead:
    // - POST /phone-numbers/:id/activate - transitions to Active (requires E911 data)
    // Other state transitions (quarantine, ported, etc.) should also use dedicated endpoints with proper validation.
    if req.state.is_some() {
        return Err(ApiError::BadRequest(
            "State cannot be set directly via PATCH. Use lifecycle endpoints like POST /phone-numbers/:id/activate instead.".to_string()
        ));
    }

    if let Some(e911) = req.e911_address {
        number.e911_address = Some(e911.into());
    }
    if req.e911_callback.is_some() {
        number.e911_callback = req.e911_callback;
    }
    if let Some(cnam) = req.cnam {
        number
            .set_cnam(cnam)
            .map_err(|e| ApiError::BadRequest(format!("Invalid CNAM: {}", e)))?;
    }
    if let Some(enabled) = req.recording_enabled {
        number.recording_enabled = enabled;
    }
    if let Some(enabled) = req.block_anonymous {
        number.block_anonymous = enabled;
    }
    if req.allow_list.is_some() {
        number.allow_list = req.allow_list.unwrap_or_default();
    }
    if req.block_list.is_some() {
        number.block_list = req.block_list.unwrap_or_default();
    }
    if req.tags.is_some() {
        number.tags = req.tags.unwrap_or_default();
    }

    // Update timestamp
    number.updated_at = std::time::SystemTime::now();

    // Store updated number
    storage
        .put_phone_number(number.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update phone number: {}", e)))?;

    info!("Updated phone number {} ({})", number.id, number.e164);

    Ok(Json(number.into()))
}

/// Delete phone number.
pub async fn delete_phone_number(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    storage
        .delete_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete phone number: {}", e)))?;

    info!("Deleted phone number {}", id);
    Ok(StatusCode::NO_CONTENT)
}

/// Activate phone number.
pub async fn activate_phone_number(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<PhoneNumberResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    // Get existing number
    let mut number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get phone number: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Phone number {} not found", id)))?;

    // Activate the number
    number
        .activate()
        .map_err(|e| ApiError::BadRequest(format!("Failed to activate phone number: {}", e)))?;

    // Update timestamp
    number.updated_at = std::time::SystemTime::now();

    // Store updated number
    storage
        .put_phone_number(number.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update phone number: {}", e)))?;

    info!("Activated phone number {} ({})", number.id, number.e164);

    Ok(Json(number.into()))
}
