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
    StreamToken, StreamTokenError, DEFAULT_STREAM_TOKEN_TTL, MAX_REGISTRANT_STREAM_TTL,
    MAX_STREAM_TOKEN_TTL,
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
    /// The watch page on this admin (W8): the stream in a browser, no
    /// client needed.
    pub watch_path: String,
    /// The same with this admin's public address, when it knows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watch_url: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: SystemTime,
}

/// The watch page of a link.
fn watch_path(meeting: &MeetingId, token: &str) -> String {
    format!("/watch/{meeting}/{token}")
}

/// A minted link as the API answers it.
fn link_response(
    state: &ApiState,
    meeting: &MeetingId,
    token: &StreamToken,
    wire: String,
) -> StreamLinkResponse {
    let public = |path: &str| {
        state
            .sso
            .as_ref()
            .map(|s| format!("{}{path}", s.public_url.trim_end_matches('/')))
    };
    let path = playlist_path(meeting, &wire);
    let watch = watch_path(meeting, &wire);
    StreamLinkResponse {
        playlist_url: public(&path),
        watch_url: public(&watch),
        playlist_path: path,
        watch_path: watch,
        token: wire,
        expires_at: UNIX_EPOCH + Duration::from_secs(token.expires_at),
    }
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
    info!(meeting = %meeting.id, ttl_secs = ttl, "Stream link minted");
    Ok((
        StatusCode::CREATED,
        Json(link_response(&state, &meeting.id, &token, wire)),
    ))
}

