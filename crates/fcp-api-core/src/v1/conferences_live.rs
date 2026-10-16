//! Live conference controls, proxied to the conference server.
//!
//! The admin API does not host rooms; a conference server does. These
//! endpoints resolve the room to a meeting in the caller's tenant, then
//! forward the operation to the configured conference server with the
//! tenant in `x-fcp-tenant-id`. Any conference node will do as the entry
//! point: it applies the operation when it hosts the room and otherwise
//! locates or broadcasts to the hosting nodes through its `/internal`
//! routes (see `docs/DISTRIBUTED_CONFERENCING.md`). Upstream answers,
//! status and body, come back as they are.

use crate::auth::PLATFORM_TENANT_HEADER;
use crate::{auth::TenantContext, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use fcp_core::storage::ConferenceStore;
use fcp_core::{Meeting, TenantId};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, info, warn};

/// How long a forwarded request may take, including the conference
/// server's own hop to a peer node.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// The upstream's answer, passed through.
pub type Proxied = (StatusCode, Json<Value>);

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// Body of `PUT …/video/layout`, and of `PUT …/video/content-layout`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetLayoutRequest {
    pub layout: String,
}

/// What a caller may say in a room or participant id: what the
/// conference server accepts, minus anything that would change the path.
fn valid_segment(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@' | b'+'))
}

fn check_segment(id: &str, what: &str) -> Result<(), ApiError> {
    if valid_segment(id) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!("Invalid {what}")))
    }
}

/// The meeting behind a room number in the caller's tenant, or 404.
async fn scoped_meeting(
    state: &ApiState,
    tenant: &TenantId,
    room: &str,
) -> Result<Meeting, ApiError> {
    check_segment(room, "room number")?;
    let storage = require_storage!(state);
    storage
        .get_meeting_by_room_number(tenant, room)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("Conference room '{room}' not found")))
}

/// Forward one request to the conference server on the meeting's behalf.
pub async fn forward(
    state: &ApiState,
    meeting: &Meeting,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Proxied, ApiError> {
    let Some(base) = state.conference_server_url.as_deref() else {
        return Err(ApiError::ServiceUnavailable(
            "No conference server configured for live controls ([conference] server_url)"
                .to_string(),
        ));
    };
    let url = format!("{base}{path}");
    debug!(%url, %method, "Proxying live conference request");
    let client = reqwest::Client::builder()
        .timeout(UPSTREAM_TIMEOUT)
        .build()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut req = client
        .request(method, &url)
        .header(PLATFORM_TENANT_HEADER, meeting.tenant_id.to_string());
    if let Some(token) = state.conference_server_auth_token.as_deref() {
        req = req.bearer_auth(token);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await.map_err(|e| {
        warn!(%url, error = %e, "Conference server unreachable");
        ApiError::ServiceUnavailable(format!("Conference server unreachable: {e}"))
    })?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let text = resp.text().await.unwrap_or_default();
    let body = serde_json::from_str::<Value>(&text)
        .unwrap_or_else(|_| serde_json::json!({ "error": text }));
    if !status.is_success() {
        // The API's one error shape (`crate::error`), whatever the
        // conference server said, with what it said as the detail.
        let error = crate::error::ErrorResponse::from_upstream(status, &body);
        return Ok((
            status,
            Json(serde_json::to_value(error).unwrap_or_default()),
        ));
    }
    Ok((status, Json(body)))
}

/// GET /conferences/live/:room/video
#[utoipa::path(
    get,
    path = "/api/v1/conferences/live/{room}/video",
    tag = "conferences_live",
    operation_id = "conferences_live_video_status",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn video_status(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(room): Path<String>,
) -> Result<Proxied, ApiError> {
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::GET,
        &format!("/conferences/{room}/video"),
        None,
    )
    .await
}

/// PUT /conferences/live/:room/video/layout
#[utoipa::path(
    put,
    path = "/api/v1/conferences/live/{room}/video/layout",
    tag = "conferences_live",
    operation_id = "conferences_live_set_video_layout",
    params(("room" = String, Path)),
    request_body = SetLayoutRequest,
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn set_video_layout(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(room): Path<String>,
    Json(req): Json<SetLayoutRequest>,
) -> Result<Proxied, ApiError> {
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    let out = forward(
        &state,
        &meeting,
        reqwest::Method::PUT,
        &format!("/conferences/{room}/video/layout"),
        Some(serde_json::json!({ "layout": req.layout })),
    )
    .await?;
    if out.0.is_success() {
        info!(room = %room, layout = %req.layout, "Video layout set");
    }
    Ok(out)
}

/// POST /conferences/live/:room/video/pin/:pid
#[utoipa::path(
    post,
    path = "/api/v1/conferences/live/{room}/video/pin/{pid}",
    tag = "conferences_live",
    operation_id = "conferences_live_pin",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn pin(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/video/pin/{pid}"),
        None,
    )
    .await
}

/// DELETE /conferences/live/:room/video/pin
#[utoipa::path(
    delete,
    path = "/api/v1/conferences/live/{room}/video/pin",
    tag = "conferences_live",
    operation_id = "conferences_live_unpin",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn unpin(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(room): Path<String>,
) -> Result<Proxied, ApiError> {
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::DELETE,
        &format!("/conferences/{room}/video/pin"),
        None,
    )
    .await
}

