//! Presence management API endpoints.

use crate::{auth::TenantContext, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::storage::{DeviceStore, LineBindingStore, PhoneNumberStore};
use fcp_presence::{PresenceAuthPolicy, PresenceState, PresenceUpdate};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::info;

/// Helper macro to get presence_manager or return 404.
macro_rules! require_presence_manager {
    ($state:expr) => {
        $state
            .presence_manager
            .as_ref()
            .ok_or_else(|| ApiError::NotFound("Presence service is not configured".to_string()))?
    };
}

// ---- Response types ----

/// Presence info response for a single presentity.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PresenceResponse {
    pub aor: String,
    pub state: String,
    pub note: Option<String>,
    pub watcher_count: usize,
    /// Extension number (e.g., "1234") if this AOR is an extension or device belonging to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    /// Human-readable display name (user or device name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Watcher info response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WatcherResponse {
    pub watcher_aor: String,
    pub subscription_id: String,
    pub event_package: String,
    pub expires_in_secs: u64,
}

/// Presence stats response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PresenceStatsResponse {
    pub total_presentities: usize,
    pub total_watchers: usize,
}

// ---- Request types ----

/// Update presence request body.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePresenceRequest {
    pub state: String,
    pub note: Option<String>,
}

// ---- Handlers ----

