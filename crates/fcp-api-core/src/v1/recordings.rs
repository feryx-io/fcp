//! Conference recordings: the tenant's registry, and the file behind
//! each row.
//!
//! A recording is a row first and a file second (design §11). The row
//! lives here — listable and filterable per tenant — and names the node
//! whose disk holds the file. Downloads and deletes of the file are
//! therefore proxied: this API asks the configured conference server for
//! the file, naming the holding node, and that node either serves it or
//! fetches it from the peer over `/internal` (see
//! `docs/DISTRIBUTED_CONFERENCING.md`).

use crate::auth::{TenantContext, PLATFORM_TENANT_HEADER};
use crate::{error::ApiError, state::ApiState};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use fcp_core::recording::{Recording, RecordingFilter, RecordingStore};
use fcp_core::{MeetingId, RecordingId, TenantId};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// How long the conference server has to answer a metadata request. A
/// download is not bounded this way: only its connect is.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);

fn require_store(state: &ApiState) -> Result<&Arc<dyn RecordingStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.recording_store())
        .ok_or_else(|| ApiError::Internal("Recording registry not configured".to_string()))
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn parse_id(id: &str) -> Result<RecordingId, ApiError> {
    RecordingId::from_str(id)
        .map_err(|_| ApiError::BadRequest(format!("Invalid recording id {id}")))
}

// ─── Responses ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RecordingResponse {
    pub id: String,
    pub tenant_id: String,
    pub meeting_id: Option<String>,
    pub room_number: String,
    pub node_id: String,
    /// The file's name; the path on the node is not a caller's business.
    pub file_name: String,
    /// `room`, `participant`, or `stream`: a webinar's stream out, which
    /// downloads as one MP4.
    pub kind: String,
    pub format: String,
    pub trigger: String,
    pub consent: String,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub started_at: u64,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub ended_at: Option<u64>,
    pub duration_secs: Option<u64>,
    pub size_bytes: Option<u64>,
    pub participants: u32,
    /// Still being written.
    pub running: bool,
    /// Where the bytes are, on this API.
    pub download_url: String,
}

impl From<Recording> for RecordingResponse {
    fn from(r: Recording) -> Self {
        Self {
            download_url: format!("/v1/recordings/{}/download", r.id),
            id: r.id.to_string(),
            tenant_id: r.tenant_id.to_string(),
            meeting_id: r.meeting_id.map(|m| m.to_string()),
            room_number: r.room_number.clone(),
            node_id: r.node_id.clone(),
            file_name: r.download_name(),
            kind: r.kind.as_str().to_string(),
            format: r.format.as_str().to_string(),
            trigger: r.trigger.as_str().to_string(),
            consent: r.consent.as_str().to_string(),
            started_at: unix_secs(r.started_at),
            ended_at: r.ended_at.map(unix_secs),
            duration_secs: r.duration_secs,
            size_bytes: r.size_bytes,
            participants: r.participants,
            running: r.ended_at.is_none(),
        }
    }
}

// ─── List ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[schema(as = recordings::ListQuery)]
pub struct ListQuery {
    pub room_number: Option<String>,
    pub meeting_id: Option<String>,
    /// Unix seconds: recordings that started at or after this.
    pub since: Option<u64>,
    /// Unix seconds: recordings that started before this.
    pub until: Option<u64>,
    /// Only those still being written, or only those finished.
    pub running: Option<bool>,
    /// Only one kind: `room`, `participant` or `stream`.
    pub kind: Option<String>,
}

fn to_filter(q: &ListQuery, page: &crate::page::PageQuery) -> Result<RecordingFilter, ApiError> {
    Ok(RecordingFilter {
        room_number: q.room_number.clone(),
        meeting_id: q
            .meeting_id
            .as_deref()
            .map(MeetingId::from_str)
            .transpose()
            .map_err(|_| ApiError::BadRequest("Invalid meeting_id".to_string()))?,
        since: q.since.map(|s| UNIX_EPOCH + Duration::from_secs(s)),
        until: q.until.map(|s| UNIX_EPOCH + Duration::from_secs(s)),
        running: q.running,
        kind: q
            .kind
            .as_deref()
            .map(|k| {
                fcp_core::RecordingKind::parse(k)
                    .ok_or_else(|| ApiError::BadRequest(format!("Invalid kind {k}")))
            })
            .transpose()?,
        // One more than the page, to know whether there is another.
        limit: page.limit()? + 1,
        offset: page.offset()?,
    })
}

