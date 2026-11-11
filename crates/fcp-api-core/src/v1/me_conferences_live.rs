//! A host's controls of their room from their own app
//! (`docs/SOFTPHONE_VIDEO.md` §2.6): layout, pin, spotlight, a
//! participant's mute and camera, removing them, and stopping a share,
//! under the user's own session rather than an administrator's.
//!
//! A user is a host of a room when they own its meeting, or when they show
//! a host join token for it (`X-FCP-Join`, the token their join used) that
//! is still good. Anyone else is refused with 403. The operations are the
//! administrator's live controls (`conferences_live`), forwarded to the
//! conference server the same way. Participants are named by the ids the
//! roster gives them (`sip:<pid>@…`, `docs/SOFTPHONE_VIDEO.md` F2a).

use crate::error::ApiError;
use crate::state::ApiState;
use crate::v1::auth_session::Me;
use crate::v1::conferences_live::{
    check_segment, forward, scoped_meeting, Proxied, SetLayoutRequest,
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use fcp_core::{JoinRole, JoinToken, Meeting, JOIN_TOKEN_HEADER};
use tracing::info;

/// The meeting behind `room` in the user's tenant, when they host it.
async fn hosted(
    state: &ApiState,
    claims: &crate::v1::auth_session::Claims,
    headers: &HeaderMap,
    room: &str,
) -> Result<Meeting, ApiError> {
    let tenant = crate::v1::self_service::resolve_tenant_from_claims(claims, state);
    let meeting = scoped_meeting(state, &tenant, room).await?;
    if meeting.owner_id.as_deref() == Some(claims.sub.as_str()) {
        return Ok(meeting);
    }
    let by_token = headers
        .get(JOIN_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .zip(state.conference_join_secret.as_deref())
        .and_then(|(token, secret)| JoinToken::verify(token, secret.as_bytes()).ok())
        .is_some_and(|t| {
            t.role == JoinRole::Host
                && t.tenant_id == meeting.tenant_id
                && t.room_number == meeting.room_number
        });
    if by_token {
        return Ok(meeting);
    }
    Err(ApiError::Forbidden(format!(
        "Not a host of conference room '{room}'"
    )))
}

/// Forward one host operation, logging it as the user's.
async fn as_host(
    state: &ApiState,
    claims: &crate::v1::auth_session::Claims,
    headers: &HeaderMap,
    room: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<Proxied, ApiError> {
    let meeting = hosted(state, claims, headers, room).await?;
    let out = forward(
        state,
        &meeting,
        method.clone(),
        &format!("/conferences/{room}{path}"),
        body,
    )
    .await?;
    if out.0.is_success() && method != reqwest::Method::GET {
        info!(room = %room, user = %claims.sub, %method, %path, "A host's control of their room");
    }
    Ok(out)
}

/// GET /me/conferences/live/:room — the room as it is now.
#[utoipa::path(
    get,
    path = "/api/v1/me/conferences/live/{room}",
    tag = "self_service",
    operation_id = "self_service_my_live_conference",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn room(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Result<Proxied, ApiError> {
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::GET,
        "",
        None,
    )
    .await
}

/// PUT /me/conferences/live/:room/video/layout
#[utoipa::path(
    put,
    path = "/api/v1/me/conferences/live/{room}/video/layout",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_layout",
    params(("room" = String, Path)),
    request_body = SetLayoutRequest,
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn set_layout(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path(room): Path<String>,
    Json(req): Json<SetLayoutRequest>,
) -> Result<Proxied, ApiError> {
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::PUT,
        "/video/layout",
        Some(serde_json::json!({ "layout": req.layout })),
    )
    .await
}

/// POST /me/conferences/live/:room/video/pin/:pid
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/video/pin/{pid}",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_pin",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn pin(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let path = format!("/video/pin/{pid}");
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::POST,
        &path,
        None,
    )
    .await
}

/// DELETE /me/conferences/live/:room/video/pin
#[utoipa::path(
    delete,
    path = "/api/v1/me/conferences/live/{room}/video/pin",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_unpin",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn unpin(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Result<Proxied, ApiError> {
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::DELETE,
        "/video/pin",
        None,
    )
    .await
}

/// POST /me/conferences/live/:room/video/spotlight/:pid
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/video/spotlight/{pid}",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_spotlight",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn spotlight(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let path = format!("/video/spotlight/{pid}");
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::POST,
        &path,
        None,
    )
    .await
}

/// DELETE /me/conferences/live/:room/video/spotlight
#[utoipa::path(
    delete,
    path = "/api/v1/me/conferences/live/{room}/video/spotlight",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_unspotlight",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn unspotlight(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Result<Proxied, ApiError> {
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::DELETE,
        "/video/spotlight",
        None,
    )
    .await
}

/// POST /me/conferences/live/:room/participants/:pid/mute
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/participants/{pid}/mute",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_mute",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn mute(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let path = format!("/mute/{pid}");
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::POST,
        &path,
        None,
    )
    .await
}

/// POST /me/conferences/live/:room/participants/:pid/unmute
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/participants/{pid}/unmute",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_unmute",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn unmute(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let path = format!("/unmute/{pid}");
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::POST,
        &path,
        None,
    )
    .await
}

/// POST /me/conferences/live/:room/participants/:pid/video/{on|off}
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/participants/{pid}/video/{state}",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_participant_video",
    params(("room" = String, Path), ("pid" = String, Path), ("state" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn participant_video(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid, video_state)): Path<(String, String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    if video_state != "on" && video_state != "off" {
        return Err(ApiError::BadRequest(
            "Video state must be 'on' or 'off'".to_string(),
        ));
    }
    let path = format!("/participants/{pid}/video/{video_state}");
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::POST,
        &path,
        None,
    )
    .await
}

/// DELETE /me/conferences/live/:room/participants/:pid — remove them.
#[utoipa::path(
    delete,
    path = "/api/v1/me/conferences/live/{room}/participants/{pid}",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_remove",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn remove(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let path = format!("/kick/{pid}");
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::POST,
        &path,
        None,
    )
    .await
}

/// POST /me/conferences/live/:room/video/content/stop — end the share
/// under way.
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/video/content/stop",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_stop_content",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn stop_content(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Result<Proxied, ApiError> {
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::POST,
        "/video/content/stop",
        None,
    )
    .await
}
