//! Phone number management API endpoints.

use crate::{auth::AuthIdentity, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::storage::{PhoneNumberStore, SiteLocationStore};
use fcp_core::SiteStore;
use fcp_core::{E911Address, LocationId, PhoneNumber, PhoneNumberId, TenantId};
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

    // Location
    pub location_id: Option<String>,
    /// The site whose phones answer this number, in a hybrid deployment
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_id: Option<String>,

    // Carrier metadata
    pub carrier: Option<String>,
    pub lrn: Option<String>,
    pub rate_center: Option<String>,

    // Lifecycle
    #[serde(with = "crate::time::rfc3339")]
    pub acquired_at: SystemTime,
    #[serde(default, with = "crate::time::rfc3339_opt")]
    pub activated_at: Option<SystemTime>,

    // Features
    pub recording_enabled: bool,
    pub block_anonymous: bool,
    pub tags: Vec<String>,

    // Audit
    #[serde(with = "crate::time::rfc3339")]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
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
    /// Whether a validator has confirmed this address. Read-only: an
    /// address is validated by `POST …/e911/validate`, never by claiming
    /// it here (`docs/E911_BLOCK_2.md` §7).
    #[serde(default)]
    pub validated: bool,
    #[serde(default, with = "crate::time::rfc3339_opt")]
    pub validated_at: Option<SystemTime>,
    /// The validator that confirmed it, and the reference it gave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validated_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reference: Option<String>,
    /// RFC 5139 civic elements (`house_number`, `street_name`,
    /// `street_suffix`, `unit`, …), beside the lines above
    /// (`docs/E911_BLOCK_2.md` §5.1).
    #[serde(flatten, default)]
    pub civic: fcp_core::phone_number::CivicElements,
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
            validated_by: addr.validated_by,
            provider_reference: addr.provider_reference,
            civic: addr.civic,
        }
    }
}

/// An address given to the API arrives unvalidated whatever it claims:
/// only a validator marks one, and the handler carries a previous
/// validation over when the address did not change
/// (`E911Address::carry_validation`).
impl From<E911AddressResponse> for E911Address {
    fn from(resp: E911AddressResponse) -> Self {
        E911Address {
            address1: resp.address1,
            address2: resp.address2,
            city: resp.city,
            state: resp.state,
            postal_code: resp.postal_code,
            country: resp.country,
            validated: false,
            validated_at: None,
            validated_by: None,
            provider_reference: None,
            civic: resp.civic,
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
            location_id: number.location_id.map(|id| id.to_string()),
            site_id: number.site_id.map(|id| id.to_string()),
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
    pub location_id: Option<String>,
    /// The site whose phones answer this number; empty to answer it here.
    pub site_id: Option<String>,
}

fn default_tenant_id() -> TenantId {
    TenantId::nil()
}

fn ensure_phone_number_in_tenant(
    number: PhoneNumber,
    tenant_id: &TenantId,
    id: &PhoneNumberId,
) -> Result<PhoneNumber, ApiError> {
    if &number.tenant_id == tenant_id {
        Ok(number)
    } else {
        Err(ApiError::NotFound(format!("Phone number {} not found", id)))
    }
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
    pub location_id: Option<String>,
    /// The site whose phones answer this number; empty to answer it here.
    pub site_id: Option<String>,
}

/// The site a number is answered at, checked to be the tenant's; an empty
/// value is none (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
async fn site_for_number(
    storage: &fcp_core::DynStorage,
    tenant: &crate::auth::TenantContext,
    site: &str,
) -> Result<Option<fcp_core::SiteId>, ApiError> {
    let site = site.trim();
    if site.is_empty() {
        return Ok(None);
    }
    let site_id = fcp_core::SiteId::from_str(site)
        .map_err(|e| ApiError::BadRequest(format!("Invalid site_id: {e}")))?;
    let found = storage
        .get_site(&site_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get site: {e}")))?
        .ok_or_else(|| ApiError::BadRequest(format!("Site {site_id} not found")))?;
    if found.tenant_id != tenant.0 {
        return Err(ApiError::BadRequest(format!(
            "Site {site_id} is not in the authenticated tenant"
        )));
    }
    Ok(Some(site_id))
}

/// List phone numbers.
pub async fn list_phone_numbers(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberRead,
        state.deployment_mode,
    )?;
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
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<PhoneNumberResponse>, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberRead,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    let number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get phone number: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Phone number {} not found", id)))
        .and_then(|number| ensure_phone_number_in_tenant(number, &tenant.0, &id))?;

    Ok(Json(number.into()))
}

/// Create phone number.
pub async fn create_phone_number(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreatePhoneNumberRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberCreate,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);

    // Parse and validate E.164
    let mut number = PhoneNumber::new(req.e164.clone(), tenant.0)
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
    if let Some(ref loc_id) = req.location_id {
        if !loc_id.is_empty() {
            let location_id = LocationId::from_str(loc_id)
                .map_err(|e| ApiError::BadRequest(format!("Invalid location_id: {}", e)))?;
            let location = storage
                .get_location(&location_id)
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to get location: {}", e)))?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("Location {} not found", location_id))
                })?;
            if location.tenant_id != tenant.0 {
                return Err(ApiError::BadRequest(format!(
                    "Location {} is not in the authenticated tenant",
                    location_id
                )));
            }
            number.location_id = Some(location_id);
        }
    }
    if let Some(ref site) = req.site_id {
        number.site_id = site_for_number(storage, &tenant, site).await?;
    }

    // Store the phone number
    storage
        .put_phone_number(number.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create phone number: {}", e)))?;

    info!("Created phone number {} ({})", number.id, number.e164);

    Ok((StatusCode::CREATED, Json(PhoneNumberResponse::from(number))))
}

