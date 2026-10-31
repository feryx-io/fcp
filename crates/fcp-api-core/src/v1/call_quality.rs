//! A call's quality as a phone measured it (`docs/CALL_DIAGNOSTICS.md` §2,
//! phase 3): the app sends a call's final figures when it ends
//! (`POST /me/calls/{call_id}/quality`), they are kept beside the call's
//! record, shown on the record (`GET /cdrs/{call_id}/quality`) and in the
//! user's recents, and a poor call is published as `call.quality_poor`.

use crate::auth::{can_access_all_tenants, AuthIdentity, TenantContext, PLATFORM_TENANT_HEADER};
use crate::error::ApiError;
use crate::state::ApiState;
use crate::v1::auth_session::Me;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::call_quality::{CallQuality, CallQualityStore, POOR_MOS};
use fcp_core::legal_hold::{self, SubjectIdentifier};
use fcp_core::storage::{CdrStore, CredentialStore};
use fcp_core::CallId;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The longest codec name and media address a report may carry.
const MAX_CODEC_LEN: usize = 32;
const MAX_ADDRESS_LEN: usize = 64;

/// A call's final figures, as the app measured them.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CallQualityReport {
    /// `incoming` or `outgoing`, from the phone's side.
    pub direction: String,
    /// The codec (`pcmu`, `opus`, …).
    pub codec: String,
    #[serde(default)]
    pub srtp: bool,
    /// Talk time, as the phone counted it.
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    pub packets_sent: u64,
    pub packets_received: u64,
    /// The far end's packets its sequence numbers say were sent, and those
    /// that never arrived.
    pub packets_expected: u64,
    pub packets_lost: u64,
    /// The phone's jitter estimate on what it received, at the end, in ms.
    pub jitter_ms: f32,
    /// The round trip from RTCP, when it was measured.
    #[serde(default)]
    pub rtt_ms: Option<f32>,
    /// What the far end last reported over RTCP: its loss in its last
    /// interval in percent, its jitter in ms, its losses in all.
    #[serde(default)]
    pub far_loss_pct: Option<f32>,
    #[serde(default)]
    pub far_jitter_ms: Option<f32>,
    #[serde(default)]
    pub far_lost: Option<i64>,
    /// The estimated score (E-model MOS, 1 to 4.5) of what the user heard,
    /// and of what the far end heard once it reported.
    pub mos: f32,
    #[serde(default)]
    pub far_mos: Option<f32>,
    /// Where media flowed between, as the phone saw it (`address:port`).
    #[serde(default)]
    pub local_rtp: Option<String>,
    #[serde(default)]
    pub remote_rtp: Option<String>,
}

/// One phone's report of a call.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CallQualityResponse {
    pub call_id: String,
    pub user_id: String,
    /// The reporting user's name, while they exist.
    pub username: Option<String>,
    pub device_id: Option<String>,
    pub direction: String,
    pub codec: String,
    pub srtp: bool,
    pub duration_seconds: Option<u32>,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub packets_expected: u64,
    pub packets_lost: u64,
    /// `packets_lost` of `packets_expected`, in percent.
    pub loss_pct: f32,
    pub jitter_ms: f32,
    pub rtt_ms: Option<f32>,
    pub far_loss_pct: Option<f32>,
    pub far_jitter_ms: Option<f32>,
    pub far_lost: Option<i64>,
    pub mos: f32,
    pub far_mos: Option<f32>,
    /// Either score under 3.1.
    pub poor: bool,
    pub local_rtp: Option<String>,
    pub remote_rtp: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub reported_at: SystemTime,
}

impl CallQualityResponse {
    fn new(q: CallQuality, username: Option<String>) -> Self {
        Self {
            poor: q.is_poor(),
            call_id: q.call_id.to_string(),
            user_id: q.user_id.to_string(),
            username,
            device_id: q.device_id,
            direction: q.direction,
            codec: q.codec,
            srtp: q.srtp,
            duration_seconds: q.duration_seconds,
            packets_sent: q.packets_sent,
            packets_received: q.packets_received,
            packets_expected: q.packets_expected,
            packets_lost: q.packets_lost,
            loss_pct: q.loss_pct,
            jitter_ms: q.jitter_ms,
            rtt_ms: q.rtt_ms,
            far_loss_pct: q.far_loss_pct,
            far_jitter_ms: q.far_jitter_ms,
            far_lost: q.far_lost,
            mos: q.mos,
            far_mos: q.far_mos,
            local_rtp: q.local_rtp,
            remote_rtp: q.remote_rtp,
            reported_at: q.reported_at,
        }
    }
}