/// GET /presence — List all tracked presentities (filtered by tenant).
///
/// Device AORs that belong to an extension are merged into the extension entry
/// so each extension appears as a single card in the UI.
#[utoipa::path(
    get,
    path = "/api/v1/presence",
    tag = "presence",
    operation_id = "presence_list_presences",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<PresenceResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_presences(
    State(state): State<ApiState>,
    tenant: TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);
    let tenant_id = &tenant.0;

    let presences = pm.get_presences_by_tenant(tenant_id);

    // Build extension→device AOR mapping so we can merge device entries into extensions.
    // Also build reverse map: device AOR → (extension number, extension description).
    // Phone number AORs are also mapped to their extensions so they get merged.
    let mut device_aor_to_ext: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut phone_aor_to_ext: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut ext_aor_info: HashMap<String, (String, Option<String>)> = HashMap::new(); // ext AOR → (number, description)

    if let Some(storage) = &state.storage {
        use fcp_core::storage::ExtensionStore;
        if let Ok(extensions) = storage.list_extensions(tenant_id).await {
            for ext in &extensions {
                if !ext.enabled {
                    continue;
                }
                // Figure out which host this extension AOR uses by checking existing presences
                let ext_aor_prefix = format!("{}@", ext.number);
                for p in &presences {
                    if p.aor.starts_with(&ext_aor_prefix) {
                        ext_aor_info
                            .insert(p.aor.clone(), (ext.number.clone(), ext.description.clone()));
                        break;
                    }
                }

                // Resolve extension target to device AOR(s)
                let device_aors = resolve_ext_to_device_aors(ext, storage.as_ref()).await;
                for dev_aor in device_aors {
                    device_aor_to_ext
                        .insert(dev_aor, (ext.number.clone(), ext.description.clone()));
                }

                // If the extension targets a phone number, map that phone number's AOR
                // to this extension so it gets merged/hidden in the UI.
                if let fcp_core::ExtensionTarget::PhoneNumber(ref phone_number_id) = ext.target {
                    if let Ok(Some(pn)) = storage.get_phone_number(phone_number_id).await {
                        // The phone number E.164 (e.g. "+15185551234") appears in presence
                        // as an AOR like "15185551234@host" (digits only, no +).
                        let digits: String =
                            pn.e164.chars().filter(|c| c.is_ascii_digit()).collect();
                        if !digits.is_empty() {
                            // Find any presence entries that match this phone number
                            let pn_aor_prefix = format!("{}@", digits);
                            for p in &presences {
                                if p.aor.starts_with(&pn_aor_prefix) {
                                    phone_aor_to_ext.insert(
                                        p.aor.clone(),
                                        (ext.number.clone(), ext.description.clone()),
                                    );
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Collect device AORs and phone number AORs that should be hidden (merged into extension entries).
    let hidden_aors: HashSet<&String> = device_aor_to_ext
        .keys()
        .chain(phone_aor_to_ext.keys())
        .filter(|merged_aor| {
            // Only hide if the extension AOR also has a presence entry
            let ext_info = device_aor_to_ext
                .get(*merged_aor)
                .or_else(|| phone_aor_to_ext.get(*merged_aor));
            let ext_aor_prefix = ext_info.map(|(num, _)| format!("{}@", num));
            ext_aor_prefix
                .map(|prefix| presences.iter().any(|p| p.aor.starts_with(&prefix)))
                .unwrap_or(false)
        })
        .collect();

    let responses: Vec<PresenceResponse> = presences
        .into_iter()
        .filter(|p| !hidden_aors.contains(&p.aor))
        .map(|p| {
            let mut watcher_count = pm.get_watchers_for_tenant(&p.aor, tenant_id).len();

            // Add watcher counts from merged device and phone number AORs
            if ext_aor_info.contains_key(&p.aor) {
                let ext_num = ext_aor_info.get(&p.aor).map(|(n, _)| n);
                for (merged_aor, (num, _)) in
                    device_aor_to_ext.iter().chain(phone_aor_to_ext.iter())
                {
                    if ext_num == Some(num) {
                        watcher_count += pm.get_watchers_for_tenant(merged_aor, tenant_id).len();
                    }
                }
            }

            let (extension, display_name) = if let Some((num, desc)) = ext_aor_info.get(&p.aor) {
                (Some(num.clone()), desc.clone())
            } else if let Some((num, desc)) = device_aor_to_ext.get(&p.aor) {
                // Standalone device AOR that has no extension presence entry
                (Some(num.clone()), desc.clone())
            } else if let Some((num, desc)) = phone_aor_to_ext.get(&p.aor) {
                // Standalone phone number AOR that has no extension presence entry
                (Some(num.clone()), desc.clone())
            } else {
                (None, None)
            };

            PresenceResponse {
                aor: p.aor,
                state: p.state.to_string(),
                note: p.note,
                watcher_count,
                extension,
                display_name,
            }
        })
        .collect();

    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r| r.aor.clone(),
    )?))
}

/// One person's presence as the tenant's people see it
/// (`docs/PRESENCE.md`): what they chose, a call, DND, idle and whether any
/// of their phones is registered, worked out by the admin.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PersonPresenceResponse {
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub extension: Option<String>,
    /// `available`, `away`, `busy`, `dnd`, `on_call`, `offline`; null when
    /// this admin does not work presence out.
    pub state: Option<String>,
    /// Their own status's label ("WFH").
    pub label: Option<String>,
    pub note: Option<String>,
    /// Away because idle, not because they said so.
    pub idle: bool,
}

/// GET /presence/people — every person in the tenant with their presence
/// (D8 finding 69: `/presence` lists only what this process's presence
/// table holds, which phones subscribing to the call managers never fill).
#[utoipa::path(
    get,
    path = "/api/v1/presence/people",
    tag = "presence",
    operation_id = "presence_list_people",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<PersonPresenceResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_people(
    State(state): State<ApiState>,
    tenant: TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    use fcp_core::storage::CredentialStore;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let users = storage
        .list_users_in_tenant(&tenant.0, &fcp_core::storage::UserFilter::default())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let people = match &state.people_presence {
        Some(p) => Some(p.tenant_presence(&tenant.0).await),
        None => None,
    };
    let rows = users
        .iter()
        .filter(|u| u.may_use_services())
        .map(|u| {
            let shown = people.as_ref().and_then(|p| p.get(&u.id));
            PersonPresenceResponse {
                user_id: u.id.to_string(),
                username: u.username.clone(),
                display_name: u.shown_name(),
                extension: u.extension.clone(),
                state: shown.map(|p| p.state.as_str().to_string()),
                label: shown.and_then(|p| p.label.clone()),
                note: shown.and_then(|p| p.note.clone()),
                idle: shown.is_some_and(|p| p.idle),
            }
        })
        .collect();
    Ok(Json(crate::page::paginate(
        rows,
        &page,
        crate::page::Order::Ascending,
        |r: &PersonPresenceResponse| r.username.clone(),
    )?))
}

/// Resolve an extension's target to device AOR(s).
async fn resolve_ext_to_device_aors(
    ext: &fcp_core::Extension,
    storage: &fcp_core::DynStorage,
) -> Vec<String> {
    let mut aors = Vec::new();
    match &ext.target {
        fcp_core::ExtensionTarget::Device(device_id) => {
            if let Ok(Some(device)) = storage.get_device(device_id).await {
                if let Some(sip_uri) = &device.sip_uri {
                    if let Some(aor) = fcp_sip::presence::extract_aor(sip_uri) {
                        aors.push(aor);
                    }
                }
            }
        }
        fcp_core::ExtensionTarget::PhoneNumber(phone_number_id) => {
            if let Ok(Some(pn)) = storage.get_phone_number(phone_number_id).await {
                if let Ok(Some(lb)) = storage.get_line_binding_by_phone_number(&pn.id).await {
                    for db in lb.enabled_devices() {
                        if let Ok(Some(device)) = storage.get_device(&db.device_id).await {
                            if let Some(sip_uri) = &device.sip_uri {
                                if let Some(aor) = fcp_sip::presence::extract_aor(sip_uri) {
                                    aors.push(aor);
                                }
                            }
                        }
                    }
                }
            }
        }
        fcp_core::ExtensionTarget::User(user_id) => {
            if let Ok(devices) = storage.list_devices_by_user(user_id).await {
                for device in &devices {
                    if let Some(sip_uri) = &device.sip_uri {
                        if let Some(aor) = fcp_sip::presence::extract_aor(sip_uri) {
                            aors.push(aor);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    aors
}

/// GET /presence/stats — Presentity and watcher counts.
#[utoipa::path(
    get,
    path = "/api/v1/presence/stats",
    tag = "presence",
    operation_id = "presence_get_stats",
    responses(
        (status = 200, description = "OK", body = PresenceStatsResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_stats(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);
    let presences = pm.get_presences_by_tenant(&tenant.0);
    let total_watchers = presences
        .iter()
        .map(|presence| pm.get_watchers_for_tenant(&presence.aor, &tenant.0).len())
        .sum();

    Ok(Json(PresenceStatsResponse {
        total_presentities: presences.len(),
        total_watchers,
    }))
}

/// GET /presence/:aor — Get a single presentity with its watchers.
/// One presentity with its watchers.
#[derive(Serialize, utoipa::ToSchema)]
pub struct PresenceDetailResponse {
    aor: String,
    state: String,
    note: Option<String>,
    watcher_count: usize,
    watchers: Vec<WatcherResponse>,
}

#[utoipa::path(
    get,
    path = "/api/v1/presence/{aor}",
    tag = "presence",
    operation_id = "presence_get_presence",
    params(("aor" = String, Path)),
    responses(
        (status = 200, description = "OK", body = PresenceDetailResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_presence(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let info = pm
        .get_presence_for_tenant(&aor, &tenant.0)
        .ok_or_else(|| ApiError::NotFound(format!("Presentity '{}' not found", aor)))?;

    let now = Instant::now();
    let watchers: Vec<WatcherResponse> = pm
        .get_watchers_for_tenant(&aor, &tenant.0)
        .into_iter()
        .map(|w| WatcherResponse {
            watcher_aor: w.watcher_aor,
            subscription_id: w.subscription_id,
            event_package: w.event_package,
            expires_in_secs: w.expires_at.saturating_duration_since(now).as_secs(),
        })
        .collect();

    Ok(Json(PresenceDetailResponse {
        aor: info.aor,
        state: info.state.to_string(),
        note: info.note,
        watcher_count: watchers.len(),
        watchers,
    }))
}

/// PUT /presence/:aor — Set presence state via API.
#[utoipa::path(
    put,
    path = "/api/v1/presence/{aor}",
    tag = "presence",
    operation_id = "presence_update_presence",
    params(("aor" = String, Path)),
    request_body = UpdatePresenceRequest,
    responses(
        (status = 200, description = "OK", body = PresenceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_presence(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
    Json(req): Json<UpdatePresenceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let presence_state = parse_presence_state(&req.state)?;

    let update = PresenceUpdate {
        aor: aor.clone(),
        state: presence_state,
        note: req.note,
    };

    let info = pm
        .update_presence(update, &tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    info!(aor = %aor, state = %info.state, "Presence updated via API");

    Ok(Json(PresenceResponse {
        aor: info.aor,
        state: info.state.to_string(),
        note: info.note,
        watcher_count: pm.get_watchers_for_tenant(&aor, &tenant.0).len(),
        extension: None,
        display_name: None,
    }))
}

/// DELETE /presence/:aor — Remove a presentity.
#[utoipa::path(
    delete,
    path = "/api/v1/presence/{aor}",
    tag = "presence",
    operation_id = "presence_delete_presence",
    params(("aor" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_presence(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    if pm.get_presence_for_tenant(&aor, &tenant.0).is_none() {
        return Err(ApiError::NotFound(format!(
            "Presentity '{}' not found",
            aor
        )));
    }

    pm.remove_presence(&aor, &tenant.0).await;

    info!(aor = %aor, "Presentity removed via API");

    Ok(StatusCode::NO_CONTENT)
}

/// GET /presence/:aor/watchers — List SIP watchers for an AOR.
#[utoipa::path(
    get,
    path = "/api/v1/presence/{aor}/watchers",
    tag = "presence",
    operation_id = "presence_list_watchers",
    params(("aor" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<WatcherResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_watchers(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    // Verify presentity exists
    if pm.get_presence_for_tenant(&aor, &tenant.0).is_none() {
        return Err(ApiError::NotFound(format!(
            "Presentity '{}' not found",
            aor
        )));
    }

    let now = Instant::now();
    let watchers: Vec<WatcherResponse> = pm
        .get_watchers_for_tenant(&aor, &tenant.0)
        .into_iter()
        .map(|w| WatcherResponse {
            watcher_aor: w.watcher_aor,
            subscription_id: w.subscription_id,
            event_package: w.event_package,
            expires_in_secs: w.expires_at.saturating_duration_since(now).as_secs(),
        })
        .collect();

    Ok(Json(crate::page::paginate(
        watchers,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.watcher_aor, r.subscription_id),
    )?))
}

/// GET /presence/:aor/policy — Get authorization policy for a presentity.
#[utoipa::path(
    get,
    path = "/api/v1/presence/{aor}/policy",
    tag = "presence",
    operation_id = "presence_get_policy",
    params(("aor" = String, Path)),
    responses(
        (status = 200, description = "OK", body = PresenceAuthPolicy),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_policy(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    match pm.get_auth_policy_for_tenant(&aor, &tenant.0) {
        Some(policy) => Ok(Json(policy)),
        None => {
            // Return default open policy
            Ok(Json(PresenceAuthPolicy {
                default_allow: true,
                ..Default::default()
            }))
        }
    }
}

/// PUT /presence/:aor/policy — Set authorization policy for a presentity.
#[utoipa::path(
    put,
    path = "/api/v1/presence/{aor}/policy",
    tag = "presence",
    operation_id = "presence_set_policy",
    params(("aor" = String, Path)),
    request_body = PresenceAuthPolicy,
    responses(
        (status = 200, description = "OK", body = PresenceAuthPolicy),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn set_policy(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
    Json(policy): Json<PresenceAuthPolicy>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    pm.set_auth_policy(&aor, policy.clone(), &tenant.0);

    info!(aor = %aor, default_allow = policy.default_allow, "Presence auth policy updated via API");

    Ok(Json(policy))
}

/// DELETE /presence/:aor/policy — Remove authorization policy (revert to default-allow).
#[utoipa::path(
    delete,
    path = "/api/v1/presence/{aor}/policy",
    tag = "presence",
    operation_id = "presence_delete_policy",
    params(("aor" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_policy(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    pm.remove_auth_policy_for_tenant(&aor, &tenant.0);

    info!(aor = %aor, "Presence auth policy removed via API");

    Ok(StatusCode::NO_CONTENT)
}

/// GET /presence/:aor/pres-rules — Get authorization policy as XCAP pres-rules XML (RFC 5025).
#[utoipa::path(
    get,
    path = "/api/v1/presence/{aor}/pres-rules",
    tag = "presence",
    operation_id = "presence_get_pres_rules",
    params(("aor" = String, Path)),
    responses(
        (status = 200, description = "The RFC 5025 rules", content_type = "application/auth-policy+xml", body = String),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_pres_rules(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let policy = pm
        .get_auth_policy_for_tenant(&aor, &tenant.0)
        .unwrap_or(PresenceAuthPolicy {
            default_allow: true,
            ..Default::default()
        });

    let xml = fcp_presence::xcap::generate_pres_rules(&policy);

    Ok((
        StatusCode::OK,
        [("content-type", "application/auth-policy+xml")],
        xml,
    ))
}

/// PUT /presence/:aor/pres-rules — Set authorization policy from XCAP pres-rules XML (RFC 5025).
#[utoipa::path(
    put,
    path = "/api/v1/presence/{aor}/pres-rules",
    tag = "presence",
    operation_id = "presence_set_pres_rules",
    params(("aor" = String, Path)),
    request_body(content = String, content_type = "application/auth-policy+xml"),
    responses(
        (status = 200, description = "The RFC 5025 rules stored", content_type = "application/auth-policy+xml", body = String),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn set_pres_rules(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(aor): Path<String>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    let policy = fcp_presence::xcap::parse_pres_rules(&body)
        .map_err(|e| ApiError::BadRequest(format!("Invalid pres-rules XML: {e}")))?;

    pm.set_auth_policy(&aor, policy.clone(), &tenant.0);

    info!(aor = %aor, default_allow = policy.default_allow, "Presence auth policy updated via XCAP pres-rules");

    Ok((
        StatusCode::OK,
        [("content-type", "application/auth-policy+xml")],
        fcp_presence::xcap::generate_pres_rules(&policy),
    ))
}

/// DELETE /presence/:aor/watchers/:subscription_id — Force-terminate a specific watcher subscription.
///
/// Removes the watcher from the manager and announces `presence.watcher_removed`.
/// No NOTIFY is sent from here.
#[utoipa::path(
    delete,
    path = "/api/v1/presence/{aor}/watchers/{subscription_id}",
    tag = "presence",
    operation_id = "presence_delete_subscription",
    params(("aor" = String, Path), ("subscription_id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_subscription(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((aor, subscription_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let pm = require_presence_manager!(state);

    if pm.get_presence_for_tenant(&aor, &tenant.0).is_none() {
        return Err(ApiError::NotFound(format!(
            "Presentity '{}' not found",
            aor
        )));
    }

    // This tenant's watcher only: the same address may be a presentity in
    // another tenant, with watchers of its own.
    let watching = pm
        .get_watchers_for_tenant(&aor, &tenant.0)
        .iter()
        .any(|w| w.subscription_id == subscription_id);
    if !watching {
        return Err(ApiError::NotFound(format!(
            "Subscription '{}' not found for '{}'",
            subscription_id, aor
        )));
    }
    // Removed, counted and announced as an unsubscribe is.
    pm.remove_watcher_for_tenant(&aor, &subscription_id, &tenant.0)
        .await;

    info!(
        aor = %aor,
        subscription_id = %subscription_id,
        "Watcher force-terminated via API"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Parse a presence state string into a `PresenceState` enum.
fn parse_presence_state(s: &str) -> Result<PresenceState, ApiError> {
    match s {
        "open" => Ok(PresenceState::Open),
        "closed" => Ok(PresenceState::Closed),
        "away" => Ok(PresenceState::Away),
        "busy" => Ok(PresenceState::Busy),
        "on_the_phone" => Ok(PresenceState::OnThePhone),
        "ringing" => Ok(PresenceState::Ringing),
        other => Err(ApiError::BadRequest(format!(
            "Invalid presence state '{}'. Must be one of: open, closed, away, busy, on_the_phone, ringing",
            other
        ))),
    }
}