/// Update phone number.
pub async fn update_phone_number(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdatePhoneNumberRequest>,
) -> Result<Json<PhoneNumberResponse>, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberUpdate,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    // Get existing number
    let mut number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get phone number: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Phone number {} not found", id)))
        .and_then(|number| ensure_phone_number_in_tenant(number, &tenant.0, &id))?;

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
        // Editing the address clears its validation; re-sending the same
        // address keeps it (`docs/E911_BLOCK_2.md` §7).
        let mut address: fcp_core::E911Address = e911.into();
        address.carry_validation(number.e911_address.as_ref());
        number.e911_address = Some(address);
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
    if let Some(ref loc_id) = req.location_id {
        if loc_id.is_empty() {
            number.location_id = None;
        } else {
            let location_id = LocationId::from_str(loc_id)
                .map_err(|e| ApiError::BadRequest(format!("Invalid location_id: {}", e)))?;
            let location = storage
                .get_location(&location_id)
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to get location: {}", e)))?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("Location {} not found", location_id))
                })?;
            if location.tenant_id != tenant.0 {
                return Err(ApiError::BadRequest(format!(
                    "Location {} is not in the authenticated tenant",
                    location_id
                )));
            }
            number.location_id = Some(location_id);
        }
    }
    if let Some(ref site) = req.site_id {
        number.site_id = site_for_number(storage, &tenant, site).await?;
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
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberDelete,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    let number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get phone number: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Phone number {} not found", id)))
        .and_then(|number| ensure_phone_number_in_tenant(number, &tenant.0, &id))?;

    storage
        .delete_phone_number(&number.id)
        .await
        .map_err(ApiError::storage)?;

    info!("Deleted phone number {}", id);
    Ok(StatusCode::NO_CONTENT)
}

/// Activate phone number.
pub async fn activate_phone_number(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<PhoneNumberResponse>, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberUpdate,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    // Get existing number
    let mut number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get phone number: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Phone number {} not found", id)))
        .and_then(|number| ensure_phone_number_in_tenant(number, &tenant.0, &id))?;

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

// ============================================================================
// DID Inventory / Pool Number Endpoints
// ============================================================================

/// Query parameters for searching available pool phone numbers.
#[derive(Debug, Deserialize)]
pub struct SearchAvailableQuery {
    pub area_code: Option<String>,
    pub pattern: Option<String>,
    pub rate_center: Option<String>,
    pub limit: Option<usize>,
}

/// Search available pool phone numbers.
/// GET /phone-numbers/available?area_code=518&pattern=*7034&limit=20
pub async fn search_available_numbers(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Query(params): Query<SearchAvailableQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberRead,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);

    let query = fcp_core::storage::AvailableNumberQuery {
        area_code: params.area_code,
        pattern: params.pattern,
        rate_center: params.rate_center,
        region: None,
        limit: params.limit.unwrap_or(50),
    };

    let numbers = storage
        .search_available_numbers(query)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let response: Vec<PhoneNumberResponse> = numbers.into_iter().map(Into::into).collect();
    Ok(Json(response))
}

/// Purchase (assign) a pool phone number to the caller's tenant.
/// POST /phone-numbers/:id/purchase
pub async fn purchase_phone_number(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<PhoneNumberResponse>, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberCreate,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    let acquired_at = SystemTime::now();
    let number = match storage
        .purchase_available_phone_number(&id, &tenant.0, acquired_at)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Some(number) => number,
        None => {
            let existing = storage
                .get_phone_number(&id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            if existing.is_none() {
                return Err(ApiError::NotFound("Phone number not found".into()));
            }
            return Err(ApiError::Conflict(
                "Phone number is not available for purchase".into(),
            ));
        }
    };

    info!(
        number = %number.e164,
        tenant = %number.tenant_id,
        "Phone number purchased"
    );
    Ok(Json(number.into()))
}

/// Release a phone number back to the pool.
/// POST /phone-numbers/:id/release
pub async fn release_phone_number(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::PhoneNumberDelete,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);

    let id = PhoneNumberId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    let mut number = storage
        .get_phone_number(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Phone number not found".into()))?;

    // Verify the number belongs to the caller's tenant
    if number.tenant_id != tenant.0 {
        return Err(ApiError::NotFound("Phone number not found".into()));
    }

    number.tenant_id = TenantId::nil();
    number.state = fcp_core::NumberState::Available;

    storage
        .put_phone_number(number.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(number = %number.e164, "Phone number released to pool");
    Ok(StatusCode::OK)
}