/// How one of the user's calls went, by their phone: on `/me/calls`.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MyCallQuality {
    pub mos: f32,
    pub far_mos: Option<f32>,
    /// Either score under 3.1.
    pub poor: bool,
}

impl From<&CallQuality> for MyCallQuality {
    fn from(q: &CallQuality) -> Self {
        Self {
            mos: q.mos,
            far_mos: q.far_mos,
            poor: q.is_poor(),
        }
    }
}

pub(crate) fn store(state: &ApiState) -> Result<&std::sync::Arc<dyn CallQualityStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.call_quality_store())
        .ok_or_else(|| ApiError::ServiceUnavailable("Call quality is not kept here".to_string()))
}

/// Whether `call` may be one `user` was on: its record, once written,
/// names them. A call whose record is not written yet is taken on the
/// app's word — an app reports at hangup, before the record lands.
pub(crate) async fn may_be_users_call(
    storage: &fcp_core::storage::DynStorage,
    user: &fcp_core::User,
    call: &CallId,
) -> Result<bool, ApiError> {
    let Some(cdr) = storage.get_cdr(call).await? else {
        return Ok(true);
    };
    if cdr.tenant_id != user.tenant_id {
        return Ok(false);
    }
    let matcher = legal_hold::expand(
        storage,
        &user.tenant_id,
        &[SubjectIdentifier::User {
            username: user.username.clone(),
        }],
    )
    .await?
    .matcher();
    Ok(matcher.matches(&cdr.from_uri) || matcher.matches(&cdr.to_uri))
}

fn check(report: &CallQualityReport) -> Result<(), ApiError> {
    let bad = |m: &str| Err(ApiError::BadRequest(m.to_string()));
    let score = |m: f32| (1.0..=4.5).contains(&m);
    let pct = |p: f32| (0.0..=100.0).contains(&p);
    let ms = |v: f32| (0.0..=600_000.0).contains(&v);
    if !matches!(report.direction.as_str(), "incoming" | "outgoing") {
        return bad("direction is incoming or outgoing");
    }
    if report.codec.trim().is_empty() || report.codec.len() > MAX_CODEC_LEN {
        return bad("codec names the call's codec");
    }
    if !score(report.mos) || report.far_mos.is_some_and(|m| !score(m)) {
        return bad("a score (MOS) is from 1 to 4.5");
    }
    if report.packets_lost > report.packets_expected {
        return bad("packets_lost is more than packets_expected");
    }
    if report.far_loss_pct.is_some_and(|p| !pct(p)) {
        return bad("far_loss_pct is a percentage");
    }
    if !ms(report.jitter_ms)
        || [report.rtt_ms, report.far_jitter_ms]
            .into_iter()
            .flatten()
            .any(|v| !ms(v))
    {
        return bad("jitter and the round trip are milliseconds, 0 or more");
    }
    if [&report.local_rtp, &report.remote_rtp]
        .into_iter()
        .flatten()
        .any(|a| a.len() > MAX_ADDRESS_LEN)
    {
        return bad("a media address is address:port");
    }
    Ok(())
}

