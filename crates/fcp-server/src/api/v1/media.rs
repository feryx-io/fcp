//! Media and recording control API endpoints

use crate::api::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_sip::call::CallId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct StartRecordingRequest {
    /// Path where recording should be saved
    /// Extension determines format (.wav or .opus)
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct RecordingResponse {
    pub call_id: String,
    pub message: String,
}

/// POST /v1/calls/:call_id/recording/start
/// Start recording a call
pub async fn start_recording(
    State(state): State<ApiState>,
    Path(call_id_str): Path<String>,
    Json(req): Json<StartRecordingRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Parse call ID (sip-specific)
    let sip_call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;

    // Convert to fcp_core::CallId for media manager
    let core_call_id = fcp_core::CallId::from_uuid(*sip_call_id.as_uuid());

    // Get media manager
    let media_manager = state
        .media_manager
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Media manager not available".into()))?;

    // Start recording
    let result: Result<(), fcp_media::Error> = media_manager
        .start_recording(&core_call_id, req.path.clone())
        .await;

    result.map_err(|e| ApiError::NotFound(format!("Failed to start recording: {}", e)))?;

    Ok((
        StatusCode::OK,
        Json(RecordingResponse {
            call_id: call_id_str,
            message: format!("Recording started to {}", req.path),
        }),
    ))
}

/// POST /v1/calls/:call_id/recording/stop
/// Stop recording a call
pub async fn stop_recording(
    State(state): State<ApiState>,
    Path(call_id_str): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Parse call ID (sip-specific)
    let sip_call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;

    // Convert to fcp_core::CallId for media manager
    let core_call_id = fcp_core::CallId::from_uuid(*sip_call_id.as_uuid());

    // Get media manager
    let media_manager = state
        .media_manager
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Media manager not available".into()))?;

    // Stop recording
    let result: Result<(), fcp_media::Error> = media_manager.stop_recording(&core_call_id).await;

    result.map_err(|e| ApiError::NotFound(format!("Failed to stop recording: {}", e)))?;

    Ok((
        StatusCode::OK,
        Json(RecordingResponse {
            call_id: call_id_str,
            message: "Recording stopped".into(),
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
    fn from_call(call: &fcp_sip::call::Call) -> Self {
        Self {
            id: call.id.to_string(),
            state: format!("{:?}", call.state),
            from_uri: call.a_leg.remote_uri.clone(),
            to_uri: call.b_leg.as_ref()
                .map(|leg| leg.remote_uri.clone())
                .unwrap_or_else(|| call.callee().unwrap_or("").to_string()),
            from_display_name: call.a_leg.remote_display_name.clone(),
            to_display_name: call.b_leg.as_ref()
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
/// List active calls
pub async fn list_calls(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    let calls = state.call_manager.active_calls();

    let call_infos: Vec<CallInfo> = calls
        .iter()
        .map(CallInfo::from_call)
        .collect();

    Ok(Json(call_infos))
}

/// GET /v1/calls/:call_id
/// Get details of a specific call
pub async fn get_call(
    State(state): State<ApiState>,
    Path(call_id_str): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;

    let call = state
        .call_manager
        .get_call(&call_id)
        .ok_or_else(|| ApiError::NotFound(format!("Call not found: {}", call_id_str)))?;

    Ok(Json(CallInfo::from_call(&call)))
}

#[derive(Debug, Deserialize)]
pub struct OriginateCallRequest {
    pub from: String,
    pub to: String,
    pub from_display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OriginateCallResponse {
    pub call_id: String,
    pub status: String,
}

/// POST /v1/calls
/// Originate a new call
pub async fn originate_call(
    State(_state): State<ApiState>,
    Json(_req): Json<OriginateCallRequest>,
) -> Result<Json<OriginateCallResponse>, ApiError> {
    // TODO: Implement call origination
    // This requires the SIP handler to expose a method to initiate outbound calls
    // For now, return "not implemented"
    Err(ApiError::NotFound("Call origination not yet implemented - requires handler enhancement".into()))
}

/// DELETE /v1/calls/:call_id
/// Hangup an active call
pub async fn hangup_call(
    State(state): State<ApiState>,
    Path(call_id_str): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;

    // Terminate the call
    state
        .call_manager
        .terminate_call(&call_id, Some("Hungup via API".to_string()))
        .map_err(|e| ApiError::NotFound(format!("Failed to hangup call: {}", e)))?;

    // TODO: Send BYE requests on both legs
    // This requires access to the SIP handler to send BYE

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "call_id": call_id_str,
            "status": "terminated"
        })),
    ))
}
