//! A webinar's stream out, for viewers who are not on FCP
//! (`docs/WEBINARS.md` §2.8, W6.3b).
//!
//! - **The stream's state** (`GET …/stream`): whether the meeting streams,
//!   and its newest stream, the recording row its stage keeps (W6.3a).
//! - **Links** (`POST …/stream/links`): a signed [`StreamToken`] naming the
//!   meeting, minted by whoever manages it, as a registrant's link is.
//! - **The stream** (`GET /api/v1/conferences/meetings/{id}/stream/{token}/{file}`):
//!   no bearer, the link is the credential. The token is a path segment
//!   because a player resolves the playlist's segments relative to its URL
//!   and drops a query. The newest stream's row names its sitting and the
//!   node writing it; the request goes through a conference node to it.

use crate::auth::{TenantContext, PLATFORM_TENANT_HEADER};
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    body::Body,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use fcp_core::recording::{Recording, RecordingFilter, RecordingKind};
use fcp_core::storage::ConferenceStore;
use fcp_core::stream_token::{
    StreamToken, StreamTokenError, DEFAULT_STREAM_TOKEN_TTL, MAX_STREAM_TOKEN_TTL,
};
use fcp_core::{Meeting, MeetingId};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// How long the conference node may take to connect.
const UPSTREAM_CONNECT: Duration = Duration::from_secs(5);

/// The meeting's newest stream: its stage's `stream` row.
async fn newest_stream(state: &ApiState, meeting: &Meeting) -> Result<Option<Recording>, ApiError> {
    let Some(store) = state.storage.as_ref().and_then(|s| s.recording_store()) else {
        return Ok(None);
    };
    let filter = RecordingFilter {
        meeting_id: Some(meeting.id),
        kind: Some(RecordingKind::Stream),
        limit: 1,
        ..Default::default()
    };
    store
        .list_recordings(&meeting.tenant_id, &filter)
        .await
        .map(|rows| rows.into_iter().next())
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Where a link's playlist is, on this admin.
fn playlist_path(meeting: &MeetingId, token: &str) -> String {
    format!("/api/v1/conferences/meetings/{meeting}/stream/{token}/index.m3u8")
}

/// A webinar's stream, as the API shows it.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct StreamStateResponse {
    /// `off` (the meeting does not stream), `waiting` (it will, once live),
    /// `live`, or `ended` (its newest stream ended, kept for replay).
    pub state: String,
    /// The newest stream's recording.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_id: Option<String>,
    /// The sitting it streamed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sitting_id: Option<String>,
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub started_at: Option<SystemTime>,
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub ended_at: Option<SystemTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
}

/// GET /conferences/meetings/:id/stream — the webinar's stream.
#[utoipa::path(
    get,
    path = "/api/v1/conferences/meetings/{id}/stream",
    tag = "conferences",
    operation_id = "webinar_stream_state",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "The stream", body = StreamStateResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn stream_state(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<StreamStateResponse>, ApiError> {
    let meeting = super::webinars::meeting_of(&state, &tenant, &id).await?;
    let newest = if meeting.streams_hls() {
        newest_stream(&state, &meeting).await?
    } else {
        None
    };
    let state_word = match (&newest, meeting.streams_hls()) {
        (_, false) => "off",
        (None, true) => "waiting",
        (Some(r), true) if r.is_running() => "live",
        (Some(_), true) => "ended",
    };
    Ok(Json(StreamStateResponse {
        state: state_word.to_string(),
        recording_id: newest.as_ref().map(|r| r.id.to_string()),
        sitting_id: newest.as_ref().map(|r| r.file_name().to_string()),
        started_at: newest.as_ref().map(|r| r.started_at),
        ended_at: newest.as_ref().and_then(|r| r.ended_at),
        duration_secs: newest.as_ref().and_then(|r| r.duration_secs),
    }))
}

/// `POST …/stream/links` body.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamLinkRequest {
    /// The link's lifetime in seconds: a day by default, a week at most.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// A stream link.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct StreamLinkResponse {
    /// The signed token, the credential.
    pub token: String,
    /// The playlist on this admin, for a player.
    pub playlist_path: String,
    /// The same with this admin's public address, when it knows it
    /// (`[api.sso] public_url`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_url: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: SystemTime,
}

