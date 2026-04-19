//! Device management API endpoints.

use crate::{
    auth::AuthIdentity, error::ApiError, state::ApiState, v1::phone_numbers::E911AddressResponse,
    validation,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::storage::{CredentialStore, DeviceStore, RegistrationStore, SiteLocationStore};
use fcp_core::{ConfigTemplateId, Device, DeviceId, DeviceType, TenantId, UserId};
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use std::time::SystemTime;
use tracing::info;
use uuid::Uuid;

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
    pub location_id: Option<String>,
    pub e911_override: Option<E911AddressResponse>,
    pub provisioning_url: Option<String>,
    pub config_template_id: Option<String>,
    pub has_provisioning_token: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioning_token: Option<String>,
    pub enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<Device> for DeviceResponse {
    fn from(device: Device) -> Self {
        let has_provisioning_token = device
            .provisioning_token_hash
            .as_deref()
            .is_some_and(|hash| !hash.trim().is_empty());
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
            location_id: device.location_id.map(|id| id.to_string()),
            e911_override: device.e911_override.map(|a| a.into()),
            provisioning_url: device.provisioning_url,
            config_template_id: device.config_template_id,
            has_provisioning_token,
            provisioning_token: None,
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
    pub location_id: Option<String>,
    pub e911_override: Option<E911AddressResponse>,
    /// Optional per-device provisioning token. If omitted for a device with a
    /// MAC address, a token is generated and returned once in the create response.
    pub provisioning_token: Option<String>,
}

fn default_tenant_id() -> TenantId {
    TenantId::nil()
}

fn normalize_optional_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_mac_address(mac_address: &str) -> String {
    mac_address.replace([':', '-', '.'], "").to_lowercase()
}

fn hash_provisioning_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn generate_provisioning_token() -> String {
    format!(
        "fcp_prov_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn validate_device_provisioning_token(
    state: &ApiState,
    token: &str,
) -> Result<String, ApiError> {
    validation::reject_crlf("provisioning_token", token)?;
    let token = token.trim();
    if token.len() < 32 {
        return Err(ApiError::BadRequest(
            "provisioning_token must be at least 32 characters".to_string(),
        ));
    }
    if state
        .provisioning_auth_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|shared_token| shared_token == token)
    {
        return Err(ApiError::BadRequest(
            "provisioning_token must be distinct from api.provisioning_auth_token".to_string(),
        ));
    }
    Ok(token.to_string())
}

/// Map a storage-layer error from `put_device` into an `ApiError`.
///
/// Recognises the typed [`fcp_core::StorageError::Conflict`] surfaced by the
/// Postgres backend on unique-index violations (e.g. two concurrent creates
/// claiming the same normalized MAC) and returns HTTP 409 instead of the
/// default 500 that a generic anyhow error would produce.
fn map_put_device_error(context: &'static str, err: anyhow::Error) -> ApiError {
    if let Some(fcp_core::Error::Storage(fcp_core::StorageError::Conflict(msg))) =
        err.downcast_ref::<fcp_core::Error>()
    {
        return ApiError::Conflict(msg.clone());
    }
    ApiError::Internal(format!("{}: {}", context, err))
}

async fn ensure_mac_available(
    storage: &fcp_core::DynStorage,
    normalized_mac: &str,
    current_device_id: Option<&DeviceId>,
) -> Result<(), ApiError> {
    if normalized_mac.is_empty() {
        return Ok(());
    }

    let existing = storage
        .get_device_by_mac_address(normalized_mac)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to check MAC address: {}", e)))?;

    if let Some(existing) = existing {
        if current_device_id.is_none_or(|id| existing.id != *id) {
            return Err(ApiError::Conflict(
                "MAC address is already assigned to another device".to_string(),
            ));
        }
    }

    Ok(())
}

fn ensure_device_in_tenant(
    device: Device,
    tenant_id: &TenantId,
    id: &DeviceId,
) -> Result<Device, ApiError> {
    if &device.tenant_id == tenant_id {
        Ok(device)
    } else {
        Err(ApiError::NotFound(format!("Device {} not found", id)))
    }
}

async fn validate_config_template_for_tenant(
    storage: &fcp_core::DynStorage,
    tenant_id: &TenantId,
    config_template_id: &str,
) -> Result<String, ApiError> {
    let template_id = ConfigTemplateId::from_str(config_template_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid config template ID: {}", e)))?;
    let template_store = storage
        .config_template_store()
        .ok_or_else(|| ApiError::Internal("Config template store not configured".to_string()))?;
    let template = template_store
        .get_config_template(&template_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get config template: {}", e)))?
        .ok_or_else(|| {
            ApiError::BadRequest(format!("Config template {} not found", template_id))
        })?;
    if &template.tenant_id != tenant_id {
        return Err(ApiError::BadRequest(format!(
            "Config template {} is not in the authenticated tenant",
            template_id
        )));
    }
    Ok(template_id.to_string())
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
    pub location_id: Option<String>,
    pub e911_override: Option<E911AddressResponse>,
    pub provisioning_url: Option<String>,
    pub config_template_id: Option<String>,
    /// Set or rotate the per-device provisioning token. An empty string clears it.
    pub provisioning_token: Option<String>,
    pub enabled: Option<bool>,
}

/// List devices.
pub async fn list_devices(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let storage = require_storage!(state);

    let tenant_id = tenant.0;

    let devices = storage
        .list_devices(&tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list devices: {}", e)))?;

    // Fetch live registrations and build a lookup by AOR
    let live_regs = match storage.get_all_registrations_in_tenant(&tenant_id).await {
        Ok(regs) => {
            info!(
                "Fetched {} live registrations for device enrichment",
                regs.len()
            );
            regs
        }
        Err(e) => {
            info!("No registrations available for device enrichment: {}", e);
            vec![]
        }
    };
    let now = std::time::SystemTime::now();

    // Build map: AOR (e.g. "alice@192.168.1.225") → Vec<(contact_ip, user_agent)>
    let mut reg_map: std::collections::HashMap<String, Vec<(String, Option<String>)>> =
        std::collections::HashMap::new();
    for reg in &live_regs {
        if reg.expires_at <= now {
            continue;
        }
        let aor_str = reg
            .aor
            .trim_start_matches("sip:")
            .trim_start_matches("sips:")
            .to_string();
        let contact_ip = extract_ip_from_contact(&reg.contact);
        reg_map
            .entry(aor_str)
            .or_default()
            .push((contact_ip, reg.user_agent.clone()));
    }

    // Count how many devices share each AOR (for disambiguation)
    let mut devices_per_aor: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for d in &devices {
        if let Some(ref uri) = d.sip_uri {
            let n = uri.trim_start_matches("sip:").trim_start_matches("sips:");
            *devices_per_aor.entry(n.to_string()).or_default() += 1;
        }
    }

    let response: Vec<DeviceResponse> = devices.into_iter().map(|d| {
        let mut resp = DeviceResponse::from(d);
        // Enrich with live registration data
        if let Some(ref sip_uri) = resp.sip_uri {
            let normalized = sip_uri.trim_start_matches("sip:")
                .trim_start_matches("sips:");
            info!(device = %resp.name, normalized_uri = %normalized, reg_keys = ?reg_map.keys().collect::<Vec<_>>(), "Looking up registration");
            if let Some(contacts) = reg_map.get(normalized) {
                let device_count = devices_per_aor.get(normalized).copied().unwrap_or(1);
                let best = if contacts.len() == 1 && device_count == 1 {
                    // Only one contact AND one device for this AOR — direct match
                    Some(&contacts[0])
                } else {
                    // Multiple contacts: try to match by model or MAC in user-agent
                    let model_lower = resp.model.as_ref()
                        .map(|m| m.to_lowercase())
                        .unwrap_or_default();
                    let mac_clean = resp.mac_address.as_ref()
                        .map(|m| m.replace(':', "").replace('-', "").to_lowercase())
                        .unwrap_or_default();
                    contacts.iter().find(|(_, ua)| {
                        if let Some(ref agent) = ua {
                            let a = agent.to_lowercase();
                            // Model name in user-agent (e.g., "PolycomVVX-VVX_250-UA/...")
                            if !model_lower.is_empty() && a.contains(&model_lower) {
                                return true;
                            }
                            // Model starts with the UA brand (e.g., model="Zoiper" matches UA starting with "Z")
                            if !model_lower.is_empty() {
                                let ua_brand = a.split_whitespace().next().unwrap_or("");
                                if model_lower.starts_with(ua_brand) && !ua_brand.is_empty() {
                                    return true;
                                }
                            }
                            // MAC address in UA
                            if !mac_clean.is_empty() && a.contains(&mac_clean) {
                                return true;
                            }
                        }
                        false
                    }).or_else(|| {
                        // Fall back: match by known IP from previous registration
                        if let Some(ref known_ip) = resp.last_ip_address {
                            let known_host = known_ip.split(':').next().unwrap_or("");
                            contacts.iter().find(|(ip, _)| ip.starts_with(known_host))
                        } else {
                            None
                        }
                    })
                };

                tracing::info!(
                    device = %resp.name,
                    contacts = contacts.len(),
                    device_count = device_count,
                    best_found = best.is_some(),
                    model = ?resp.model,
                    "Device registration enrichment"
                );
                if let Some((ref ip, ref ua)) = best {
                    resp.registration_state = "registered".to_string();
                    if !ip.is_empty() {
                        resp.last_ip_address = Some(ip.clone());
                    }
                    if let Some(ref agent) = ua {
                        resp.user_agent = Some(agent.clone());
                    }
                }
            }
        }
        resp
    }).collect();

    info!("Listed {} devices", response.len());
    Ok(Json(response))
}

/// Extract IP:port from a SIP contact URI like "<sip:alice@192.168.1.80:5060;transport=tcp>"
fn extract_ip_from_contact(contact: &str) -> String {
    let stripped = contact
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_start_matches("sip:")
        .trim_start_matches("sips:");
    if let Some(at_pos) = stripped.find('@') {
        let host_part = &stripped[at_pos + 1..];
        host_part.split(';').next().unwrap_or(host_part).to_string()
    } else {
        stripped.split(';').next().unwrap_or(stripped).to_string()
    }
}

/// Get device by ID.
pub async fn get_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<DeviceResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let storage = require_storage!(state);

    let id = DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;

    let device = storage
        .get_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get device: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Device {} not found", id)))
        .and_then(|device| ensure_device_in_tenant(device, &tenant.0, &id))?;

    Ok(Json(device.into()))
}

/// Create device.
pub async fn create_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateDeviceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceCreate, state.deployment_mode)?;
    let storage = require_storage!(state);

    let device_type = parse_device_type(&req.device_type)?;

    let mut device = Device::new(req.name, device_type, tenant.0.clone());

    // Set optional fields
    if let Some(owner_id_str) = req.owner_user_id {
        let owner_user_id = UserId::from_str(&owner_id_str)
            .map_err(|e| ApiError::BadRequest(format!("Invalid user ID: {}", e)))?;
        let owner = storage
            .get_user_by_id(&owner_user_id)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to get owner user: {}", e)))?
            .ok_or_else(|| ApiError::BadRequest(format!("User {} not found", owner_user_id)))?;
        if owner.tenant_id != tenant.0 {
            return Err(ApiError::BadRequest(format!(
                "User {} is not in the authenticated tenant",
                owner_user_id
            )));
        }
        device.owner_user_id = Some(owner_user_id);
    }

    device.mac_address = req.mac_address.and_then(normalize_optional_string);
    device.model = req.model;
    device.sip_uri = req.sip_uri;

    if let Some(ref mac_address) = device.mac_address {
        let normalized_mac = normalize_mac_address(mac_address);
        ensure_mac_available(storage, &normalized_mac, None).await?;
    }

    let generated_provisioning_token = match req.provisioning_token.and_then(normalize_optional_string) {
        Some(token) => {
            let token = validate_device_provisioning_token(&state, &token)?;
            device.provisioning_token_hash = Some(hash_provisioning_token(&token));
            None
        }
        None if device.mac_address.is_some() => {
            let token = generate_provisioning_token();
            device.provisioning_token_hash = Some(hash_provisioning_token(&token));
            Some(token)
        }
        None => None,
    };

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
    if let Some(location_id_str) = req.location_id {
        let location_id = fcp_core::LocationId::from_str(&location_id_str)
            .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {}", e)))?;
        let location = storage
            .get_location(&location_id)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to get location: {}", e)))?
            .ok_or_else(|| ApiError::BadRequest(format!("Location {} not found", location_id)))?;
        if location.tenant_id != tenant.0 {
            return Err(ApiError::BadRequest(format!(
                "Location {} is not in the authenticated tenant",
                location_id
            )));
        }
        device.location_id = Some(location_id);
    }

    if let Some(e911) = req.e911_override {
        device.set_e911_override(e911.into());
    }

    // Store the device
    storage
        .put_device(device.clone())
        .await
        .map_err(|e| map_put_device_error("Failed to create device", e))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "device".to_string(),
            entity_id: device.id.to_string(),
            tenant_id: device.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

    info!("Created device {} ({})", device.id, device.name);

    let mut response = DeviceResponse::from(device);
    response.provisioning_token = generated_provisioning_token;
    Ok((StatusCode::CREATED, Json(response)))
}

/// Update device.
pub async fn update_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateDeviceRequest>,
) -> Result<Json<DeviceResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);

    let id = DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;

    // Get existing device
    let mut device = storage
        .get_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get device: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Device {} not found", id)))
        .and_then(|device| ensure_device_in_tenant(device, &tenant.0, &id))?;

    // Update fields if provided
    if let Some(name) = req.name {
        device.name = name;
    }

    if let Some(owner_id_str) = req.owner_user_id {
        let owner_user_id = UserId::from_str(&owner_id_str)
            .map_err(|e| ApiError::BadRequest(format!("Invalid user ID: {}", e)))?;
        let owner = storage
            .get_user_by_id(&owner_user_id)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to get owner user: {}", e)))?
            .ok_or_else(|| ApiError::BadRequest(format!("User {} not found", owner_user_id)))?;
        if owner.tenant_id != tenant.0 {
            return Err(ApiError::BadRequest(format!(
                "User {} is not in the authenticated tenant",
                owner_user_id
            )));
        }
        device.owner_user_id = Some(owner_user_id);
    }

    if let Some(mac_address) = req.mac_address {
        let mac_address = normalize_optional_string(mac_address);
        if let Some(ref mac_address) = mac_address {
            let normalized_mac = normalize_mac_address(mac_address);
            ensure_mac_available(storage, &normalized_mac, Some(&id)).await?;
        }
        device.mac_address = mac_address;
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

    if let Some(location_id_str) = req.location_id {
        device.location_id = if location_id_str.is_empty() {
            None
        } else {
            let location_id = fcp_core::LocationId::from_str(&location_id_str)
                .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {}", e)))?;
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
            Some(location_id)
        };
    }

    if let Some(e911) = req.e911_override {
        device.set_e911_override(e911.into());
    }

    if req.provisioning_url.is_some() {
        device.provisioning_url = req.provisioning_url;
    }

    if let Some(config_template_id_str) = req.config_template_id {
        device.config_template_id = if config_template_id_str.is_empty() {
            None
        } else {
            Some(
                validate_config_template_for_tenant(storage, &tenant.0, &config_template_id_str)
                    .await?,
            )
        };
    }

    if let Some(provisioning_token) = req.provisioning_token {
        device.provisioning_token_hash = if let Some(token) =
            normalize_optional_string(provisioning_token)
        {
            let token = validate_device_provisioning_token(&state, &token)?;
            Some(hash_provisioning_token(&token))
        } else {
            None
        };
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
        .map_err(|e| map_put_device_error("Failed to update device", e))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "device".to_string(),
            entity_id: device.id.to_string(),
            tenant_id: device.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

    info!("Updated device {} ({})", device.id, device.name);

    Ok(Json(device.into()))
}

/// Delete device.
pub async fn delete_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceDelete, state.deployment_mode)?;
    let storage = require_storage!(state);

    let id = DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;

    // Fetch device before deletion for event data
    let device = storage
        .get_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get device: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Device {} not found", id)))
        .and_then(|device| ensure_device_in_tenant(device, &tenant.0, &id))?;

    storage
        .delete_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete device: {}", e)))?;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "device".to_string(),
            entity_id: device.id.to_string(),
            tenant_id: device.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;

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
