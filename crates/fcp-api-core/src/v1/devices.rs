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

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// Device response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeviceResponse {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub mac_address: Option<String>,
    pub model: Option<String>,
    /// `yealink`, `polycom`, `grandstream`, `cisco`, `fanvil` or `snom`:
    /// with the model, it picks the template when the device names none.
    pub vendor: Option<String>,
    pub firmware_version: Option<String>,
    /// The version its tenant's firmware target points it at
    /// (`docs/PROVISIONING.md` §11), and whether it runs it: `current`,
    /// `behind`, `unknown` (it has not said). Unset without a target.
    pub firmware_target_version: Option<String>,
    pub firmware_status: Option<fcp_core::firmware::FirmwareStatus>,
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
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_registration: Option<SystemTime>,
    pub last_ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub max_lines: u8,
    pub supports_video: bool,
    pub supports_blf: bool,
    pub codec_support: Vec<String>,
    /// The keys programmed on the phone, in order.
    pub line_keys: Vec<fcp_core::LineKey>,
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
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
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
            vendor: device.vendor.map(|v| v.to_string()),
            firmware_version: device.firmware_version,
            firmware_target_version: None,
            firmware_status: None,
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
            line_keys: device.line_keys,
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
#[derive(Debug, Deserialize, utoipa::ToSchema)]
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
    /// The phone's vendor (`docs/PROVISIONING.md` §5); unset, read from
    /// the MAC where its prefix is a vendor's.
    pub vendor: Option<String>,
    pub sip_uri: Option<String>,
    pub max_lines: Option<u8>,
    pub supports_video: Option<bool>,
    pub supports_blf: Option<bool>,
    /// The keys programmed on the phone.
    pub line_keys: Option<Vec<fcp_core::LineKey>>,
    pub config_template_id: Option<String>,
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

/// A phone's vendor as a request names it; empty is none.
fn parse_vendor(value: &str) -> Result<Option<fcp_core::config_template::PhoneVendor>, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let vendor: fcp_core::config_template::PhoneVendor =
        value.parse().map_err(|e: String| ApiError::BadRequest(e))?;
    if !fcp_core::config_template::PhoneVendor::PHONES.contains(&vendor) {
        return Err(ApiError::BadRequest(format!(
            "vendor `{value}` is not a phone's vendor"
        )));
    }
    Ok(Some(vendor))
}

fn checked_line_keys(keys: Vec<fcp_core::LineKey>) -> Result<Vec<fcp_core::LineKey>, ApiError> {
    let keys: Vec<fcp_core::LineKey> = keys
        .into_iter()
        .map(|mut k| {
            k.value = k.value.trim().to_string();
            k.label = k.label.and_then(normalize_optional_string);
            k
        })
        .collect();
    fcp_core::device::validate_line_keys(&keys).map_err(ApiError::BadRequest)?;
    Ok(keys)
}

fn hash_provisioning_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// A device's provisioning token: 32 characters (`fcp_` and 28 random
/// letters and digits, ~166 bits), the most Yealink RPS keeps as the
/// password it hands a phone (`docs/PROVISIONING.md` §10).
pub(crate) fn generate_provisioning_token() -> String {
    use rand::Rng;
    let random: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(28)
        .map(char::from)
        .collect();
    format!("fcp_{random}")
}

pub(crate) fn hash_device_token(token: &str) -> String {
    hash_provisioning_token(token)
}

