//! Device auto-provisioning endpoint and template engine.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use fcp_core::storage::{DeviceStore, LineBindingStore, PhoneNumberStore};
use fcp_core::{CredentialStore, Device, HotelingStore};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const PROVISIONING_NOT_FOUND: &str = "Provisioning config not found";
const PROVISIONING_TOKEN_HEADER: &str = "x-fcp-provisioning-token";
const DEVICE_PROVISIONING_TOKEN_HEADER: &str = "x-fcp-device-provisioning-token";

/// Simple template engine -- replaces ${variable} placeholders.
fn render_template(template_body: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template_body.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("${{{}}}", key), value);
    }
    result
}

fn provisioning_not_found() -> ApiError {
    ApiError::NotFound(PROVISIONING_NOT_FOUND.to_string())
}

/// Returns `true` when the request arrived over HTTPS, as indicated by the
/// `X-Forwarded-Proto` header set by a reverse proxy.  When no header is
/// present we assume plain HTTP (fail-closed).
fn request_is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
}

fn provisioning_token_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(PROVISIONING_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn device_provisioning_token_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(DEVICE_PROVISIONING_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

async fn authorize_shared_provisioning_token(
    expected_token: Option<&str>,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let expected_token = expected_token.ok_or_else(provisioning_not_found)?;
    let presented_token =
        provisioning_token_from_headers(headers).ok_or_else(provisioning_not_found)?;

    let auth_config = crate::auth::AuthConfig::new(expected_token.to_string());
    if crate::auth::validate_token(&presented_token, &auth_config)
        .await
        .is_none()
    {
        return Err(provisioning_not_found());
    }

    Ok(())
}

fn authorize_device_provisioning_token(
    device: &Device,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let expected_device_token_hash = device
        .provisioning_token_hash
        .as_deref()
        .filter(|hash| !hash.trim().is_empty())
        .ok_or_else(provisioning_not_found)?;
    let presented_device_token =
        device_provisioning_token_from_headers(headers).ok_or_else(provisioning_not_found)?;
    let presented_device_token_hash = hash_token(&presented_device_token);
    if !fcp_core::secret::constant_time_eq(&presented_device_token_hash, expected_device_token_hash)
    {
        return Err(provisioning_not_found());
    }

    Ok(())
}

/// GET /provisioning/:mac_address -- Auto-provision a phone by MAC address.
///
/// Phones must present both:
/// - `Authorization: Bearer <token>`
/// - or `X-FCP-Provisioning-Token: <token>`
/// - and `X-FCP-Device-Provisioning-Token: <per-device-token>`
///
/// Unknown or unauthorized devices all return the same 404 to avoid turning
/// this endpoint into an enumeration surface.
#[utoipa::path(
    get,
    path = "/api/v1/provisioning/{mac_address}",
    tag = "provisioning",
    operation_id = "provisioning_auto_provision",
    params(("mac_address" = String, Path)),
    responses(
        (status = 200, description = "The device's rendered configuration", content_type = "text/plain", body = String),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn auto_provision(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(mac_address): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if state.provisioning_require_https && !request_is_https(&headers) {
        return Err(provisioning_not_found());
    }

    // Gate the storage lookup on the shared token so unauthenticated callers
    // can't probe MAC existence by timing or spin the DB.
    authorize_shared_provisioning_token(state.provisioning_auth_token.as_deref(), &headers).await?;

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".into()))?;

    // Normalize MAC: strip colons/dashes/dots, lowercase
    let normalized_mac = mac_address.replace([':', '-', '.'], "").to_lowercase();

    // Look up device by MAC
    let device = storage
        .get_device_by_mac_address(&normalized_mac)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(provisioning_not_found)?;

    // Verify the per-device token before inspecting device state so a caller
    // with only the shared token cannot distinguish "disabled" from "enabled
    // but wrong per-device token" by response timing.
    authorize_device_provisioning_token(&device, &headers)?;

    let config = render_device(&state, storage, &device, normalized_mac).await?;

    // Return as text (could be XML, INI, etc. depending on vendor)
    Ok((
        StatusCode::OK,
        [
            ("content-type", "text/plain; charset=utf-8"),
            ("cache-control", "no-store, no-cache, max-age=0"),
            ("pragma", "no-cache"),
            ("expires", "0"),
            ("x-content-type-options", "nosniff"),
        ],
        config,
    ))
}

/// A device's configuration, rendered from its template: refused (as not
/// found) when the device is disabled or has none.
pub(crate) async fn render_device(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    device: &Device,
    normalized_mac: String,
) -> Result<String, ApiError> {
    if !device.enabled {
        return Err(provisioning_not_found());
    }

    // Find the config template
    let template_id = device
        .config_template_id
        .as_ref()
        .ok_or_else(provisioning_not_found)?;

    let template_id = template_id
        .parse::<fcp_core::ConfigTemplateId>()
        .map_err(|_| provisioning_not_found())?;

    let template_store = storage
        .config_template_store()
        .ok_or_else(|| ApiError::Internal("Config template store not configured".into()))?;

    let template = template_store
        .get_config_template(&template_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(provisioning_not_found)?;
    if template.tenant_id != device.tenant_id {
        return Err(provisioning_not_found());
    }

    // Build variable map
    let mut vars = HashMap::new();
    vars.insert("mac_address".to_string(), normalized_mac);
    vars.insert("device_name".to_string(), device.name.clone());

    // The owner, when the device has one: its credentials unless the
    // device has its own (`docs/ENTERPRISE_CALLING.md` §3.8).
    let owner = match device.owner_user_id {
        Some(ref owner_id) => Some(
            storage
                .get_user_by_id(owner_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .filter(|owner| owner.tenant_id == device.tenant_id && owner.may_use_services())
                .ok_or_else(provisioning_not_found)?,
        ),
        None => None,
    };

    if let (Some(auth_username), Some(ha1)) = (&device.sip_username, &device.sip_ha1_hash) {
        let account = device
            .aor_user()
            .map(str::to_string)
            .unwrap_or_else(|| auth_username.clone());
        let sip_uri = device
            .sip_uri
            .clone()
            .unwrap_or_else(|| format!("sip:{}@{}", account, state.realm));
        vars.insert("sip_uri".to_string(), sip_uri);
        vars.insert("sip_username".to_string(), account);
        vars.insert("sip_auth_username".to_string(), auth_username.clone());
        vars.insert("sip_auth_ha1_hash".to_string(), ha1.clone());
        if let Some(ref sip_password) = device.sip_provisioning_password {
            vars.insert("sip_password".to_string(), sip_password.clone());
            vars.insert(
                "sip_provisioning_password".to_string(),
                sip_password.clone(),
            );
        }
    } else if let Some(ref owner) = owner {
        let sip_ha1_hash = owner
            .sip_ha1_hash
            .clone()
            .ok_or_else(provisioning_not_found)?;
        let sip_uri = device
            .sip_uri
            .clone()
            .unwrap_or_else(|| format!("sip:{}@{}", owner.sip_name(), state.realm));

        vars.insert("sip_uri".to_string(), sip_uri);
        // The phone's account is the owner's SIP name, which a rename
        // keeps: the HA1 below is computed over it.
        vars.insert("sip_username".to_string(), owner.sip_name().to_string());
        vars.insert(
            "sip_auth_username".to_string(),
            owner.sip_name().to_string(),
        );
        vars.insert("sip_auth_ha1_hash".to_string(), sip_ha1_hash);
        if let Some(ref sip_password) = owner.sip_provisioning_password {
            vars.insert("sip_password".to_string(), sip_password.clone());
            vars.insert(
                "sip_provisioning_password".to_string(),
                sip_password.clone(),
            );
        }
    }
    if let Some(ref owner) = owner {
        vars.insert("owner_username".to_string(), owner.username.clone());
        if let Some(ref display_name) = owner.display_name {
            vars.insert("owner_display_name".to_string(), display_name.clone());
        }
        if let Some(ref email) = owner.email {
            vars.insert("owner_email".to_string(), email.clone());
        }
        vars.insert("owner_user_id".to_string(), owner.id.to_string());
    }

    // Who the phone is for now: its hoteling guest, else its owner, so a
    // host that re-provisions shows the guest's name (§3.8).
    let session = if device.hoteling_host {
        storage
            .hoteling_session_for_device(&device.id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
    } else {
        None
    };
    let effective = match (&session, &owner) {
        (Some(session), _) => storage
            .get_user_by_id(&session.user_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            // A suspended guest's session lends the phone nothing.
            .filter(|guest| guest.may_use_services()),
        (None, Some(owner)) => Some(owner.clone()),
        (None, None) => None,
    };
    if let Some(user) = effective {
        vars.insert("effective_user_username".to_string(), user.username.clone());
        vars.insert(
            "effective_user_display_name".to_string(),
            user.display_name
                .clone()
                .unwrap_or_else(|| user.username.clone()),
        );
        if let Some(extension) = user.extension {
            vars.insert("effective_user_extension".to_string(), extension);
        }
        vars.insert("effective_user_id".to_string(), user.id.to_string());
    }
    vars.insert("hoteling_guest".to_string(), session.is_some().to_string());

    // A shared line the device is on renders as its second account
    // (`docs/ENTERPRISE_CALLING.md` §3.9): the line's number as the
    // address, the device's credentials, the line's appearances as keys.
    // A device on none gets the account turned off, so a template never
    // shows a placeholder.
    let shared = storage
        .list_line_bindings_by_device(&device.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .find(|b| b.shared && b.tenant_id == device.tenant_id);
    let shared_number = match &shared {
        Some(binding) => storage
            .get_phone_number(&binding.phone_number_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .map(|n| n.e164),
        None => None,
    };
    match (shared, shared_number) {
        (Some(binding), Some(number)) => {
            vars.insert("shared_line_enabled".to_string(), "1".to_string());
            vars.insert("shared_line_yes_no".to_string(), "Yes".to_string());
            vars.insert("shared_line_number".to_string(), number);
            vars.insert(
                "shared_line_appearances".to_string(),
                binding.max_appearances.to_string(),
            );
        }
        _ => {
            vars.insert("shared_line_enabled".to_string(), "0".to_string());
            vars.insert("shared_line_yes_no".to_string(), "No".to_string());
            vars.insert("shared_line_number".to_string(), String::new());
            vars.insert("shared_line_appearances".to_string(), "0".to_string());
        }
    }

    // Add SIP server info
    vars.insert("sip_server".to_string(), state.realm.clone());
    vars.insert("sip_port".to_string(), "5060".to_string());

    if let Some(ref model) = device.model {
        vars.insert("model".to_string(), model.clone());
    }

    // Render template
    let config = render_template(&template.template_body, &vars);
    Ok(config)
}

// ---- The tenant's provisioning URL (`docs/PROVISIONING.md`, phase 7a) ----

const DEVICE_CREDENTIALS_REALM: &str = "Basic realm=\"FCP provisioning\"";

fn provisioning_store(
    storage: &fcp_core::DynStorage,
) -> Result<&std::sync::Arc<dyn fcp_core::provisioning::ProvisioningStore>, ApiError> {
    storage
        .provisioning_store()
        .ok_or_else(|| ApiError::NotFound("Provisioning is not available here".to_string()))
}

/// The password of a request's HTTP Basic credentials.
fn basic_password(headers: &HeaderMap) -> Option<String> {
    use base64::Engine;
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    decoded.split_once(':').map(|(_, p)| p.to_string())
}

/// What a fetch was refused as, or served.
enum Fetched {
    Served(String, &'static str),
    Refused(&'static str, StatusCode),
}

/// GET /provision/:key/:file — a phone's file, as its vendor names it,
/// from the tenant whose key is in the path (`docs/PROVISIONING.md` §2).
pub async fn serve_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((key, file)): Path<(String, String)>,
) -> Response {
    use fcp_core::provisioning::{parse_file, polycom_master, FileKind, RequestedFile};
    use fcp_core::TenantStore;
    let not_found = || provisioning_not_found().into_response();
    if state.provisioning_require_https && !request_is_https(&headers) {
        return not_found();
    }
    let Some(storage) = state.storage.as_deref() else {
        return not_found();
    };
    let Ok(store) = provisioning_store(storage) else {
        return not_found();
    };
    let tenant_id = match store.tenant_for_provisioning_key(&key).await {
        Ok(Some(t)) => t,
        _ => return not_found(),
    };
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let address = crate::audit::source_ip(&headers);
    let requested = parse_file(
        &file,
        fcp_core::provisioning::is_polycom(user_agent.as_deref()),
    );
    let mut fetch = fcp_core::provisioning::DeviceFetch {
        tenant_id,
        device_id: None,
        mac_address: None,
        file: file.chars().take(200).collect(),
        at: std::time::SystemTime::now(),
        address: address.clone(),
        user_agent: user_agent.clone(),
        outcome: String::new(),
    };

    let outcome: Fetched = async {
        let settings = match storage.get_tenant(&tenant_id).await {
            Ok(Some(t)) => t.provisioning,
            _ => return Fetched::Refused("unknown_tenant", StatusCode::NOT_FOUND),
        };
        let (mac, kind, master) = match requested {
            None => return Fetched::Refused("not_a_device_file", StatusCode::NOT_FOUND),
            // Nothing every phone of a vendor shares: each has its own.
            Some(RequestedFile::Common) => {
                return Fetched::Refused("common", StatusCode::NOT_FOUND)
            }
            Some(RequestedFile::PolycomMaster { mac }) => (mac, FileKind::Xml, true),
            Some(RequestedFile::Device { mac, kind }) => (mac, kind, false),
        };
        fetch.mac_address = Some(mac.clone());
        let device = match storage.get_device_by_mac_address(&mac).await {
            Ok(Some(d)) if d.tenant_id == tenant_id => d,
            _ => return Fetched::Refused("unknown_device", StatusCode::NOT_FOUND),
        };
        fetch.device_id = Some(device.id);
        // Its networks only, when the tenant names them: refused on the
        // device's own log, so a phone that cannot fetch can be told why.
        let ip = address.as_deref().and_then(|a| a.parse().ok());
        if !settings.allows(ip) {
            return Fetched::Refused("network_refused", StatusCode::NOT_FOUND);
        }
        // A device's own credentials: its provisioning token as the Basic
        // password, wanted when the tenant says so or the phone sends them.
        let presented = basic_password(&headers);
        if settings.require_device_credentials || presented.is_some() {
            let ok = match (presented, device.provisioning_token_hash.as_deref()) {
                (Some(p), Some(expected)) => {
                    fcp_core::secret::constant_time_eq(hash_token(&p), expected)
                }
                _ => false,
            };
            if !ok {
                return Fetched::Refused("unauthorized", StatusCode::UNAUTHORIZED);
            }
        }
        if master {
            if !device.enabled {
                return Fetched::Refused("not_served", StatusCode::NOT_FOUND);
            }
            return Fetched::Served(polycom_master(&mac), kind.content_type());
        }
        match render_device(&state, storage, &device, mac).await {
            Ok(body) => Fetched::Served(body, kind.content_type()),
            Err(_) => Fetched::Refused("not_served", StatusCode::NOT_FOUND),
        }
    }
    .await;

    let response = match outcome {
        Fetched::Served(body, content_type) => {
            fetch.outcome = "served".to_string();
            (
                StatusCode::OK,
                [
                    ("content-type", content_type),
                    ("cache-control", "no-store, no-cache, max-age=0"),
                    ("pragma", "no-cache"),
                    ("expires", "0"),
                    ("x-content-type-options", "nosniff"),
                ],
                body,
            )
                .into_response()
        }
        Fetched::Refused(why, StatusCode::UNAUTHORIZED) => {
            fetch.outcome = why.to_string();
            (
                StatusCode::UNAUTHORIZED,
                [("www-authenticate", DEVICE_CREDENTIALS_REALM)],
                "",
            )
                .into_response()
        }
        Fetched::Refused(why, _) => {
            fetch.outcome = why.to_string();
            not_found()
        }
    };
    // Every phone ask for its vendor's common file: not worth a row.
    if fetch.outcome != "common" {
        if let Err(e) = store.record_fetch(&fetch).await {
            tracing::warn!(error = %e, "Could not record a provisioning fetch");
        }
    }
    tracing::info!(
        tenant_id = %tenant_id,
        file = %fetch.file,
        outcome = %fetch.outcome,
        "Provisioning fetch"
    );
    response
}

/// The tenant's provisioning URL.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ProvisioningKeyResponse {
    /// The secret in the URL's path; anyone with it may fetch the tenant's
    /// phones' files (by MAC), so it is shown to administrators only.
    pub key: String,
    /// The path phones fetch under: `/provision/<key>/`.
    pub path: String,
    /// The whole URL, when the admin knows its public address
    /// (`[api.sso] public_url`).
    pub url: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: std::time::SystemTime,
}

fn key_response(
    state: &ApiState,
    key: fcp_core::provisioning::ProvisioningKey,
) -> ProvisioningKeyResponse {
    let path = format!("/provision/{}/", key.key);
    ProvisioningKeyResponse {
        url: state
            .sso
            .as_ref()
            .map(|s| format!("{}{path}", s.public_url)),
        path,
        key: key.key,
        created_at: key.created_at,
    }
}

/// GET /api/v1/provisioning/key
#[utoipa::path(
    get,
    path = "/api/v1/provisioning/key",
    tag = "provisioning",
    operation_id = "provisioning_get_key",
    responses(
        (status = 200, description = "OK", body = ProvisioningKeyResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_key(
    State(state): State<ApiState>,
    axum::Extension(identity): axum::Extension<crate::auth::AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<axum::Json<ProvisioningKeyResponse>, ApiError> {
    crate::auth::check_permission(
        &identity,
        fcp_core::permissions::Permission::DeviceUpdate,
        state.deployment_mode,
    )?;
    let storage = state
        .storage
        .as_deref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".into()))?;
    let key = provisioning_store(storage)?
        .get_provisioning_key(&tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound("The tenant has no provisioning key yet".to_string()))?;
    Ok(axum::Json(key_response(&state, key)))
}

/// POST /api/v1/provisioning/key — a new key: the old URL stops working.
#[utoipa::path(
    post,
    path = "/api/v1/provisioning/key",
    tag = "provisioning",
    operation_id = "provisioning_rotate_key",
    responses(
        (status = 201, description = "Made", body = ProvisioningKeyResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn rotate_key(
    State(state): State<ApiState>,
    axum::Extension(identity): axum::Extension<crate::auth::AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        fcp_core::permissions::Permission::DeviceUpdate,
        state.deployment_mode,
    )?;
    let storage = state
        .storage
        .as_deref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".into()))?;
    let key = fcp_core::provisioning::ProvisioningKey::generate(tenant.0);
    provisioning_store(storage)?
        .put_provisioning_key(&key)
        .await?;
    tracing::warn!(event = "provisioning_key_rotated", tenant_id = %tenant.0, "Provisioning key made");
    Ok((StatusCode::CREATED, axum::Json(key_response(&state, key))))
}

/// One fetch of a device's file.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct DeviceFetchResponse {
    pub file: String,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub at: std::time::SystemTime,
    pub address: Option<String>,
    pub user_agent: Option<String>,
    /// `served`, `unauthorized`, `not_served` (disabled, or no template)…
    pub outcome: String,
}

/// GET /api/v1/devices/:id/fetches — the device's provisioning fetches,
/// newest first.
#[utoipa::path(
    get,
    path = "/api/v1/devices/{id}/fetches",
    tag = "provisioning",
    operation_id = "provisioning_device_fetches",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<DeviceFetchResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn device_fetches(
    State(state): State<ApiState>,
    axum::Extension(identity): axum::Extension<crate::auth::AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        fcp_core::permissions::Permission::DeviceRead,
        state.deployment_mode,
    )?;
    let storage = state
        .storage
        .as_deref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".into()))?;
    let device_id: fcp_core::DeviceId = id
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("Invalid device ID: {id}")))?;
    let device = storage
        .get_device(&device_id)
        .await?
        .filter(|d| d.tenant_id == tenant.0)
        .ok_or_else(|| ApiError::NotFound(format!("Device {id} not found")))?;
    let fetches: Vec<DeviceFetchResponse> = provisioning_store(storage)?
        .device_fetches(&tenant.0, &device.id, 100)
        .await?
        .into_iter()
        .map(|f| DeviceFetchResponse {
            file: f.file,
            at: f.at,
            address: f.address,
            user_agent: f.user_agent,
            outcome: f.outcome,
        })
        .collect();
    Ok(axum::Json(crate::page::paginate(
        fetches,
        &crate::page::PageQuery::default(),
        crate::page::Order::Descending,
        |f| {
            format!(
                "{:020}",
                f.at.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros()
            )
        },
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template() {
        let template = "server=${sip_server}\nuser=${sip_username}\nmac=${mac_address}";
        let mut vars = HashMap::new();
        vars.insert("sip_server".to_string(), "pbx.example.com".to_string());
        vars.insert("sip_username".to_string(), "alice".to_string());
        vars.insert("mac_address".to_string(), "001122334455".to_string());

        let result = render_template(template, &vars);
        assert_eq!(
            result,
            "server=pbx.example.com\nuser=alice\nmac=001122334455"
        );
    }

    #[test]
    fn test_render_template_missing_vars_left_as_is() {
        let template = "server=${sip_server}\npassword=${password}";
        let mut vars = HashMap::new();
        vars.insert("sip_server".to_string(), "pbx.example.com".to_string());

        let result = render_template(template, &vars);
        assert_eq!(result, "server=pbx.example.com\npassword=${password}");
    }

    #[test]
    fn test_request_is_https() {
        let mut headers = HeaderMap::new();
        assert!(!request_is_https(&headers), "no header → plain HTTP");

        headers.insert("x-forwarded-proto", "http".parse().unwrap());
        assert!(!request_is_https(&headers));

        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(request_is_https(&headers));

        headers.insert("x-forwarded-proto", "HTTPS".parse().unwrap());
        assert!(request_is_https(&headers), "case-insensitive");
    }

    #[test]
    fn test_provisioning_token_from_custom_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            PROVISIONING_TOKEN_HEADER,
            "shared-token".parse().expect("valid header"),
        );

        assert_eq!(
            provisioning_token_from_headers(&headers).as_deref(),
            Some("shared-token")
        );
    }

    #[test]
    fn test_provisioning_token_from_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer shared-token".parse().expect("valid header"),
        );

        assert_eq!(
            provisioning_token_from_headers(&headers).as_deref(),
            Some("shared-token")
        );
    }

    #[test]
    fn test_device_provisioning_token_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DEVICE_PROVISIONING_TOKEN_HEADER,
            " device-token ".parse().expect("valid header"),
        );

        assert_eq!(
            device_provisioning_token_from_headers(&headers).as_deref(),
            Some("device-token")
        );
    }

    fn test_device_with_token_hash(hash: Option<&str>) -> Device {
        let mut device = Device::new(
            "Test Phone".to_string(),
            fcp_core::DeviceType::DeskPhone,
            fcp_core::TenantId::new(),
        );
        device.provisioning_token_hash = hash.map(str::to_string);
        device
    }

    #[test]
    fn test_authorize_device_provisioning_rejects_when_hash_unset() {
        let device = test_device_with_token_hash(None);
        let mut headers = HeaderMap::new();
        headers.insert(
            DEVICE_PROVISIONING_TOKEN_HEADER,
            "anything".parse().unwrap(),
        );

        let err = authorize_device_provisioning_token(&device, &headers)
            .expect_err("device without a hash must fail closed");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[test]
    fn test_authorize_device_provisioning_rejects_when_header_missing() {
        let hash = hash_token("correct-horse-battery-staple-correct-horse");
        let device = test_device_with_token_hash(Some(&hash));
        let headers = HeaderMap::new();

        let err = authorize_device_provisioning_token(&device, &headers)
            .expect_err("missing header must 404");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[test]
    fn test_authorize_device_provisioning_rejects_wrong_token() {
        let hash = hash_token("correct-horse-battery-staple-correct-horse");
        let device = test_device_with_token_hash(Some(&hash));
        let mut headers = HeaderMap::new();
        headers.insert(
            DEVICE_PROVISIONING_TOKEN_HEADER,
            "wrong-token-but-also-32-chars-long-xx".parse().unwrap(),
        );

        let err = authorize_device_provisioning_token(&device, &headers)
            .expect_err("wrong token must 404");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[test]
    fn test_authorize_device_provisioning_accepts_correct_token() {
        let token = "correct-horse-battery-staple-correct-horse";
        let hash = hash_token(token);
        let device = test_device_with_token_hash(Some(&hash));
        let mut headers = HeaderMap::new();
        headers.insert(DEVICE_PROVISIONING_TOKEN_HEADER, token.parse().unwrap());

        authorize_device_provisioning_token(&device, &headers)
            .expect("correct token must authorize");
    }

    #[tokio::test]
    async fn test_authorize_shared_provisioning_rejects_when_unconfigured() {
        let mut headers = HeaderMap::new();
        headers.insert(PROVISIONING_TOKEN_HEADER, "any-token".parse().unwrap());

        let err = authorize_shared_provisioning_token(None, &headers)
            .await
            .expect_err("unset deployment token must fail closed");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_authorize_shared_provisioning_rejects_when_header_missing() {
        let headers = HeaderMap::new();
        let err = authorize_shared_provisioning_token(Some("shared-token"), &headers)
            .await
            .expect_err("missing header must 404");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_authorize_shared_provisioning_rejects_wrong_token() {
        let mut headers = HeaderMap::new();
        headers.insert(PROVISIONING_TOKEN_HEADER, "wrong-token".parse().unwrap());
        let err = authorize_shared_provisioning_token(Some("shared-token"), &headers)
            .await
            .expect_err("wrong token must 404");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_authorize_shared_provisioning_accepts_correct_token() {
        let mut headers = HeaderMap::new();
        headers.insert(PROVISIONING_TOKEN_HEADER, "shared-token".parse().unwrap());
        authorize_shared_provisioning_token(Some("shared-token"), &headers)
            .await
            .expect("correct token must authorize");
    }

    #[test]
    fn test_constant_time_compare_basics() {
        assert!(fcp_core::secret::constant_time_eq("abc", "abc"));
        assert!(!fcp_core::secret::constant_time_eq("abc", "abd"));
        assert!(
            !fcp_core::secret::constant_time_eq("abc", "abcd"),
            "length mismatch must fail"
        );
        assert!(fcp_core::secret::constant_time_eq("", ""));
    }
}