/// POST /me/calls/:call_id/quality — the app's final figures for a call
/// it was on, sent when the call ends. A second report from the same app
/// replaces its first. 404 when the call's record names other people (one
/// not yet written is taken on the app's word); `call.quality_poor` when
/// either score is under 3.1.
#[utoipa::path(
    post,
    path = "/api/v1/me/calls/{call_id}/quality",
    tag = "self_service",
    operation_id = "self_service_report_call_quality",
    params(("call_id" = String, Path, description = "FCP's id for the call (`X-FCP-Call-Id`)")),
    request_body = CallQualityReport,
    responses(
        (status = 201, description = "Kept", body = CallQualityResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn report_call_quality(
    State(state): State<ApiState>,
    me: Me,
    Path(call_id): Path<String>,
    Json(report): Json<CallQualityReport>,
) -> Result<impl IntoResponse, ApiError> {
    let call_id = call_id
        .parse::<CallId>()
        .map_err(|_| ApiError::NotFound("No such call".to_string()))?;
    check(&report)?;
    let (user, device) = crate::v1::softphone::app_device(&state, &me).await?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let quality = store(&state)?;

    if !may_be_users_call(storage, &user, &call_id).await? {
        return Err(ApiError::NotFound("No such call".to_string()));
    }

    let loss_pct = if report.packets_expected == 0 {
        0.0
    } else {
        (report.packets_lost as f64 * 100.0 / report.packets_expected as f64) as f32
    };
    let kept = CallQuality {
        tenant_id: user.tenant_id,
        call_id,
        user_id: user.id,
        device_id: Some(device.id.to_string()),
        direction: report.direction,
        codec: report.codec.trim().to_ascii_lowercase(),
        srtp: report.srtp,
        duration_seconds: report.duration_seconds,
        packets_sent: report.packets_sent,
        packets_received: report.packets_received,
        packets_expected: report.packets_expected,
        packets_lost: report.packets_lost,
        loss_pct,
        jitter_ms: report.jitter_ms,
        rtt_ms: report.rtt_ms,
        far_loss_pct: report.far_loss_pct,
        far_jitter_ms: report.far_jitter_ms,
        far_lost: report.far_lost,
        mos: report.mos,
        far_mos: report.far_mos,
        local_rtp: report.local_rtp,
        remote_rtp: report.remote_rtp,
        reported_at: SystemTime::now(),
    };
    quality.put_call_quality(&kept).await?;

    if kept.is_poor() {
        tracing::info!(
            call_id = %kept.call_id,
            user = %user.username,
            mos = kept.mos,
            far_mos = ?kept.far_mos,
            poor_under = POOR_MOS,
            "a phone reported a poor call"
        );
        state
            .publish_event(fcp_events::PlatformEvent::Call(
                fcp_events::CallEvent::QualityPoor {
                    tenant_id: Some(kept.tenant_id),
                    call_id: kept.call_id,
                    user_id: kept.user_id.to_string(),
                    username: user.username.clone(),
                    device_id: kept.device_id.clone(),
                    mos: kept.mos,
                    far_mos: kept.far_mos,
                    loss_pct: kept.loss_pct,
                    jitter_ms: kept.jitter_ms,
                    rtt_ms: kept.rtt_ms,
                    timestamp: kept.reported_at,
                },
            ))
            .await;
    }
    Ok((
        StatusCode::CREATED,
        Json(CallQualityResponse::new(kept, Some(user.username))),
    ))
}

/// GET /cdrs/:call_id/quality — every phone's report of a call, oldest
/// first, as one page; empty when none reported, 404 for another
/// tenant's call.
#[utoipa::path(
    get,
    path = "/api/v1/cdrs/{call_id}/quality",
    tag = "cdrs",
    operation_id = "cdrs_get_call_quality",
    params(("call_id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<CallQualityResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_call_quality(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Path(call_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let call_id = call_id
        .parse::<CallId>()
        .map_err(|e| ApiError::BadRequest(format!("Invalid call_id: {e}")))?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    // The call's tenant is its record's: a platform administrator reads
    // it, anyone else finds another tenant's call no more than its record.
    let all_tenants = can_access_all_tenants(identity.as_ref().map(|e| &e.0))
        && !headers.contains_key(PLATFORM_TENANT_HEADER);
    let tenant_id = match storage.get_cdr(&call_id).await? {
        Some(cdr) if all_tenants => cdr.tenant_id,
        Some(cdr) if cdr.tenant_id != tenant.0 => {
            return Err(ApiError::NotFound(format!(
                "CDR not found for call_id: {call_id}"
            )))
        }
        _ => tenant.0,
    };
    let mut out = Vec::new();
    for q in store(&state)?.call_quality(&tenant_id, &call_id).await? {
        let username = storage
            .get_user_by_id(&q.user_id)
            .await?
            .map(|u| u.username);
        out.push(CallQualityResponse::new(q, username));
    }
    // One report per phone on the call: never more than a page.
    let total = out.len() as u64;
    Ok(Json(crate::page::Page::new(out, None, Some(total))))
}
