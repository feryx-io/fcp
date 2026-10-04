//! A user's second factor (`docs/IDENTITY_AND_DEVICES.md` phase 2b): set
//! up, confirmed, turned off and its recovery codes renewed by the user,
//! signed in; reset by an administrator for a user who lost their phone.
//! Signing in with it is `auth_session`'s.

use crate::{
    auth::AuthIdentity,
    error::ApiError,
    state::ApiState,
    v1::auth_session::{confirm, enroll, Me, TotpEnrollment},
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::{CredentialStore, TenantStore};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Whether the caller has a second factor, and whether their tenant
/// requires one of them.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TotpStatus {
    /// Set up and confirmed: every password sign-in asks for a code.
    pub enabled: bool,
    /// Set up, waiting for a code to confirm it.
    pub pending: bool,
    pub recovery_codes_left: usize,
    /// The tenant requires a second factor of the caller's role.
    pub required: bool,
}

/// A code from the app.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpCodeRequest {
    pub code: String,
}

/// Recovery codes, shown once.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RecoveryCodes {
    pub recovery_codes: Vec<String>,
}

/// The signed-in caller (not a token), as their user record.
async fn signed_in_user(state: &ApiState, me: &Me) -> Result<fcp_core::User, ApiError> {
    if me.0.personal {
        return Err(ApiError::Forbidden(
            "Sign in to manage your second factor".to_string(),
        ));
    }
    let id: fcp_core::UserId =
        me.0.user_id
            .as_deref()
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| ApiError::Unauthorized("Sign in again".to_string()))?;
    require_storage(state)?
        .get_user_by_id(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("Sign in again".to_string()))
}

fn require_storage(state: &ApiState) -> Result<&std::sync::Arc<fcp_core::DynStorage>, ApiError> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))
}

fn store(state: &ApiState) -> Result<std::sync::Arc<dyn fcp_core::totp::TotpStore>, ApiError> {
    require_storage(state)?
        .totp_store()
        .cloned()
        .ok_or_else(|| ApiError::Internal("Second factors are not available".to_string()))
}

async fn required_of(state: &ApiState, user: &fcp_core::User) -> Result<bool, ApiError> {
    let Some(required) = require_storage(state)?
        .get_tenant(&user.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|t| t.sign_in.require_totp)
    else {
        return Ok(false);
    };
    // A group's role counts: an administrator by a group is one.
    Ok(crate::v1::auth_session::roles_of(state, user)
        .await
        .iter()
        .any(|role| required.applies(role)))
}

/// A code for the user's confirmed second factor, spent.
async fn check_code(state: &ApiState, user: &fcp_core::User, code: &str) -> Result<(), ApiError> {
    let store = store(state)?;
    let totp = store
        .get_user_totp(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|t| t.is_confirmed())
        .ok_or_else(|| ApiError::Conflict("No second factor is set up".to_string()))?;
    let wrong = || ApiError::Unauthorized("The code is not right".to_string());
    let step =
        fcp_core::totp::matching_step(&totp.secret, code, SystemTime::now()).ok_or_else(wrong)?;
    if !store
        .use_totp_step(&user.id, step)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(wrong());
    }
    Ok(())
}

