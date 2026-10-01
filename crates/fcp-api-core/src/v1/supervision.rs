//! A supervisor on a call (`docs/ENTERPRISE_CALLING.md` §3.11): monitor
//! (hear both parties), whisper (heard by one) or barge (heard by both).
//!
//! The calls are on the call managers, not here: each request goes to the
//! node holding the call through [`crate::CallSupervisor`], and its answer
//! comes back as the node gave it.

use crate::{auth::AuthIdentity, error::ApiError, state::ApiState, SupervisionRequest};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use fcp_core::Permission;
use serde::Deserialize;

/// Who supervises, and who hears a whisper.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SuperviseRequest {
    /// A user's extension or name, or a SIP URI.
    pub supervisor: String,
    /// For a whisper: `a` (the caller) or `b` (the callee, the default).
    #[serde(default)]
    pub heard_by: Option<String>,
}

async fn forward(
    state: &ApiState,
    identity: &AuthIdentity,
    tenant: fcp_core::TenantId,
    call_id: &str,
    request: SupervisionRequest,
) -> Result<Response, ApiError> {
    forward_to_node(
        state,
        identity,
        Permission::CallSupervise,
        tenant,
        call_id,
        request,
    )
    .await
}

/// Put `request` to the node holding the call, as `identity` holding
/// `permission`, and answer what the node answered.
pub(crate) async fn forward_to_node(
    state: &ApiState,
    identity: &AuthIdentity,
    permission: Permission,
    tenant: fcp_core::TenantId,
    call_id: &str,
    request: SupervisionRequest,
) -> Result<Response, ApiError> {
    crate::auth::check_permission(identity, permission, state.deployment_mode)?;
    let supervisor = state.call_supervisor.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("no call manager can be reached".to_string())
    })?;
    let (status, body) = supervisor
        .supervise(tenant, call_id, request)
        .await
        .map_err(ApiError::ServiceUnavailable)?;
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        // The API's one error shape, whatever the node said.
        let error = crate::error::ErrorResponse::from_upstream(status, &body);
        return Ok((status, Json(error)).into_response());
    }
    Ok((status, Json(body)).into_response())
}

async fn join(
    state: ApiState,
    identity: AuthIdentity,
    tenant: fcp_core::TenantId,
    call_id: String,
    mode: &'static str,
    req: SuperviseRequest,
) -> Result<Response, ApiError> {
    if req.supervisor.trim().is_empty() {
        return Err(ApiError::BadRequest("supervisor is required".to_string()));
    }
    forward(
        &state,
        &identity,
        tenant,
        &call_id,
        SupervisionRequest::Join {
            mode,
            supervisor: req.supervisor.trim().to_string(),
            heard_by: req.heard_by,
        },
    )
    .await
}

/// POST /api/v1/calls/:call_id/monitor
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/monitor",
    tag = "supervision",
    operation_id = "supervision_monitor",
    params(("call_id" = String, Path)),
    request_body = SuperviseRequest,
    responses(
        (status = 200, description = "The node's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn monitor(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
    Json(req): Json<SuperviseRequest>,
) -> Result<Response, ApiError> {
    join(state, identity, tenant.0, call_id, "monitor", req).await
}

/// POST /api/v1/calls/:call_id/whisper
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/whisper",
    tag = "supervision",
    operation_id = "supervision_whisper",
    params(("call_id" = String, Path)),
    request_body = SuperviseRequest,
    responses(
        (status = 200, description = "The node's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn whisper(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
    Json(req): Json<SuperviseRequest>,
) -> Result<Response, ApiError> {
    join(state, identity, tenant.0, call_id, "whisper", req).await
}

/// POST /api/v1/calls/:call_id/barge
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/barge",
    tag = "supervision",
    operation_id = "supervision_barge",
    params(("call_id" = String, Path)),
    request_body = SuperviseRequest,
    responses(
        (status = 200, description = "The node's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn barge(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
    Json(req): Json<SuperviseRequest>,
) -> Result<Response, ApiError> {
    join(state, identity, tenant.0, call_id, "barge", req).await
}

/// GET /api/v1/calls/:call_id/supervisor
#[utoipa::path(
    get,
    path = "/api/v1/calls/{call_id}/supervisor",
    tag = "supervision",
    operation_id = "supervision_get",
    params(("call_id" = String, Path)),
    responses(
        (status = 200, description = "The node's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
) -> Result<Response, ApiError> {
    forward(
        &state,
        &identity,
        tenant.0,
        &call_id,
        SupervisionRequest::Get,
    )
    .await
}

/// DELETE /api/v1/calls/:call_id/supervisor
#[utoipa::path(
    delete,
    path = "/api/v1/calls/{call_id}/supervisor",
    tag = "supervision",
    operation_id = "supervision_leave",
    params(("call_id" = String, Path)),
    responses(
        (status = 204, description = "Left"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn leave(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(call_id): Path<String>,
) -> Result<Response, ApiError> {
    forward(
        &state,
        &identity,
        tenant.0,
        &call_id,
        SupervisionRequest::Leave,
    )
    .await
}