/// POST /conferences/live/:room/video/spotlight/:pid
#[utoipa::path(
    post,
    path = "/api/v1/conferences/live/{room}/video/spotlight/{pid}",
    tag = "conferences_live",
    operation_id = "conferences_live_spotlight",
    params(("room" = String, Path), ("pid" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn spotlight(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((room, pid)): Path<(String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/video/spotlight/{pid}"),
        None,
    )
    .await
}

/// DELETE /conferences/live/:room/video/spotlight
#[utoipa::path(
    delete,
    path = "/api/v1/conferences/live/{room}/video/spotlight",
    tag = "conferences_live",
    operation_id = "conferences_live_unspotlight",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn unspotlight(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(room): Path<String>,
) -> Result<Proxied, ApiError> {
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::DELETE,
        &format!("/conferences/{room}/video/spotlight"),
        None,
    )
    .await
}

/// POST /conferences/live/:room/participants/:pid/video/{on|off}
#[utoipa::path(
    post,
    path = "/api/v1/conferences/live/{room}/participants/{pid}/video/{state}",
    tag = "conferences_live",
    operation_id = "conferences_live_participant_video",
    params(("room" = String, Path), ("pid" = String, Path), ("state" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn participant_video(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((room, pid, video_state)): Path<(String, String, String)>,
) -> Result<Proxied, ApiError> {
    check_segment(&pid, "participant id")?;
    if video_state != "on" && video_state != "off" {
        return Err(ApiError::BadRequest(
            "Video state must be 'on' or 'off'".to_string(),
        ));
    }
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    forward(
        &state,
        &meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/participants/{pid}/video/{video_state}"),
        None,
    )
    .await
}

/// POST /conferences/live/:room/video/content/stop — a host ends the
/// share under way (§15.6).
#[utoipa::path(
    post,
    path = "/api/v1/conferences/live/{room}/video/content/stop",
    tag = "conferences_live",
    operation_id = "conferences_live_stop_content",
    params(("room" = String, Path)),
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn stop_content(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(room): Path<String>,
) -> Result<Proxied, ApiError> {
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    let out = forward(
        &state,
        &meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/video/content/stop"),
        None,
    )
    .await?;
    if out.0.is_success() {
        info!(room = %room, "Screen share stopped");
    }
    Ok(out)
}

/// PUT /conferences/live/:room/video/content-layout
#[utoipa::path(
    put,
    path = "/api/v1/conferences/live/{room}/video/content-layout",
    tag = "conferences_live",
    operation_id = "conferences_live_set_content_layout",
    params(("room" = String, Path)),
    request_body = SetLayoutRequest,
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn set_content_layout(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(room): Path<String>,
    Json(req): Json<SetLayoutRequest>,
) -> Result<Proxied, ApiError> {
    if !fcp_core::meeting::VIDEO_CONTENT_LAYOUTS.contains(&req.layout.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "layout must be one of {}",
            fcp_core::meeting::VIDEO_CONTENT_LAYOUTS.join(", ")
        )));
    }
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    let out = forward(
        &state,
        &meeting,
        reqwest::Method::PUT,
        &format!("/conferences/{room}/video/content-layout"),
        Some(serde_json::json!({ "layout": req.layout })),
    )
    .await?;
    if out.0.is_success() {
        info!(room = %room, layout = %req.layout, "Content layout set");
    }
    Ok(out)
}

/// POST /conferences/live/:room/dial-out — body passed through
/// (`destination`, optional `greeting_tts` / `greeting_prompt`).
#[utoipa::path(
    post,
    path = "/api/v1/conferences/live/{room}/dial-out",
    tag = "conferences_live",
    operation_id = "conferences_live_dial_out",
    params(("room" = String, Path)),
    request_body = Value,
    responses(
        (status = 200, description = "The conference server's answer, passed through", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn dial_out(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(room): Path<String>,
    Json(body): Json<Value>,
) -> Result<Proxied, ApiError> {
    let meeting = scoped_meeting(&state, &tenant.0, &room).await?;
    dial_out_meeting(&state, &meeting, body).await
}

/// Dial-out on behalf of a meeting already resolved and authorised.
pub async fn dial_out_meeting(
    state: &ApiState,
    meeting: &Meeting,
    body: Value,
) -> Result<Proxied, ApiError> {
    if !body.is_object() {
        return Err(ApiError::BadRequest(
            "Body must be a JSON object".to_string(),
        ));
    }
    let room = &meeting.room_number;
    let out = forward(
        state,
        meeting,
        reqwest::Method::POST,
        &format!("/conferences/{room}/dial-out"),
        Some(body),
    )
    .await?;
    if out.0.is_success() {
        info!(room = %room, "Dial-out requested");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_reject_path_changing_characters() {
        assert!(valid_segment("8000"));
        assert!(valid_segment("p-1_2.3@x+y"));
        assert!(!valid_segment(""));
        assert!(!valid_segment("80/00"));
        assert!(!valid_segment("../8000"));
        assert!(!valid_segment("80 00"));
        assert!(!valid_segment("8000?x=1"));
        assert!(!valid_segment(&"9".repeat(129)));
    }
}