/// GET /api/v1/me/totp
#[utoipa::path(
    get,
    path = "/api/v1/me/totp",
    tag = "self_service",
    operation_id = "self_service_get_totp",
    responses(
        (status = 200, description = "OK", body = TotpStatus),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_totp(State(state): State<ApiState>, me: Me) -> Result<Json<TotpStatus>, ApiError> {
    let user = signed_in_user(&state, &me).await?;
    let totp = store(&state)?
        .get_user_totp(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(TotpStatus {
        enabled: totp.as_ref().is_some_and(|t| t.is_confirmed()),
        pending: totp.as_ref().is_some_and(|t| !t.is_confirmed()),
        recovery_codes_left: totp.as_ref().map_or(0, |t| t.recovery_hashes.len()),
        required: required_of(&state, &user).await?,
    }))
}

/// POST /api/v1/me/totp/enroll — a secret for an authenticator app.
#[utoipa::path(
    post,
    path = "/api/v1/me/totp/enroll",
    tag = "self_service",
    operation_id = "self_service_enroll_totp",
    responses(
        (status = 200, description = "The secret to add to an app", body = TotpEnrollment),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn enroll_totp(
    State(state): State<ApiState>,
    me: Me,
) -> Result<Json<TotpEnrollment>, ApiError> {
    let user = signed_in_user(&state, &me).await?;
    Ok(Json(enroll(&state, &user).await?))
}

/// POST /api/v1/me/totp/confirm — a code from the app turns it on.
#[utoipa::path(
    post,
    path = "/api/v1/me/totp/confirm",
    tag = "self_service",
    operation_id = "self_service_confirm_totp",
    request_body = TotpCodeRequest,
    responses(
        (status = 200, description = "On, with the recovery codes", body = RecoveryCodes),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn confirm_totp(
    State(state): State<ApiState>,
    me: Me,
    Json(req): Json<TotpCodeRequest>,
) -> Result<Json<RecoveryCodes>, ApiError> {
    let user = signed_in_user(&state, &me).await?;
    let recovery_codes = confirm(&state, &user, &req.code)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("The code is not right".to_string()))?;
    tracing::info!(username = %user.username, "Second factor turned on");
    Ok(Json(RecoveryCodes { recovery_codes }))
}

/// POST /api/v1/me/totp/disable — off, with a current code; not when the
/// tenant requires one.
#[utoipa::path(
    post,
    path = "/api/v1/me/totp/disable",
    tag = "self_service",
    operation_id = "self_service_disable_totp",
    request_body = TotpCodeRequest,
    responses(
        (status = 204, description = "Off"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn disable_totp(
    State(state): State<ApiState>,
    me: Me,
    Json(req): Json<TotpCodeRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = signed_in_user(&state, &me).await?;
    if required_of(&state, &user).await? {
        return Err(ApiError::Forbidden(
            "Your organisation requires a second factor".to_string(),
        ));
    }
    check_code(&state, &user, &req.code).await?;
    store(&state)?
        .delete_user_totp(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tracing::info!(username = %user.username, "Second factor turned off");
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/me/totp/recovery-codes — new recovery codes, the old ones
/// spent, with a current code.
#[utoipa::path(
    post,
    path = "/api/v1/me/totp/recovery-codes",
    tag = "self_service",
    operation_id = "self_service_renew_recovery_codes",
    request_body = TotpCodeRequest,
    responses(
        (status = 200, description = "New codes", body = RecoveryCodes),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn renew_recovery_codes(
    State(state): State<ApiState>,
    me: Me,
    Json(req): Json<TotpCodeRequest>,
) -> Result<Json<RecoveryCodes>, ApiError> {
    let user = signed_in_user(&state, &me).await?;
    check_code(&state, &user, &req.code).await?;
    let (recovery_codes, hashes) = fcp_core::totp::new_recovery_codes();
    store(&state)?
        .set_recovery_codes(&user.id, &hashes)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(RecoveryCodes { recovery_codes }))
}

/// DELETE /api/v1/users/:username/totp — reset a user's second factor (a
/// lost phone): they set one up again at their next sign-in if their
/// tenant requires it.
#[utoipa::path(
    delete,
    path = "/api/v1/users/{username}/totp",
    tag = "users",
    operation_id = "users_reset_totp",
    params(("username" = String, Path)),
    responses(
        (status = 204, description = "Reset"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn reset_totp(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserUpdate, state.deployment_mode)?;
    let user = require_storage(&state)?
        .get_user_in_tenant(&username, &tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("User '{username}' not found")))?;
    store(&state)?
        .delete_user_totp(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tracing::info!(username = %username, by = %identity.actor(), "Second factor reset");
    Ok(StatusCode::NO_CONTENT)
}