/// The default lifetime of a registrant's stream link: a week.
const DEFAULT_REGISTRANT_STREAM_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// POST /conferences/meetings/:id/registrants/:rid/stream-link — a link to
/// the webinar's stream and its watch page for one registration (W8): its
/// watching counted under it, refused once it is revoked.
#[utoipa::path(
    post,
    path = "/api/v1/conferences/meetings/{id}/registrants/{rid}/stream-link",
    tag = "conferences",
    operation_id = "webinar_registrant_stream_link",
    params(("id" = String, Path), ("rid" = String, Path)),
    request_body = StreamLinkRequest,
    responses(
        (status = 201, description = "Minted", body = StreamLinkResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn mint_registrant_link(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((id, rid)): Path<(String, String)>,
    body: Option<Json<StreamLinkRequest>>,
) -> Result<(StatusCode, Json<StreamLinkResponse>), ApiError> {
    let meeting = super::webinars::webinar_of(&state, &tenant, &id).await?;
    if !meeting.streams_hls() {
        return Err(ApiError::Conflict(format!(
            "webinar '{id}' does not stream: set its streaming to hls"
        )));
    }
    let registrant = super::webinars::registrant_of(&state, &meeting, &rid).await?;
    if !registrant.stands() {
        return Err(ApiError::Conflict(format!(
            "registration '{rid}' is revoked"
        )));
    }
    let secret = join_secret(&state)?;
    let ttl = body
        .and_then(|Json(b)| b.ttl_secs)
        .unwrap_or(DEFAULT_REGISTRANT_STREAM_TTL.as_secs());
    if ttl == 0 || ttl > MAX_REGISTRANT_STREAM_TTL.as_secs() {
        return Err(ApiError::BadRequest(format!(
            "ttl_secs must be between 1 and {}",
            MAX_REGISTRANT_STREAM_TTL.as_secs()
        )));
    }
    let token = StreamToken::for_registrant(
        meeting.tenant_id,
        meeting.id,
        &registrant.id,
        Duration::from_secs(ttl),
    );
    let wire = token.mint(secret.as_bytes());
    info!(meeting = %meeting.id, registrant = %rid, ttl_secs = ttl, "A registrant's stream link minted");
    Ok((
        StatusCode::CREATED,
        Json(link_response(&state, &meeting.id, &token, wire)),
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
    // A registrant's link opens nothing once their registration is revoked
    // (W8), as their join token does not.
    if let Some(rid) = link.registrant.as_deref() {
        let stands = super::webinars::registrant_of(&state, &meeting, rid)
            .await
            .is_ok_and(|r| r.stands());
        if !stands {
            return Err(ApiError::Forbidden(
                "The registration this link is for is revoked".into(),
            ));
        }
    }
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
    // A segment in the stream's bucket comes back as a redirect, passed
    // on to the player (W6.3c).
    let client = reqwest::Client::builder()
        .connect_timeout(UPSTREAM_CONNECT)
        .redirect(reqwest::redirect::Policy::none())
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
    if status.is_redirection() {
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .filter(|l| l.starts_with("https://") || l.starts_with("http://"))
            .ok_or_else(|| {
                ApiError::ServiceUnavailable("The conference server redirected nowhere".into())
            })?
            .to_string();
        return Ok((
            StatusCode::FOUND,
            [
                ("location", location),
                ("cache-control", "no-cache".to_string()),
                ("access-control-allow-origin", "*".to_string()),
            ],
        )
            .into_response());
    }
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
    if let (Some(rid), "index.m3u8") = (link.registrant.as_deref(), file.as_str()) {
        count_view(&state, &meeting, stream.file_name(), rid).await;
    }
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

/// Gaps between a player's playlist fetches longer than this are time it
/// was not watching.
const WATCHING_GAP: Duration = Duration::from_secs(30);
/// A viewer's row is written at most this often.
const VIEW_WRITE_EVERY: Duration = Duration::from_secs(15);

/// What this admin has seen of one viewer of one sitting, between writes.
struct Seen {
    last_fetch: std::time::Instant,
    last_write: Option<std::time::Instant>,
    pending_secs: u64,
}

fn seen() -> &'static std::sync::Mutex<std::collections::HashMap<(String, String), Seen>> {
    static SEEN: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(String, String), Seen>>,
    > = std::sync::OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// A playlist fetch at `now`: the gap since the last, when short, is time
/// watched; what is owed to the row when a write is due.
fn tick(entry: &mut Seen, now: std::time::Instant) -> Option<u64> {
    let gap = now.duration_since(entry.last_fetch);
    if gap < WATCHING_GAP {
        entry.pending_secs += gap.as_secs();
    }
    entry.last_fetch = now;
    let due = entry
        .last_write
        .is_none_or(|w| now.duration_since(w) >= VIEW_WRITE_EVERY);
    if due {
        entry.last_write = Some(now);
        Some(std::mem::take(&mut entry.pending_secs))
    } else {
        None
    }
}

/// A registrant's player fetched the playlist of `sitting`: the time since
/// its last fetch, when short, is time watched; written now and then.
async fn count_view(state: &ApiState, meeting: &Meeting, sitting: &str, registrant: &str) {
    let now = std::time::Instant::now();
    let write = {
        let mut map = seen().lock().unwrap_or_else(|e| e.into_inner());
        // Viewers gone a while are forgotten.
        if map.len() > 10_000 {
            map.retain(|_, s| now.duration_since(s.last_fetch) < WATCHING_GAP * 10);
        }
        let key = (sitting.to_string(), registrant.to_string());
        let entry = map.entry(key).or_insert(Seen {
            last_fetch: now,
            last_write: None,
            pending_secs: 0,
        });
        tick(entry, now)
    };
    let Some(add_secs) = write else { return };
    let Ok(store) = super::webinars::registrant_store(state) else {
        return;
    };
    let view = fcp_core::webinar::StreamViewSeen {
        tenant_id: meeting.tenant_id,
        meeting_id: meeting.id.to_string(),
        sitting_id: sitting.to_string(),
        registrant_id: registrant.to_string(),
        at: SystemTime::now(),
        add_secs,
    };
    if let Err(e) = store.record_stream_view(&view).await {
        warn!(registrant, sitting, error = %e, "A viewer's watching was not recorded");
    }
}

/// The watch page's own files, in the binary (W8): hls.js at a pinned
/// version (Apache-2.0, `assets/watch/HLS_JS_LICENSE`), its script and its
/// style. A CDN would tell a third party who watches.
const HLS_JS: &str = include_str!("../../assets/watch/hls-1.5.20.min.js");
const WATCH_JS: &str = include_str!("../../assets/watch/watch.js");
const WATCH_CSS: &str = include_str!("../../assets/watch/watch.css");

/// The page's policy: its own files alone, the player's media from here or
/// the stream's bucket, nothing framing it.
const WATCH_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    img-src 'self' data:; media-src 'self' blob: http: https:; \
    connect-src 'self' http: https:; worker-src blob:; frame-ancestors 'none'; base-uri 'none'";

/// GET /watch/:file — the watch page's files.
pub async fn watch_asset(Path(file): Path<String>) -> Response {
    let (body, kind) = match file.as_str() {
        "hls-1.5.20.min.js" => (HLS_JS, "text/javascript; charset=utf-8"),
        "watch.js" => (WATCH_JS, "text/javascript; charset=utf-8"),
        "watch.css" => (WATCH_CSS, "text/css; charset=utf-8"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let cache = if file.starts_with("hls-") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=300"
    };
    (
        StatusCode::OK,
        [
            ("content-type", kind),
            ("cache-control", cache),
            ("x-content-type-options", "nosniff"),
        ],
        body,
    )
        .into_response()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// A page saying why a link opens nothing.
fn watch_refused(status: StatusCode, why: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><title>Webinar</title>\
         <link rel=\"stylesheet\" href=\"/watch/watch.css\"></head>\
         <body><main><h1>Webinar</h1><p id=\"state\">{}</p></main></body></html>",
        escape_html(why)
    );
    (status, watch_headers(), html).into_response()
}

fn watch_headers() -> [(&'static str, &'static str); 5] {
    [
        ("content-type", "text/html; charset=utf-8"),
        ("cache-control", "no-store"),
        ("content-security-policy", WATCH_CSP),
        ("x-robots-tag", "noindex"),
        ("referrer-policy", "no-referrer"),
    ]
}

/// GET /watch/:id/:token — a webinar's watch page (W8): the stream in the
/// browser by the link in the path, no client and no sign-in. The page
/// plays the link's playlist; its fetches are what count a registrant's
/// watching.
pub async fn watch_page(
    State(state): State<ApiState>,
    Path((id, token)): Path<(String, String)>,
) -> Response {
    let Ok(secret) = join_secret(&state) else {
        return watch_refused(
            StatusCode::SERVICE_UNAVAILABLE,
            "Watching is not set up here.",
        );
    };
    let link = match StreamToken::verify(&token, secret.as_bytes()) {
        Ok(l) => l,
        Err(StreamTokenError::Expired) => {
            return watch_refused(StatusCode::UNAUTHORIZED, "This link has expired.")
        }
        Err(_) => return watch_refused(StatusCode::UNAUTHORIZED, "This is not a webinar's link."),
    };
    let Ok(meeting_id) = MeetingId::from_str(&id) else {
        return watch_refused(StatusCode::NOT_FOUND, "There is no such webinar.");
    };
    if link.meeting != meeting_id {
        return watch_refused(StatusCode::FORBIDDEN, "This link is for another webinar.");
    }
    let meeting = match state.storage.as_ref() {
        Some(s) => s.get_meeting(&meeting_id).await.ok().flatten(),
        None => None,
    };
    let Some(meeting) = meeting.filter(|m| m.tenant_id == link.tenant_id && m.streams_hls()) else {
        return watch_refused(StatusCode::NOT_FOUND, "This webinar is not streamed.");
    };
    if let Some(rid) = link.registrant.as_deref() {
        let stands = super::webinars::registrant_of(&state, &meeting, rid)
            .await
            .is_ok_and(|r| r.stands());
        if !stands {
            return watch_refused(
                StatusCode::FORBIDDEN,
                "This link no longer opens the webinar.",
            );
        }
    }
    let name = escape_html(&meeting.name);
    let playlist = escape_html(&playlist_path(&meeting.id, &token));
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><title>{name}</title>\
         <link rel=\"stylesheet\" href=\"/watch/watch.css\"></head>\
         <body data-playlist=\"{playlist}\"><main><h1>{name}</h1>\
         <p id=\"state\" aria-live=\"polite\">Connecting…</p>\
         <div class=\"frame\"><video id=\"video\" controls playsinline></video></div>\
         <footer>A webinar on FCP.</footer></main>\
         <script src=\"/watch/hls-1.5.20.min.js\"></script><script src=\"/watch/watch.js\"></script>\
         </body></html>"
    );
    (StatusCode::OK, watch_headers(), html).into_response()
}

#[cfg(test)]
mod watching_tests {
    use super::*;

    #[test]
    fn watching_is_the_short_gaps_between_fetches_written_now_and_then() {
        let t0 = std::time::Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut e = Seen {
            last_fetch: t0,
            last_write: None,
            pending_secs: 0,
        };
        assert_eq!(
            tick(&mut e, at(0)),
            Some(0),
            "the first fetch makes the row"
        );
        // A live player every two seconds: owed, written every fifteen.
        for s in (2..=14).step_by(2) {
            assert_eq!(tick(&mut e, at(s)), None, "at {s}");
        }
        assert_eq!(tick(&mut e, at(16)), Some(16));
        // Away a minute: not watching.
        assert_eq!(tick(&mut e, at(76)), Some(0));
        assert_eq!(tick(&mut e, at(78)), None);
        assert_eq!(tick(&mut e, at(92)), Some(16));
    }
}
