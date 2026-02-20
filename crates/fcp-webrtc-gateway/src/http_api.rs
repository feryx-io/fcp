//! HTTP API routes for SIP call control.
//!
//! These routes are merged with forge-api's existing WebRTC routes to provide
//! a unified API: the browser-facing WebRTC endpoints live under `/v1/webrtc/`
//! (from forge-api) while call control lives under `/v1/calls/` (this module).

use crate::call_bridge::{CallBridgeManager, CallInfo};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared state for call routes.
#[derive(Clone)]
pub struct CallState {
    pub bridge_manager: Arc<CallBridgeManager>,
}

/// Request to initiate a call.
#[derive(Debug, Deserialize)]
pub struct InitiateCallRequest {
    /// WebRTC connection ID (from POST /v1/webrtc/connections)
    pub connection_id: String,
    /// SIP destination URI (e.g., "sip:alice@example.com")
    pub destination: String,
}

/// Call list response.
#[derive(Debug, Serialize)]
pub struct CallListResponse {
    pub calls: Vec<CallInfo>,
    pub count: usize,
}

/// API error type.
#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        let body = serde_json::json!({
            "error": message,
        });

        (status, Json(body)).into_response()
    }
}

/// POST /v1/calls — Initiate a WebRTC→SIP call.
async fn initiate_call(
    State(state): State<CallState>,
    Json(request): Json<InitiateCallRequest>,
) -> Result<Response, ApiError> {
    if request.connection_id.is_empty() {
        return Err(ApiError::BadRequest(
            "connection_id is required".to_string(),
        ));
    }
    if request.destination.is_empty() {
        return Err(ApiError::BadRequest("destination is required".to_string()));
    }

    let info = state
        .bridge_manager
        .initiate_call(request.connection_id, request.destination)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(info)).into_response())
}

/// GET /v1/calls — List all active calls.
async fn list_calls(State(state): State<CallState>) -> Result<Json<CallListResponse>, ApiError> {
    let calls = state.bridge_manager.list_calls().await;
    let count = calls.len();
    Ok(Json(CallListResponse { calls, count }))
}

/// GET /v1/calls/:id — Get a single call.
async fn get_call(
    State(state): State<CallState>,
    Path(call_id): Path<String>,
) -> Result<Json<CallInfo>, ApiError> {
    state
        .bridge_manager
        .get_call(&call_id)
        .await
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("Call '{}' not found", call_id)))
}

/// DELETE /v1/calls/:id — Hang up a call.
async fn hangup_call(
    State(state): State<CallState>,
    Path(call_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .bridge_manager
        .hangup(&call_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Build the call control router.
pub fn call_routes(bridge_manager: Arc<CallBridgeManager>) -> Router {
    let state = CallState { bridge_manager };

    Router::new()
        .route("/v1/calls", post(initiate_call))
        .route("/v1/calls", get(list_calls))
        .route("/v1/calls/{id}", get(get_call))
        .route("/v1/calls/{id}", delete(hangup_call))
        .with_state(state)
}
