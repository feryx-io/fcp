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
    /// Leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    pub drain: Arc<fcp_sip::drain::Drain>,
    /// Operator tokens.
    pub operator_tokens: Arc<Vec<String>>,
    /// Proxies whose `X-Forwarded-For` is believed (`[http] trusted_proxies`).
    pub trusted_proxies: Arc<Vec<std::net::IpAddr>>,
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
        // A refusal says when to come back, as every other FCP node does
        // (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
        if let SessionError::Refused {
            detail,
            retry_after_secs,
        } = &self.0
        {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::RETRY_AFTER, retry_after_secs.to_string())],
                Json(serde_json::json!({ "error": detail })),
            )
                .into_response();
        }
        let status = match &self.0 {
            SessionError::BadRequest(_) => StatusCode::BAD_REQUEST,
            SessionError::NotFound(_) => StatusCode::NOT_FOUND,
            SessionError::Conflict(_) => StatusCode::CONFLICT,
            SessionError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            SessionError::Forbidden(_) => StatusCode::FORBIDDEN,
            SessionError::Unavailable(_) | SessionError::Refused { .. } => {
                StatusCode::SERVICE_UNAVAILABLE
            }
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
    // `/join` is the page itself, and `/ready` is what a load balancer
    // reads to stop choosing a draining node — neither is a credential's
    // business. `/drain` and `/status` are operator routes and are not
    // listed here.
    if path == "/join" || path == "/ready" || method == Method::OPTIONS {
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
            .any(|t| fcp_core::secret::constant_time_eq(t.as_bytes(), token.as_bytes()))
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

/// `POST /v1/rooms/{id}/view` — what a room that forwards should send this
/// browser (`docs/FORWARDING_ROOMS.md` §2.3): the body is the view, sent
/// on to the room as an `fcp-view` INFO. What was granted shows in the
/// session's `forwarding` once the room's roster says.
async fn set_view(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    if !state.sessions.exists(&id, true).await {
        return Err(SessionError::NotFound(format!("room session '{id}' not found")).into());
    }
    match state.sessions.set_view(&id, &body).await? {
        200..=299 => Ok(StatusCode::NO_CONTENT),
        400 => Err(SessionError::BadRequest("the room refused the view".into()).into()),
        code => Err(SessionError::Internal(format!("the room answered the view {code}")).into()),
    }
}

/// The body of `POST /v1/rooms/{id}/hand`.
#[derive(Debug, Deserialize)]
struct HandRequest {
    raised: bool,
}

/// `POST /v1/rooms/{id}/hand` — raise or lower the browser's hand
/// (`docs/WEBINARS.md` §2.5), sent on to the room as an `fcp-qa` INFO. The
/// session's `qa` says where it stands.
async fn set_hand(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<HandRequest>,
) -> Result<StatusCode, ApiError> {
    if !state.sessions.exists(&id, true).await {
        return Err(SessionError::NotFound(format!("room session '{id}' not found")).into());
    }
    match state.sessions.set_hand(&id, req.raised).await? {
        200..=299 => Ok(StatusCode::NO_CONTENT),
        403 => Err(SessionError::BadRequest("a host has no hand to raise".into()).into()),
        code => Err(SessionError::Internal(format!("the room answered the hand {code}")).into()),
    }
}

/// `POST /v1/rooms/{id}/chat` — the browser posts, or (a host or
/// panelist) deletes a message or mutes a participant
/// (`docs/WEBINARS.md` §2.5). The body is the `fcp-chat` message, sent on
/// to the room as an INFO; the session's `chat` follows the room's word.
async fn send_chat(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(message): Json<fcp_core::chat::ChatMessage>,
) -> Result<StatusCode, ApiError> {
    use fcp_core::chat::ChatMessage;
    if !matches!(
        message,
        ChatMessage::Post { .. } | ChatMessage::Delete { .. } | ChatMessage::Mute { .. }
    ) {
        return Err(SessionError::BadRequest("not a chat operation".into()).into());
    }
    if !state.sessions.exists(&id, true).await {
        return Err(SessionError::NotFound(format!("room session '{id}' not found")).into());
    }
    match state.sessions.send_chat(&id, &message).await? {
        200..=299 => Ok(StatusCode::NO_CONTENT),
        400 => Err(SessionError::BadRequest("the room refused the message".into()).into()),
        403 => Err(SessionError::Forbidden(
            "the room does not allow it: chat off or muted, too many messages, or not a host's"
                .into(),
        )
        .into()),
        404 => Err(SessionError::NotFound("no such message or participant".into()).into()),
        code => Err(SessionError::Internal(format!("the room answered the chat {code}")).into()),
    }
}

/// `POST /v1/rooms/{id}/questions` — the browser asks, upvotes, or (a
/// host or panelist) publishes, answers or dismisses a question
/// (`docs/WEBINARS.md` §2.5). The body is the `fcp-qa` message, sent on to
/// the room as an INFO; the session's `qa.questions` follows the room's
/// word.
async fn send_question(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(message): Json<fcp_core::qa::QaMessage>,
) -> Result<StatusCode, ApiError> {
    use fcp_core::qa::QaMessage;
    if !matches!(
        message,
        QaMessage::Ask { .. }
            | QaMessage::Upvote { .. }
            | QaMessage::Publish { .. }
            | QaMessage::Answer { .. }
            | QaMessage::Dismiss { .. }
    ) {
        return Err(SessionError::BadRequest("not a question's operation".into()).into());
    }
    if !state.sessions.exists(&id, true).await {
        return Err(SessionError::NotFound(format!("room session '{id}' not found")).into());
    }
    match state.sessions.send_qa(&id, &message).await? {
        200..=299 => Ok(StatusCode::NO_CONTENT),
        400 => Err(SessionError::BadRequest("the room refused the question".into()).into()),
        403 => Err(SessionError::Forbidden(
            "the room does not allow it: no questions, or not a host's".into(),
        )
        .into()),
        404 => Err(SessionError::NotFound("no such question".into()).into()),
        code => {
            Err(SessionError::Internal(format!("the room answered the question {code}")).into())
        }
    }
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
    axum::extract::ConnectInfo(client): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(request): Json<InitiateCallRequest>,
) -> Result<(StatusCode, Json<CreatedSession>), ApiError> {
    // Where the browser is, for the call manager to place it by network
    // (`docs/E911_BLOCK_2.md` §6.2). A forwarded address is believed only
    // from a proxy the operator listed.
    let client_address = client_address(&state, client.ip(), &headers);
    let created = state
        .sessions
        .call(
            &request.destination,
            request.connection_id.as_deref(),
            request.caller,
            client_address,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// The browser's address: the connection's, or the first address in
/// `X-Forwarded-For` when the connection came from a trusted proxy.
fn client_address(
    state: &ApiState,
    peer: std::net::IpAddr,
    headers: &axum::http::HeaderMap,
) -> std::net::IpAddr {
    if !state.trusted_proxies.contains(&peer) {
        return peer;
    }
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.trim().parse::<std::net::IpAddr>().ok())
        .unwrap_or(peer)
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

/// Whether this node should be sent browsers: `503` while it is draining,
/// so a load balancer stops choosing it before it stops taking them
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). forge-api's `/health` keeps
/// answering `200`, so a liveness probe does not kill the process
/// mid-drain.
async fn ready(State(state): State<ApiState>) -> Response {
    drain_answer(fcp_sip::drain::ready_body(&state.drain).await)
}

/// `GET /status` — what this node is carrying and whether it is leaving.
async fn status(State(state): State<ApiState>) -> Response {
    let draining = state.drain.is_draining();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": if draining { "draining" } else { "healthy" },
            "service": "fcp-webrtc-gw",
            "version": env!("CARGO_PKG_VERSION"),
            "sessions": state.sessions.live_count().await,
            "draining": draining,
            "drain_wait_secs": state.drain.wait_secs(),
        })),
    )
        .into_response()
}

