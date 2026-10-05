//! A user's devices (`docs/SOFTPHONE.md` §2.1): their desk phones and the
//! app installs that signed in, each with its app session, and the button
//! that removes one — ending its app's session and its registrations.

use crate::{
    auth::AuthIdentity,
    error::ApiError,
    state::ApiState,
    v1::auth_session::{session_store, Me},
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::storage::{CredentialStore, DeviceStore};
use fcp_core::{Device, DeviceId, UserId};
use serde::Serialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::SystemTime;

/// An app install's session, on its device.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeviceAppResponse {
    pub install_id: String,
    pub platform: Option<String>,
    pub version: Option<String>,
    /// Whether the app is signed in now.
    pub signed_in: bool,
    /// When it signed in, while it is.
    #[serde(default, with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub signed_in_at: Option<SystemTime>,
    /// When it last refreshed its session, while it is signed in.
    #[serde(default, with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_seen: Option<SystemTime>,
}

/// One of a user's devices.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UserDeviceResponse {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub model: Option<String>,
    pub sip_username: Option<String>,
    pub registration_state: String,
    #[serde(default, with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_registration: Option<SystemTime>,
    pub user_agent: Option<String>,
    /// The app install it is, for a softphone that signed in.
    pub app: Option<DeviceAppResponse>,
}

async fn devices_of(
    state: &ApiState,
    user_id: &UserId,
) -> Result<Vec<UserDeviceResponse>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let devices = storage.list_devices_by_user(user_id).await?;
    let mut installs: HashMap<DeviceId, String> = HashMap::new();
    let mut live: HashMap<DeviceId, fcp_core::session::LoginSession> = HashMap::new();
    if let Some(sessions) = session_store(state) {
        installs.extend(sessions.app_installs_of_user(user_id).await?);
        for s in sessions.list_user_sessions(user_id).await? {
            if let Some(device) = s.device_id {
                live.entry(device).or_insert(s);
            }
        }
    }
    Ok(devices
        .into_iter()
        .map(|d| {
            let app = installs.get(&d.id).map(|install| {
                let session = live.get(&d.id);
                let client = session.and_then(|s| s.app.clone());
                DeviceAppResponse {
                    install_id: install.clone(),
                    platform: client.as_ref().and_then(|c| c.platform.clone()),
                    version: client.as_ref().and_then(|c| c.version.clone()),
                    signed_in: session.is_some(),
                    signed_in_at: session.map(|s| s.created_at),
                    last_seen: session.map(|s| s.refreshed_at),
                }
            });
            response(d, app)
        })
        .collect())
}

fn response(d: Device, app: Option<DeviceAppResponse>) -> UserDeviceResponse {
    UserDeviceResponse {
        id: d.id.to_string(),
        name: d.name,
        device_type: d.device_type.to_string(),
        model: d.model,
        sip_username: d.sip_username,
        registration_state: d.registration_state.to_string(),
        last_registration: d.last_registration,
        user_agent: d.user_agent,
        app,
    }
}

fn me_id(me: &Me) -> Result<UserId, ApiError> {
    me.0.user_id
        .as_deref()
        .and_then(|id| id.trim_start_matches("user-").parse().ok())
        .ok_or_else(|| ApiError::Unauthorized("Sign in again".to_string()))
}

/// GET /api/v1/me/devices — your devices, with each app install's session.
#[utoipa::path(
    get,
    path = "/api/v1/me/devices",
    tag = "user_devices",
    operation_id = "user_devices_list_mine",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<UserDeviceResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_mine(
    State(state): State<ApiState>,
    me: Me,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let devices = devices_of(&state, &me_id(&me)?).await?;
    Ok(Json(crate::page::paginate(
        devices,
        &page,
        crate::page::Order::Ascending,
        |d| format!("{}\u{0}{}", d.name.to_lowercase(), d.id),
    )?))
}

/// DELETE /api/v1/me/devices/{id} — remove one of your devices: an app
/// install's session ends and its registrations go.
#[utoipa::path(
    delete,
    path = "/api/v1/me/devices/{id}",
    tag = "user_devices",
    operation_id = "user_devices_delete_mine",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Removed"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_mine(
    State(state): State<ApiState>,
    me: Me,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let user_id = me_id(&me)?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let not_found = || ApiError::NotFound(format!("Device {id} not found"));
    let device_id = DeviceId::from_str(&id).map_err(|_| not_found())?;
    let device = storage
        .get_device(&device_id)
        .await?
        .filter(|d| d.owner_user_id == Some(user_id))
        .ok_or_else(not_found)?;
    crate::v1::devices::remove_device(&state, storage, &device).await?;
    tracing::info!(device = %device.id, user = %user_id, "A user removed their device");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/users/{username}/devices — a user's devices, as they see
/// them at `/me/devices`.
#[utoipa::path(
    get,
    path = "/api/v1/users/{username}/devices",
    tag = "user_devices",
    operation_id = "user_devices_list",
    params(("username" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<UserDeviceResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_users(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User {username} not found")))?;
    let devices = devices_of(&state, &user.id).await?;
    Ok(Json(crate::page::paginate(
        devices,
        &page,
        crate::page::Order::Ascending,
        |d| format!("{}\u{0}{}", d.name.to_lowercase(), d.id),
    )?))
}
