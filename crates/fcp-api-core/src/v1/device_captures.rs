//! Captures from people's apps (`docs/CALL_DIAGNOSTICS.md` §3, phase 4).
//!
//! Off by default: a tenant turns them on (`diagnostics.capture_allowed`),
//! then an administrator allows a user or one device until a date at most
//! 30 days away (`/captures/allows`). While allowed, `/me/softphone` says
//! so and the app may upload what it captured when a call ends
//! (`POST /me/devices/{id}/captures`, the pcapng as the body). Uploads are
//! kept in `[api.captures]` for the tenant's `capture_keep_days` unless a
//! legal hold keeps them; administrators list them (`GET /captures`) and
//! download them (`GET /captures/{id}/file`, every download audited).

use crate::auth::{AuthIdentity, TenantContext};
use crate::error::ApiError;
use crate::page::{paginate, system_time_key, Order, Page, PageQuery};
use crate::state::ApiState;
use crate::v1::auth_session::Me;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use fcp_core::device_capture::{
    allowed_until, latest_until, AllowSubject, CaptureAllow, DeviceCapture, DeviceCaptureStore,
    MAX_ALLOW_DAYS, PCAPNG_MAGIC,
};
use fcp_core::legal_hold::{self, DataClass, SubjectIdentifier};
use fcp_core::storage::{CdrStore, CredentialStore, DeviceStore, TenantStore};
use fcp_core::{CallId, DeviceId, DynStorage, TenantId, User, UserId};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

const CAPTURE_PREFIX: &str = "cap-";
const ALLOW_PREFIX: &str = "allow-";
/// The longest reason an allow may carry.
const MAX_REASON_LEN: usize = 500;

fn storage(state: &ApiState) -> Result<&Arc<DynStorage>, ApiError> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))
}

fn store(state: &ApiState) -> Result<&Arc<dyn DeviceCaptureStore>, ApiError> {
    storage(state)?
        .device_capture_store()
        .ok_or_else(|| ApiError::ServiceUnavailable("Captures are not kept here".to_string()))
}

fn parse_ref(raw: &str, prefix: &str, what: &str) -> Result<uuid::Uuid, ApiError> {
    let raw = raw.trim();
    uuid::Uuid::parse_str(raw.strip_prefix(prefix).unwrap_or(raw))
        .map_err(|_| ApiError::NotFound(format!("No such {what}")))
}

fn capture_ref(id: &uuid::Uuid) -> String {
    format!("{CAPTURE_PREFIX}{}", id.simple())
}

fn allow_ref(id: &uuid::Uuid) -> String {
    format!("{ALLOW_PREFIX}{}", id.simple())
}

// ── What the app is told ────────────────────────────────────────────────

/// Whether the app may capture (`capture` on `/me/softphone`).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SoftphoneCapture {
    /// The tenant has captures on, an administrator allows this user or
    /// app, and this admin keeps captures.
    pub allowed: bool,
    /// Until when, while allowed.
    #[serde(with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub until: Option<SystemTime>,
    /// The largest capture the app should keep or upload, in MB.
    pub max_mb: u32,
}

/// Whether `user`'s app `device` may capture now, and the tenant's limit.
pub(crate) async fn capture_permit(
    state: &ApiState,
    user: &User,
    device: &DeviceId,
) -> Result<SoftphoneCapture, ApiError> {
    let tenant = storage(state)?.get_tenant(&user.tenant_id).await?;
    let diagnostics = tenant.map(|t| t.diagnostics).unwrap_or_default();
    let mut permit = SoftphoneCapture {
        allowed: false,
        until: None,
        max_mb: diagnostics.capture_max_mb,
    };
    if !diagnostics.capture_allowed || state.capture_files.is_none() {
        return Ok(permit);
    }
    let Some(store) = storage(state)?.device_capture_store() else {
        return Ok(permit);
    };
    let allows = store
        .capture_allows_for(&user.tenant_id, &user.id, device)
        .await?;
    permit.until = allowed_until(&allows, &user.id, device, SystemTime::now());
    permit.allowed = permit.until.is_some();
    Ok(permit)
}

// ── Allows ──────────────────────────────────────────────────────────────