/// GET /v1/recordings
#[utoipa::path(
    get,
    path = "/api/v1/recordings",
    tag = "recordings",
    operation_id = "recordings_list_recordings",
    params(ListQuery, crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<RecordingResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_recordings(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(query): Query<ListQuery>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<RecordingResponse>>, ApiError> {
    let store = require_store(&state)?;
    let filter = to_filter(&query, &page)?;
    let recordings = store
        .list_recordings(&tenant.0, &filter)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let limit = page.limit()?;
    let more = recordings.len() > limit;
    let data = recordings.into_iter().take(limit).map(Into::into).collect();
    Ok(Json(crate::page::offset_page(
        data,
        filter.offset,
        more,
        None,
    )))
}

/// GET /v1/recordings/:id
#[utoipa::path(
    get,
    path = "/api/v1/recordings/{id}",
    tag = "recordings",
    operation_id = "recordings_get_recording",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = RecordingResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_recording(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<RecordingResponse>, ApiError> {
    let recording = scoped_recording(&state, &tenant.0, &id).await?;
    Ok(Json(recording.into()))
}

/// The recording, or 404 — including when it belongs to another tenant,
/// which a caller has no business telling apart from one that is gone.
async fn scoped_recording(
    state: &ApiState,
    tenant: &TenantId,
    id: &str,
) -> Result<Recording, ApiError> {
    let store = require_store(state)?;
    let recording = store
        .get_recording(&parse_id(id)?)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|r| &r.tenant_id == tenant)
        .ok_or_else(|| ApiError::NotFound(format!("Recording {id} not found")))?;
    Ok(recording)
}

// ─── Delete ──────────────────────────────────────────────────────────────

/// DELETE /v1/recordings/:id
///
/// The file goes first, through the node holding it: a row without a
/// file is a broken listing, while a file without a row is caught by the
/// node's own sweep.
#[utoipa::path(
    delete,
    path = "/api/v1/recordings/{id}",
    tag = "recordings",
    operation_id = "recordings_delete_recording",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_recording(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let recording = scoped_recording(&state, &tenant.0, &id).await?;
    if recording.is_running() {
        return Err(ApiError::Conflict(
            "The recording is still being written; stop it first".to_string(),
        ));
    }
    let store = require_store(&state)?;
    // The file goes before the row, so a hold is asked first: a held
    // recording keeps its file too (docs/COMPLIANCE_FOLLOWUPS.md §5).
    if let Some(compliance) = state.storage.as_ref().and_then(|s| s.compliance_store()) {
        if let Some(hold) = compliance
            .hold_on(
                &recording.tenant_id,
                fcp_core::legal_hold::DataClass::ConferenceRecording,
                &recording.id.to_string(),
            )
            .await
            .map_err(ApiError::storage)?
        {
            return Err(ApiError::storage(anyhow::Error::new(
                fcp_core::legal_hold::LegalHoldViolation {
                    hold_id: hold.to_string(),
                },
            )));
        }
    }

    // A recording a site holds: the site is asked to remove the file
    // through its link (`docs/HYBRID_CONTROL_PLANE.md` §3.3), which
    // answers on its own time; the row goes now.
    let site_asked = match (recording.site_id, state.site_files.as_ref()) {
        (Some(site), Some(files)) => Some(
            files
                .remove(
                    site,
                    recording.tenant_id,
                    "conference_recording",
                    &recording.path,
                )
                .await,
        ),
        (Some(site), None) => Some(Err(format!("held at site {site}, and no link runs here"))),
        (None, _) => None,
    };
    let file_deleted = match (site_asked, upstream(&state)) {
        (Some(Ok(())), _) => true,
        (Some(Err(e)), _) => {
            warn!(recording = %recording.id, error = %e, "The site could not be asked to remove the file");
            false
        }
        (None, Ok((base, token))) => {
            let url = format!(
                "{base}/recordings/{}?node={}",
                urlencoding::encode(recording.file_name()),
                urlencoding::encode(&recording.node_id)
            );
            let client = reqwest::Client::builder()
                .timeout(UPSTREAM_TIMEOUT)
                .build()
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let mut req = client
                .delete(&url)
                .header(PLATFORM_TENANT_HEADER, recording.tenant_id.to_string());
            if let Some(token) = token {
                req = req.bearer_auth(token);
            }
            match req.send().await {
                // A file already gone is not a reason to keep the row.
                Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 404 => true,
                Ok(resp) => {
                    warn!(recording = %recording.id, status = %resp.status(), "Conference server refused to delete the file");
                    false
                }
                Err(e) => {
                    warn!(recording = %recording.id, error = %e, "Conference server unreachable for the file delete");
                    false
                }
            }
        }
        (None, Err(_)) => {
            warn!(recording = %recording.id, "No conference server configured; deleting the row only");
            false
        }
    };

    store
        .delete_recording(&recording.id)
        .await
        .map_err(ApiError::storage)?;
    info!(recording = %recording.id, room = %recording.room_number, file_deleted, "AUDIT: conference recording deleted");
    Ok(Json(serde_json::json!({
        "status": "deleted",
        "id": recording.id.to_string(),
        // False when the holding node could not be reached: the row is
        // gone, and the node's retention sweep takes the file by age.
        "file_deleted": file_deleted,
    })))
}

// ─── Download ────────────────────────────────────────────────────────────

/// GET /v1/recordings/:id/download — the bytes, streamed from whichever
/// node holds them.
#[utoipa::path(
    get,
    path = "/api/v1/recordings/{id}/download",
    tag = "recordings",
    operation_id = "recordings_download_recording",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "The recording", content_type = "application/octet-stream", body = Vec<u8>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn download_recording(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let recording = scoped_recording(&state, &tenant.0, &id).await?;
    // A recording a site holds is fetched through the site's link
    // (`docs/HYBRID_CONTROL_PLANE.md` §3.3), not from a conference node.
    if let Some(site) = recording.site_id {
        let files = state.site_files.as_ref().ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "the recording is at a site and no link runs here".to_string(),
            )
        })?;
        let bytes = files
            .fetch(
                site,
                recording.tenant_id,
                "conference_recording",
                &recording.path,
            )
            .await
            .map_err(|e| ApiError::ServiceUnavailable(format!("the site did not answer: {e}")))?;
        return Ok((
            StatusCode::OK,
            [
                ("content-type", recording.format.content_type().to_string()),
                (
                    "content-disposition",
                    format!("attachment; filename=\"{}\"", recording.download_name()),
                ),
            ],
            bytes,
        )
            .into_response());
    }
    let (base, token) = upstream(&state)?;
    let url = format!(
        "{base}/recordings/{}?node={}",
        urlencoding::encode(recording.file_name()),
        urlencoding::encode(&recording.node_id)
    );
    // Only the connect is bounded: a long recording takes as long as it
    // takes to send.
    let client = reqwest::Client::builder()
        .connect_timeout(UPSTREAM_TIMEOUT)
        .build()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut req = client
        .get(&url)
        .header(PLATFORM_TENANT_HEADER, recording.tenant_id.to_string());
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.map_err(|e| {
        warn!(%url, error = %e, "Conference server unreachable for a recording download");
        ApiError::ServiceUnavailable(format!("Conference server unreachable: {e}"))
    })?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let error = crate::error::ErrorResponse::for_status(status, Some(body));
        return Ok((status, Json(error)).into_response());
    }
    Ok((
        StatusCode::OK,
        [
            ("content-type", recording.format.content_type().to_string()),
            (
                "content-disposition",
                format!("attachment; filename=\"{}\"", recording.download_name()),
            ),
        ],
        Body::from_stream(resp.bytes_stream()),
    )
        .into_response())
}

/// The conference server this API forwards file operations to.
fn upstream(state: &ApiState) -> Result<(&str, Option<&str>), ApiError> {
    let base = state.conference_server_url.as_deref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "No conference server configured for recordings ([conference] server_url)".to_string(),
        )
    })?;
    Ok((base, state.conference_server_auth_token.as_deref()))
}
