//! Media and recording control API endpoints

use crate::auth::{can_access_all_tenants, AuthIdentity, TenantContext};
use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_sip::CallId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

fn scoped_call(
    state: &ApiState,
    call_id: &CallId,
    tenant: &TenantContext,
    identity: Option<&AuthIdentity>,
) -> Result<fcp_sip::Call, ApiError> {
    let call = state
        .call_manager
        .get_call(call_id)
        .ok_or_else(|| ApiError::NotFound(format!("Call not found: {}", call_id)))?;

    if can_access_all_tenants(identity) || call.tenant_id == tenant.0 {
        Ok(call)
    } else {
        Err(ApiError::NotFound(format!("Call not found: {}", call_id)))
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct StartRecordingRequest {
    /// Where the recording is written, under the tenant's directory; the
    /// extension picks the format (`.wav` or `.opus`). Absent: a WAV file
    /// under the tenant and the day, named for the call.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RecordingResponse {
    pub call_id: String,
    pub message: String,
    /// The recording's id, when one started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_id: Option<String>,
    /// What the parties were told: `announced`, `failed` or `not_required`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent: Option<String>,
    /// True when a recording-consent announcement was played first.
    #[serde(default)]
    pub consent_announced: bool,
}

fn recording_error(e: fcp_sip::recording::RecordingError) -> ApiError {
    use fcp_sip::recording::RecordingError as E;
    match e {
        E::CallNotFound => ApiError::NotFound("Call not found".to_string()),
        E::InvalidPath => ApiError::BadRequest(e.to_string()),
        E::NoMedia => ApiError::ServiceUnavailable(e.to_string()),
        E::CallEnded
        | E::NotAnchored
        | E::AlreadyRecording
        | E::NotRecording
        | E::ConsentRefused => ApiError::Conflict(e.to_string()),
        E::Media(_) | E::NotRegistered(_) => ApiError::Internal(e.to_string()),
    }
}

/// POST /v1/calls/:call_id/recording/start
///
/// Record a call, through the tenant's consent step
/// (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1): the announcement plays to both
/// parties and ends before the recorder starts, so this returns once the
/// recording is running. `409` when the call is not anchored, is already
/// recorded, or the announcement failed under a policy that does not
/// record without one.
pub async fn start_recording(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(call_id_str): Path<String>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let sip_call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;
    scoped_call(
        &state,
        &sip_call_id,
        &tenant,
        identity.as_ref().map(|ext| &ext.0),
    )?;
    // The body is optional; one that is sent must parse.
    let req: StartRecordingRequest = if body.iter().all(u8::is_ascii_whitespace) {
        StartRecordingRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| ApiError::BadRequest(format!("Invalid request body: {e}")))?
    };
    let path = match req.path {
        Some(supplied) => fcp_sip::recording::RecordingPath::Supplied(supplied),
        None => fcp_sip::recording::RecordingPath::wav(),
    };

    let started = state
        .handler
        .call_recorder()
        .start(
            &sip_call_id,
            fcp_core::recording::CallRecordingSource::Api,
            path,
        )
        .await
        .map_err(recording_error)?;

    Ok((
        StatusCode::OK,
        Json(RecordingResponse {
            call_id: call_id_str,
            message: format!("Recording started to {}", started.file),
            recording_id: Some(started.recording_id.to_string()),
            consent: Some(started.consent.as_str().to_string()),
            consent_announced: started.consent == fcp_core::RecordingConsent::Announced,
        }),
    ))
}

/// POST /v1/calls/:call_id/recording/stop
/// Stop recording a call
pub async fn stop_recording(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(call_id_str): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sip_call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;
    scoped_call(
        &state,
        &sip_call_id,
        &tenant,
        identity.as_ref().map(|ext| &ext.0),
    )?;

    state
        .handler
        .call_recorder()
        .stop(&sip_call_id)
        .await
        .map_err(recording_error)?;

    Ok((
        StatusCode::OK,
        Json(RecordingResponse {
            call_id: call_id_str,
            message: "Recording stopped".into(),
            recording_id: None,
            consent: None,
            consent_announced: false,
        }),
    ))
}

#[derive(Debug, Serialize)]
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
            created_at: format!("{:?}", call.created_at),
            established_at: call.established_at.map(|t| format!("{:?}", t)),
            ended_at: call.ended_at.map(|t| format!("{:?}", t)),
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
pub async fn list_calls(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let allow_all_tenants = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0));
    let tenant_id = tenant.0.to_string();

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
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| ApiError::Internal(format!("Events server parse error: {}", e)))?;
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
                    return Ok(Json(crate::page::paginate(
                        call_infos,
                        &page,
                        crate::page::Order::Ascending,
                        |r: &CallInfo| r.id.clone(),
                    )?));
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
    let calls: Vec<_> = state
        .call_manager
        .active_calls()
        .into_iter()
        .filter(|call| allow_all_tenants || call.tenant_id == tenant.0)
        .collect();
    let call_infos: Vec<CallInfo> = calls.iter().map(CallInfo::from_call).collect();
    Ok(Json(crate::page::paginate(
        call_infos,
        &page,
        crate::page::Order::Ascending,
        |r: &CallInfo| r.id.clone(),
    )?))
}

/// GET /v1/calls/:call_id
/// Get details of a specific call
pub async fn get_call(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(call_id_str): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;

    let call = scoped_call(
        &state,
        &call_id,
        &tenant,
        identity.as_ref().map(|ext| &ext.0),
    )?;

    Ok(Json(CallInfo::from_call(&call)))
}

#[derive(Debug, Deserialize)]
pub struct OriginateCallRequest {
    /// Extension or SIP URI of the party to ring first (caller).
    pub from: String,
    /// Extension or SIP URI of the destination party (callee).
    pub to: String,
    /// Optional display name for the caller.
    pub from_display_name: Option<String>,
    /// Optional tenant ID (uses server default if omitted).
    pub tenant_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OriginateCallResponse {
    pub call_id: String,
    pub status: String,
}

/// POST /v1/calls
/// Originate a new call (click-to-call).
///
/// Rings the `from` party first; when they answer, rings the `to` party.
/// Returns immediately with a call ID — poll GET /v1/calls/:id for status.
pub async fn originate_call(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<OriginateCallRequest>,
) -> Result<Json<OriginateCallResponse>, ApiError> {
    if req.from.is_empty() {
        return Err(ApiError::BadRequest("'from' is required".into()));
    }
    if req.to.is_empty() {
        return Err(ApiError::BadRequest("'to' is required".into()));
    }

    let tenant_id = tenant.0;

    let call_id = state
        .handler
        .originate_call(&req.from, &req.to, tenant_id, req.from_display_name)
        .await
        .map_err(|e| ApiError::BadRequest(format!("Failed to originate call: {}", e)))?;

    Ok(Json(OriginateCallResponse {
        call_id: call_id.to_string(),
        status: "initiated".to_string(),
    }))
}

/// DELETE /v1/calls/:call_id
/// Hangup an active call
pub async fn hangup_call(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(call_id_str): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;

    scoped_call(
        &state,
        &call_id,
        &tenant,
        identity.as_ref().map(|ext| &ext.0),
    )?;

    // Send BYE on both legs and terminate the call
    state
        .handler
        .hangup_call_by_id_with(
            &call_id,
            fcp_sip::TerminationReason::system(fcp_sip::SystemCause::Admin, "Hungup via API"),
        )
        .map_err(|e| ApiError::NotFound(format!("Failed to hangup call: {}", e)))?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "call_id": call_id_str,
            "status": "terminated"
        })),
    ))
}
