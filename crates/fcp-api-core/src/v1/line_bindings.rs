//! Line binding management API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::storage::LineBindingStore;
use fcp_core::{
    BindingType, DeviceBinding, DeviceId, LineBinding, LineBindingId, PhoneNumberId, RingStrategy,
    TenantId, UserId,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::{Duration, SystemTime};
use tracing::info;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// Line binding response.
#[derive(Debug, Serialize)]
pub struct LineBindingResponse {
    pub id: String,
    pub phone_number_id: String,
    pub owner_user_id: Option<String>,
    pub binding_type: String,
    pub devices: Vec<DeviceBindingResponse>,
    pub ring_strategy: String,
    pub tenant_id: String,
    pub no_answer_timeout_secs: u64,
    pub forward_on_busy: Option<String>,
    pub forward_on_no_answer: Option<String>,
    pub forward_unconditional: Option<String>,
    pub outbound_caller_id: Option<String>,
    pub outbound_cnam: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceBindingResponse {
    pub device_id: String,
    pub priority: u8,
    pub line_appearance: u8,
    pub shared_line: bool,
    pub enabled: bool,
}

impl From<DeviceBinding> for DeviceBindingResponse {
    fn from(binding: DeviceBinding) -> Self {
        Self {
            device_id: binding.device_id.to_string(),
            priority: binding.priority,
            line_appearance: binding.line_appearance,
            shared_line: binding.shared_line,
            enabled: binding.enabled,
        }
    }
}

impl From<DeviceBindingResponse> for DeviceBinding {
    fn from(resp: DeviceBindingResponse) -> Self {
        let device_id = DeviceId::from_str(&resp.device_id).unwrap_or_else(|_| DeviceId::new());
        DeviceBinding {
            device_id,
            priority: resp.priority,
            line_appearance: resp.line_appearance,
            shared_line: resp.shared_line,
            enabled: resp.enabled,
        }
    }
}

impl From<LineBinding> for LineBindingResponse {
    fn from(binding: LineBinding) -> Self {
        Self {
            id: binding.id.to_string(),
            phone_number_id: binding.phone_number_id.to_string(),
            owner_user_id: binding.owner_user_id.map(|id| id.to_string()),
            binding_type: binding.binding_type.to_string(),
            devices: binding.devices.into_iter().map(|d| d.into()).collect(),
            ring_strategy: binding.ring_strategy.to_string(),
            tenant_id: binding.tenant_id.to_string(),
            no_answer_timeout_secs: binding.no_answer_timeout.as_secs(),
            forward_on_busy: binding.forward_on_busy,
            forward_on_no_answer: binding.forward_on_no_answer,
            forward_unconditional: binding.forward_unconditional,
            outbound_caller_id: binding.outbound_caller_id,
            outbound_cnam: binding.outbound_cnam,
            created_at: binding.created_at,
            updated_at: binding.updated_at,
        }
    }
}

/// Create line binding request.
#[derive(Debug, Deserialize)]
pub struct CreateLineBindingRequest {
    pub phone_number_id: String,
    pub owner_user_id: Option<String>,
    pub binding_type: String, // "user", "ivr", "queue", etc.
    #[serde(default = "default_tenant_id")]
    pub tenant_id: TenantId,
    pub devices: Option<Vec<DeviceBindingResponse>>,
    pub ring_strategy: Option<String>,
    pub no_answer_timeout_secs: Option<u64>,
    pub forward_on_busy: Option<String>,
    pub forward_on_no_answer: Option<String>,
    pub forward_unconditional: Option<String>,
}

fn default_tenant_id() -> TenantId {
    TenantId::nil()
}

/// Update line binding request.
#[derive(Debug, Deserialize)]
pub struct UpdateLineBindingRequest {
    pub devices: Option<Vec<DeviceBindingResponse>>,
    pub ring_strategy: Option<String>,
    pub no_answer_timeout_secs: Option<u64>,
    pub forward_on_busy: Option<String>,
    pub forward_on_no_answer: Option<String>,
    pub forward_unconditional: Option<String>,
    pub outbound_caller_id: Option<String>,
    pub outbound_cnam: Option<String>,
}

/// List line bindings.
pub async fn list_line_bindings(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let bindings = storage
        .list_line_bindings(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list line bindings: {}", e)))?;

    let response: Vec<LineBindingResponse> = bindings.into_iter().map(|b| b.into()).collect();

    info!("Listed {} line bindings", response.len());
    Ok(Json(response))
}

/// Get line binding by ID.
pub async fn get_line_binding(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<LineBindingResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = LineBindingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid line binding ID: {}", e)))?;

    let binding = storage
        .get_line_binding(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get line binding: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Line binding {} not found", id)))?;

    Ok(Json(binding.into()))
}

/// Create line binding.
pub async fn create_line_binding(
    State(state): State<ApiState>,
    Json(req): Json<CreateLineBindingRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let phone_number_id = PhoneNumberId::from_str(&req.phone_number_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid phone number ID: {}", e)))?;

    let owner_user_id = if let Some(user_id_str) = req.owner_user_id {
        Some(
            UserId::from_str(&user_id_str)
                .map_err(|e| ApiError::BadRequest(format!("Invalid user ID: {}", e)))?,
        )
    } else {
        None
    };

    let binding_type = parse_binding_type(&req.binding_type)?;

    let mut binding = LineBinding::new(phone_number_id, owner_user_id, binding_type, req.tenant_id);

    // Set optional fields
    if let Some(devices) = req.devices {
        binding.devices = devices.into_iter().map(|d| d.into()).collect();
    }

    if let Some(strategy) = req.ring_strategy {
        binding.ring_strategy = parse_ring_strategy(&strategy)?;
    }

    if let Some(timeout) = req.no_answer_timeout_secs {
        binding.no_answer_timeout = Duration::from_secs(timeout);
    }

    binding.forward_on_busy = req.forward_on_busy;
    binding.forward_on_no_answer = req.forward_on_no_answer;
    binding.forward_unconditional = req.forward_unconditional;

    // Store the line binding
    storage
        .put_line_binding(binding.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create line binding: {}", e)))?;

    info!(
        "Created line binding {} for phone number {}",
        binding.id, binding.phone_number_id
    );

    Ok((
        StatusCode::CREATED,
        Json(LineBindingResponse::from(binding)),
    ))
}

/// Update line binding.
pub async fn update_line_binding(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateLineBindingRequest>,
) -> Result<Json<LineBindingResponse>, ApiError> {
    let storage = require_storage!(state);

    let id = LineBindingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid line binding ID: {}", e)))?;

    // Get existing binding
    let mut binding = storage
        .get_line_binding(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get line binding: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Line binding {} not found", id)))?;

    // Update fields if provided
    if let Some(devices) = req.devices {
        binding.devices = devices.into_iter().map(|d| d.into()).collect();
    }

    if let Some(strategy) = req.ring_strategy {
        binding.ring_strategy = parse_ring_strategy(&strategy)?;
    }

    if let Some(timeout) = req.no_answer_timeout_secs {
        binding.no_answer_timeout = Duration::from_secs(timeout);
    }

    if req.forward_on_busy.is_some() {
        binding.forward_on_busy = req.forward_on_busy;
    }

    if req.forward_on_no_answer.is_some() {
        binding.forward_on_no_answer = req.forward_on_no_answer;
    }

    if req.forward_unconditional.is_some() {
        binding.forward_unconditional = req.forward_unconditional;
    }

    if req.outbound_caller_id.is_some() {
        binding.outbound_caller_id = req.outbound_caller_id;
    }

    if req.outbound_cnam.is_some() {
        binding.outbound_cnam = req.outbound_cnam;
    }

    // Update timestamp
    binding.updated_at = std::time::SystemTime::now();

    // Store updated binding
    storage
        .put_line_binding(binding.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update line binding: {}", e)))?;

    info!(
        "Updated line binding {} for phone number {}",
        binding.id, binding.phone_number_id
    );

    Ok(Json(binding.into()))
}

/// Delete line binding.
pub async fn delete_line_binding(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    let id = LineBindingId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid line binding ID: {}", e)))?;

    storage
        .delete_line_binding(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete line binding: {}", e)))?;

    info!("Deleted line binding {}", id);
    Ok(StatusCode::NO_CONTENT)
}

// Helper functions

fn parse_binding_type(s: &str) -> Result<BindingType, ApiError> {
    match s.to_lowercase().as_str() {
        "user" => Ok(BindingType::User),
        "ivr" => Ok(BindingType::Ivr),
        "queue" => Ok(BindingType::Queue),
        "hunt_group" => Ok(BindingType::HuntGroup),
        "conference_bridge" => Ok(BindingType::ConferenceBridge),
        "voicemail" => Ok(BindingType::Voicemail),
        "parking" => Ok(BindingType::Parking),
        "paging" => Ok(BindingType::Paging),
        "fax" => Ok(BindingType::Fax),
        "emergency" => Ok(BindingType::Emergency),
        _ => Err(ApiError::BadRequest(format!("Invalid binding type: {}", s))),
    }
}

fn parse_ring_strategy(s: &str) -> Result<RingStrategy, ApiError> {
    match s.to_lowercase().as_str() {
        "simultaneous" => Ok(RingStrategy::Simultaneous),
        "sequential" => Ok(RingStrategy::Sequential),
        "round_robin" => Ok(RingStrategy::RoundRobin),
        "longest_idle" => Ok(RingStrategy::LongestIdle),
        "primary" => Ok(RingStrategy::Primary),
        _ => Err(ApiError::BadRequest(format!(
            "Invalid ring strategy: {}",
            s
        ))),
    }
}
