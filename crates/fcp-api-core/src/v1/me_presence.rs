//! Presence a person sets (`docs/PRESENCE.md`): `/me/presence`, the status
//! a user chose with a note and a time it clears at, their own statuses,
//! and what others see of them; and an administrator's read of anyone's.
//!
//! Picking a status whose state is `dnd` turns the user's calling DND on;
//! picking any other turns it off, so DND is one switch.

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;
use crate::v1::auth_session::Me;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::presence_status::{
    ChosenStatus, OwnStatus, Presence, PresenceStatusStore, Resolved, Shown, StatusState,
    MAX_LABEL, MAX_NOTE, MAX_STATUSES, MAX_UNTIL,
};
use fcp_core::storage::{DynStorage, User};
use fcp_core::{CallingStore, CredentialStore, Permission, UserId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// One of a user's own statuses.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct OwnStatusResponse {
    pub id: String,
    pub label: String,
    pub state: StatusState,
}

impl From<OwnStatus> for OwnStatusResponse {
    fn from(s: OwnStatus) -> Self {
        Self {
            id: s.id,
            label: s.label,
            state: s.state,
        }
    }
}

/// A user's presence: what others see, what they chose, and their own
/// statuses to choose from.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MyPresenceResponse {
    /// What others see.
    pub presence: Presence,
    /// What they chose: a built-in state's name or one of `statuses`' ids.
    pub status: String,
    /// The chosen status's state.
    pub status_state: StatusState,
    pub note: Option<String>,
    /// When the chosen status clears back to available.
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub until: Option<SystemTime>,
    pub statuses: Vec<OwnStatusResponse>,
}

/// What may change. `status` sets the chosen status (a built-in state's
/// name or one of the user's statuses' ids) with `note` and `until`, both
/// cleared when left out; without `status`, a `note` or `until` given
/// changes only that (`null` clears it). `idle` is an app's word that its
/// person is idle (`true`) or back (`false`), lasting 15 minutes unless
/// repeated.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct UpdateMyPresenceRequest {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub note: Option<Option<String>>,
    /// RFC 3339, in the future and within 7 days.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub until: Option<Option<String>>,
    #[serde(default)]
    pub idle: Option<bool>,
}

/// A field that may be left out (`None`), `null` (`Some(None)`) or set.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// One of a user's own statuses, as written.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct OwnStatusRequest {
    pub label: String,
    pub state: StatusState,
}

/// What may change of one of a user's own statuses.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct UpdateOwnStatusRequest {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub state: Option<StatusState>,
}

/// A user's own statuses in a new order: every id, once.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct OrderStatusesRequest {
    pub ids: Vec<String>,
}

/// A user's presence, as an administrator reads it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UserPresenceResponse {
    pub username: String,
    pub presence: Presence,
    pub status: String,
    pub status_state: StatusState,
    pub status_label: Option<String>,
    pub note: Option<String>,
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub until: Option<SystemTime>,
}

fn store(state: &ApiState) -> Result<&Arc<dyn PresenceStatusStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.presence_status_store())
        .ok_or_else(|| ApiError::NotFound("Presence statuses are not kept here".to_string()))
}

