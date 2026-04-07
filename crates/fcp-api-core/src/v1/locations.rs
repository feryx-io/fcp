//! Location management API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::storage::SiteLocationStore;
use fcp_core::{LocationId, SiteLocation, TenantId};
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

/// Location response.
#[derive(Debug, Serialize)]
pub struct LocationResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub floor: Option<String>,
    pub building: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub phone_number: Option<String>,
    pub timezone: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<SiteLocation> for LocationResponse {
    fn from(loc: SiteLocation) -> Self {
        Self {
            id: loc.id.to_string(),
            tenant_id: loc.tenant_id.to_string(),
            name: loc.name,
            description: loc.description,
            address_line1: loc.address_line1,
            address_line2: loc.address_line2,
            city: loc.city,
            state: loc.state,
            postal_code: loc.postal_code,
            country: loc.country,
            floor: loc.floor,
            building: loc.building,
            latitude: loc.latitude,
            longitude: loc.longitude,
            phone_number: loc.phone_number,
            timezone: loc.timezone,
            emergency_contact_name: loc.emergency_contact_name,
            emergency_contact_phone: loc.emergency_contact_phone,
            created_at: loc.created_at,
            updated_at: loc.updated_at,
        }
    }
}

/// Create location request.
#[derive(Debug, Deserialize)]
pub struct CreateLocationRequest {
    pub name: String,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: TenantId,
    pub description: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub floor: Option<String>,
    pub building: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub phone_number: Option<String>,
    pub timezone: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
}

fn default_tenant_id() -> TenantId {
    TenantId::nil()
}

fn ensure_location_in_tenant(
    location: SiteLocation,
    tenant_id: &TenantId,
    id: &LocationId,
) -> Result<SiteLocation, ApiError> {
    if &location.tenant_id == tenant_id {
        Ok(location)
    } else {
        Err(ApiError::NotFound(format!("Location {} not found", id)))
    }
}

/// Update location request.
#[derive(Debug, Deserialize)]
pub struct UpdateLocationRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub floor: Option<String>,
    pub building: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub phone_number: Option<String>,
    pub timezone: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
}

/// List locations.
pub async fn list_locations(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let locations = storage
        .list_locations(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list locations: {}", e)))?;

    let response: Vec<LocationResponse> = locations.into_iter().map(|l| l.into()).collect();

    info!("Listed {} locations", response.len());
    Ok(Json(response))
}

/// Get location by ID.
pub async fn get_location(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<LocationResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = LocationId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {}", e)))?;

    let location = storage
        .get_location(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get location: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Location {} not found", id)))
        .and_then(|location| ensure_location_in_tenant(location, &tenant.0, &id))?;

    Ok(Json(location.into()))
}

/// Create location.
pub async fn create_location(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateLocationRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let mut location = SiteLocation::new(req.name, tenant.0.clone());

    location.description = req.description;
    location.address_line1 = req.address_line1;
    location.address_line2 = req.address_line2;
    location.city = req.city;
    location.state = req.state;
    location.postal_code = req.postal_code;
    location.country = req.country;
    location.floor = req.floor;
    location.building = req.building;
    location.latitude = req.latitude;
    location.longitude = req.longitude;
    location.phone_number = req.phone_number;
    location.timezone = req.timezone;
    location.emergency_contact_name = req.emergency_contact_name;
    location.emergency_contact_phone = req.emergency_contact_phone;

    // Store the location
    storage
        .put_location(location.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create location: {}", e)))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "location".to_string(),
            entity_id: location.id.to_string(),
            tenant_id: location.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

    info!("Created location {} ({})", location.id, location.name);

    Ok((StatusCode::CREATED, Json(LocationResponse::from(location))))
}

/// Update location.
pub async fn update_location(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateLocationRequest>,
) -> Result<Json<LocationResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = LocationId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {}", e)))?;

    // Get existing location
    let mut location = storage
        .get_location(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get location: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Location {} not found", id)))
        .and_then(|location| ensure_location_in_tenant(location, &tenant.0, &id))?;

    // Update fields if provided
    if let Some(name) = req.name {
        location.name = name;
    }
    if req.description.is_some() {
        location.description = req.description;
    }
    if req.address_line1.is_some() {
        location.address_line1 = req.address_line1;
    }
    if req.address_line2.is_some() {
        location.address_line2 = req.address_line2;
    }
    if req.city.is_some() {
        location.city = req.city;
    }
    if req.state.is_some() {
        location.state = req.state;
    }
    if req.postal_code.is_some() {
        location.postal_code = req.postal_code;
    }
    if req.country.is_some() {
        location.country = req.country;
    }
    if req.floor.is_some() {
        location.floor = req.floor;
    }
    if req.building.is_some() {
        location.building = req.building;
    }
    if req.latitude.is_some() {
        location.latitude = req.latitude;
    }
    if req.longitude.is_some() {
        location.longitude = req.longitude;
    }
    if req.phone_number.is_some() {
        location.phone_number = req.phone_number;
    }
    if req.timezone.is_some() {
        location.timezone = req.timezone;
    }
    if req.emergency_contact_name.is_some() {
        location.emergency_contact_name = req.emergency_contact_name;
    }
    if req.emergency_contact_phone.is_some() {
        location.emergency_contact_phone = req.emergency_contact_phone;
    }

    // Update timestamp
    location.updated_at = SystemTime::now();

    // Store updated location
    storage
        .put_location(location.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update location: {}", e)))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "location".to_string(),
            entity_id: location.id.to_string(),
            tenant_id: location.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

    info!("Updated location {} ({})", location.id, location.name);

    Ok(Json(location.into()))
}

/// Delete location.
pub async fn delete_location(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let id = LocationId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {}", e)))?;

    // Fetch location before deletion for event data
    let location = storage
        .get_location(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get location: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Location {} not found", id)))
        .and_then(|location| ensure_location_in_tenant(location, &tenant.0, &id))?;

    storage
        .delete_location(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete location: {}", e)))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "location".to_string(),
            entity_id: location.id.to_string(),
            tenant_id: location.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

    info!("Deleted location {}", id);
    Ok(StatusCode::NO_CONTENT)
}
