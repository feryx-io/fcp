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
use fcp_core::{ConfigTemplateId, Device, DeviceId, DeviceType, HotelingStore, TenantId, UserId};
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
    /// The username of the device's own credentials; the password and its
    /// hash are never returned (`docs/ENTERPRISE_CALLING.md` §3.8).
    pub sip_username: Option<String>,
    pub has_sip_credentials: bool,
    /// A shared phone users may log in on.
    pub hoteling_host: bool,
    pub sip_uri: Option<String>,
    pub registration_state: String,
    #[serde(default, with = "crate::time::rfc3339_opt")]
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
    /// Whether this device is trusted to send its own location with a
    /// call (`docs/E911_BLOCK_2.md` §6.3); `null` follows its type.
    pub e911_trust_location: Option<bool>,
    pub provisioning_url: Option<String>,
    pub config_template_id: Option<String>,
    pub has_provisioning_token: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioning_token: Option<String>,
    pub enabled: bool,
    #[serde(with = "crate::time::rfc3339")]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    pub updated_at: SystemTime,
}

impl From<Device> for DeviceResponse {
    fn from(device: Device) -> Self {
        let has_sip_credentials = device.has_own_credentials();
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
            has_sip_credentials,
            sip_username: device.sip_username,
            hoteling_host: device.hoteling_host,
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
            e911_trust_location: device.e911_trust_location,
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
    /// Credentials of the device's own (§3.8): both, or neither.
    pub sip_username: Option<String>,
    pub sip_password: Option<String>,
    /// A shared phone users may log in on.
    pub hoteling_host: Option<bool>,
    pub mac_address: Option<String>,
    pub model: Option<String>,
    pub sip_uri: Option<String>,
    pub max_lines: Option<u8>,
    pub supports_video: Option<bool>,
    pub supports_blf: Option<bool>,
    pub location: Option<String>,
    pub location_id: Option<String>,
    pub e911_override: Option<E911AddressResponse>,
    /// Trust this device's own location (`docs/E911_BLOCK_2.md` §6.3).
    pub e911_trust_location: Option<bool>,
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

fn validate_device_provisioning_token(state: &ApiState, token: &str) -> Result<String, ApiError> {
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

/// The owner a request names, checked to be the tenant's.
async fn owner_in_tenant(
    storage: &fcp_core::DynStorage,
    tenant_id: &TenantId,
    owner_id: &str,
) -> Result<UserId, ApiError> {
    let owner_user_id = UserId::from_str(owner_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid user ID: {}", e)))?;
    let owner = storage
        .get_user_by_id(&owner_user_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get owner user: {}", e)))?
        .ok_or_else(|| ApiError::BadRequest(format!("User {} not found", owner_user_id)))?;
    if owner.tenant_id != *tenant_id {
        return Err(ApiError::BadRequest(format!(
            "User {} is not in the authenticated tenant",
            owner_user_id
        )));
    }
    Ok(owner_user_id)
}

/// Set, change or remove a device's own credentials
/// (`docs/ENTERPRISE_CALLING.md` §3.8). A username is the tenant's alone:
/// no user and no other device may be called by it over SIP.
async fn apply_sip_credentials(
    storage: &fcp_core::DynStorage,
    realm: &str,
    device: &mut Device,
    username: Option<Option<String>>,
    password: Option<String>,
) -> Result<(), ApiError> {
    match username.map(|u| u.and_then(normalize_optional_string)) {
        Some(None) => {
            if password.is_some() {
                return Err(ApiError::BadRequest(
                    "sip_password needs a sip_username".to_string(),
                ));
            }
            device.sip_username = None;
            device.sip_ha1_hash = None;
            device.sip_provisioning_password = None;
            return Ok(());
        }
        Some(Some(username)) => {
            validation::validate_username(&username)?;
            let changed = !device
                .sip_username
                .as_deref()
                .is_some_and(|u| u.eq_ignore_ascii_case(&username));
            if changed {
                if password.is_none() {
                    return Err(ApiError::BadRequest(
                        "a new sip_username needs a sip_password".to_string(),
                    ));
                }
                if storage
                    .get_user_in_tenant(&username, &device.tenant_id)
                    .await
                    .map_err(|e| ApiError::Internal(format!("Failed to check users: {}", e)))?
                    .is_some()
                {
                    return Err(ApiError::Conflict(format!(
                        "sip_username `{username}` is a user's username"
                    )));
                }
                if storage
                    .get_device_by_sip_name(&device.tenant_id, &username)
                    .await
                    .map_err(|e| ApiError::Internal(format!("Failed to check devices: {}", e)))?
                    .is_some_and(|other| other.id != device.id)
                {
                    return Err(ApiError::Conflict(format!(
                        "sip_username `{username}` is another device's name"
                    )));
                }
            }
            device.sip_username = Some(username);
        }
        None => {}
    }
    if let Some(password) = password {
        validation::reject_crlf("sip_password", &password)?;
        if password.len() < 8 {
            return Err(ApiError::BadRequest(
                "sip_password must be at least 8 characters".to_string(),
            ));
        }
        let username = device
            .sip_username
            .clone()
            .ok_or_else(|| ApiError::BadRequest("sip_password needs a sip_username".to_string()))?;
        device.sip_ha1_hash = Some(fcp_core::storage::password::compute_ha1(
            &username, realm, &password,
        ));
        device.sip_provisioning_password = Some(password);
    }
    Ok(())
}

/// Update device request.
#[derive(Debug, Deserialize)]
pub struct UpdateDeviceRequest {
    pub name: Option<String>,
    /// `null` (or an empty string) leaves the device without an owner.
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub owner_user_id: Option<Option<String>>,
    /// The username of the device's own credentials; `null` removes them.
    /// A new username needs `sip_password` too, since the digest hash is
    /// made from both.
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub sip_username: Option<Option<String>>,
    pub sip_password: Option<String>,
    pub hoteling_host: Option<bool>,
    pub mac_address: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub sip_uri: Option<String>,
    pub max_lines: Option<u8>,
    pub supports_video: Option<bool>,
    pub supports_blf: Option<bool>,
    pub location: Option<String>,
    pub location_id: Option<String>,
    /// The address for this phone alone. `null` removes it, so a phone
    /// that goes back to its site stops carrying where it used to be
    /// (`docs/E911.md`).
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub e911_override: Option<Option<E911AddressResponse>>,
    /// Trust this device's own location (`docs/E911_BLOCK_2.md` §6.3).
    /// `null` returns it to whatever the tenant trusts its device type with.
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub e911_trust_location: Option<Option<bool>>,
    pub provisioning_url: Option<String>,
    pub config_template_id: Option<String>,
    /// Set or rotate the per-device provisioning token. An empty string clears it.
    pub provisioning_token: Option<String>,
    pub enabled: Option<bool>,
}

/// Tell "absent" from "present and null", so a field can be cleared.
fn deserialize_double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    use serde::Deserialize;
    Option::<T>::deserialize(d).map(Some)
}

/// List devices.
pub async fn list_devices(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
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
                        .map(|m| m.replace([':', '-'], "").to_lowercase())
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
    Ok(Json(crate::page::paginate(
        response,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.name, r.id),
    )?))
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

    let mut device = Device::new(req.name, device_type, tenant.0);

    // Set optional fields
    if let Some(owner_id_str) = req.owner_user_id.and_then(normalize_optional_string) {
        device.owner_user_id = Some(owner_in_tenant(storage, &tenant.0, &owner_id_str).await?);
    }

    device.mac_address = req.mac_address.and_then(normalize_optional_string);
    device.model = req.model;
    device.sip_uri = req.sip_uri;
    if req.sip_username.is_some() || req.sip_password.is_some() {
        apply_sip_credentials(
            storage,
            &state.realm,
            &mut device,
            Some(req.sip_username),
            req.sip_password,
        )
        .await?;
    }
    device.hoteling_host = req.hoteling_host.unwrap_or(false);

    if let Some(ref mac_address) = device.mac_address {
        let normalized_mac = normalize_mac_address(mac_address);
        ensure_mac_available(storage, &normalized_mac, None).await?;
    }

    let generated_provisioning_token =
        match req.provisioning_token.and_then(normalize_optional_string) {
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

    if req.e911_trust_location.is_some() {
        device.e911_trust_location = req.e911_trust_location;
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

    if let Some(owner) = req.owner_user_id {
        device.owner_user_id = match owner.and_then(normalize_optional_string) {
            Some(owner_id_str) => Some(owner_in_tenant(storage, &tenant.0, &owner_id_str).await?),
            None => None,
        };
    }

    if req.sip_username.is_some() || req.sip_password.is_some() {
        apply_sip_credentials(
            storage,
            &state.realm,
            &mut device,
            req.sip_username,
            req.sip_password,
        )
        .await?;
    }

    // A phone that stops being a host sends its guest home.
    let hosting_ended = req.hoteling_host == Some(false) && device.hoteling_host;
    if let Some(host) = req.hoteling_host {
        device.hoteling_host = host;
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
        match e911 {
            // Removing an override puts the phone back on its site's
            // address; a stale one would dispatch to where it used to be.
            None => device.e911_override = None,
            Some(address) => device.set_e911_override(address.into()),
        }
    }

    if let Some(trust) = req.e911_trust_location {
        device.e911_trust_location = trust;
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
        device.provisioning_token_hash =
            if let Some(token) = normalize_optional_string(provisioning_token) {
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

    if hosting_ended {
        if let Some(session) = storage
            .hoteling_session_for_device(&device.id)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to read the host's session: {}", e)))?
        {
            crate::v1::hoteling::end_session(
                &state,
                storage,
                &session,
                fcp_core::SessionEnd::Admin,
            )
            .await?;
        }
    }

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
