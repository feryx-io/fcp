//! Hoteling sessions (`docs/ENTERPRISE_CALLING.md` §3.8): who is logged in
//! on which host phone, and ending a session. A login happens at the phone
//! (`*88`); the API reads sessions and ends them.

use crate::{auth::AuthIdentity, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::storage::{CredentialStore, DeviceStore};
use fcp_core::{DynStorage, HotelingSession, HotelingStore, SessionEnd};
use fcp_events::{PlatformEvent, UserEvent};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::SystemTime;
use tracing::info;

macro_rules! require_storage {
    ($state:expr) => {
        $state
            .storage
            .as_ref()
            .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?
    };
}

/// A session, with the names of the host and the guest.
#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub device_id: String,
    pub device_name: Option<String>,
    pub user_id: String,
    pub username: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    pub started_at: SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    pub expires_at: SystemTime,
    #[serde(default, with = "crate::time::rfc3339_opt")]
    pub ended_at: Option<SystemTime>,
    /// `logout`, `admin`, `expired` or `replaced`.
    pub ended_by: Option<String>,
    pub active: bool,
}

pub async fn session_response(storage: &DynStorage, session: HotelingSession) -> SessionResponse {
    let device_name = storage
        .get_device(&session.device_id)
        .await
        .ok()
        .flatten()
        .map(|d| d.name);
    let username = storage
        .get_user_by_id(&session.user_id)
        .await
        .ok()
        .flatten()
        .map(|u| u.username);
    SessionResponse {
        id: session.id.to_string(),
        device_id: session.device_id.to_string(),
        device_name,
        user_id: session.user_id.to_string(),
        username,
        started_at: session.started_at,
        expires_at: session.expires_at,
        ended_at: session.ended_at,
        ended_by: session.ended_by.map(|e| e.as_str().to_string()),
        active: session.is_active_at(SystemTime::now()),
    }
}

/// End `session` and say so. The call managers hear the event and tell
/// the phone to re-read its configuration, which the API cannot.
pub async fn end_session(
    state: &ApiState,
    storage: &DynStorage,
    session: &HotelingSession,
    by: SessionEnd,
) -> Result<bool, ApiError> {
    let ended = storage
        .end_hoteling_session(&session.id, by)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to end the session: {}", e)))?;
    let Some(ended) = ended else {
        return Ok(false);
    };
    info!(session_id = %ended.id, device_id = %ended.device_id, reason = by.as_str(), "Hoteling session ended by the API");
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::HotelingLogout {
            session_id: ended.id.to_string(),
            user_id: ended.user_id.to_string(),
            device_id: ended.device_id.to_string(),
            tenant_id: ended.tenant_id,
            reason: by.as_str().to_string(),
            resynced: false,
            timestamp: SystemTime::now(),
        }))
        .await;
    Ok(true)
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Ended sessions too, newest first.
    #[serde(default)]
    pub all: bool,
}

/// GET /v1/hoteling/sessions
pub async fn list_sessions(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Query(query): Query<ListQuery>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let sessions = storage
        .list_hoteling_sessions(&tenant.0, query.all)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list sessions: {}", e)))?;
    let mut out = Vec::with_capacity(sessions.len());
    for session in sessions {
        out.push(session_response(storage, session).await);
    }
    Ok(Json(crate::page::paginate(
        out,
        &page,
        crate::page::Order::Descending,
        |r| crate::page::system_time_key(r.started_at, &r.id),
    )?))
}

/// DELETE /v1/hoteling/sessions/:id
pub async fn end_session_by_id(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let id = fcp_core::HotelingSessionId::from_str(&id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid session ID: {}", e)))?;
    let session = storage
        .get_hoteling_session(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read the session: {}", e)))?
        .filter(|s| s.tenant_id == tenant.0)
        .ok_or_else(|| ApiError::NotFound(format!("Session {} not found", id)))?;
    if !end_session(&state, storage, &session, SessionEnd::Admin).await? {
        return Err(ApiError::NotFound(format!("Session {} is not active", id)));
    }
    Ok(StatusCode::NO_CONTENT)
}