async fn me(state: &ApiState, me: &Me) -> Result<User, ApiError> {
    let storage = require_storage!(state);
    crate::v1::self_service::load_claims_user(storage, &me.0, state).await
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

fn trimmed(v: Option<String>, field: &str, max: usize) -> Result<Option<String>, ApiError> {
    let v = v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if v.as_ref().is_some_and(|v| v.chars().count() > max) {
        return Err(ApiError::BadRequest(format!(
            "{field} may be at most {max} characters"
        )));
    }
    Ok(v)
}

fn until_of(given: Option<String>, now: SystemTime) -> Result<Option<SystemTime>, ApiError> {
    let Some(given) = given.filter(|g| !g.trim().is_empty()) else {
        return Ok(None);
    };
    let at = chrono::DateTime::parse_from_rfc3339(given.trim())
        .map_err(|_| ApiError::BadRequest(format!("until: not an RFC 3339 time: {given}")))?;
    let at: SystemTime = at.with_timezone(&chrono::Utc).into();
    if at <= now {
        return Err(ApiError::BadRequest("until: must be in the future".into()));
    }
    if at.duration_since(now).unwrap_or_default() > MAX_UNTIL {
        return Err(ApiError::BadRequest("until: at most 7 days ahead".into()));
    }
    Ok(Some(at))
}

/// Turn a user's calling DND on or off; whether it changed. The admin's
/// presence turns it off when a `dnd` status clears.
pub async fn set_dnd(storage: &DynStorage, user: &UserId, on: bool) -> anyhow::Result<bool> {
    let Some(u) = storage.get_user_by_id(user).await? else {
        return Ok(false);
    };
    let mut settings = storage
        .get_calling_settings(&u.tenant_id, &u.id)
        .await?
        .unwrap_or_else(|| fcp_core::CallingSettings::defaults(u.tenant_id, u.id));
    if settings.dnd == on {
        return Ok(false);
    }
    settings.dnd = on;
    settings.updated_at = SystemTime::now();
    storage.put_calling_settings(settings).await?;
    Ok(true)
}

/// Announce a user's calling settings changed, as `/me/calling` does.
async fn calling_changed(state: &ApiState, user: &User) {
    let _ = state
        .event_bus
        .publish(fcp_events::PlatformEvent::User(
            fcp_events::UserEvent::CallingChanged {
                user_id: user.id.to_string(),
                username: user.username.clone(),
                tenant_id: user.tenant_id,
                timestamp: SystemTime::now(),
            },
        ))
        .await;
}

/// DND to match a chosen status's state, announced when it changed.
async fn follow_dnd(state: &ApiState, user: &User, chosen: StatusState) -> Result<(), ApiError> {
    let storage = require_storage!(state);
    if set_dnd(storage, &user.id, chosen == StatusState::Dnd)
        .await
        .map_err(internal)?
    {
        calling_changed(state, user).await;
    }
    Ok(())
}

/// What others see of `user`: the admin's reckoning, or (where none is
/// kept) what they chose.
async fn presence_of(state: &ApiState, user: &User, chosen: &Resolved) -> Presence {
    if let Some(people) = &state.people_presence {
        if let Some(p) = people.of(&user.tenant_id, &user.id).await {
            return p;
        }
    }
    Presence {
        state: Shown::from(chosen.state),
        label: chosen.label.clone(),
        note: chosen.note.clone(),
        idle: false,
    }
}

async fn response(state: &ApiState, user: &User) -> Result<MyPresenceResponse, ApiError> {
    let store = store(state)?;
    let chosen = store.get_chosen_status(&user.id).await.map_err(internal)?;
    let own = store.list_own_statuses(&user.id).await.map_err(internal)?;
    let now = SystemTime::now();
    let resolved = Resolved::of(chosen.as_ref(), &own, now);
    let status = chosen
        .filter(|c| {
            !c.cleared(now) && (StatusState::parse(&c.status).is_some() || c_is_own(c, &own))
        })
        .map(|c| c.status)
        .unwrap_or_else(|| StatusState::Available.as_str().to_string());
    Ok(MyPresenceResponse {
        presence: presence_of(state, user, &resolved).await,
        status,
        status_state: resolved.state,
        note: resolved.note,
        until: resolved.until,
        statuses: own.into_iter().map(Into::into).collect(),
    })
}

/// Whether `c` names one of `own`.
fn c_is_own(c: &ChosenStatus, own: &[OwnStatus]) -> bool {
    own.iter().any(|s| s.id == c.status)
}

/// GET /me/presence
#[utoipa::path(
    get,
    path = "/api/v1/me/presence",
    tag = "self_service",
    operation_id = "self_service_get_presence",
    responses(
        (status = 200, description = "OK", body = MyPresenceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_my_presence(
    State(state): State<ApiState>,
    me_: Me,
) -> Result<Json<MyPresenceResponse>, ApiError> {
    let user = me(&state, &me_).await?;
    Ok(Json(response(&state, &user).await?))
}

/// PUT /me/presence
#[utoipa::path(
    put,
    path = "/api/v1/me/presence",
    tag = "self_service",
    operation_id = "self_service_update_presence",
    request_body = UpdateMyPresenceRequest,
    responses(
        (status = 200, description = "OK", body = MyPresenceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_my_presence(
    State(state): State<ApiState>,
    me_: Me,
    Json(req): Json<UpdateMyPresenceRequest>,
) -> Result<Json<MyPresenceResponse>, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let now = SystemTime::now();
    let own = store.list_own_statuses(&user.id).await.map_err(internal)?;
    let kept = store
        .get_chosen_status(&user.id)
        .await
        .map_err(internal)?
        .filter(|c| !c.cleared(now));

    let chosen = if let Some(status) = req.status {
        let status = status.trim().to_string();
        if StatusState::parse(&status).is_none() && !own.iter().any(|s| s.id == status) {
            return Err(ApiError::BadRequest(format!("status: no status {status}")));
        }
        Some(ChosenStatus {
            tenant_id: user.tenant_id,
            user_id: user.id,
            status,
            note: trimmed(req.note.flatten(), "note", MAX_NOTE)?,
            until: until_of(req.until.flatten(), now)?,
            updated_at: now,
        })
    } else if req.note.is_some() || req.until.is_some() {
        let mut c = kept.clone().unwrap_or(ChosenStatus {
            tenant_id: user.tenant_id,
            user_id: user.id,
            status: StatusState::Available.as_str().to_string(),
            note: None,
            until: None,
            updated_at: now,
        });
        if let Some(note) = req.note {
            c.note = trimmed(note, "note", MAX_NOTE)?;
        }
        if let Some(until) = req.until {
            c.until = until_of(until, now)?;
        }
        c.updated_at = now;
        Some(c)
    } else {
        None
    };

    if let Some(c) = chosen {
        // Available with nothing to say is no row.
        if c.status == StatusState::Available.as_str() && c.note.is_none() && c.until.is_none() {
            store
                .delete_chosen_status(&user.id)
                .await
                .map_err(internal)?;
        } else {
            store.put_chosen_status(&c).await.map_err(internal)?;
        }
        let resolved = Resolved::of(Some(&c), &own, now);
        follow_dnd(&state, &user, resolved.state).await?;
        if let Some(people) = &state.people_presence {
            people.chosen_changed(&user.tenant_id, &user.id).await;
        }
    }
    if let Some(idle) = req.idle {
        if let Some(people) = &state.people_presence {
            let session = me_.0.sid.clone().unwrap_or_else(|| "session".to_string());
            people
                .set_idle(&user.tenant_id, &user.id, &session, idle)
                .await;
        }
    }
    Ok(Json(response(&state, &user).await?))
}

/// GET /me/presence/statuses
#[utoipa::path(
    get,
    path = "/api/v1/me/presence/statuses",
    tag = "self_service",
    operation_id = "self_service_list_presence_statuses",
    responses(
        (status = 200, description = "OK", body = crate::page::Page<OwnStatusResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_statuses(
    State(state): State<ApiState>,
    me_: Me,
) -> Result<impl IntoResponse, ApiError> {
    let user = me(&state, &me_).await?;
    let own = store(&state)?
        .list_own_statuses(&user.id)
        .await
        .map_err(internal)?;
    let total = own.len() as u64;
    let data: Vec<OwnStatusResponse> = own.into_iter().map(Into::into).collect();
    Ok(Json(crate::page::Page::new(data, None, Some(total))))
}

/// POST /me/presence/statuses
#[utoipa::path(
    post,
    path = "/api/v1/me/presence/statuses",
    tag = "self_service",
    operation_id = "self_service_create_presence_status",
    request_body = OwnStatusRequest,
    responses(
        (status = 201, description = "Kept", body = OwnStatusResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_status(
    State(state): State<ApiState>,
    me_: Me,
    Json(req): Json<OwnStatusRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let label = trimmed(Some(req.label), "label", MAX_LABEL)?
        .ok_or_else(|| ApiError::BadRequest("A status needs a label".into()))?;
    let own = store.list_own_statuses(&user.id).await.map_err(internal)?;
    if own.len() >= MAX_STATUSES {
        return Err(ApiError::Conflict(format!(
            "A user may keep at most {MAX_STATUSES} statuses"
        )));
    }
    let status = OwnStatus {
        id: fcp_core::presence_status::new_status_id(),
        tenant_id: user.tenant_id,
        user_id: user.id,
        label,
        state: req.state,
        position: own.iter().map(|s| s.position + 1).max().unwrap_or(0),
        updated_at: SystemTime::now(),
        deleted_at: None,
    };
    store.put_own_status(&status).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(OwnStatusResponse::from(status))))
}

/// One of the user's own statuses, or 404.
async fn mine(
    store: &Arc<dyn PresenceStatusStore>,
    user: &User,
    id: &str,
) -> Result<(OwnStatus, Vec<OwnStatus>), ApiError> {
    let own = store.list_own_statuses(&user.id).await.map_err(internal)?;
    let found = own
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("No status {id}")))?;
    Ok((found, own))
}

/// PUT /me/presence/statuses/{id}
#[utoipa::path(
    put,
    path = "/api/v1/me/presence/statuses/{id}",
    tag = "self_service",
    operation_id = "self_service_update_presence_status",
    params(("id" = String, Path)),
    request_body = UpdateOwnStatusRequest,
    responses(
        (status = 200, description = "OK", body = OwnStatusResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_status(
    State(state): State<ApiState>,
    me_: Me,
    Path(id): Path<String>,
    Json(req): Json<UpdateOwnStatusRequest>,
) -> Result<Json<OwnStatusResponse>, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let (mut status, _) = mine(store, &user, &id).await?;
    if let Some(label) = req.label {
        status.label = trimmed(Some(label), "label", MAX_LABEL)?
            .ok_or_else(|| ApiError::BadRequest("A status needs a label".into()))?;
    }
    let state_changed = req.state.is_some_and(|s| s != status.state);
    if let Some(s) = req.state {
        status.state = s;
    }
    status.updated_at = SystemTime::now();
    store.put_own_status(&status).await.map_err(internal)?;
    // The status in use: DND and what others see follow it.
    let in_use = store
        .get_chosen_status(&user.id)
        .await
        .map_err(internal)?
        .is_some_and(|c| c.status == status.id);
    if in_use {
        if state_changed {
            follow_dnd(&state, &user, status.state).await?;
        }
        if let Some(people) = &state.people_presence {
            people.chosen_changed(&user.tenant_id, &user.id).await;
        }
    }
    Ok(Json(status.into()))
}

/// DELETE /me/presence/statuses/{id}
#[utoipa::path(
    delete,
    path = "/api/v1/me/presence/statuses/{id}",
    tag = "self_service",
    operation_id = "self_service_delete_presence_status",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Deleted"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_status(
    State(state): State<ApiState>,
    me_: Me,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let (status, _) = mine(store, &user, &id).await?;
    let in_use = store
        .get_chosen_status(&user.id)
        .await
        .map_err(internal)?
        .is_some_and(|c| c.status == status.id);
    // The status in use goes back to available first.
    if in_use {
        store
            .delete_chosen_status(&user.id)
            .await
            .map_err(internal)?;
        follow_dnd(&state, &user, StatusState::Available).await?;
    }
    store
        .delete_own_status(&user.id, &status.id)
        .await
        .map_err(internal)?;
    if in_use {
        if let Some(people) = &state.people_presence {
            people.chosen_changed(&user.tenant_id, &user.id).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// PUT /me/presence/statuses/order
#[utoipa::path(
    put,
    path = "/api/v1/me/presence/statuses/order",
    tag = "self_service",
    operation_id = "self_service_order_presence_statuses",
    request_body = OrderStatusesRequest,
    responses(
        (status = 200, description = "OK", body = crate::page::Page<OwnStatusResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn order_statuses(
    State(state): State<ApiState>,
    me_: Me,
    Json(req): Json<OrderStatusesRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = me(&state, &me_).await?;
    let store = store(&state)?;
    let own = store.list_own_statuses(&user.id).await.map_err(internal)?;
    let mut given = req.ids.clone();
    given.sort();
    given.dedup();
    let mut have: Vec<String> = own.iter().map(|s| s.id.clone()).collect();
    have.sort();
    if given != have || req.ids.len() != own.len() {
        return Err(ApiError::BadRequest(
            "ids: every one of the user's statuses, once".into(),
        ));
    }
    let now = SystemTime::now();
    let mut ordered = Vec::new();
    for (position, id) in req.ids.iter().enumerate() {
        let mut s = own
            .iter()
            .find(|s| &s.id == id)
            .cloned()
            .expect("checked above");
        s.position = position as i32;
        s.updated_at = now;
        store.put_own_status(&s).await.map_err(internal)?;
        ordered.push(OwnStatusResponse::from(s));
    }
    let total = ordered.len() as u64;
    Ok(Json(crate::page::Page::new(ordered, None, Some(total))))
}

/// GET /users/{username}/presence
#[utoipa::path(
    get,
    path = "/api/v1/users/{username}/presence",
    tag = "users",
    operation_id = "users_get_presence",
    params(("username" = String, Path)),
    responses(
        (status = 200, description = "OK", body = UserPresenceResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_user_presence(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<Json<UserPresenceResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserRead, state.deployment_mode)?;
    let user = crate::v1::calling::user_named(&state, &tenant.0, &username).await?;
    let mine = response(&state, &user).await?;
    let label = mine
        .statuses
        .iter()
        .find(|s| s.id == mine.status)
        .map(|s| s.label.clone());
    Ok(Json(UserPresenceResponse {
        username: user.username,
        presence: mine.presence,
        status: mine.status,
        status_state: mine.status_state,
        status_label: label,
        note: mine.note,
        until: mine.until,
    }))
}
