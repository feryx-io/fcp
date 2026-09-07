//! The gateway's own HTTP routes, merged with forge-api's: rooms and
//! calls under `/v1/`, the browser-facing WebRTC routes staying forge-api's
//! (`/v1/webrtc/connections/{id}/answer` and `/ice-candidate` complete
//! what a room join or call starts).

use crate::session::{CreatedSession, JoinRequest, SessionError, SessionInfo, SessionManager};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared state for the routes.
#[derive(Clone)]
pub struct ApiState {
    pub sessions: Arc<SessionManager>,
}

/// `POST /v1/calls` body.
#[derive(Debug, Deserialize)]
pub struct InitiateCallRequest {
    /// An existing WebRTC connection (from `POST /v1/webrtc/connections`)
    /// to use; without it the call creates its own and returns the offer.
    #[serde(default)]
    pub connection_id: Option<String>,
    /// SIP destination (`sip:alice@example.com`, `1001@10.0.0.7`, `tel:…`).
    pub destination: String,
}

/// A list of sessions.
#[derive(Debug, Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionInfo>,
    pub count: usize,
}

/// API error → status.
#[derive(Debug)]
pub struct ApiError(SessionError);

impl From<SessionError> for ApiError {
    fn from(e: SessionError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            SessionError::BadRequest(_) => StatusCode::BAD_REQUEST,
            SessionError::NotFound(_) => StatusCode::NOT_FOUND,
            SessionError::Conflict(_) => StatusCode::CONFLICT,
            SessionError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({ "error": self.0.to_string() });
        (status, Json(body)).into_response()
    }
}

/// `POST /v1/rooms/{room}/join` — start a room join; returns the offer the
/// browser answers through `/v1/webrtc/connections/{connection_id}/answer`.
async fn join_room(
    State(state): State<ApiState>,
    Path(room): Path<String>,
    body: Option<Json<JoinRequest>>,
) -> Result<(StatusCode, Json<CreatedSession>), ApiError> {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let created = state.sessions.join_room(&room, req).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// `GET /v1/rooms` — every room session.
async fn list_rooms(State(state): State<ApiState>) -> Json<SessionListResponse> {
    let sessions = state.sessions.list(true).await;
    let count = sessions.len();
    Json(SessionListResponse { sessions, count })
}

/// `GET /v1/rooms/{id}` — one room session.
async fn get_room(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<SessionInfo>, ApiError> {
    state
        .sessions
        .get(&id, true)
        .await
        .map(Json)
        .ok_or_else(|| SessionError::NotFound(format!("room session '{id}' not found")).into())
}

/// `DELETE /v1/rooms/{id}` — leave the room.
async fn leave_room(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !state.sessions.exists(&id, true).await {
        return Err(SessionError::NotFound(format!("room session '{id}' not found")).into());
    }
    state.sessions.hangup(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/calls` — call a SIP destination.
async fn initiate_call(
    State(state): State<ApiState>,
    Json(request): Json<InitiateCallRequest>,
) -> Result<(StatusCode, Json<CreatedSession>), ApiError> {
    let created = state
        .sessions
        .call(&request.destination, request.connection_id.as_deref())
        .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// `GET /v1/calls` — every call session.
async fn list_calls(State(state): State<ApiState>) -> Json<SessionListResponse> {
    let sessions = state.sessions.list(false).await;
    let count = sessions.len();
    Json(SessionListResponse { sessions, count })
}

/// `GET /v1/calls/{id}` — one call.
async fn get_call(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<SessionInfo>, ApiError> {
    state
        .sessions
        .get(&id, false)
        .await
        .map(Json)
        .ok_or_else(|| SessionError::NotFound(format!("call '{id}' not found")).into())
}

/// `DELETE /v1/calls/{id}` — hang up.
async fn hangup_call(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !state.sessions.exists(&id, false).await {
        return Err(SessionError::NotFound(format!("call '{id}' not found")).into());
    }
    state.sessions.hangup(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The router for rooms and calls.
pub fn routes(sessions: Arc<SessionManager>) -> Router {
    Router::new()
        .route("/v1/rooms", get(list_rooms))
        .route("/v1/rooms/{room}/join", post(join_room))
        .route("/v1/rooms/{id}", get(get_room).delete(leave_room))
        .route("/v1/calls", post(initiate_call).get(list_calls))
        .route("/v1/calls/{id}", get(get_call).delete(hangup_call))
        .with_state(ApiState { sessions })
}
