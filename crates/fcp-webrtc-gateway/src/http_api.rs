//! The gateway's own HTTP routes, merged with forge-api's: rooms and
//! calls under `/v1/`, the join page at `/join`.
//!
//! Two credentials open these routes. An operator token (`[auth] tokens`)
//! opens everything. A session token — handed out when a session is
//! created — opens only that session's routes: its answer and candidate
//! endpoints, its status and its leave. Creating a room session needs an
//! operator token unless `[http] public_join` is on, in which case the
//! join token in the request is what admits the caller (the conference
//! server verifies it). The join page itself needs nothing.

use crate::session::{CreatedSession, JoinRequest, SessionError, SessionInfo, SessionManager};
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The built-in join page.
pub const JOIN_PAGE: &str = include_str!("../browser/join.html");

/// Shared state for the routes.
#[derive(Clone)]
pub struct ApiState {
    pub sessions: Arc<SessionManager>,
    /// Operator tokens.
    pub operator_tokens: Arc<Vec<String>>,
    pub public_join: bool,
    pub serve_join_page: bool,
}

/// `POST /v1/calls` body.
#[derive(Debug, Deserialize)]
pub struct InitiateCallRequest {
    /// An existing WebRTC connection (from `POST /v1/webrtc/connections`)
    /// to use; without it the call creates its own and returns the offer.
    #[serde(default)]
    pub connection_id: Option<String>,
    /// SIP destination (`sip:alice@example.com`, `1001@10.0.0.7`, `tel:…`),
    /// or a bare number sent through `[sip] outbound_proxy`.
    pub destination: String,
    /// The tenant user the call is placed for. Required for an emergency
    /// number, which is refused without it.
    #[serde(default)]
    pub caller: Option<crate::session::CallerIdentity>,
}

/// `POST /v1/{rooms|calls}/{id}/answer` body.
#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub sdp_answer: String,
}

/// `POST /v1/{rooms|calls}/{id}/ice-candidate` body.
#[derive(Debug, Deserialize)]
pub struct IceCandidateRequest {
    pub candidate: String,
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
            SessionError::Forbidden(_) => StatusCode::FORBIDDEN,
            SessionError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        };
        let body = serde_json::json!({ "error": self.0.to_string() });
        (status, Json(body)).into_response()
    }
}

fn unauthorized(why: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        Json(serde_json::json!({ "error": why })),
    )
        .into_response()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The session id a path addresses, when it is a session route
/// (`/v1/rooms/{id}[/…]` or `/v1/calls/{id}[/…]`, but not a join).
fn session_id_of(path: &str) -> Option<&str> {
    let rest = path
        .strip_prefix("/v1/rooms/")
        .or_else(|| path.strip_prefix("/v1/calls/"))?;
    let id = rest.split('/').next()?;
    (!id.is_empty() && id.starts_with("sess-")).then_some(id)
}

/// Whether a path is a room join (`POST /v1/rooms/{room}/join`).
fn is_join(path: &str, method: &Method) -> bool {
    method == Method::POST
        && path
            .strip_prefix("/v1/rooms/")
            .map(|rest| rest.split('/').collect::<Vec<_>>())
            .map(|parts| parts.len() == 2 && parts[1] == "join")
            .unwrap_or(false)
}

/// The auth for the gateway's own routes (see the module docs).
pub async fn gateway_auth(State(state): State<ApiState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    if path == "/join" || method == Method::OPTIONS {
        return next.run(req).await;
    }
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .map(str::to_string);
    if let Some(token) = &bearer {
        if state
            .operator_tokens
            .iter()
            .any(|t| constant_time_eq(t.as_bytes(), token.as_bytes()))
        {
            return next.run(req).await;
        }
        if let Some(id) = session_id_of(&path) {
            if state.sessions.session_token_matches(id, token).await {
                return next.run(req).await;
            }
        }
        return unauthorized("invalid token for this route");
    }
    if state.public_join && is_join(&path, &method) {
        return next.run(req).await;
    }
    unauthorized("authorization required")
}

/// `GET /join` — the built-in join page.
async fn join_page(State(state): State<ApiState>) -> Response {
    if state.serve_join_page {
        Html(JOIN_PAGE).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "the join page is not enabled ([http] serve_join_page)" })),
        )
            .into_response()
    }
}

/// `POST /v1/rooms/{room}/join` — start a room join; returns the offer the
/// browser answers through `/v1/rooms/{id}/answer`.
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

/// `POST /v1/{rooms|calls}/{id}/answer` — the browser's answer.
async fn answer(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<AnswerRequest>,
) -> Result<StatusCode, ApiError> {
    state.sessions.answer(&id, &req.sdp_answer).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/{rooms|calls}/{id}/ice-candidate` — a trickled candidate.
async fn ice_candidate(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<IceCandidateRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .sessions
        .add_ice_candidate(&id, &req.candidate)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/calls` — call a SIP destination.
async fn initiate_call(
    State(state): State<ApiState>,
    Json(request): Json<InitiateCallRequest>,
) -> Result<(StatusCode, Json<CreatedSession>), ApiError> {
    let created = state
        .sessions
        .call(
            &request.destination,
            request.connection_id.as_deref(),
            request.caller,
        )
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

/// The router for rooms, calls and the join page, with its auth applied.
pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/join", get(join_page))
        .route("/v1/rooms", get(list_rooms))
        .route("/v1/rooms/{room}/join", post(join_room))
        .route("/v1/rooms/{id}", get(get_room).delete(leave_room))
        .route("/v1/rooms/{id}/answer", post(answer))
        .route("/v1/rooms/{id}/ice-candidate", post(ice_candidate))
        .route("/v1/calls", post(initiate_call).get(list_calls))
        .route("/v1/calls/{id}", get(get_call).delete(hangup_call))
        .route("/v1/calls/{id}/answer", post(answer))
        .route("/v1/calls/{id}/ice-candidate", post(ice_candidate))
        .layer(middleware::from_fn_with_state(state.clone(), gateway_auth))
        .with_state(state)
}

#[allow(dead_code)]
fn _body_type(_: Body) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_routes_are_recognised() {
        assert_eq!(session_id_of("/v1/rooms/sess-1/answer"), Some("sess-1"));
        assert_eq!(session_id_of("/v1/calls/sess-2"), Some("sess-2"));
        assert_eq!(session_id_of("/v1/rooms/8001/join"), None);
        assert_eq!(session_id_of("/v1/rooms"), None);
        assert_eq!(session_id_of("/v1/webrtc/connections/x"), None);
        assert!(is_join("/v1/rooms/8001/join", &Method::POST));
        assert!(!is_join("/v1/rooms/8001/join", &Method::GET));
        assert!(!is_join("/v1/rooms/sess-1/answer", &Method::POST));
        assert!(!is_join("/v1/rooms", &Method::POST));
    }
}
