//! Device management API endpoints.

use crate::{error::ApiError, state::ApiState, v1::phone_numbers::E911AddressResponse};
use fcp_core::storage::DeviceStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::{Device, DeviceId, DeviceType, TenantId, UserId};
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

/// Device response.
#[derive(Debug, Serialize)]
pub struct DeviceResponse {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub mac_address: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub tenant_id: String,
    pub owner_user_id: Option<String>,
    pub sip_uri: Option<String>,
    pub registration_state: String,
    pub last_registration: Option<SystemTime>,
    pub last_ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub max_lines: u8,
    pub supports_video: bool,
    pub supports_blf: bool,
    pub codec_support: Vec<String>,
    pub location: Option<String>,
    pub e911_override: Option<E911AddressResponse>,
    pub provisioning_url: Option<String>,
    pub config_template_id: Option<String>,
    pub enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<Device> for DeviceResponse {
    fn from(device: Device) -> Self {
        Self {
            id: device.id.to_string(),
            name: device.name,
            device_type: device.device_type.to_string(),
            mac_address: device.mac_address,
            model: device.model,
            firmware_version: device.firmware_version,
            tenant_id: device.tenant_id.to_string(),
            owner_user_id: device.owner_user_id.map(|id| id.to_string()),
            sip_uri: device.sip_uri,
            registration_state: device.registration_state.to_string(),
            last_registration: device.last_registration,
            last_ip_address: device.last_ip_address,
            user_agent: device.user_agent,
            max_lines: device.max_lines,
            supports_video: device.supports_video,
            supports_blf: device.supports_blf,
            codec_support: device
                .codec_support
                .into_iter()
                .map(|c| format!("{:?}", c))
                .collect(),
            location: device.location,
            e911_override: device.e911_override.map(|a| a.into()),
            provisioning_url: device.provisioning_url,
            config_template_id: device.config_template_id,
            enabled: device.enabled,
            created_at: device.created_at,
            updated_at: device.updated_at,
        }
    }
}

/// Create device request.
#[derive(Debug, Deserialize)]
pub struct CreateDeviceRequest {
    pub name: String,
    pub device_type: String,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: TenantId,
    pub owner_user_id: Option<String>,
    pub mac_address: Option<String>,
    pub model: Option<String>,
    pub sip_uri: Option<String>,
    pub max_lines: Option<u8>,
    pub supports_video: Option<bool>,
    pub supports_blf: Option<bool>,
    pub location: Option<String>,
    pub e911_override: Option<E911AddressResponse>,
}

fn default_tenant_id() -> TenantId {
    TenantId::nil()
}

/// Update device request.
#[derive(Debug, Deserialize)]
pub struct UpdateDeviceRequest {
    pub name: Option<String>,
    pub owner_user_id: Option<String>,
    pub mac_address: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub sip_uri: Option<String>,
    pub max_lines: Option<u8>,
    pub supports_video: Option<bool>,
    pub supports_blf: Option<bool>,
    pub location: Option<String>,
    pub e911_override: Option<E911AddressResponse>,
    pub provisioning_url: Option<String>,
    pub config_template_id: Option<String>,
    pub enabled: Option<bool>,
}

/// List devices.
pub async fn list_devices(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    // TODO: Get tenant_id from authentication context
    let tenant_id = fcp_core::TenantId::nil();

    let devices = storage
        .list_devices(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list devices: {}", e)))?;

    let response: Vec<DeviceResponse> = devices.into_iter().map(|d| d.into()).collect();

    info!("Listed {} devices", response.len());
    Ok(Json(response))
}

/// Get device by ID.
pub async fn get_device(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<DeviceResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;

    let device = storage
        .get_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get device: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Device {} not found", id)))?;

    Ok(Json(device.into()))
}

/// Create device.
pub async fn create_device(
    State(state): State<ApiState>,
    Json(req): Json<CreateDeviceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let device_type = parse_device_type(&req.device_type)?;

    let mut device = Device::new(req.name, device_type, req.tenant_id);

    // Set optional fields
    if let Some(owner_id_str) = req.owner_user_id {
        device.owner_user_id = Some(
            UserId::from_str(&owner_id_str)
                .map_err(|e| ApiError::BadRequest(format!("Invalid user ID: {}", e)))?,
        );
    }

    device.mac_address = req.mac_address;
    device.model = req.model;
    device.sip_uri = req.sip_uri;

    if let Some(max_lines) = req.max_lines {
        device.max_lines = max_lines;
    }
    if let Some(video) = req.supports_video {
        device.supports_video = video;
    }
    if let Some(blf) = req.supports_blf {
        device.supports_blf = blf;
    }

    device.location = req.location;

    if let Some(e911) = req.e911_override {
        device.set_e911_override(e911.into());
    }

    // Store the device
    storage
        .put_device(device.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create device: {}", e)))?;

    info!("Created device {} ({})", device.id, device.name);

    Ok((StatusCode::CREATED, Json(DeviceResponse::from(device))))
}

/// Update device.
pub async fn update_device(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateDeviceRequest>,
) -> Result<Json<DeviceResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;

    // Get existing device
    let mut device = storage
        .get_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get device: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Device {} not found", id)))?;

    // Update fields if provided
    if let Some(name) = req.name {
        device.name = name;
    }

    if let Some(owner_id_str) = req.owner_user_id {
        device.owner_user_id = Some(
            UserId::from_str(&owner_id_str)
                .map_err(|e| ApiError::BadRequest(format!("Invalid user ID: {}", e)))?,
        );
    }

    if req.mac_address.is_some() {
        device.mac_address = req.mac_address;
    }

    if req.model.is_some() {
        device.model = req.model;
    }

    if req.firmware_version.is_some() {
        device.firmware_version = req.firmware_version;
    }

    if req.sip_uri.is_some() {
        device.sip_uri = req.sip_uri;
    }

    if let Some(max_lines) = req.max_lines {
        device.max_lines = max_lines;
    }

    if let Some(video) = req.supports_video {
        device.supports_video = video;
    }

    if let Some(blf) = req.supports_blf {
        device.supports_blf = blf;
    }

    if req.location.is_some() {
        device.location = req.location;
    }

    if let Some(e911) = req.e911_override {
        device.set_e911_override(e911.into());
    }

    if req.provisioning_url.is_some() {
        device.provisioning_url = req.provisioning_url;
    }

    if req.config_template_id.is_some() {
        device.config_template_id = req.config_template_id;
    }

    if let Some(enabled) = req.enabled {
        device.enabled = enabled;
    }

    // Update timestamp
    device.updated_at = std::time::SystemTime::now();

    // Store updated device
    storage
        .put_device(device.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update device: {}", e)))?;

    info!("Updated device {} ({})", device.id, device.name);

    Ok(Json(device.into()))
}

/// Delete device.
pub async fn delete_device(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let id = DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;

    storage
        .delete_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete device: {}", e)))?;

    info!("Deleted device {}", id);
    Ok(StatusCode::NO_CONTENT)
}

// Helper functions

fn parse_device_type(s: &str) -> Result<DeviceType, ApiError> {
    match s.to_lowercase().as_str() {
        "desk_phone" => Ok(DeviceType::DeskPhone),
        "softphone" => Ok(DeviceType::Softphone),
        "webrtc" => Ok(DeviceType::WebRtc),
        "conference_room" => Ok(DeviceType::ConferenceRoom),
        "ata" => Ok(DeviceType::Ata),
        "gateway" => Ok(DeviceType::Gateway),
        "mobile" => Ok(DeviceType::Mobile),
        "other" => Ok(DeviceType::Other),
        _ => Err(ApiError::BadRequest(format!("Invalid device type: {}", s))),
    }
}
