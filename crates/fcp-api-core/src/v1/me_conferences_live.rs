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

/// The body of `POST /me/conferences/live/:room/hand`.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct OwnHandRequest {
    /// The call the app is in the room on: the `X-FCP-Call-Id` its 200 OK
    /// named. Knowing it is what makes the hand the caller's.
    pub call_id: String,
    pub raised: bool,
}

/// POST /me/conferences/live/:room/hand — raise or lower one's own hand
/// (`docs/WEBINARS.md` §2.5), from an app in the room. Any user of the
/// room's tenant may; the call names whose hand it is.
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/hand",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_hand",
    params(("room" = String, Path)),
    request_body = OwnHandRequest,
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn own_hand(
    State(state): State<ApiState>,
    Me(claims): Me,
    Path(room): Path<String>,
    Json(req): Json<OwnHandRequest>,
) -> Result<Proxied, ApiError> {
    if req.call_id.trim().is_empty() {
        return Err(ApiError::BadRequest("call_id is required".into()));
    }
    let tenant = crate::v1::self_service::resolve_tenant_from_claims(&claims, &state);
    let meeting = scoped_meeting(&state, &tenant, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/hand"),
        Some(serde_json::json!({ "call_id": req.call_id.trim(), "raised": req.raised })),
    )
    .await
}

/// DELETE /me/conferences/live/:room/participants/:pid/hand — a host
/// lowers one hand.
#[utoipa::path(
    delete,
    path = "/api/v1/me/conferences/live/{room}/participants/{pid}/hand",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_lower_hand",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn lower_hand(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let path = format!("/participants/{pid}/hand");
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::DELETE,
        &path,
        None,
    )
    .await
}

/// DELETE /me/conferences/live/:room/hands — a host lowers every hand.
#[utoipa::path(
    delete,
    path = "/api/v1/me/conferences/live/{room}/hands",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_lower_hands",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn lower_hands(
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
        "/hands",
        None,
    )
    .await
}

/// POST /me/conferences/live/:room/participants/:pid/stage — a host
/// brings a webinar's attendee to the stage.
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/participants/{pid}/stage",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_stage_on",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn stage_on(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let path = format!("/participants/{pid}/stage");
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

/// DELETE /me/conferences/live/:room/participants/:pid/stage — a host
/// sends an attendee back to the audience.
#[utoipa::path(
    delete,
    path = "/api/v1/me/conferences/live/{room}/participants/{pid}/stage",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_stage_off",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn stage_off(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let path = format!("/participants/{pid}/stage");
    as_host(
        &state,
        &claims,
        &headers,
        &room,
        reqwest::Method::DELETE,
        &path,
        None,
    )
    .await
}

/// The meeting behind `room` in the user's tenant, when they moderate its
/// questions: they host it (see [`hosted`]), are one of its panelists, or
/// show a panelist join token for it.
async fn moderated(
    state: &ApiState,
    claims: &crate::v1::auth_session::Claims,
    headers: &HeaderMap,
    room: &str,
) -> Result<Meeting, ApiError> {
    let refused = match hosted(state, claims, headers, room).await {
        Ok(meeting) => return Ok(meeting),
        Err(e) => e,
    };
    let tenant = crate::v1::self_service::resolve_tenant_from_claims(claims, state);
    let meeting = scoped_meeting(state, &tenant, room).await?;
    if meeting.is_webinar() && !meeting.panelists.is_empty() {
        // Panelists are kept by user id; the session names the username.
        use fcp_core::storage::CredentialStore;
        let storage = state
            .storage
            .as_ref()
            .ok_or_else(|| ApiError::ServiceUnavailable("no storage".into()))?;
        let me = storage
            .get_user_in_tenant(&claims.sub, &meeting.tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .filter(|u| u.may_use_services());
        if me.is_some_and(|u| meeting.panelists.contains(&u.id.to_string())) {
            return Ok(meeting);
        }
    }
    let by_token = headers
        .get(JOIN_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .zip(state.conference_join_secret.as_deref())
        .and_then(|(token, secret)| JoinToken::verify(token, secret.as_bytes()).ok())
        .is_some_and(|t| {
            t.role == JoinRole::Panelist
                && t.tenant_id == meeting.tenant_id
                && t.room_number == meeting.room_number
        });
    if by_token {
        return Ok(meeting);
    }
    Err(refused)
}

/// Forward one moderator's operation on a question, logging it as the
/// user's.
async fn as_moderator(
    state: &ApiState,
    claims: &crate::v1::auth_session::Claims,
    headers: &HeaderMap,
    room: &str,
    qid: &str,
    action: &str,
    body: Option<serde_json::Value>,
) -> Result<Proxied, ApiError> {
    check_segment(qid, "question id")?;
    let meeting = moderated(state, claims, headers, room).await?;
    let out = forward(
        state,
        &meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/questions/{qid}/{action}"),
        body,
    )
    .await?;
    if out.0.is_success() {
        info!(room = %room, user = %claims.sub, question = %qid, %action, "A question moderated");
    }
    Ok(out)
}

/// The query of `GET /me/conferences/live/:room/questions`.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct QuestionsQuery {
    /// The app's call in the room (its `X-FCP-Call-Id`): the questions as
    /// that participant sees them. Without it, every question, for a host
    /// or panelist.
    pub call_id: Option<String>,
}

/// GET /me/conferences/live/:room/questions — the room's questions
/// (`docs/WEBINARS.md` §2.5): as the participant on `call_id` sees them
/// (any user of the room's tenant, the call naming whose view it is), or
/// every one for a host or panelist.
#[utoipa::path(
    get,
    path = "/api/v1/me/conferences/live/{room}/questions",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_questions",
    params(("room" = String, Path), QuestionsQuery),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn questions(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path(room): Path<String>,
    axum::extract::Query(q): axum::extract::Query<QuestionsQuery>,
) -> Result<Proxied, ApiError> {
    let call_id = q
        .call_id
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let meeting = match call_id {
        Some(_) => {
            let tenant = crate::v1::self_service::resolve_tenant_from_claims(&claims, &state);
            scoped_meeting(&state, &tenant, &room).await?
        }
        None => moderated(&state, &claims, &headers, &room).await?,
    };
    let path = match call_id {
        Some(c) => format!(
            "/conferences/{room}/questions?call_id={}",
            urlencoding::encode(c)
        ),
        None => format!("/conferences/{room}/questions"),
    };
    forward(&state, &meeting, reqwest::Method::GET, &path, None).await
}

/// The body of `POST /me/conferences/live/:room/questions`.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct AskQuestionRequest {
    /// The app's call in the room: the `X-FCP-Call-Id` its 200 OK named.
    pub call_id: String,
    /// The question, at most 500 characters.
    pub text: String,
    /// Asked without the asker's name, where the meeting allows it.
    #[serde(default)]
    pub anonymous: bool,
}