/// An administrator's allow.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CaptureAllowResponse {
    pub id: String,
    /// `user` or `device`.
    pub kind: String,
    pub user_id: Option<String>,
    /// The user's name, while they exist.
    pub username: Option<String>,
    pub device_id: Option<String>,
    /// The device's name, while it exists.
    pub device_name: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub until: SystemTime,
    /// Not yet ended.
    pub active: bool,
    pub reason: Option<String>,
    pub created_by: String,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: SystemTime,
}

async fn allow_response(
    storage: &DynStorage,
    allow: CaptureAllow,
) -> Result<CaptureAllowResponse, ApiError> {
    let (kind, user_id, username, device_id, device_name) = match &allow.subject {
        AllowSubject::User { user_id } => (
            "user",
            Some(user_id.to_string()),
            storage.get_user_by_id(user_id).await?.map(|u| u.username),
            None,
            None,
        ),
        AllowSubject::Device { device_id } => (
            "device",
            None,
            None,
            Some(device_id.to_string()),
            storage.get_device(device_id).await?.map(|d| d.name),
        ),
    };
    Ok(CaptureAllowResponse {
        id: allow_ref(&allow.id),
        kind: kind.to_string(),
        user_id,
        username,
        device_id,
        device_name,
        active: allow.is_active(SystemTime::now()),
        until: allow.until,
        reason: allow.reason,
        created_by: allow.created_by,
        created_at: allow.created_at,
    })
}

