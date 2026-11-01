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
#[derive(Debug, Serialize, utoipa::ToSchema)]
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
    /// Find-me / follow-me (§3.6): `enabled`, and `steps` of `targets`
    /// (`devices`, `ext:…`, `number:+…`, `user:…`, a SIP URI), `ring_secs`
    /// and `confirm`.
    pub find_me: Option<fcp_core::FindMe>,
    /// `None`: the node's default ring time.
    pub no_answer_secs: Option<u32>,
    /// The user's class of service (§3.7); `None` is the tenant's default.
    pub class_of_service_id: Option<String>,
    /// The user as an executive (§3.10): `enabled`, `filter` (`all`,
    /// `external`, `none`), `alert` (`assistants`,
    /// `assistants_and_executive`), and `assistants` (`user_id`,
    /// `opted_in`) in order.
    pub executive: Option<ExecutiveView>,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: u64,
}

/// A user as an executive, as the API shows it: user ids in their
/// prefixed form, as everywhere else.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ExecutiveView {
    pub enabled: bool,
    pub filter: fcp_core::ExecutiveFilter,
    pub alert: fcp_core::ExecutiveAlert,
    pub assistants: Vec<AssistantView>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AssistantView {
    pub user_id: String,
    pub opted_in: bool,
}

impl From<&fcp_core::Executive> for ExecutiveView {
    fn from(e: &fcp_core::Executive) -> Self {
        Self {
            enabled: e.enabled,
            filter: e.filter,
            alert: e.alert,
            assistants: e
                .assistants
                .iter()
                .map(|a| AssistantView {
                    user_id: a.user_id.to_string(),
                    opted_in: a.opted_in,
                })
                .collect(),
        }
    }
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
            find_me: settings.find_me.clone(),
            no_answer_secs: settings.no_answer_secs,
            class_of_service_id: settings.class_of_service_id.map(|id| id.to_string()),
            executive: settings.executive.as_ref().map(ExecutiveView::from),
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
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
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
    /// Replaces the find-me whole when given.
    pub find_me: Option<fcp_core::FindMe>,
    pub no_answer_secs: Option<u32>,
    /// A class of service of the tenant, or an empty string for the
    /// tenant's default. An admin's to set: refused at `/me/calling`.
    pub class_of_service_id: Option<String>,
    /// The user as an executive (§3.10); each field kept when left out.
    pub executive: Option<ExecutiveRequest>,
}

/// What may change of a user as an executive.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct ExecutiveRequest {
    pub enabled: Option<bool>,
    pub filter: Option<fcp_core::ExecutiveFilter>,
    pub alert: Option<fcp_core::ExecutiveAlert>,
    /// The assistants in order, each a username or a user id of the
    /// tenant; replaces the list, keeping each one's opt-in. An admin's to
    /// set: refused at `/me/calling`.
    pub assistants: Option<Vec<String>>,
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
/// A hold-music choice as given (a prompt of the tenant, `none` for
/// silence, empty for the tenant's) as it is kept.
pub(crate) async fn hold_music_choice(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    given: &str,
) -> Result<Option<String>, ApiError> {
    Ok(match given.trim() {
        "" => None,
        fcp_core::calling::HOLD_MUSIC_NONE => Some(fcp_core::calling::HOLD_MUSIC_NONE.to_string()),
        id => {
            let prompt_id = id
                .parse::<fcp_core::PromptId>()
                .map_err(|_| ApiError::BadRequest(format!("hold_music: not a prompt id: {id}")))?;
            let prompt = require_storage!(state)
                .prompt_store()
                .ok_or_else(|| ApiError::BadRequest("hold_music: no prompt store".into()))?
                .get_prompt(&prompt_id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .filter(|p| p.visible_to(tenant))
                .ok_or_else(|| ApiError::BadRequest(format!("hold_music: no prompt {id}")))?;
            Some(prompt.id.to_string())
        }
    })
}

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
    if let Some(find_me) = req.find_me {
        settings.find_me = Some(find_me);
    }
    if let Some(music) = req.hold_music {
        settings.hold_music = hold_music_choice(state, &user.tenant_id, &music).await?;
    }
    if let Some(given) = req.class_of_service_id {
        if by != "admin" {
            return Err(ApiError::Forbidden(
                "class_of_service_id: a user's class of service is set by an admin".into(),
            ));
        }
        settings.class_of_service_id = match given.trim() {
            "" => None,
            id => {
                let id = id.parse::<fcp_core::ClassOfServiceId>().map_err(|_| {
                    ApiError::BadRequest(format!("class_of_service_id: not a class id: {id}"))
                })?;
                crate::v1::classes_of_service::tenant_class(state, &user.tenant_id, &id).await?;
                Some(id)
            }
        };
    }
    if let Some(secs) = req.no_answer_secs {
        settings.no_answer_secs = if secs == 0 { None } else { Some(secs) };
    }
    if let Some(given) = req.executive {
        let mut executive = settings.executive.clone().unwrap_or_default();
        if let Some(enabled) = given.enabled {
            executive.enabled = enabled;
        }
        if let Some(filter) = given.filter {
            executive.filter = filter;
        }
        if let Some(alert) = given.alert {
            executive.alert = alert;
        }
        if let Some(names) = given.assistants {
            if by != "admin" {
                return Err(ApiError::Forbidden(
                    "executive.assistants: an executive's assistants are set by an admin".into(),
                ));
            }
            let mut assistants = Vec::with_capacity(names.len());
            for name in names {
                let assistant = assistant_named(state, &user.tenant_id, name.trim()).await?;
                let opted_in = executive
                    .assistants
                    .iter()
                    .find(|a| a.user_id == assistant.id)
                    .map(|a| a.opted_in)
                    .unwrap_or(true);
                assistants.push(fcp_core::Assistant {
                    user_id: assistant.id,
                    opted_in,
                });
            }
            executive.assistants = assistants;
        }
        settings.executive = Some(executive);
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

/// An assistant named by username or user id: an enabled user of the
/// tenant.
async fn assistant_named(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    name: &str,
) -> Result<User, ApiError> {
    let storage = require_storage!(state);
    let found = match name.parse::<fcp_core::UserId>() {
        Ok(id) => storage.get_user_by_id(&id).await?,
        Err(_) => storage.get_user_in_tenant(name, tenant).await?,
    };
    found
        .filter(|u| u.tenant_id == *tenant && u.enabled)
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "executive.assistants: no user '{name}' in the tenant"
            ))
        })
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

pub(crate) async fn user_named(
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
#[utoipa::path(
    get,
    path = "/api/v1/users/{username}/calling",
    tag = "calling",
    operation_id = "calling_get_user_calling",
    params(("username" = String, Path)),
    responses(
        (status = 200, description = "OK", body = CallingResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
#[utoipa::path(
    put,
    path = "/api/v1/users/{username}/calling",
    tag = "calling",
    operation_id = "calling_update_user_calling",
    params(("username" = String, Path)),
    request_body = UpdateCallingRequest,
    responses(
        (status = 200, description = "OK", body = CallingResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
