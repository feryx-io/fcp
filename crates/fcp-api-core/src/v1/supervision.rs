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
#[derive(Debug, Deserialize)]
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
    crate::auth::check_permission(identity, Permission::CallSupervise, state.deployment_mode)?;
    let supervisor = state.call_supervisor.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("no call manager can be reached".to_string())
    })?;
    let (status, body) = supervisor
        .supervise(tenant, call_id, request)
        .await
        .map_err(ApiError::ServiceUnavailable)?;
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
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
