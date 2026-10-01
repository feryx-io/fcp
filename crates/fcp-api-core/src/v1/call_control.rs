//! Live call control (`docs/DEVELOPER_SURFACE.md` phase 8): hold, resume,
//! transfer, redirect into a flow, play a prompt, say text, hang up.
//!
//! The calls are on the call managers, not here: each request goes to the
//! node holding the call, as supervision does, and the node's answer comes
//! back. A party is `caller` (the call's A leg, the default) or `callee`.

use super::supervision::forward_to_node;
use crate::{auth::AuthIdentity, error::ApiError, state::ApiState, SupervisionRequest};
use axum::{
    extract::{Path, State},
    response::Response,
    Extension, Json,
};
use fcp_core::Permission;
use serde::{Deserialize, Serialize};

/// What the node answered a control with.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CallControlResponse {
    pub call_id: String,
    /// `held`, `resumed`, `transferring`, `redirecting`, `playing` or
    /// `ended`.
    pub status: String,
    /// Who was held, transferred or redirected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    /// A playback, for play and say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_id: Option<String>,
}

/// Who is put on hold.
#[derive(Debug, Default, Deserialize, Serialize, utoipa::ToSchema)]
pub struct HoldRequest {
    /// `caller` (the default) or `callee`: they hear the hold music, and
    /// the other party stops hearing them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party: Option<String>,
}

/// Where a party is transferred.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct TransferRequest {
    /// A user, an extension or a number, routed as a call to it would be.
    pub target: String,
    /// Who is transferred: `caller` (the default) or `callee`. The other
    /// party is dropped once the target answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party: Option<String>,
}

/// The flow a party is moved into.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct RedirectRequest {
    /// One of the tenant's flows, enabled.
    pub flow_id: String,
    /// Who is moved: `caller` (the default) or `callee`. The other party is
    /// dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party: Option<String>,
}

/// A prompt played into the call.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct PlayRequest {
    /// One of the tenant's prompts, with audio.
    pub prompt_id: String,
    /// Who hears it: `both` (the default), `caller` or `callee`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// Text spoken into the call by the node's text-to-speech.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SayRequest {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// Who hears it: `both` (the default), `caller` or `callee`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

async fn control(
    state: &ApiState,
    identity: &AuthIdentity,
    tenant: fcp_core::TenantId,
    call_id: &str,
    action: &'static str,
    body: serde_json::Value,
) -> Result<Response, ApiError> {
    forward_to_node(
        state,
        identity,
        Permission::CallControl,
        tenant,
        call_id,
        SupervisionRequest::Control { action, body },
    )
    .await
}

fn to_json(body: impl Serialize) -> serde_json::Value {
    serde_json::to_value(body).unwrap_or_else(|_| serde_json::json!({}))
}

/// POST /api/v1/calls/:call_id/hold
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/hold",
    tag = "call-control",
    operation_id = "call_control_hold",
    params(("call_id" = String, Path)),
    request_body = HoldRequest,
    responses(
        (status = 200, description = "Held", body = CallControlResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn hold(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
    body: Option<Json<HoldRequest>>,
) -> Result<Response, ApiError> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    control(&state, &identity, tenant.0, &call_id, "hold", to_json(body)).await
}

/// POST /api/v1/calls/:call_id/resume
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/resume",
    tag = "call-control",
    operation_id = "call_control_resume",
    params(("call_id" = String, Path)),
    responses(
        (status = 200, description = "Resumed", body = CallControlResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn resume(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
) -> Result<Response, ApiError> {
    control(
        &state,
        &identity,
        tenant.0,
        &call_id,
        "resume",
        serde_json::json!({}),
    )
    .await
}

/// POST /api/v1/calls/:call_id/transfer
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/transfer",
    tag = "call-control",
    operation_id = "call_control_transfer",
    params(("call_id" = String, Path)),
    request_body = TransferRequest,
    responses(
        (status = 202, description = "The target is being dialed", body = CallControlResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn transfer(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
    Json(body): Json<TransferRequest>,
) -> Result<Response, ApiError> {
    if body.target.trim().is_empty() {
        return Err(ApiError::BadRequest("target is required".to_string()));
    }
    control(
        &state,
        &identity,
        tenant.0,
        &call_id,
        "transfer",
        to_json(body),
    )
    .await
}

/// POST /api/v1/calls/:call_id/redirect
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/redirect",
    tag = "call-control",
    operation_id = "call_control_redirect",
    params(("call_id" = String, Path)),
    request_body = RedirectRequest,
    responses(
        (status = 202, description = "The party is being moved into the flow", body = CallControlResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn redirect(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
    Json(body): Json<RedirectRequest>,
) -> Result<Response, ApiError> {
    if body.flow_id.trim().is_empty() {
        return Err(ApiError::BadRequest("flow_id is required".to_string()));
    }
    control(
        &state,
        &identity,
        tenant.0,
        &call_id,
        "redirect",
        to_json(body),
    )
    .await
}

/// POST /api/v1/calls/:call_id/play
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/play",
    tag = "call-control",
    operation_id = "call_control_play",
    params(("call_id" = String, Path)),
    request_body = PlayRequest,
    responses(
        (status = 200, description = "Playing", body = CallControlResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn play(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
    Json(body): Json<PlayRequest>,
) -> Result<Response, ApiError> {
    control(&state, &identity, tenant.0, &call_id, "play", to_json(body)).await
}

/// POST /api/v1/calls/:call_id/say
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/say",
    tag = "call-control",
    operation_id = "call_control_say",
    params(("call_id" = String, Path)),
    request_body = SayRequest,
    responses(
        (status = 200, description = "Playing", body = CallControlResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn say(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
    Json(body): Json<SayRequest>,
) -> Result<Response, ApiError> {
    if body.text.trim().is_empty() {
        return Err(ApiError::BadRequest("text is required".to_string()));
    }
    control(&state, &identity, tenant.0, &call_id, "say", to_json(body)).await
}

/// POST /api/v1/calls/:call_id/hangup — the node holding the call ends
/// it, wherever this API runs.
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/hangup",
    tag = "call-control",
    operation_id = "call_control_hangup",
    params(("call_id" = String, Path)),
    responses(
        (status = 200, description = "Ended", body = CallControlResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn hangup(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
) -> Result<Response, ApiError> {
    control(
        &state,
        &identity,
        tenant.0,
        &call_id,
        "hangup",
        serde_json::json!({}),
    )
    .await
}