/// POST /conferences/meetings/:id/stream/links — a link to the webinar's
/// stream for viewers who are not on FCP.
#[utoipa::path(
    post,
    path = "/api/v1/conferences/meetings/{id}/stream/links",
    tag = "conferences",
    operation_id = "webinar_stream_mint_link",
    params(("id" = String, Path)),
    request_body = StreamLinkRequest,
    responses(
        (status = 201, description = "Minted", body = StreamLinkResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn mint_link(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    body: Option<Json<StreamLinkRequest>>,
) -> Result<(StatusCode, Json<StreamLinkResponse>), ApiError> {
    let meeting = super::webinars::webinar_of(&state, &tenant, &id).await?;
    if !meeting.streams_hls() {
        return Err(ApiError::Conflict(format!(
            "webinar '{id}' does not stream: set its streaming to hls"
        )));
    }
    let secret = join_secret(&state)?;
    let ttl = body
        .and_then(|Json(b)| b.ttl_secs)
        .unwrap_or(DEFAULT_STREAM_TOKEN_TTL.as_secs());
    if ttl == 0 || ttl > MAX_STREAM_TOKEN_TTL.as_secs() {
        return Err(ApiError::BadRequest(format!(
            "ttl_secs must be between 1 and {}",
            MAX_STREAM_TOKEN_TTL.as_secs()
        )));
    }
    let token = StreamToken::new(meeting.tenant_id, meeting.id, Duration::from_secs(ttl));
    let wire = token.mint(secret.as_bytes());
    let path = playlist_path(&meeting.id, &wire);
    let playlist_url = state
        .sso
        .as_ref()
        .map(|s| format!("{}{path}", s.public_url.trim_end_matches('/')));
    info!(meeting = %meeting.id, ttl_secs = ttl, "Stream link minted");
    Ok((
        StatusCode::CREATED,
        Json(StreamLinkResponse {
            token: wire,
            playlist_path: path,
            playlist_url,
            expires_at: UNIX_EPOCH + Duration::from_secs(token.expires_at),
        }),
    ))
}

fn join_secret(state: &ApiState) -> Result<&str, ApiError> {
    state
        .conference_join_secret
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "stream links are not configured: set [conference] join_secret".to_string(),
            )
        })
}

/// Whether `name` is a file a stream holds: the playlist, the init segment,
/// or `seg-<n>.m4s`, as the conference node serves.
fn is_stream_file(name: &str) -> bool {
    name == "index.m3u8"
        || name == "init.mp4"
        || name
            .strip_prefix("seg-")
            .and_then(|n| n.strip_suffix(".m4s"))
            .is_some_and(|n| {
                !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit())
            })
}

/// GET /api/v1/conferences/meetings/:id/stream/:token/:file — the webinar's
/// stream to a link's holder: no bearer token, the link is the credential.
#[utoipa::path(
    get,
    path = "/api/v1/conferences/meetings/{id}/stream/{token}/{file}",
    tag = "conferences",
    operation_id = "webinar_stream_serve",
    security(()),
    params(
        ("id" = String, Path),
        ("token" = String, Path, description = "A stream link's token"),
        ("file" = String, Path, description = "`index.m3u8`, `init.mp4` or `seg-<n>.m4s`"),
    ),
    responses(
        (status = 200, description = "The playlist or a segment", content_type = "application/vnd.apple.mpegurl"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn serve(
    State(state): State<ApiState>,
    Path((id, token, file)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    if !is_stream_file(&file) {
        return Err(ApiError::BadRequest("Invalid stream file".to_string()));
    }
    let meeting_id = MeetingId::from_str(&id)
        .map_err(|_| ApiError::NotFound(format!("Meeting '{id}' not found")))?;
    let link =
        StreamToken::verify(&token, join_secret(&state)?.as_bytes()).map_err(|e| match e {
            StreamTokenError::Expired => {
                ApiError::Unauthorized("The stream link has expired".into())
            }
            _ => ApiError::Unauthorized("Not a stream link".into()),
        })?;
    if link.meeting != meeting_id {
        return Err(ApiError::Forbidden(
            "The link is for another meeting".into(),
        ));
    }
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("no storage".into()))?;
    let meeting = storage
        .get_meeting(&meeting_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|m| m.tenant_id == link.tenant_id && m.streams_hls())
        .ok_or_else(|| ApiError::NotFound(format!("Meeting '{id}' does not stream")))?;
    let stream = newest_stream(&state, &meeting)
        .await?
        .ok_or_else(|| ApiError::NotFound("The webinar has not streamed yet".into()))?;
    let base = state.conference_server_url.as_deref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "No conference server configured ([conference] server_url)".into(),
        )
    })?;
    let url = format!(
        "{base}/hls/{}/{file}?node={}",
        urlencoding::encode(stream.file_name()),
        urlencoding::encode(&stream.node_id)
    );
    let client = reqwest::Client::builder()
        .connect_timeout(UPSTREAM_CONNECT)
        .build()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut req = client
        .get(&url)
        .header(PLATFORM_TENANT_HEADER, meeting.tenant_id.to_string());
    if let Some(token) = state.conference_server_auth_token.as_deref() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.map_err(|e| {
        warn!(%url, error = %e, "Conference server unreachable for a stream");
        ApiError::ServiceUnavailable(format!("Conference server unreachable: {e}"))
    })?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let error = crate::error::ErrorResponse::for_status(status, Some(body));
        return Ok((status, Json(error)).into_response());
    }
    let header = |name: &str, default: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(default)
            .to_string()
    };
    let content_type = header("content-type", "application/octet-stream");
    let cache = header("cache-control", "no-cache");
    Ok((
        StatusCode::OK,
        [
            ("content-type", content_type),
            ("cache-control", cache),
            // A player on another origin fetches it.
            ("access-control-allow-origin", "*".to_string()),
        ],
        Body::from_stream(resp.bytes_stream()),
    )
        .into_response())
}
