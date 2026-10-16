//! Calls and their recording.
//!
//! The calls are on the call managers, not here: a call's details, its
//! hangup and its recording are put to the node holding it, as live call
//! control is (`super::call_control`), and the node's answer comes back.
//! The list is the cluster's, from the events server, else this process's.

use super::supervision::forward_to_node;
use crate::auth::{can_access_all_tenants, AuthIdentity, TenantContext};
use crate::{error::ApiError, state::ApiState, SupervisionRequest};
use axum::{
    extract::{Extension, Path, State},
    response::{IntoResponse, Response},
    Json,
};
use fcp_core::Permission;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, utoipa::ToSchema)]
pub struct StartRecordingRequest {
    /// Where the recording is written, under the tenant's directory; the
    /// extension picks the format (`.wav` or `.opus`). Absent: a WAV file
    /// under the tenant and the day, named for the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// What the node answered a recording request with.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = media::RecordingResponse)]
pub struct RecordingResponse {
    pub call_id: String,
    /// `recording` or `stopped`.
    pub status: String,
    /// The recording's id, when one started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_id: Option<String>,
    /// The file it is written to, when one started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What the parties were told: `announced`, `failed` or `not_required`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<String>,
}

/// POST /v1/calls/:call_id/recording/start
///
/// Record a call, through the tenant's consent step
/// (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1): the announcement plays to both
/// parties and ends before the recorder starts, so this returns once the
/// recording is running. `409` when the call is not anchored, is already
/// recorded, or the announcement failed under a policy that does not
/// record without one.
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/recording/start",
    tag = "media",
    operation_id = "media_start_recording",
    params(("call_id" = String, Path)),
    request_body = StartRecordingRequest,
    responses(
        (status = 200, description = "Recording", body = RecordingResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn start_recording(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(call_id): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    // The body is optional; one that is sent must parse.
    let req: StartRecordingRequest = if body.iter().all(u8::is_ascii_whitespace) {
        StartRecordingRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| ApiError::BadRequest(format!("Invalid request body: {e}")))?
    };
    forward_to_node(
        &state,
        &identity,
        Permission::CallControl,
        tenant.0,
        &call_id,
        SupervisionRequest::Control {
            action: "recording/start",
            body: serde_json::to_value(req).unwrap_or_else(|_| serde_json::json!({})),
        },
    )
    .await
}

/// POST /v1/calls/:call_id/recording/stop
/// Stop recording a call
#[utoipa::path(
    post,
    path = "/api/v1/calls/{call_id}/recording/stop",
    tag = "media",
    operation_id = "media_stop_recording",
    params(("call_id" = String, Path)),
    responses(
        (status = 200, description = "Stopped", body = RecordingResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn stop_recording(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(call_id): Path<String>,
) -> Result<Response, ApiError> {
    forward_to_node(
        &state,
        &identity,
        Permission::CallControl,
        tenant.0,
        &call_id,
        SupervisionRequest::Control {
            action: "recording/stop",
            body: serde_json::json!({}),
        },
    )
    .await
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CallInfo {
    pub id: String,
    pub state: String,
    pub from_uri: String,
    pub to_uri: String,
    pub from_display_name: Option<String>,
    pub to_display_name: Option<String>,
    pub created_at: String,
    pub established_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_seconds: Option<u64>,
    pub media_anchored: bool,
    pub failure_reason: Option<String>,
}

impl CallInfo {
    fn from_call(call: &fcp_sip::Call) -> Self {
        Self {
            id: call.id.to_string(),
            state: format!("{:?}", call.state),
            from_uri: call.a_leg.remote_uri.clone(),
            to_uri: call
                .b_leg
                .as_ref()
                .map(|leg| leg.remote_uri.clone())
                .unwrap_or_else(|| call.callee().unwrap_or("").to_string()),
            from_display_name: call.a_leg.remote_display_name.clone(),
            to_display_name: call
                .b_leg
                .as_ref()
                .and_then(|leg| leg.remote_display_name.clone()),
            created_at: crate::time::format_system_time(call.created_at),
            established_at: call.established_at.map(crate::time::format_system_time),
            ended_at: call.ended_at.map(crate::time::format_system_time),
            duration_seconds: call.duration().map(|d| d.as_secs()),
            media_anchored: call.media_anchored,
            failure_reason: call.failure_reason.clone(),
        }
    }
}

/// GET /v1/calls
/// List active calls.
///
/// When an events server URL is configured, proxies to the events server
/// for a cluster-wide view. Otherwise falls back to the local call manager.
#[utoipa::path(
    get,
    path = "/api/v1/calls",
    tag = "media",
    operation_id = "media_list_calls",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<CallInfo>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_calls(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let allow_all_tenants = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0));
    let call_infos = active_calls(&state, &tenant.0, allow_all_tenants).await;
    Ok(Json(crate::page::paginate(
        call_infos,
        &page,
        crate::page::Order::Ascending,
        |r: &CallInfo| r.id.clone(),
    )?))
}

/// The calls up now, the tenant's (or every tenant's): the cluster's, from
/// the events server when one is configured, else this process's own.
pub(crate) async fn active_calls(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    allow_all_tenants: bool,
) -> Vec<CallInfo> {
    let tenant_id = tenant.to_string();
    // Try events server first for cluster-wide view
    if let Some(ref events_url) = state.events_server_url {
        let url = format!("{}/calls", events_url);
        tracing::debug!(%url, "Proxying calls request to events server");
        let client = reqwest::Client::new();
        let mut request = client.get(&url);
        if let Some(token) = state.events_server_auth_token.as_deref() {
            request = request.bearer_auth(token);
        }

        match request.send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_else(|e| {
                    tracing::warn!(%e, "Events server answered calls unreadably, falling back to local");
                    serde_json::Value::Null
                });
                // The events server returns { calls: [...], count: N }
                if let Some(calls) = body.get("calls").and_then(|c| c.as_array()) {
                    let call_infos: Vec<CallInfo> = calls
                        .iter()
                        .filter(|c| {
                            allow_all_tenants
                                || c.get("tenant_id")
                                    .and_then(|v| v.as_str())
                                    .map(|v| v == tenant_id.as_str())
                                    .unwrap_or(false)
                        })
                        .filter_map(|c| {
                            let state_str = c.get("state")?.as_str()?;
                            if state_str == "ended" || state_str == "failed" {
                                return None;
                            }
                            let epoch_to_rfc3339 = |v: &serde_json::Value| -> Option<String> {
                                let ts = v.as_u64()?;
                                Some(chrono::DateTime::from_timestamp(ts as i64, 0)?.to_rfc3339())
                            };
                            Some(CallInfo {
                                id: c.get("call_id")?.as_str()?.to_string(),
                                state: state_str.to_string(),
                                from_uri: c
                                    .get("from")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                to_uri: c
                                    .get("to")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                from_display_name: None,
                                to_display_name: None,
                                created_at: c
                                    .get("initiated_at")
                                    .and_then(epoch_to_rfc3339)
                                    .unwrap_or_default(),
                                established_at: c.get("answered_at").and_then(epoch_to_rfc3339),
                                ended_at: None,
                                duration_seconds: c
                                    .get("duration_ms")
                                    .and_then(|v| v.as_u64())
                                    .map(|ms| ms / 1000),
                                media_anchored: true,
                                failure_reason: c
                                    .get("failure_reason")
                                    .and_then(|v| v.as_str())
                                    .map(String::from),
                            })
                        })
                        .collect();
                    return call_infos;
                }
            }
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), "Events server returned error, falling back to local");
            }
            Err(e) => {
                tracing::warn!(%e, "Failed to reach events server, falling back to local");
            }
        }
    }

    // Fallback: local call manager (works when admin is co-located with call-manager)
    state
        .call_manager
        .active_calls()
        .into_iter()
        .filter(|call| allow_all_tenants || call.tenant_id == *tenant)
        .map(|call| CallInfo::from_call(&call))
        .collect()
}

/// GET /v1/calls/:call_id
/// A call, as the node holding it sees it.
#[utoipa::path(
    get,
    path = "/api/v1/calls/{call_id}",
    tag = "media",
    operation_id = "media_get_call",
    params(("call_id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = CallInfo),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_call(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(call_id): Path<String>,
) -> Result<Response, ApiError> {
    forward_to_node(
        &state,
        &identity,
        Permission::CallView,
        tenant.0,
        &call_id,
        SupervisionRequest::Show,
    )
    .await
}

/// DELETE /v1/calls/:call_id
/// Hang up a call: `POST /calls/:call_id/hangup` by another name.
#[utoipa::path(
    delete,
    path = "/api/v1/calls/{call_id}",
    tag = "media",
    operation_id = "media_hangup_call",
    params(("call_id" = String, Path)),
    responses(
        (status = 200, description = "Ended", body = super::call_control::CallControlResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn hangup_call(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(call_id): Path<String>,
) -> Result<Response, ApiError> {
    forward_to_node(
        &state,
        &identity,
        Permission::CallControl,
        tenant.0,
        &call_id,
        SupervisionRequest::Control {
            action: "hangup",
            body: serde_json::json!({}),
        },
    )
    .await
}
