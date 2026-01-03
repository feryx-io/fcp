//! Media and recording control API endpoints

use crate::api::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::CallId;
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
    // Parse call ID
    let call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;

    // Get media manager
    let media_manager = state
        .media_manager
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Media manager not available".into()))?;

    // Start recording
    let result: Result<(), fcp_media::Error> = media_manager
        .start_recording(&call_id, req.path.clone())
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
    // Parse call ID
    let call_id = CallId::from_str(&call_id_str)
        .map_err(|_| ApiError::BadRequest(format!("Invalid call ID: {}", call_id_str)))?;

    // Get media manager
    let media_manager = state
        .media_manager
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Media manager not available".into()))?;

    // Stop recording
    let result: Result<(), fcp_media::Error> = media_manager.stop_recording(&call_id).await;

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
pub struct CallSessionInfo {
    pub call_id: String,
    pub dialog_id: String,
    pub state: String,
    pub rtp_port: u16,
    pub rtcp_port: u16,
}

/// GET /v1/calls
/// List active calls
pub async fn list_calls(State(_state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    // For now, return empty list - this would need to track active calls
    // In a real implementation, we'd query the media manager for active sessions
    let calls: Vec<CallSessionInfo> = vec![];

    Ok(Json(calls))
}