/// POST /me/conferences/live/:room/questions — ask a question from an app
/// in the room. Any user of the room's tenant may; the call names the
/// asker. 403 when the room takes no questions.
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/questions",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_ask",
    params(("room" = String, Path)),
    request_body = AskQuestionRequest,
    responses(
        (status = 201, description = "The question as the asker sees it", body = serde_json::Value),
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn ask(
    State(state): State<ApiState>,
    Me(claims): Me,
    Path(room): Path<String>,
    Json(req): Json<AskQuestionRequest>,
) -> Result<Proxied, ApiError> {
    if req.call_id.trim().is_empty() {
        return Err(ApiError::BadRequest("call_id is required".into()));
    }
    let tenant = crate::v1::self_service::resolve_tenant_from_claims(&claims, &state);
    let meeting = scoped_meeting(&state, &tenant, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/questions"),
        Some(serde_json::json!({
            "call_id": req.call_id.trim(),
            "text": req.text,
            "anonymous": req.anonymous,
        })),
    )
    .await
}

/// The body of `POST /me/conferences/live/:room/questions/:qid/upvote`.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpvoteQuestionRequest {
    /// The app's call in the room.
    pub call_id: String,
    /// Give the upvote (`true`) or take it back.
    pub on: bool,
}

/// POST /me/conferences/live/:room/questions/:qid/upvote — upvote another's
/// question from an app in the room, or take the upvote back.
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/questions/{qid}/upvote",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_upvote",
    params(("room" = String, Path), ("qid" = String, Path)),
    request_body = UpvoteQuestionRequest,
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn upvote(
    State(state): State<ApiState>,
    Me(claims): Me,
    Path((room, qid)): Path<(String, String)>,
    Json(req): Json<UpvoteQuestionRequest>,
) -> Result<Proxied, ApiError> {
    check_segment(&qid, "question id")?;
    if req.call_id.trim().is_empty() {
        return Err(ApiError::BadRequest("call_id is required".into()));
    }
    let tenant = crate::v1::self_service::resolve_tenant_from_claims(&claims, &state);
    let meeting = scoped_meeting(&state, &tenant, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/questions/{qid}/upvote"),
        Some(serde_json::json!({ "call_id": req.call_id.trim(), "on": req.on })),
    )
    .await
}

/// POST /me/conferences/live/:room/questions/:qid/publish — a host or
/// panelist shows a question to the room.
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/questions/{qid}/publish",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_publish_question",
    params(("room" = String, Path), ("qid" = String, Path)),
    responses(
        (status = 200, description = "The question as a host sees it", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn publish_question(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, qid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    as_moderator(&state, &claims, &headers, &room, &qid, "publish", None).await
}

/// The body of `POST /me/conferences/live/:room/questions/:qid/answer`.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct AnswerQuestionRequest {
    /// A written answer, at most 1000 characters.
    #[serde(default)]
    pub text: Option<String>,
    /// Answered aloud.
    #[serde(default)]
    pub live: bool,
}

/// POST /me/conferences/live/:room/questions/:qid/answer — a host or
/// panelist answers a question in text, aloud (`live`), or both. A written
/// answer to a question not published is seen by its asker alone.
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/questions/{qid}/answer",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_answer_question",
    params(("room" = String, Path), ("qid" = String, Path)),
    request_body = AnswerQuestionRequest,
    responses(
        (status = 200, description = "The question as a host sees it", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn answer_question(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, qid)): Path<(String, String)>,
    Json(req): Json<AnswerQuestionRequest>,
) -> Result<Proxied, ApiError> {
    as_moderator(
        &state,
        &claims,
        &headers,
        &room,
        &qid,
        "answer",
        Some(serde_json::json!({ "text": req.text, "live": req.live })),
    )
    .await
}

/// POST /me/conferences/live/:room/questions/:qid/dismiss — a host or
/// panelist dismisses a question.
#[utoipa::path(
    post,
    path = "/api/v1/me/conferences/live/{room}/questions/{qid}/dismiss",
    tag = "self_service",
    operation_id = "self_service_my_live_conference_dismiss_question",
    params(("room" = String, Path), ("qid" = String, Path)),
    responses(
        (status = 200, description = "The question as a host sees it", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn dismiss_question(
    State(state): State<ApiState>,
    Me(claims): Me,
    headers: HeaderMap,
    Path((room, qid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    as_moderator(&state, &claims, &headers, &room, &qid, "dismiss", None).await
}
