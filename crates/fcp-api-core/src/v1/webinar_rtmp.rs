//! Where a webinar is pushed over RTMP (`docs/WEBINARS.md` §2.8, W7): a
//! live service's ingest URL and the stream key it handed out.
//!
//! - `GET …/stream/rtmp`: the URL and whether a key is set. The key is
//!   never returned.
//! - `PUT …/stream/rtmp`: the URL, and the key. A key left out keeps the
//!   one there is, so the URL can change without the key being typed again.
//! - `DELETE …/stream/rtmp`: no push.
//!
//! The key is sealed at rest and kept apart from the meeting, so it never
//! travels with it to a site.

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use fcp_core::webinar::{check_rtmp_url, RtmpDestination, RtmpDestinationStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::info;

/// A webinar's RTMP destination, as the API shows it.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RtmpDestinationResponse {
    /// The ingest URL, `rtmp://…` or `rtmps://…`; absent when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether a stream key is set. The key itself is never returned.
    pub key_set: bool,
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub updated_at: Option<SystemTime>,
}

/// `PUT …/stream/rtmp` body.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RtmpDestinationRequest {
    /// The service's ingest URL, its application in the path:
    /// `rtmp://a.rtmp.youtube.com/live2`.
    pub url: String,
    /// The stream key. Left out, the key already set is kept.
    #[serde(default)]
    pub key: Option<String>,
}

fn store(state: &ApiState) -> Result<&Arc<dyn RtmpDestinationStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.rtmp_destination_store())
        .ok_or_else(|| ApiError::ServiceUnavailable("no RTMP destination store".into()))
}

fn response(d: Option<&RtmpDestination>) -> RtmpDestinationResponse {
    RtmpDestinationResponse {
        url: d.map(|d| d.url.clone()),
        key_set: d.is_some_and(|d| !d.key.is_empty()),
        updated_at: d.map(|d| d.updated_at),
    }
}

/// GET /conferences/meetings/:id/stream/rtmp — where the webinar is pushed.
#[utoipa::path(
    get,
    path = "/api/v1/conferences/meetings/{id}/stream/rtmp",
    tag = "conferences",
    operation_id = "webinar_rtmp_get",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "The destination", body = RtmpDestinationResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_destination(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<RtmpDestinationResponse>, ApiError> {
    let meeting = super::webinars::meeting_of(&state, &tenant, &id).await?;
    let d = store(&state)?
        .get_rtmp_destination(&meeting.tenant_id, &meeting.id.to_string())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(response(d.as_ref())))
}

/// PUT /conferences/meetings/:id/stream/rtmp — push the webinar here.
#[utoipa::path(
    put,
    path = "/api/v1/conferences/meetings/{id}/stream/rtmp",
    tag = "conferences",
    operation_id = "webinar_rtmp_put",
    params(("id" = String, Path)),
    request_body = RtmpDestinationRequest,
    responses(
        (status = 200, description = "The destination", body = RtmpDestinationResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn put_destination(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<RtmpDestinationRequest>,
) -> Result<Json<RtmpDestinationResponse>, ApiError> {
    let meeting = super::webinars::meeting_of(&state, &tenant, &id).await?;
    if !meeting.is_webinar() {
        return Err(ApiError::Conflict(
            "Only a webinar is pushed over RTMP".to_string(),
        ));
    }
    let url = req.url.trim().to_string();
    check_rtmp_url(&url).map_err(ApiError::BadRequest)?;
    let store = store(&state)?;
    let meeting_id = meeting.id.to_string();
    let key = match req.key.map(|k| k.trim().to_string()) {
        Some(k) if k.is_empty() => {
            return Err(ApiError::BadRequest("The stream key is empty".into()))
        }
        Some(k) if k.len() > 512 || k.chars().any(char::is_control) => {
            return Err(ApiError::BadRequest(
                "The stream key is too long or holds control characters".into(),
            ))
        }
        Some(k) => k,
        None => store
            .get_rtmp_destination(&meeting.tenant_id, &meeting_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .map(|d| d.key)
            .ok_or_else(|| ApiError::BadRequest("A stream key is needed".into()))?,
    };
    let d = RtmpDestination {
        tenant_id: meeting.tenant_id,
        meeting_id,
        url,
        key,
        updated_at: SystemTime::now(),
    };
    store
        .put_rtmp_destination(&d)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    info!(meeting = %meeting.id, url = %d.url, "A webinar's RTMP destination was set");
    Ok(Json(response(Some(&d))))
}

/// DELETE /conferences/meetings/:id/stream/rtmp — push the webinar nowhere.
#[utoipa::path(
    delete,
    path = "/api/v1/conferences/meetings/{id}/stream/rtmp",
    tag = "conferences",
    operation_id = "webinar_rtmp_delete",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "No destination now"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_destination(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let meeting = super::webinars::meeting_of(&state, &tenant, &id).await?;
    let gone = store(&state)?
        .delete_rtmp_destination(&meeting.tenant_id, &meeting.id.to_string())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !gone {
        return Err(ApiError::NotFound(
            "The webinar has no RTMP destination".into(),
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}