fn validate_device_provisioning_token(token: &str) -> Result<String, ApiError> {
    validation::reject_crlf("provisioning_token", token)?;
    let token = token.trim();
    if token.len() < 32 {
        return Err(ApiError::BadRequest(
            "provisioning_token must be at least 32 characters".to_string(),
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
                    .get_user_by_sip_name(&username, &device.tenant_id)
                    .await
                    .map_err(|e| ApiError::Internal(format!("Failed to check users: {}", e)))?
                    .is_some()
                {
                    return Err(ApiError::Conflict(format!(
                        "sip_username `{username}` is a user's SIP name"
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
#[derive(Debug, Deserialize, utoipa::ToSchema)]
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
    /// The phone's vendor; an empty string clears it.
    pub vendor: Option<String>,
    pub firmware_version: Option<String>,
    pub sip_uri: Option<String>,
    pub max_lines: Option<u8>,
    pub supports_video: Option<bool>,
    pub supports_blf: Option<bool>,
    /// The keys programmed on the phone, replacing what it had.
    pub line_keys: Option<Vec<fcp_core::LineKey>>,
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
#[utoipa::path(
    get,
    path = "/api/v1/devices",
    tag = "devices",
    operation_id = "devices_list_devices",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<DeviceResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

    let (targets, images) = firmware_lists(storage, &tenant_id).await;
    let response: Vec<DeviceResponse> = devices.into_iter().map(|d| {
        let firmware = crate::v1::firmware::status_of(&d, &targets, &images);
        let mut resp = DeviceResponse::from(d);
        if let Some((version, status)) = firmware {
            resp.firmware_target_version = Some(version);
            resp.firmware_status = Some(status);
        }
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
        confirm_registration(&mut resp, &live_regs, now);
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

/// A device the registrar wrote as registered (`docs/PROVISIONING.md` §8)
/// is shown so only while a live registration is its: one whose
/// User-Agent carries its MAC, or whose contact is its last address. A
/// phone that went away without unregistering is shown `expired`.
fn confirm_registration(
    resp: &mut DeviceResponse,
    live: &[fcp_core::storage::Registration],
    now: SystemTime,
) {
    if resp.registration_state != "registered" {
        return;
    }
    let mac = resp
        .mac_address
        .as_deref()
        .and_then(fcp_core::provisioning::mac_of);
    let mine = live.iter().filter(|r| r.expires_at > now).find(|r| {
        let by_mac = mac.as_ref().is_some_and(|m| {
            r.user_agent.as_deref().is_some_and(|ua| {
                ua.to_ascii_lowercase()
                    .replace([':', '-'], "")
                    .contains(m.as_str())
            })
        });
        let by_address = resp
            .last_ip_address
            .as_deref()
            .is_some_and(|a| extract_ip_from_contact(&r.contact) == a);
        by_mac || by_address
    });
    match mine {
        Some(r) => {
            resp.last_ip_address = Some(extract_ip_from_contact(&r.contact));
            if r.user_agent.is_some() {
                resp.user_agent = r.user_agent.clone();
            }
        }
        None => resp.registration_state = "expired".to_string(),
    }
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
#[utoipa::path(
    get,
    path = "/api/v1/devices/{id}",
    tag = "devices",
    operation_id = "devices_get_device",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = DeviceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

    let (targets, images) = firmware_lists(storage, &tenant.0).await;
    let firmware = crate::v1::firmware::status_of(&device, &targets, &images);
    let mut response = DeviceResponse::from(device);
    if let Some((version, status)) = firmware {
        response.firmware_target_version = Some(version);
        response.firmware_status = Some(status);
    }
    if response.registration_state == "registered" {
        let live = storage
            .get_all_registrations_in_tenant(&tenant.0)
            .await
            .unwrap_or_default();
        confirm_registration(&mut response, &live, SystemTime::now());
    }
    Ok(Json(response))
}

/// Create device.
#[utoipa::path(
    post,
    path = "/api/v1/devices",
    tag = "devices",
    operation_id = "devices_create_device",
    request_body = CreateDeviceRequest,
    responses(
        (status = 201, description = "OK", body = DeviceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: axum::http::HeaderMap,
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
    device.vendor = match req.vendor {
        Some(v) => parse_vendor(&v)?,
        None => device
            .mac_address
            .as_deref()
            .and_then(fcp_core::provisioning::vendor_of_mac),
    };
    if let Some(keys) = req.line_keys {
        device.line_keys = checked_line_keys(keys)?;
    }
    if let Some(template) = req.config_template_id.and_then(normalize_optional_string) {
        device.config_template_id =
            Some(validate_config_template_for_tenant(storage, &tenant.0, &template).await?);
    }
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

    let mut token_in_hand = None;
    let generated_provisioning_token =
        match req.provisioning_token.and_then(normalize_optional_string) {
            Some(token) => {
                let token = validate_device_provisioning_token(&token)?;
                device.provisioning_token_hash = Some(hash_provisioning_token(&token));
                token_in_hand = Some(token);
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
    forget_pending(storage, &device).await;
    crate::v1::redirection::device_changed(
        &state,
        &device,
        &headers,
        token_in_hand.or_else(|| generated_provisioning_token.clone()),
    );

    let mut response = DeviceResponse::from(device);
    response.provisioning_token = generated_provisioning_token;
    Ok((StatusCode::CREATED, Json(response)))
}

/// Update device.
#[utoipa::path(
    patch,
    path = "/api/v1/devices/{id}",
    tag = "devices",
    operation_id = "devices_update_device",
    params(("id" = String, Path)),
    request_body = UpdateDeviceRequest,
    responses(
        (status = 200, description = "OK", body = DeviceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
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

    if let Some(vendor) = req.vendor {
        device.vendor = parse_vendor(&vendor)?;
    }

    if let Some(keys) = req.line_keys {
        device.line_keys = checked_line_keys(keys)?;
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

    let mut token_in_hand = None;
    if let Some(provisioning_token) = req.provisioning_token {
        device.provisioning_token_hash =
            if let Some(token) = normalize_optional_string(provisioning_token) {
                let token = validate_device_provisioning_token(&token)?;
                let hash = hash_provisioning_token(&token);
                token_in_hand = Some(token);
                Some(hash)
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
    forget_pending(storage, &device).await;
    crate::v1::redirection::device_changed(&state, &device, &headers, token_in_hand);

    Ok(Json(device.into()))
}

/// Delete device.
#[utoipa::path(
    delete,
    path = "/api/v1/devices/{id}",
    tag = "devices",
    operation_id = "devices_delete_device",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

    remove_device(&state, storage, &device).await?;
    info!("Deleted device {}", id);
    Ok(StatusCode::NO_CONTENT)
}

/// Remove a device, and everything it was signed in with: its app's
/// sessions end (`docs/SOFTPHONE.md` §2.1), its own registrations are
/// dropped, caches hear, and its redirection service forgets it.
pub(crate) async fn remove_device(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    device: &Device,
) -> Result<(), ApiError> {
    let now = SystemTime::now();
    if let Some(sessions) = storage.login_session_store() {
        let ended = sessions
            .revoke_device_sessions(&device.id, "the device was removed", now)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to end the device's sessions: {e}")))?;
        if ended > 0 {
            info!(device = %device.id, ended, "Ended a removed device's app sessions");
        }
    }
    drop_device_registrations(state, storage, device).await;
    storage
        .delete_device(&device.id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete device: {}", e)))?;
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "device".to_string(),
            entity_id: device.id.to_string(),
            tenant_id: device.tenant_id,
            timestamp: now,
        }))
        .await;
    crate::v1::redirection::device_removed(state, device);
    Ok(())
}

/// Drop the registrations a device made: those at the contact its last
/// registration came from, under its owner's names or its own.
async fn drop_device_registrations(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    device: &Device,
) {
    let Some(address) = device.last_ip_address.as_deref().filter(|a| !a.is_empty()) else {
        return;
    };
    let mut names: Vec<String> = device
        .sip_username
        .iter()
        .map(|n| n.to_lowercase())
        .collect();
    if let Some(owner) = device.owner_user_id {
        if let Ok(Some(user)) = storage.get_user_by_id(&owner).await {
            names.push(user.username.to_lowercase());
            names.push(user.sip_name().to_lowercase());
            if let Some(ext) = user.extension.as_deref().filter(|e| !e.is_empty()) {
                names.push(ext.to_lowercase());
            }
        }
    }
    let Ok(registrations) = storage
        .get_all_registrations_in_tenant(&device.tenant_id)
        .await
    else {
        return;
    };
    let at_address = |contact: &str| {
        contact
            .split(['@', '>', ';'])
            .map(|part| {
                part.trim()
                    .trim_start_matches('<')
                    .trim_start_matches("sips:")
                    .trim_start_matches("sip:")
            })
            .any(|part| part == address)
    };
    for registration in registrations {
        let user_part = registration
            .aor
            .trim_start_matches("sips:")
            .trim_start_matches("sip:")
            .split('@')
            .next()
            .unwrap_or_default()
            .to_lowercase();
        if !names.contains(&user_part) || !at_address(&registration.contact) {
            continue;
        }
        if storage
            .delete_registration_in_tenant(
                &device.tenant_id,
                &registration.aor,
                &registration.contact,
            )
            .await
            .is_ok()
        {
            crate::v1::registrations::announce_removed(
                state,
                device.tenant_id,
                &registration.aor,
                &registration.contact,
            )
            .await;
        }
    }
}

/// The tenant's firmware targets and the images they name, for the device
/// views; empty where firmware is not kept.
async fn firmware_lists(
    storage: &fcp_core::DynStorage,
    tenant: &TenantId,
) -> (
    Vec<fcp_core::firmware::FirmwareTarget>,
    Vec<fcp_core::firmware::FirmwareImage>,
) {
    let Some(store) = storage.provisioning_store() else {
        return (Vec::new(), Vec::new());
    };
    let targets = store
        .list_firmware_targets(tenant)
        .await
        .unwrap_or_default();
    if targets.is_empty() {
        return (targets, Vec::new());
    }
    (
        targets,
        store.list_firmware_images().await.unwrap_or_default(),
    )
}

/// A device with a MAC is no longer a pending phone.
async fn forget_pending(storage: &fcp_core::DynStorage, device: &Device) {
    let (Some(store), Some(mac)) = (
        storage.provisioning_store(),
        device
            .mac_address
            .as_deref()
            .and_then(fcp_core::provisioning::mac_of),
    ) else {
        return;
    };
    if let Err(e) = store.remove_pending_device(&device.tenant_id, &mac).await {
        tracing::warn!(error = %e, "Could not forget a pending phone");
    }
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

// ---- Pending phones, import and preview (`docs/PROVISIONING.md` §5–§7) ----

fn provisioning_store(
    storage: &fcp_core::DynStorage,
) -> Result<&std::sync::Arc<dyn fcp_core::provisioning::ProvisioningStore>, ApiError> {
    storage
        .provisioning_store()
        .ok_or_else(|| ApiError::NotFound("Provisioning is not available here".to_string()))
}

async fn publish_device_changed(state: &ApiState, device: &Device) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "device".to_string(),
            entity_id: device.id.to_string(),
            tenant_id: device.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// A phone that fetched with a MAC no device has.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PendingDeviceResponse {
    pub mac_address: String,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub first_seen: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub last_seen: SystemTime,
    pub address: Option<String>,
    pub user_agent: Option<String>,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub fetches: u32,
}

impl From<fcp_core::provisioning::PendingDevice> for PendingDeviceResponse {
    fn from(p: fcp_core::provisioning::PendingDevice) -> Self {
        Self {
            mac_address: p.mac_address,
            first_seen: p.first_seen,
            last_seen: p.last_seen,
            address: p.address,
            user_agent: p.user_agent,
            vendor: p.vendor.map(|v| v.to_string()),
            model: p.model,
            firmware: p.firmware,
            fetches: p.fetches,
        }
    }
}

/// GET /api/v1/provisioning/pending — phones waiting to be claimed, the
/// most recently seen first.
#[utoipa::path(
    get,
    path = "/api/v1/provisioning/pending",
    tag = "provisioning",
    operation_id = "provisioning_list_pending",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<PendingDeviceResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_pending(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let pending: Vec<PendingDeviceResponse> = provisioning_store(storage)?
        .list_pending_devices(&tenant.0)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(crate::page::paginate(
        pending,
        &page,
        crate::page::Order::Descending,
        |p| {
            format!(
                "{:020}/{}",
                p.last_seen
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros(),
                p.mac_address
            )
        },
    )?))
}

/// What a claimed or imported phone becomes.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct ClaimDeviceRequest {
    /// Unset: its vendor, model and MAC.
    pub name: Option<String>,
    /// Unset: `desk_phone`.
    pub device_type: Option<String>,
    /// Its owner, by id or by username.
    pub owner_user_id: Option<String>,
    pub owner_username: Option<String>,
    /// Unset: picked by vendor and model when the phone fetches.
    pub config_template_id: Option<String>,
    pub location_id: Option<String>,
}

/// One phone to import.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ImportDeviceRow {
    pub mac_address: String,
    pub name: Option<String>,
    pub model: Option<String>,
    /// Unset: read from the MAC's prefix where it is a vendor's.
    pub vendor: Option<String>,
    pub device_type: Option<String>,
    pub owner_user_id: Option<String>,
    pub owner_username: Option<String>,
    pub config_template_id: Option<String>,
    pub location_id: Option<String>,
}

/// Phones by MAC, all made or none.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ImportDevicesRequest {
    pub devices: Vec<ImportDeviceRow>,
}

/// What an import made: each device with its provisioning token, shown
/// this once.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ImportDevicesResponse {
    pub created: Vec<DeviceResponse>,
}

/// The most phones one import makes.
const MAX_IMPORT: usize = 1000;

/// A device for a MAC, checked against the tenant: nothing stored.
#[allow(clippy::too_many_arguments)]
async fn device_for_mac(
    storage: &fcp_core::DynStorage,
    tenant_id: &TenantId,
    mac: &str,
    name: Option<String>,
    device_type: Option<String>,
    model: Option<String>,
    vendor: Option<fcp_core::config_template::PhoneVendor>,
    owner_user_id: Option<String>,
    owner_username: Option<String>,
    config_template_id: Option<String>,
    location_id: Option<String>,
) -> Result<(Device, String), ApiError> {
    let mac = fcp_core::provisioning::mac_of(mac)
        .ok_or_else(|| ApiError::BadRequest(format!("`{mac}` is not a MAC address")))?;
    ensure_mac_available(storage, &mac, None).await?;
    let device_type = match device_type.and_then(normalize_optional_string) {
        Some(t) => parse_device_type(&t)?,
        None => DeviceType::DeskPhone,
    };
    let model = model.and_then(normalize_optional_string);
    let vendor = vendor.or_else(|| fcp_core::provisioning::vendor_of_mac(&mac));
    let name = name.and_then(normalize_optional_string).unwrap_or_else(|| {
        [
            vendor.map(|v| {
                let v = v.to_string();
                let mut c = v.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            }),
            model.clone(),
            Some(mac.clone()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
    });
    let mut device = Device::new(name, device_type, *tenant_id);
    device.mac_address = Some(mac.clone());
    device.model = model;
    device.vendor = vendor;
    device.owner_user_id = match (
        owner_user_id.and_then(normalize_optional_string),
        owner_username.and_then(normalize_optional_string),
    ) {
        (Some(id), _) => Some(owner_in_tenant(storage, tenant_id, &id).await?),
        (None, Some(username)) => Some(
            storage
                .get_user_in_tenant(&username, tenant_id)
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to get user: {e}")))?
                .ok_or_else(|| ApiError::BadRequest(format!("User `{username}` not found")))?
                .id,
        ),
        (None, None) => None,
    };
    if let Some(template) = config_template_id.and_then(normalize_optional_string) {
        device.config_template_id =
            Some(validate_config_template_for_tenant(storage, tenant_id, &template).await?);
    }
    if let Some(location) = location_id.and_then(normalize_optional_string) {
        let location_id = fcp_core::LocationId::from_str(&location)
            .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {e}")))?;
        storage
            .get_location(&location_id)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to get location: {e}")))?
            .filter(|l| l.tenant_id == *tenant_id)
            .ok_or_else(|| ApiError::BadRequest(format!("Location {location_id} not found")))?;
        device.location_id = Some(location_id);
    }
    let token = generate_provisioning_token();
    device.provisioning_token_hash = Some(hash_provisioning_token(&token));
    Ok((device, token))
}

/// POST /api/v1/provisioning/pending/:mac/claim — make the pending phone a
/// device in one step: its vendor and model as it gave them, an owner, a
/// template if one is named.
#[utoipa::path(
    post,
    path = "/api/v1/provisioning/pending/{mac}/claim",
    tag = "provisioning",
    operation_id = "provisioning_claim_pending",
    params(("mac" = String, Path)),
    request_body = ClaimDeviceRequest,
    responses(
        (status = 201, description = "The device, with its provisioning token shown once", body = DeviceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn claim_pending(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: axum::http::HeaderMap,
    Path(mac): Path<String>,
    body: Option<Json<ClaimDeviceRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceCreate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let store = provisioning_store(storage)?;
    let normalized = fcp_core::provisioning::mac_of(&mac)
        .ok_or_else(|| ApiError::BadRequest(format!("`{mac}` is not a MAC address")))?;
    let pending = store
        .get_pending_device(&tenant.0, &normalized)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("No pending phone {normalized}")))?;
    let (mut device, token) = device_for_mac(
        storage,
        &tenant.0,
        &normalized,
        req.name,
        req.device_type,
        pending.model.clone(),
        pending.vendor,
        req.owner_user_id,
        req.owner_username,
        req.config_template_id,
        req.location_id,
    )
    .await?;
    device.firmware_version = pending.firmware.clone();
    storage
        .put_device(device.clone())
        .await
        .map_err(|e| map_put_device_error("Failed to create device", e))?;
    store.remove_pending_device(&tenant.0, &normalized).await?;
    publish_device_changed(&state, &device).await;
    crate::v1::redirection::device_changed(&state, &device, &headers, Some(token.clone()));
    info!(device_id = %device.id, mac = %normalized, "Claimed a pending phone");
    let mut response = DeviceResponse::from(device);
    response.provisioning_token = Some(token);
    Ok((StatusCode::CREATED, Json(response)))
}

/// DELETE /api/v1/provisioning/pending/:mac — forget a pending phone; it
/// comes back if it fetches again.
#[utoipa::path(
    delete,
    path = "/api/v1/provisioning/pending/{mac}",
    tag = "provisioning",
    operation_id = "provisioning_dismiss_pending",
    params(("mac" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn dismiss_pending(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(mac): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceDelete, state.deployment_mode)?;
    let storage = require_storage!(state);
    let normalized = fcp_core::provisioning::mac_of(&mac)
        .ok_or_else(|| ApiError::BadRequest(format!("`{mac}` is not a MAC address")))?;
    if !provisioning_store(storage)?
        .remove_pending_device(&tenant.0, &normalized)
        .await?
    {
        return Err(ApiError::NotFound(format!("No pending phone {normalized}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/devices/import — phones by MAC, each checked before any is
/// made: all are made, or none and every row's fault is named.
#[utoipa::path(
    post,
    path = "/api/v1/devices/import",
    tag = "devices",
    operation_id = "devices_import_devices",
    request_body = ImportDevicesRequest,
    responses(
        (status = 201, description = "Made", body = ImportDevicesResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn import_devices(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: axum::http::HeaderMap,
    Json(req): Json<ImportDevicesRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceCreate, state.deployment_mode)?;
    let storage = require_storage!(state);
    if req.devices.is_empty() {
        return Err(ApiError::BadRequest("No devices to import".to_string()));
    }
    if req.devices.len() > MAX_IMPORT {
        return Err(ApiError::BadRequest(format!(
            "At most {MAX_IMPORT} devices at once"
        )));
    }
    let mut made = Vec::with_capacity(req.devices.len());
    let mut faults = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, row) in req.devices.into_iter().enumerate() {
        let n = i + 1;
        let checked = async {
            let vendor = match row.vendor.as_deref() {
                Some(v) => parse_vendor(v)?,
                None => None,
            };
            let mac = fcp_core::provisioning::mac_of(&row.mac_address).ok_or_else(|| {
                ApiError::BadRequest(format!("`{}` is not a MAC address", row.mac_address))
            })?;
            if !seen.insert(mac.clone()) {
                return Err(ApiError::BadRequest(format!("{mac} is in the file twice")));
            }
            device_for_mac(
                storage,
                &tenant.0,
                &mac,
                row.name,
                row.device_type,
                row.model,
                vendor,
                row.owner_user_id,
                row.owner_username,
                row.config_template_id,
                row.location_id,
            )
            .await
        }
        .await;
        match checked {
            Ok(device) => made.push(device),
            Err(e) => faults.push(format!("row {n}: {}", e.message())),
        }
    }
    if !faults.is_empty() {
        let shown = faults.len().min(20);
        let more = faults.len() - shown;
        let mut message = format!(
            "Nothing imported; {} row(s) have faults: {}",
            faults.len(),
            faults[..shown].join("; ")
        );
        if more > 0 {
            message.push_str(&format!("; and {more} more"));
        }
        return Err(ApiError::BadRequest(message));
    }
    let store = storage.provisioning_store();
    let mut created = Vec::with_capacity(made.len());
    for (device, token) in made {
        storage
            .put_device(device.clone())
            .await
            .map_err(|e| map_put_device_error("Failed to import device", e))?;
        if let (Some(store), Some(mac)) = (store, device.mac_address.as_deref()) {
            let _ = store.remove_pending_device(&tenant.0, mac).await;
        }
        publish_device_changed(&state, &device).await;
        crate::v1::redirection::device_changed(&state, &device, &headers, Some(token.clone()));
        let mut response = DeviceResponse::from(device);
        response.provisioning_token = Some(token);
        created.push(response);
    }
    info!(count = created.len(), "Imported devices");
    Ok((StatusCode::CREATED, Json(ImportDevicesResponse { created })))
}

/// A template to try, in place of the device's.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct RenderDeviceRequest {
    /// One of the tenant's templates.
    pub config_template_id: Option<String>,
    /// A body not yet saved, rendered as `vendor`'s.
    pub template_body: Option<String>,
    pub vendor: Option<String>,
}

/// A rendered configuration.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RenderedConfigResponse {
    /// The template it came from, when it is a saved one.
    pub config_template_id: Option<String>,
    pub config_template_name: Option<String>,
    pub vendor: String,
    pub content: String,
}

/// POST /api/v1/devices/:id/render — the device's configuration as its
/// phone would get it now (credentials included), from its template or
/// the one given.
#[utoipa::path(
    post,
    path = "/api/v1/devices/{id}/render",
    tag = "provisioning",
    operation_id = "provisioning_render_device",
    params(("id" = String, Path)),
    request_body = RenderDeviceRequest,
    responses(
        (status = 200, description = "OK", body = RenderedConfigResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn render_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<RenderDeviceRequest>>,
) -> Result<Json<RenderedConfigResponse>, ApiError> {
    use crate::v1::provisioning as prov;
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let id = DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;
    let mut device = storage
        .get_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get device: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Device {} not found", id)))
        .and_then(|device| ensure_device_in_tenant(device, &tenant.0, &id))?;
    // A preview of a disabled phone still shows what it would get.
    device.enabled = true;
    let hint = fcp_core::provisioning::PhoneIdentity::default();
    let (body, vendor, template) = match (req.template_body, req.config_template_id) {
        (Some(body), _) => {
            let vendor = match req.vendor.as_deref() {
                Some(v) => parse_vendor(v)?,
                None => device.vendor,
            }
            .unwrap_or(fcp_core::config_template::PhoneVendor::Generic);
            (body, vendor, None)
        }
        (None, Some(template_id)) => {
            device.config_template_id =
                Some(validate_config_template_for_tenant(storage, &tenant.0, &template_id).await?);
            let t = prov::choose_template(storage, &device, &hint)
                .await?
                .ok_or_else(|| ApiError::NotFound("Template not found".to_string()))?;
            (t.template_body.clone(), t.vendor, Some(t))
        }
        (None, None) => {
            let t = prov::choose_template(storage, &device, &hint)
                .await?
                .ok_or_else(|| {
                    ApiError::NotFound(
                        "No template for this device: it names none and none matches its vendor and model"
                            .to_string(),
                    )
                })?;
            (t.template_body.clone(), t.vendor, Some(t))
        }
    };
    let mac = device
        .mac_address
        .as_deref()
        .and_then(fcp_core::provisioning::mac_of)
        .unwrap_or_default();
    let base = prov::public_base(&state, &headers);
    let context = prov::device_context(&state, storage, &device, mac, &hint, base.as_deref(), None)
        .await
        .map_err(|e| match e {
            ApiError::NotFound(_) => ApiError::BadRequest(
                "The device's owner may not use services, or has no SIP credentials".to_string(),
            ),
            other => other,
        })?;
    let content = fcp_core::config_template::render(&body, vendor, &context)
        .map_err(|e| ApiError::BadRequest(format!("The template fails: {e}")))?;
    Ok(Json(RenderedConfigResponse {
        config_template_id: template.as_ref().map(|t| t.id.to_string()),
        config_template_name: template.map(|t| t.name),
        vendor: vendor.to_string(),
        content,
    }))
}

/// What a resync did.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResyncResponse {
    pub device_id: String,
    /// The phone's registered contacts sent the NOTIFY: 0 when it is not
    /// registered.
    pub sent: u64,
}

/// Ask a call manager to resync one device: what it sent.
pub(crate) async fn resync_one(state: &ApiState, device: &Device) -> Result<u64, ApiError> {
    let supervisor = state.call_supervisor.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("no call manager can be reached".to_string())
    })?;
    let (status, body) = supervisor
        .resync_device(device.tenant_id, &device.id.to_string())
        .await
        .map_err(ApiError::ServiceUnavailable)?;
    match status {
        200..=299 => Ok(body["sent"].as_u64().unwrap_or(0)),
        404 => Err(ApiError::NotFound(format!(
            "Device {} not found",
            device.id
        ))),
        _ => Err(ApiError::ServiceUnavailable(
            body["error"]
                .as_str()
                .unwrap_or("the call manager could not resync the phone")
                .to_string(),
        )),
    }
}

/// POST /api/v1/devices/:id/resync — tell the phone to fetch its
/// configuration again, with its vendor's NOTIFY (`docs/PROVISIONING.md`
/// §8), sent by a call manager to each contact it is registered from.
#[utoipa::path(
    post,
    path = "/api/v1/devices/{id}/resync",
    tag = "provisioning",
    operation_id = "provisioning_resync_device",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = ResyncResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn resync_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<ResyncResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let id = DeviceId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {}", e)))?;
    let device = storage
        .get_device(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get device: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Device {} not found", id)))
        .and_then(|device| ensure_device_in_tenant(device, &tenant.0, &id))?;
    let sent = resync_one(&state, &device).await?;
    info!(device_id = %device.id, sent, "Resynced a phone");
    Ok(Json(ResyncResponse {
        device_id: device.id.to_string(),
        sent,
    }))
}
