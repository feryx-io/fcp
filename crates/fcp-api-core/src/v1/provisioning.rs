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

const PROVISIONING_NOT_FOUND: &str = "Provisioning config not found";

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

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Where phones reach this admin: `[api.provisioning] public_url`, else
/// `[api.sso] public_url`, else the host the request came to, over HTTPS.
pub(crate) fn public_base(state: &ApiState, headers: &HeaderMap) -> Option<String> {
    let configured = state
        .provisioning
        .public_url
        .as_deref()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty());
    if configured.is_some() {
        return configured;
    }
    if let Some(sso) = state.sso.as_ref() {
        return Some(sso.public_url.trim_end_matches('/').to_string());
    }
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split(',').next())
        .map(str::trim)
        .filter(|h| {
            !h.is_empty()
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        })?;
    Some(format!("https://{host}"))
}

/// The template a device gets: the one it names, else one picked by its
/// vendor and model (`docs/PROVISIONING.md` §5). `None` when there is none.
pub(crate) async fn choose_template(
    storage: &fcp_core::DynStorage,
    device: &Device,
    hint: &fcp_core::provisioning::PhoneIdentity,
) -> Result<Option<fcp_core::config_template::ConfigTemplate>, ApiError> {
    let template_store = storage
        .config_template_store()
        .ok_or_else(|| ApiError::Internal("Config template store not configured".into()))?;
    if let Some(id) = device.config_template_id.as_deref() {
        let Ok(id) = id.parse::<fcp_core::ConfigTemplateId>() else {
            return Ok(None);
        };
        return Ok(template_store
            .get_config_template(&id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .filter(|t| t.tenant_id == device.tenant_id));
    }
    let templates = template_store
        .list_config_templates(&device.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let vendor = device.vendor.or(hint.vendor).or_else(|| {
        device
            .mac_address
            .as_deref()
            .and_then(fcp_core::provisioning::vendor_of_mac)
    });
    let model = device.model.as_deref().or(hint.model.as_deref());
    Ok(fcp_core::config_template::pick_template(&templates, vendor, model).cloned())
}

/// Everything a device's template is rendered with. Refused (as not
/// found) when the device is disabled or its owner may not use services.
pub(crate) async fn device_context(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    device: &Device,
    normalized_mac: String,
    hint: &fcp_core::provisioning::PhoneIdentity,
    base: Option<&str>,
    key: Option<&str>,
) -> Result<fcp_core::config_template::TemplateContext, ApiError> {
    use fcp_core::config_template::{TemplateAccount, TemplateContext, TemplateLineKey};
    use fcp_core::TenantStore;
    if !device.enabled {
        return Err(provisioning_not_found());
    }
    let internal = |e: anyhow::Error| ApiError::Internal(e.to_string());
    let config = &state.provisioning;
    let mut c = TemplateContext {
        mac_address: normalized_mac,
        device_name: device.name.clone(),
        vendor: device
            .vendor
            .or(hint.vendor)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        model: device
            .model
            .clone()
            .or_else(|| hint.model.clone())
            .unwrap_or_default(),
        firmware_version: device
            .firmware_version
            .clone()
            .or_else(|| hint.firmware.clone())
            .unwrap_or_default(),
        sip_server: config
            .sip_server
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| state.realm.clone()),
        sip_port: config.port(),
        sip_transport: config.transport().to_string(),
        ntp_server: config.ntp_server().to_string(),
        ..Default::default()
    };

    let tenant = storage
        .get_tenant(&device.tenant_id)
        .await
        .map_err(internal)?;
    // The domain names the tenant to the registrar (`docs/PROVISIONING.md`
    // §5); without one, the server's address.
    c.sip_domain = tenant
        .as_ref()
        .and_then(|t| t.sip_domains.iter().find(|d| !d.trim().is_empty()))
        .map(|d| d.trim().to_string())
        .unwrap_or_else(|| c.sip_server.clone());
    c.set_timezone(tenant.as_ref().and_then(|t| t.calling.timezone.as_deref()));
    let key = match key {
        Some(k) => Some(k.to_string()),
        None => match storage.provisioning_store() {
            Some(store) => store
                .get_provisioning_key(&device.tenant_id)
                .await
                .map_err(internal)?
                .map(|k| k.key),
            None => None,
        },
    };
    if let (Some(base), Some(key)) = (base, key) {
        c.provisioning_url = format!("{base}/provision/{key}/");
    }
    // The firmware the phone is pointed at, under the tenant's URL (§11).
    if !c.provisioning_url.is_empty() {
        if let Some(image) = crate::v1::firmware::image_for(storage, device).await? {
            c.firmware_directory = format!("{}firmware/{}/", c.provisioning_url, image.id.simple());
            c.firmware_url = format!("{}{}", c.firmware_directory, image.file_name);
            c.firmware_target_version = image.version;
        }
    }

    // The owner, when the device has one: its credentials unless the
    // device has its own (`docs/ENTERPRISE_CALLING.md` §3.8).
    let owner = match device.owner_user_id {
        Some(ref owner_id) => Some(
            storage
                .get_user_by_id(owner_id)
                .await
                .map_err(internal)?
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
        c.sip_uri = device
            .sip_uri
            .clone()
            .unwrap_or_else(|| format!("sip:{}@{}", account, c.sip_domain));
        c.sip_username = account;
        c.sip_auth_username = auth_username.clone();
        c.sip_auth_ha1_hash = ha1.clone();
        c.sip_password = device.sip_provisioning_password.clone().unwrap_or_default();
    } else if let Some(ref owner) = owner {
        c.sip_auth_ha1_hash = owner
            .sip_ha1_hash
            .clone()
            .ok_or_else(provisioning_not_found)?;
        c.sip_uri = device
            .sip_uri
            .clone()
            .unwrap_or_else(|| format!("sip:{}@{}", owner.sip_name(), c.sip_domain));
        // The phone's account is the owner's SIP name, which a rename
        // keeps: the HA1 is computed over it.
        c.sip_username = owner.sip_name().to_string();
        c.sip_auth_username = owner.sip_name().to_string();
        c.sip_password = owner.sip_provisioning_password.clone().unwrap_or_default();
    }
    c.sip_provisioning_password = c.sip_password.clone();
    if let Some(ref owner) = owner {
        c.owner_username = owner.username.clone();
        c.owner_display_name = owner.display_name.clone().unwrap_or_default();
        c.owner_email = owner.email.clone().unwrap_or_default();
        c.owner_user_id = owner.id.to_string();
    }

    // Who the phone is for now: its hoteling guest, else its owner, so a
    // host that re-provisions shows the guest's name (§3.8).
    let session = if device.hoteling_host {
        storage
            .hoteling_session_for_device(&device.id)
            .await
            .map_err(internal)?
    } else {
        None
    };
    let effective = match (&session, &owner) {
        (Some(session), _) => storage
            .get_user_by_id(&session.user_id)
            .await
            .map_err(internal)?
            // A suspended guest's session lends the phone nothing.
            .filter(|guest| guest.may_use_services()),
        (None, Some(owner)) => Some(owner.clone()),
        (None, None) => None,
    };
    if let Some(user) = effective {
        c.effective_user_username = user.username.clone();
        c.effective_user_display_name = user
            .display_name
            .clone()
            .unwrap_or_else(|| user.username.clone());
        c.effective_user_extension = user.extension.clone().unwrap_or_default();
        c.effective_user_id = user.id.to_string();
    }
    c.hoteling_guest = session.is_some();

    if !c.sip_username.is_empty() {
        c.accounts.push(TemplateAccount {
            index: 1,
            label: if c.effective_user_extension.is_empty() {
                device.name.clone()
            } else {
                c.effective_user_extension.clone()
            },
            display_name: if c.effective_user_display_name.is_empty() {
                c.sip_username.clone()
            } else {
                c.effective_user_display_name.clone()
            },
            user_name: c.sip_username.clone(),
            auth_name: c.sip_auth_username.clone(),
            password: c.sip_password.clone(),
            shared: false,
            appearances: 0,
        });
    }

    // A shared line the device is on renders as its second account
    // (`docs/ENTERPRISE_CALLING.md` §3.9): the line's number as the
    // address, the device's credentials, the line's appearances as keys.
    // A device on none gets the account turned off.
    let shared = storage
        .list_line_bindings_by_device(&device.id)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|b| b.shared && b.tenant_id == device.tenant_id);
    let shared_number = match &shared {
        Some(binding) => storage
            .get_phone_number(&binding.phone_number_id)
            .await
            .map_err(internal)?
            .map(|n| n.e164),
        None => None,
    };
    match (shared, shared_number) {
        (Some(binding), Some(number)) => {
            c.shared_line_enabled = 1;
            c.shared_line_yes_no = "Yes".to_string();
            c.shared_line_number = number.clone();
            c.shared_line_appearances = binding.max_appearances.into();
            if !c.sip_username.is_empty() {
                c.accounts.push(TemplateAccount {
                    index: c.accounts.len() as u32 + 1,
                    label: number.clone(),
                    display_name: number.clone(),
                    user_name: number,
                    auth_name: c.sip_auth_username.clone(),
                    password: c.sip_password.clone(),
                    shared: true,
                    appearances: binding.max_appearances.into(),
                });
            }
        }
        _ => {
            c.shared_line_enabled = 0;
            c.shared_line_yes_no = "No".to_string();
            c.shared_line_appearances = 0;
        }
    }

    c.line_keys = device
        .line_keys
        .iter()
        .enumerate()
        .map(|(i, k)| TemplateLineKey {
            index: i as u32 + 1,
            kind: k.kind.as_str().to_string(),
            value: k.value.clone(),
            label: k.label.clone().unwrap_or_else(|| k.value.clone()),
        })
        .collect();
    Ok(c)
}

/// A device's configuration, rendered from its template: refused (as not
/// found) when the device is disabled, has no template, or its template
/// fails.
pub(crate) async fn render_device(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    device: &Device,
    normalized_mac: String,
    hint: &fcp_core::provisioning::PhoneIdentity,
    base: Option<&str>,
    key: Option<&str>,
) -> Result<String, ApiError> {
    if !device.enabled {
        return Err(provisioning_not_found());
    }
    let template = choose_template(storage, device, hint)
        .await?
        .ok_or_else(provisioning_not_found)?;
    let context = device_context(state, storage, device, normalized_mac, hint, base, key).await?;
    match fcp_core::config_template::render(&template.template_body, template.vendor, &context) {
        Ok(body) => Ok(body),
        Err(e) => {
            tracing::warn!(
                device_id = %device.id,
                template = %template.name,
                error = %e,
                "A device's template failed to render"
            );
            Err(provisioning_not_found())
        }
    }
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
    let base = public_base(&state, &headers);
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
        // A deployment's default tenant may have no row: its settings are
        // the defaults. The key was made by its administrator either way.
        let settings = match storage.get_tenant(&tenant_id).await {
            Ok(Some(t)) => t.provisioning,
            Ok(None) => Default::default(),
            Err(_) => return Fetched::Refused("unknown_tenant", StatusCode::NOT_FOUND),
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
        let hint = fcp_core::provisioning::PhoneIdentity::of_request(
            &file,
            user_agent.as_deref(),
            Some(&mac),
        );
        let device = match storage.get_device_by_mac_address(&mac).await {
            Ok(Some(d)) if d.tenant_id == tenant_id => d,
            // No device has the MAC: a phone to be claimed, if it fetches
            // from where the tenant's phones may (`docs/PROVISIONING.md` §6).
            Ok(None) => {
                let ip = address.as_deref().and_then(|a| a.parse().ok());
                if !settings.allows(ip) {
                    return Fetched::Refused("unknown_device", StatusCode::NOT_FOUND);
                }
                let now = std::time::SystemTime::now();
                let pending = fcp_core::provisioning::PendingDevice {
                    tenant_id,
                    mac_address: mac.clone(),
                    first_seen: now,
                    last_seen: now,
                    address: address.clone(),
                    user_agent: user_agent.clone(),
                    vendor: hint.vendor,
                    model: hint.model.clone(),
                    firmware: hint.firmware.clone(),
                    fetches: 1,
                };
                if let Err(e) = store.note_pending_device(&pending).await {
                    tracing::warn!(error = %e, "Could not note a pending phone");
                }
                return Fetched::Refused("pending", StatusCode::NOT_FOUND);
            }
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
            // The image the phone is pointed at, as its application (§11).
            let firmware = match crate::v1::firmware::image_for(storage, &device).await {
                Ok(Some(image)) => base.as_deref().map(|b| {
                    format!(
                        "{b}/provision/{key}/firmware/{}/{}",
                        image.id.simple(),
                        image.file_name
                    )
                }),
                _ => None,
            };
            return Fetched::Served(
                polycom_master(&mac, firmware.as_deref()),
                kind.content_type(),
            );
        }
        match render_device(
            &state,
            storage,
            &device,
            mac,
            &hint,
            base.as_deref(),
            Some(&key),
        )
        .await
        {
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

/// GET /provision/:key/firmware/:image/:file — a firmware image the
/// tenant targets (`docs/PROVISIONING.md` §11), under the same guards as
/// its phones' files.
pub async fn serve_firmware(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((key, image, file)): Path<(String, String, String)>,
) -> Response {
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
    let address = crate::audit::source_ip(&headers);
    let settings = match storage.get_tenant(&tenant_id).await {
        Ok(Some(t)) => t.provisioning,
        Ok(None) => Default::default(),
        Err(_) => return not_found(),
    };
    let ip = address.as_deref().and_then(|a| a.parse().ok());
    let served = if settings.allows(ip) {
        crate::v1::firmware::serve_image(&state, tenant_id, &image, &file).await
    } else {
        None
    };
    let fetch = fcp_core::provisioning::DeviceFetch {
        tenant_id,
        device_id: None,
        mac_address: None,
        file: format!("firmware/{}", file.chars().take(190).collect::<String>()),
        at: std::time::SystemTime::now(),
        address,
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(String::from),
        outcome: if served.is_some() {
            "served"
        } else {
            "not_served"
        }
        .to_string(),
    };
    if let Err(e) = store.record_fetch(&fetch).await {
        tracing::warn!(error = %e, "Could not record a firmware fetch");
    }
    served.unwrap_or_else(not_found)
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