/// `POST /drain` — this node takes no new sessions, tells the browsers in
/// rooms where to go, and at `drain_wait_secs` ends what is left. Behind
/// the operator token, with the gateway's other operator routes.
async fn start_drain(State(state): State<ApiState>) -> Response {
    drain_answer(fcp_sip::drain::start_body(&state.drain).await)
}

/// `DELETE /drain` — the node serves again, and the sessions it was
/// carrying, which the drain never touched, carry on.
async fn stop_drain(State(state): State<ApiState>) -> Response {
    drain_answer(fcp_sip::drain::stop_body(&state.drain).await)
}

/// What the shared drain answered, as this router's response.
fn drain_answer((code, body): (u16, serde_json::Value)) -> Response {
    let code = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(body)).into_response()
}

/// The router for rooms, calls and the join page, with its auth applied.
pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/join", get(join_page))
        .route("/ready", get(ready))
        .route("/status", get(status))
        .route("/drain", post(start_drain).delete(stop_drain))
        .route("/v1/rooms", get(list_rooms))
        .route("/v1/rooms/{room}/join", post(join_room))
        .route("/v1/rooms/{id}", get(get_room).delete(leave_room))
        .route("/v1/rooms/{id}/answer", post(answer))
        .route("/v1/rooms/{id}/ice-candidate", post(ice_candidate))
        .route("/v1/rooms/{id}/view", post(set_view))
        .route("/v1/rooms/{id}/hand", post(set_hand))
        .route("/v1/rooms/{id}/questions", post(send_question))
        .route("/v1/rooms/{id}/chat", post(send_chat))
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