/// GET /captures/allows — the tenant's allows, ended ones too, newest
/// first.
#[utoipa::path(
    get,
    path = "/api/v1/captures/allows",
    tag = "captures",
    operation_id = "captures_list_allows",
    params(PageQuery),
    responses(
        (status = 200, description = "OK", body = Page<CaptureAllowResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_allows(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<CaptureAllowResponse>>, ApiError> {
    let storage = storage(&state)?;
    let mut out = Vec::new();
    for allow in store(&state)?.list_capture_allows(&tenant.0).await? {
        out.push(allow_response(storage, allow).await?);
    }
    Ok(Json(paginate(out, &page, Order::Descending, |a| {
        system_time_key(a.created_at, &a.id)
    })?))
}

/// Allow a user (every app they sign in to) or one device to capture.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureAllowRequest {
    /// The user; or name `device_id`.
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    /// When the allow ends: in the future, at most 30 days away.
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub until: SystemTime,
    /// Why, for the record.
    #[serde(default)]
    pub reason: Option<String>,
}

/// POST /captures/allows — allow a user or a device to capture until a
/// date. 409 while the tenant has captures off.
#[utoipa::path(
    post,
    path = "/api/v1/captures/allows",
    tag = "captures",
    operation_id = "captures_create_allow",
    request_body = CaptureAllowRequest,
    responses(
        (status = 201, description = "Made", body = CaptureAllowResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_allow(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Json(req): Json<CaptureAllowRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = storage(&state)?;
    let captures = store(&state)?;
    let on = storage
        .get_tenant(&tenant.0)
        .await?
        .is_some_and(|t| t.diagnostics.capture_allowed);
    if !on {
        return Err(ApiError::Conflict(
            "The tenant has captures off: set diagnostics.capture_allowed first".to_string(),
        ));
    }
    let now = SystemTime::now();
    if req.until <= now {
        return Err(ApiError::BadRequest("until is in the past".to_string()));
    }
    if req.until > latest_until(now) {
        return Err(ApiError::BadRequest(format!(
            "An allow runs at most {MAX_ALLOW_DAYS} days"
        )));
    }
    let reason = req
        .reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    if reason
        .as_ref()
        .is_some_and(|r| r.len() > MAX_REASON_LEN || r.chars().any(char::is_control))
    {
        return Err(ApiError::BadRequest(format!(
            "reason is at most {MAX_REASON_LEN} characters on one line"
        )));
    }
    let subject = match (req.username.as_deref(), req.device_id.as_deref()) {
        (Some(username), None) => {
            let user = storage
                .get_user_in_tenant(username, &tenant.0)
                .await?
                .ok_or_else(|| ApiError::NotFound(format!("No user {username}")))?;
            AllowSubject::User { user_id: user.id }
        }
        (None, Some(device)) => {
            let not_found = || ApiError::NotFound(format!("Device {device} not found"));
            let id = DeviceId::from_str(device).map_err(|_| not_found())?;
            let device = storage
                .get_device(&id)
                .await?
                .filter(|d| d.tenant_id == tenant.0)
                .ok_or_else(not_found)?;
            AllowSubject::Device {
                device_id: device.id,
            }
        }
        _ => {
            return Err(ApiError::BadRequest(
                "Name a username or a device_id".to_string(),
            ))
        }
    };
    let allow = CaptureAllow {
        id: uuid::Uuid::new_v4(),
        tenant_id: tenant.0,
        subject,
        until: req.until,
        reason,
        created_by: identity
            .as_ref()
            .map(|Extension(i)| i.actor())
            .unwrap_or_else(|| "anonymous".to_string()),
        created_at: now,
    };
    captures.put_capture_allow(&allow).await?;
    info!(tenant_id = %allow.tenant_id, allow = %allow_ref(&allow.id), subject = ?allow.subject, "Captures allowed");
    Ok((
        StatusCode::CREATED,
        Json(allow_response(storage, allow).await?),
    ))
}

/// DELETE /captures/allows/:id — end an allow now.
#[utoipa::path(
    delete,
    path = "/api/v1/captures/allows/{id}",
    tag = "captures",
    operation_id = "captures_delete_allow",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_allow(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_ref(&id, ALLOW_PREFIX, "allow")?;
    if store(&state)?.delete_capture_allow(&tenant.0, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("No such allow".to_string()))
    }
}

// ── Captures ────────────────────────────────────────────────────────────

/// An uploaded capture.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CaptureResponse {
    pub id: String,
    /// The app that took it, which may since have been removed.
    pub device_id: String,
    pub device_name: Option<String>,
    pub user_id: String,
    pub username: Option<String>,
    /// The call it holds, when the app named one.
    pub call_id: Option<String>,
    pub size: u64,
    pub sha256: String,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub uploaded_at: SystemTime,
    /// When retention removes it, unless a hold keeps it.
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: SystemTime,
    /// A legal hold keeps it past its expiry.
    pub held: bool,
}

async fn capture_response(
    state: &ApiState,
    capture: DeviceCapture,
) -> Result<CaptureResponse, ApiError> {
    let storage = storage(state)?;
    let held = match storage.compliance_store() {
        Some(holds) => holds
            .hold_on(
                &capture.tenant_id,
                DataClass::DeviceCapture,
                &capture.id.to_string(),
            )
            .await?
            .is_some(),
        None => false,
    };
    Ok(CaptureResponse {
        id: capture_ref(&capture.id),
        device_name: storage
            .get_device(&capture.device_id)
            .await?
            .map(|d| d.name),
        device_id: capture.device_id.to_string(),
        username: storage
            .get_user_by_id(&capture.user_id)
            .await?
            .map(|u| u.username),
        user_id: capture.user_id.to_string(),
        call_id: capture.call_id.map(|c| c.to_string()),
        size: capture.size,
        sha256: capture.sha256,
        uploaded_at: capture.uploaded_at,
        expires_at: capture.expires_at,
        held,
    })
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CaptureFilter {
    /// One device's captures.
    #[serde(default)]
    pub device_id: Option<String>,
    /// One user's captures.
    #[serde(default)]
    pub username: Option<String>,
}

/// GET /captures — the tenant's captures, newest first, of one device or
/// user when named.
#[utoipa::path(
    get,
    path = "/api/v1/captures",
    tag = "captures",
    operation_id = "captures_list",
    params(CaptureFilter, PageQuery),
    responses(
        (status = 200, description = "OK", body = Page<CaptureResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_captures(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(filter): Query<CaptureFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<CaptureResponse>>, ApiError> {
    let storage = storage(&state)?;
    let device = filter
        .device_id
        .as_deref()
        .map(|d| DeviceId::from_str(d).map_err(|_| ApiError::BadRequest(format!("No device {d}"))))
        .transpose()?;
    let user: Option<UserId> = match filter.username.as_deref() {
        Some(name) => Some(
            storage
                .get_user_in_tenant(name, &tenant.0)
                .await?
                .ok_or_else(|| ApiError::NotFound(format!("No user {name}")))?
                .id,
        ),
        None => None,
    };
    let rows = store(&state)?
        .list_device_captures(&tenant.0, device.as_ref(), user.as_ref())
        .await?;
    let mut out = Vec::new();
    for capture in rows {
        out.push(capture_response(&state, capture).await?);
    }
    Ok(Json(paginate(out, &page, Order::Descending, |c| {
        system_time_key(c.uploaded_at, &c.id)
    })?))
}

async fn tenant_capture(
    state: &ApiState,
    tenant: &TenantId,
    id: &str,
) -> Result<DeviceCapture, ApiError> {
    let id = parse_ref(id, CAPTURE_PREFIX, "capture")?;
    store(state)?
        .get_device_capture(tenant, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound("No such capture".to_string()))
}

/// GET /captures/:id/file — the capture itself (pcapng). Every download
/// is in the audit log.
#[utoipa::path(
    get,
    path = "/api/v1/captures/{id}/file",
    tag = "captures",
    operation_id = "captures_download",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "The capture", content_type = "application/vnd.tcpdump.pcapng", body = Vec<u8>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn download_capture(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let capture = tenant_capture(&state, &tenant.0, &id).await?;
    let files = state
        .capture_files
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Captures are not kept here".to_string()))?;
    let (size, body) = files
        .open(&capture.file_key)
        .await
        .map_err(ApiError::ServiceUnavailable)?
        .ok_or_else(|| ApiError::Gone("The capture's file is gone".to_string()))?;
    info!(tenant_id = %tenant.0, capture = %capture_ref(&capture.id), "Capture downloaded");
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/vnd.tcpdump.pcapng")
        .header("content-length", size.to_string())
        .header(
            "content-disposition",
            format!(
                "attachment; filename=\"{}.pcapng\"",
                capture_ref(&capture.id)
            ),
        )
        .header("x-content-sha256", capture.sha256)
        .body(body)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// DELETE /captures/:id — remove a capture before its expiry; 409 while a
/// legal hold keeps it.
#[utoipa::path(
    delete,
    path = "/api/v1/captures/{id}",
    tag = "captures",
    operation_id = "captures_delete",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_capture(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_ref(&id, CAPTURE_PREFIX, "capture")?;
    match store(&state)?.delete_device_capture(&tenant.0, &id).await? {
        Some(_) => {
            // The file went on the queue with the row; removed now if it
            // can be, else by the sweep.
            reap_files(&state, 16).await;
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(ApiError::NotFound("No such capture".to_string())),
    }
}

// ── The app's ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UploadQuery {
    /// FCP's id for the call the capture holds (`X-FCP-Call-Id`).
    #[serde(default)]
    pub call_id: Option<String>,
}

/// POST /me/devices/:id/captures — the app uploads a capture it took
/// (pcapng, as the body), from its own session and while it is allowed.
/// 403 when it is not allowed, 413 over the tenant's limit, 404 when the
/// call named was not the user's.
#[utoipa::path(
    post,
    path = "/api/v1/me/devices/{id}/captures",
    tag = "self_service",
    operation_id = "self_service_upload_capture",
    params(("id" = String, Path, description = "The app's device"), UploadQuery),
    request_body(content_type = "application/vnd.tcpdump.pcapng", content = Vec<u8>),
    responses(
        (status = 201, description = "Kept", body = CaptureResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn upload_capture(
    State(state): State<ApiState>,
    me: Me,
    Path(id): Path<String>,
    Query(query): Query<UploadQuery>,
    body: Body,
) -> Result<impl IntoResponse, ApiError> {
    let (user, device) = crate::v1::softphone::app_device(&state, &me).await?;
    if DeviceId::from_str(&id).ok().as_ref() != Some(&device.id) {
        return Err(ApiError::NotFound(format!("Device {id} not found")));
    }
    let permit = capture_permit(&state, &user, &device.id).await?;
    if !permit.allowed {
        return Err(ApiError::Forbidden(
            "This app is not allowed to capture calls".to_string(),
        ));
    }
    let storage = storage(&state)?;
    let captures = store(&state)?;
    let files = state
        .capture_files
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Captures are not kept here".to_string()))?;
    let call_id = match query.call_id.as_deref() {
        Some(raw) => {
            let call = raw
                .parse::<CallId>()
                .map_err(|_| ApiError::NotFound("No such call".to_string()))?;
            if !crate::v1::call_quality::may_be_users_call(storage, &user, &call).await? {
                return Err(ApiError::NotFound("No such call".to_string()));
            }
            Some(call)
        }
        None => None,
    };

    let capture_id = uuid::Uuid::new_v4();
    let staging = state
        .capture_staging
        .clone()
        .unwrap_or_else(std::env::temp_dir);
    let staged = staging.join(format!(".capture-{}", capture_id.simple()));
    let max = u64::from(permit.max_mb) * 1024 * 1024;
    let received = receive(body, &staging, &staged, max, permit.max_mb).await;
    let (size, sha256) = match received {
        Ok(got) => got,
        Err(e) => {
            let _ = tokio::fs::remove_file(&staged).await;
            return Err(e);
        }
    };

    let keep_days = storage
        .get_tenant(&user.tenant_id)
        .await?
        .map(|t| t.diagnostics.capture_keep_days)
        .unwrap_or(fcp_core::tenant::DEFAULT_CAPTURE_KEEP_DAYS);
    let subject_keys = legal_hold::expand(
        storage,
        &user.tenant_id,
        &[SubjectIdentifier::User {
            username: user.username.clone(),
        }],
    )
    .await?
    .keys();
    let now = SystemTime::now();
    let capture = DeviceCapture {
        id: capture_id,
        tenant_id: user.tenant_id,
        device_id: device.id,
        user_id: user.id,
        call_id,
        size,
        sha256,
        file_key: DeviceCapture::key_for(&user.tenant_id, &capture_id),
        subject_keys,
        uploaded_at: now,
        expires_at: now + Duration::from_secs(u64::from(keep_days) * 86_400),
    };
    let stored = files
        .put(&capture.file_key, &staged, capture.size, &capture.sha256)
        .await;
    let _ = tokio::fs::remove_file(&staged).await;
    stored.map_err(|e| ApiError::ServiceUnavailable(format!("The capture was not stored: {e}")))?;
    if let Err(e) = captures.put_device_capture(&capture).await {
        let _ = files.delete(&capture.file_key).await;
        return Err(e.into());
    }
    info!(
        tenant_id = %capture.tenant_id,
        capture = %capture_ref(&capture.id),
        user = %user.username,
        device_id = %capture.device_id,
        size = capture.size,
        "An app uploaded a capture"
    );
    Ok((
        StatusCode::CREATED,
        Json(capture_response(&state, capture).await?),
    ))
}

/// The body into `staged`, checked to be pcapng and at most `max` bytes:
/// its size and SHA-256.
async fn receive(
    body: Body,
    staging: &std::path::Path,
    staged: &PathBuf,
    max: u64,
    max_mb: u32,
) -> Result<(u64, String), ApiError> {
    let write = |e: std::io::Error| ApiError::Internal(format!("Cannot write the capture: {e}"));
    tokio::fs::create_dir_all(staging).await.map_err(write)?;
    let mut out = tokio::fs::File::create(staged).await.map_err(write)?;
    let (mut size, mut hash) = (0u64, Sha256::new());
    let mut head: Vec<u8> = Vec::with_capacity(PCAPNG_MAGIC.len());
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| ApiError::BadRequest(format!("The upload broke off: {e}")))?;
        size += chunk.len() as u64;
        if size > max {
            return Err(ApiError::PayloadTooLarge(format!(
                "A capture may be at most {max_mb} MB (the tenant's diagnostics.capture_max_mb)"
            )));
        }
        if head.len() < PCAPNG_MAGIC.len() {
            let take = (PCAPNG_MAGIC.len() - head.len()).min(chunk.len());
            head.extend_from_slice(&chunk[..take]);
            if !PCAPNG_MAGIC.starts_with(&head) {
                return Err(ApiError::BadRequest(
                    "The capture is not pcapng".to_string(),
                ));
            }
        }
        hash.update(&chunk);
        out.write_all(&chunk).await.map_err(write)?;
    }
    out.flush().await.map_err(write)?;
    if head.len() < PCAPNG_MAGIC.len() {
        return Err(ApiError::BadRequest(
            "The capture is not pcapng".to_string(),
        ));
    }
    Ok((size, format!("{:x}", hash.finalize())))
}

/// GET /me/captures — the user's own captures, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/me/captures",
    tag = "self_service",
    operation_id = "self_service_list_captures",
    params(PageQuery),
    responses(
        (status = 200, description = "OK", body = Page<CaptureResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_my_captures(
    State(state): State<ApiState>,
    me: Me,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<CaptureResponse>>, ApiError> {
    let user = crate::v1::self_service::load_claims_user(storage(&state)?, &me.0, &state).await?;
    let rows = store(&state)?
        .list_device_captures(&user.tenant_id, None, Some(&user.id))
        .await?;
    let mut out = Vec::new();
    for capture in rows {
        out.push(capture_response(&state, capture).await?);
    }
    Ok(Json(paginate(out, &page, Order::Descending, |c| {
        system_time_key(c.uploaded_at, &c.id)
    })?))
}

// ── Retention ───────────────────────────────────────────────────────────

/// How often expired captures are removed.
const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);
/// How many captures one pass removes.
const SWEEP_BATCH: usize = 500;

/// What a sweep did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CaptureSweep {
    pub expired: u64,
    pub files_removed: u64,
    pub errors: Vec<String>,
}

/// Remove captures past their expiry that no hold keeps, then the files
/// of every capture removed however (`device_capture_file_deletions`).
pub async fn run_sweep(state: &ApiState, now: SystemTime) -> CaptureSweep {
    let mut report = CaptureSweep::default();
    let Ok(captures) = store(state) else {
        return report;
    };
    match captures.expired_device_captures(now, SWEEP_BATCH).await {
        Ok(expired) => {
            for capture in expired {
                match captures
                    .delete_device_capture(&capture.tenant_id, &capture.id)
                    .await
                {
                    Ok(Some(_)) => report.expired += 1,
                    Ok(None) => {}
                    // Held since it was listed: kept.
                    Err(e) => report
                        .errors
                        .push(format!("capture {}: {e}", capture_ref(&capture.id))),
                }
            }
        }
        Err(e) => report.errors.push(format!("expired captures: {e}")),
    }
    let (removed, errors) = reap_files(state, SWEEP_BATCH).await;
    report.files_removed = removed;
    report.errors.extend(errors);
    report
}

/// Remove up to `limit` queued files from the captures' store: how many,
/// and what failed.
async fn reap_files(state: &ApiState, limit: usize) -> (u64, Vec<String>) {
    let (Ok(captures), Some(files)) = (store(state), state.capture_files.as_ref()) else {
        return (0, Vec::new());
    };
    let due = match captures.due_capture_files(limit).await {
        Ok(due) => due,
        Err(e) => return (0, vec![format!("capture files: {e}")]),
    };
    let (mut removed, mut errors) = (0, Vec::new());
    for key in due {
        let done = match files.delete(&key).await {
            Ok(()) => {
                removed += 1;
                captures.capture_file_removed(&key).await
            }
            Err(error) => {
                errors.push(format!("capture file {key}: {error}"));
                captures
                    .capture_file_failed(&key, &error, Duration::from_secs(3600))
                    .await
            }
        };
        if let Err(e) = done {
            errors.push(format!("capture file {key}: {e}"));
        }
    }
    (removed, errors)
}

/// Run the sweep once an hour, one admin at a time.
pub fn spawn_sweep(state: ApiState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            interval.tick().await;
            let Some(storage) = state.storage.clone() else {
                continue;
            };
            if storage.device_capture_store().is_none() {
                continue;
            }
            let lock = match storage.try_maintenance_lock("fcp:captures").await {
                Ok(Some(lock)) => lock,
                Ok(None) => continue,
                Err(e) => {
                    warn!(error = %e, "Capture sweep: could not take the lock");
                    continue;
                }
            };
            let report = run_sweep(&state, SystemTime::now()).await;
            if report != CaptureSweep::default() {
                info!(report = ?report, "Capture sweep completed");
            }
            lock.release().await;
        }
    });
}
