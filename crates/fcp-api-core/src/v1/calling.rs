//! A user's calling settings (`docs/ENTERPRISE_CALLING.md` §3.1): do not
//! disturb, call waiting, the four forwards and the ring time — read and
//! written by an admin at `/users/:username/calling` and by the user at
//! `/me/calling` (`self_service`), both through what is here.

use crate::{auth::AuthIdentity, error::ApiError, state::ApiState, validation};
use axum::{
    extract::{Path, State},
    Extension, Json,
};
use fcp_core::storage::CredentialStore;
use fcp_core::{CallTarget, CallingSettings, CallingStore, Permission, User};
use fcp_events::{PlatformEvent, UserEvent};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// The settings as the API shows them: targets in their string form
/// (`voicemail`, `number:+15551234567`, `ext:2001`, `user:<id>`, a SIP URI).
#[derive(Debug, Serialize)]
pub struct CallingResponse {
    pub user_id: String,
    pub username: String,
    pub dnd: bool,
    pub call_waiting: bool,
    pub forward_all: Option<String>,
    pub forward_busy: Option<String>,
    pub forward_no_answer: Option<String>,
    pub forward_unreachable: Option<String>,
    /// A prompt's id, `none` for silence, or absent for the tenant's music.
    pub hold_music: Option<String>,
    /// The user's hours and where calls go outside them (§3.5).
    pub after_hours_schedule_id: Option<String>,
    pub after_hours_forward: Option<String>,
    /// `None`: the node's default ring time.
    pub no_answer_secs: Option<u32>,
    pub updated_at: u64,
}

impl CallingResponse {
    pub fn of(user: &User, settings: &CallingSettings) -> Self {
        let target = |t: &Option<CallTarget>| t.as_ref().map(|t| t.to_string());
        Self {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            dnd: settings.dnd,
            call_waiting: settings.call_waiting,
            forward_all: target(&settings.forward_all),
            forward_busy: target(&settings.forward_busy),
            forward_no_answer: target(&settings.forward_no_answer),
            forward_unreachable: target(&settings.forward_unreachable),
            hold_music: settings.hold_music.clone(),
            after_hours_schedule_id: settings.after_hours_schedule_id.map(|id| id.to_string()),
            after_hours_forward: settings.after_hours_forward.as_ref().map(|t| t.to_string()),
            no_answer_secs: settings.no_answer_secs,
            updated_at: settings
                .updated_at
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }
}

/// What may change. A forward given as an empty string is cleared; one
/// left out is kept. A ring time of `0` clears it (the node's default).
#[derive(Debug, Default, Deserialize)]
pub struct UpdateCallingRequest {
    pub dnd: Option<bool>,
    pub call_waiting: Option<bool>,
    pub forward_all: Option<String>,
    pub forward_busy: Option<String>,
    pub forward_no_answer: Option<String>,
    pub forward_unreachable: Option<String>,
    /// A prompt of the tenant, `none`, or an empty string for the
    /// tenant's hold music.
    pub hold_music: Option<String>,
    /// A schedule of the tenant, or an empty string to clear it.
    pub after_hours_schedule_id: Option<String>,
    /// A call target, or an empty string to clear it.
    pub after_hours_forward: Option<String>,
    pub no_answer_secs: Option<u32>,
}

/// A user's settings, the defaults when none are stored.
pub async fn settings_of(state: &ApiState, user: &User) -> Result<CallingSettings, ApiError> {
    let storage = require_storage!(state);
    Ok(storage
        .get_calling_settings(&user.tenant_id, &user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| CallingSettings::defaults(user.tenant_id, user.id)))
}

/// Apply a request to a user's settings, store them and announce the
/// change (`user.calling_changed`).
pub async fn apply(
    state: &ApiState,
    user: &User,
    req: UpdateCallingRequest,
    by: &str,
) -> Result<CallingSettings, ApiError> {
    let storage = require_storage!(state);
    let mut settings = settings_of(state, user).await?;
    if let Some(dnd) = req.dnd {
        settings.dnd = dnd;
    }
    if let Some(call_waiting) = req.call_waiting {
        settings.call_waiting = call_waiting;
    }
    for (name, given, slot) in [
        ("forward_all", req.forward_all, &mut settings.forward_all),
        ("forward_busy", req.forward_busy, &mut settings.forward_busy),
        (
            "forward_no_answer",
            req.forward_no_answer,
            &mut settings.forward_no_answer,
        ),
        (
            "forward_unreachable",
            req.forward_unreachable,
            &mut settings.forward_unreachable,
        ),
    ] {
        if let Some(given) = given {
            *slot = parse_target(name, &given)?;
        }
    }
    if let Some(id) = req.after_hours_schedule_id {
        settings.after_hours_schedule_id =
            crate::v1::schedules::tenant_schedule(state, &user.tenant_id, &id).await?;
    }
    if let Some(given) = req.after_hours_forward {
        settings.after_hours_forward = parse_target("after_hours_forward", &given)?;
    }
    if let Some(music) = req.hold_music {
        settings.hold_music = match music.trim() {
            "" => None,
            fcp_core::calling::HOLD_MUSIC_NONE => {
                Some(fcp_core::calling::HOLD_MUSIC_NONE.to_string())
            }
            id => {
                let prompt_id = id.parse::<fcp_core::PromptId>().map_err(|_| {
                    ApiError::BadRequest(format!("hold_music: not a prompt id: {id}"))
                })?;
                let prompt = storage
                    .prompt_store()
                    .ok_or_else(|| ApiError::BadRequest("hold_music: no prompt store".into()))?
                    .get_prompt(&prompt_id)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .filter(|p| p.tenant_id == user.tenant_id)
                    .ok_or_else(|| ApiError::BadRequest(format!("hold_music: no prompt {id}")))?;
                Some(prompt.id.to_string())
            }
        };
    }
    if let Some(secs) = req.no_answer_secs {
        settings.no_answer_secs = if secs == 0 { None } else { Some(secs) };
    }
    settings.updated_at = SystemTime::now();
    settings.validate().map_err(ApiError::BadRequest)?;
    storage
        .put_calling_settings(settings.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    info!(username = %user.username, by, "Calling settings updated");
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::CallingChanged {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
    Ok(settings)
}

/// A target from the API: empty clears it.
fn parse_target(name: &str, given: &str) -> Result<Option<CallTarget>, ApiError> {
    if given.trim().is_empty() {
        return Ok(None);
    }
    CallTarget::parse(given)
        .map(Some)
        .map_err(|e| ApiError::BadRequest(format!("{name}: {e}")))
}

async fn user_named(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    username: &str,
) -> Result<User, ApiError> {
    validation::validate_username(username)?;
    let storage = require_storage!(state);
    storage
        .get_user_in_tenant(username, tenant)
        .await?
        .filter(|user| user.tenant_id == *tenant)
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
}

/// GET /api/v1/users/:username/calling
pub async fn get_user_calling(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<Json<CallingResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserRead, state.deployment_mode)?;
    let user = user_named(&state, &tenant.0, &username).await?;
    let settings = settings_of(&state, &user).await?;
    Ok(Json(CallingResponse::of(&user, &settings)))
}

/// PUT /api/v1/users/:username/calling
pub async fn update_user_calling(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Json(req): Json<UpdateCallingRequest>,
) -> Result<Json<CallingResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserUpdate, state.deployment_mode)?;
    let user = user_named(&state, &tenant.0, &username).await?;
    let settings = apply(&state, &user, req, "admin").await?;
    Ok(Json(CallingResponse::of(&user, &settings)))
}
