//! Managed firmware (`docs/PROVISIONING.md` §11,
//! `docs/IDENTITY_AND_DEVICES.md` phase 9): images the platform
//! administrator uploads, every tenant's targets, and who is behind.

use crate::{auth::AuthIdentity, error::ApiError, state::ApiState};
use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use fcp_core::config_template::PhoneVendor;
use fcp_core::firmware::{
    target_for, valid_file_name, FirmwareImage, FirmwareStatus, FirmwareTarget, RolloutStage,
};
use fcp_core::permissions::Permission;
use fcp_core::storage::DeviceStore;
use fcp_core::{Device, DeviceId, TenantId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::SystemTime;
use tokio::io::AsyncWriteExt;
use tracing::info;

fn storage(state: &ApiState) -> Result<&std::sync::Arc<fcp_core::DynStorage>, ApiError> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Storage is required but not configured".to_string()))
}

fn store(
    storage: &fcp_core::DynStorage,
) -> Result<&std::sync::Arc<dyn fcp_core::provisioning::ProvisioningStore>, ApiError> {
    storage
        .provisioning_store()
        .ok_or_else(|| ApiError::NotFound("Provisioning is not available here".to_string()))
}

/// Where images are kept, when the admin keeps them.
fn directory(state: &ApiState) -> Result<PathBuf, ApiError> {
    state
        .firmware
        .directory
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "Firmware is not kept here: set [api.firmware] directory".to_string(),
            )
        })
}

/// An image's file on disk.
pub(crate) fn image_path(dir: &std::path::Path, image: &FirmwareImage) -> PathBuf {
    dir.join(image.id.simple().to_string())
        .join(&image.file_name)
}

/// An image's id as the API shows it: `fw-<uuid>`.
fn image_ref(id: &uuid::Uuid) -> String {
    format!("fw-{id}")
}

/// A target's id as the API shows it: `fwt-<uuid>`.
fn target_ref(id: &uuid::Uuid) -> String {
    format!("fwt-{id}")
}

/// An id with or without its prefix.
fn parse_ref(raw: &str, prefix: &str) -> Option<uuid::Uuid> {
    let raw = raw.trim();
    uuid::Uuid::parse_str(raw.strip_prefix(prefix).unwrap_or(raw)).ok()
}

fn platform_admin(identity: &AuthIdentity) -> Result<(), ApiError> {
    if identity.is_platform_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "Firmware images are the platform administrator's to upload and remove".to_string(),
        ))
    }
}

/// An uploaded image.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct FirmwareImageResponse {
    pub id: String,
    pub vendor: String,
    pub models: Vec<String>,
    pub version: String,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub notes: Option<String>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub uploaded_at: SystemTime,
}

impl From<FirmwareImage> for FirmwareImageResponse {
    fn from(i: FirmwareImage) -> Self {
        Self {
            id: image_ref(&i.id),
            vendor: i.vendor.to_string(),
            models: i.models,
            version: i.version,
            file_name: i.file_name,
            size: i.size,
            sha256: i.sha256,
            notes: i.notes,
            uploaded_at: i.uploaded_at,
        }
    }
}

/// GET /api/v1/firmware — every uploaded image.
#[utoipa::path(
    get,
    path = "/api/v1/firmware",
    tag = "firmware",
    operation_id = "firmware_list_images",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<FirmwareImageResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_images(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let images: Vec<FirmwareImageResponse> = store(storage(&state)?)?
        .list_firmware_images()
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(crate::page::paginate(
        images,
        &page,
        crate::page::Order::Ascending,
        |i| format!("{}/{}/{}", i.vendor, i.version, i.id),
    )?))
}

/// POST /api/v1/firmware — upload an image (multipart: `vendor`,
/// `version`, `models` (comma-separated patterns), `notes`, and `file`,
/// whose name is the one it is served under). The platform
/// administrator's.
#[utoipa::path(
    post,
    path = "/api/v1/firmware",
    tag = "firmware",
    operation_id = "firmware_upload_image",
    request_body(content_type = "multipart/form-data", content = String),
    responses(
        (status = 201, description = "Made", body = FirmwareImageResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn upload_image(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::SystemConfig, state.deployment_mode)?;
    platform_admin(&identity)?;
    let dir = directory(&state)?;
    let store = store(storage(&state)?)?;
    let id = uuid::Uuid::new_v4();
    let image_dir = dir.join(id.simple().to_string());
    let max = state.firmware.max_image_mb * 1024 * 1024;
    let (mut vendor, mut version, mut models, mut notes) = (None, None, Vec::new(), None);
    let mut file: Option<(String, u64, String)> = None;
    let bad = |m: String| ApiError::BadRequest(m);

    let outcome: Result<(), ApiError> = async {
        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|e| bad(format!("The upload is malformed: {e}")))?
        {
            let name = field.name().unwrap_or("").to_string();
            if name == "file" {
                let file_name = field.file_name().unwrap_or("").to_string();
                if !valid_file_name(&file_name) {
                    return Err(bad(format!(
                        "`{file_name}` is not a file name an image can be served under"
                    )));
                }
                tokio::fs::create_dir_all(&image_dir)
                    .await
                    .map_err(|e| ApiError::Internal(format!("Cannot write images: {e}")))?;
                let path = image_dir.join(&file_name);
                let mut out = tokio::fs::File::create(&path)
                    .await
                    .map_err(|e| ApiError::Internal(format!("Cannot write the image: {e}")))?;
                let (mut size, mut hash) = (0u64, Sha256::new());
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| bad(format!("The upload broke off: {e}")))?
                {
                    size += chunk.len() as u64;
                    if size > max {
                        return Err(bad(format!(
                            "An image may be at most {} MB ([api.firmware] max_image_mb)",
                            state.firmware.max_image_mb
                        )));
                    }
                    hash.update(&chunk);
                    out.write_all(&chunk)
                        .await
                        .map_err(|e| ApiError::Internal(format!("Cannot write the image: {e}")))?;
                }
                out.flush()
                    .await
                    .map_err(|e| ApiError::Internal(format!("Cannot write the image: {e}")))?;
                file = Some((file_name, size, format!("{:x}", hash.finalize())));
            } else {
                let text = field
                    .text()
                    .await
                    .map_err(|e| bad(format!("`{name}` is unreadable: {e}")))?;
                match name.as_str() {
                    "vendor" => vendor = Some(text),
                    "version" => version = Some(text),
                    "models" => {
                        models = text
                            .split(',')
                            .map(str::trim)
                            .filter(|m| !m.is_empty())
                            .map(String::from)
                            .collect()
                    }
                    "notes" => notes = Some(text).filter(|n| !n.trim().is_empty()),
                    _ => {}
                }
            }
        }
        Ok(())
    }
    .await;
    let cleanup = || async {
        let _ = tokio::fs::remove_dir_all(&image_dir).await;
    };
    if let Err(e) = outcome {
        cleanup().await;
        return Err(e);
    }
    let vendor: PhoneVendor = match vendor.as_deref().map(str::trim).map(PhoneVendor::from_str) {
        Some(Ok(v)) if PhoneVendor::PHONES.contains(&v) => v,
        _ => {
            cleanup().await;
            return Err(bad("vendor must be a phone's vendor".to_string()));
        }
    };
    let version = match version.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() && v.len() <= 64 && !v.chars().any(char::is_control) => {
            v.to_string()
        }
        _ => {
            cleanup().await;
            return Err(bad(
                "version is required (as the phones report it)".to_string()
            ));
        }
    };
    let Some((file_name, size, sha256)) = file else {
        cleanup().await;
        return Err(bad("file is required".to_string()));
    };
    if size == 0 {
        cleanup().await;
        return Err(bad("The image is empty".to_string()));
    }
    let image = FirmwareImage {
        id,
        vendor,
        models,
        version,
        file_name,
        size,
        sha256,
        notes,
        uploaded_at: SystemTime::now(),
    };
    if let Err(e) = store.put_firmware_image(&image).await {
        cleanup().await;
        return Err(e.into());
    }
    info!(image_id = %image.id, vendor = %image.vendor, version = %image.version, size, "Firmware image uploaded");
    Ok((
        StatusCode::CREATED,
        Json(FirmwareImageResponse::from(image)),
    ))
}

fn image_id(id: &str) -> Result<uuid::Uuid, ApiError> {
    parse_ref(id, "fw-").ok_or_else(|| ApiError::NotFound(format!("Image {id} not found")))
}

/// DELETE /api/v1/firmware/:id — remove an image no tenant targets. The
/// platform administrator's.
#[utoipa::path(
    delete,
    path = "/api/v1/firmware/{id}",
    tag = "firmware",
    operation_id = "firmware_delete_image",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_image(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::SystemConfig, state.deployment_mode)?;
    platform_admin(&identity)?;
    let id = image_id(&id)?;
    let store = store(storage(&state)?)?;
    let image = store
        .get_firmware_image(&id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Image {id} not found")))?;
    let targets = store.firmware_image_targets(&id).await?;
    if targets > 0 {
        return Err(ApiError::Conflict(format!(
            "{targets} tenant target(s) still name this image"
        )));
    }
    store.delete_firmware_image(&id).await?;
    if let Ok(dir) = directory(&state) {
        let _ = tokio::fs::remove_dir_all(dir.join(image.id.simple().to_string())).await;
    }
    info!(image_id = %id, "Firmware image removed");
    Ok(StatusCode::NO_CONTENT)
}

// ---- A tenant's targets ----

/// A tenant's target.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct FirmwareTargetResponse {
    pub id: String,
    pub vendor: String,
    pub model_pattern: Option<String>,
    pub image_id: String,
    pub version: Option<String>,
    pub stage: RolloutStage,
    pub pilot_devices: Vec<String>,
    /// The tenant's phones it is for (stage aside), and where they stand.
    pub devices: u64,
    pub current: u64,
    pub behind: u64,
    pub unknown: u64,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: SystemTime,
}

/// The phones a target is for, ignoring its stage: its vendor's whose
/// model its pattern names, and which no more specific target claims.
fn phones_of<'a>(
    target: &FirmwareTarget,
    targets: &[FirmwareTarget],
    devices: &'a [Device],
) -> Vec<&'a Device> {
    devices
        .iter()
        .filter(|d| {
            let mut all = targets.to_vec();
            for t in &mut all {
                t.stage = RolloutStage::All;
            }
            target_for(&all, vendor_of(d), d.model.as_deref(), &d.id).map(|t| t.id)
                == Some(target.id)
        })
        .collect()
}

fn vendor_of(d: &Device) -> Option<PhoneVendor> {
    d.vendor.or_else(|| {
        d.mac_address
            .as_deref()
            .and_then(fcp_core::provisioning::vendor_of_mac)
    })
}

async fn target_responses(
    state: &ApiState,
    tenant: &TenantId,
) -> Result<Vec<FirmwareTargetResponse>, ApiError> {
    let storage = storage(state)?;
    let store = store(storage)?;
    let targets = store.list_firmware_targets(tenant).await?;
    let images = store.list_firmware_images().await?;
    let devices = storage.list_devices(tenant).await?;
    Ok(targets
        .iter()
        .map(|t| {
            let version = images
                .iter()
                .find(|i| i.id == t.image_id)
                .map(|i| i.version.clone());
            let phones = phones_of(t, &targets, &devices);
            let count = |s: FirmwareStatus| {
                phones
                    .iter()
                    .filter(|d| {
                        version.as_deref().is_some_and(|v| {
                            FirmwareStatus::of(d.firmware_version.as_deref(), v) == s
                        })
                    })
                    .count() as u64
            };
            FirmwareTargetResponse {
                id: target_ref(&t.id),
                vendor: t.vendor.to_string(),
                model_pattern: t.model_pattern.clone(),
                image_id: image_ref(&t.image_id),
                version: version.clone(),
                stage: t.stage,
                pilot_devices: t.pilot_devices.iter().map(|d| d.to_string()).collect(),
                devices: phones.len() as u64,
                current: count(FirmwareStatus::Current),
                behind: count(FirmwareStatus::Behind),
                unknown: count(FirmwareStatus::Unknown),
                updated_at: t.updated_at,
            }
        })
        .collect())
}

/// GET /api/v1/firmware/targets — the tenant's targets, each with how many
/// of its phones run its version.
#[utoipa::path(
    get,
    path = "/api/v1/firmware/targets",
    tag = "firmware",
    operation_id = "firmware_list_targets",
    responses(
        (status = 200, description = "OK", body = crate::page::Page<FirmwareTargetResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_targets(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let rows = target_responses(&state, &tenant.0).await?;
    Ok(Json(crate::page::paginate(
        rows,
        &crate::page::PageQuery::default(),
        crate::page::Order::Ascending,
        |r| {
            format!(
                "{}/{}/{}",
                r.vendor,
                r.model_pattern.as_deref().unwrap_or(""),
                r.id
            )
        },
    )?))
}

/// A target, made or changed.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FirmwareTargetRequest {
    pub vendor: String,
    /// Unset: every model of the vendor.
    pub model_pattern: Option<String>,
    pub image_id: String,
    /// Unset: `pilot`.
    pub stage: Option<RolloutStage>,
    #[serde(default)]
    pub pilot_devices: Vec<String>,
}

async fn checked_target(
    state: &ApiState,
    tenant: TenantId,
    id: uuid::Uuid,
    req: FirmwareTargetRequest,
) -> Result<FirmwareTarget, ApiError> {
    let storage = storage(state)?;
    let vendor: PhoneVendor = req.vendor.trim().parse().map_err(ApiError::BadRequest)?;
    let image_id = parse_ref(&req.image_id, "fw-")
        .ok_or_else(|| ApiError::BadRequest("image_id is not an image's id".to_string()))?;
    let image = store(storage)?
        .get_firmware_image(&image_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest(format!("Image {image_id} not found")))?;
    if image.vendor != vendor {
        return Err(ApiError::BadRequest(format!(
            "The image is {}'s, not {vendor}'s",
            image.vendor
        )));
    }
    let model_pattern = req
        .model_pattern
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    if model_pattern.as_deref().is_some_and(|p| p.len() > 64) {
        return Err(ApiError::BadRequest(
            "model_pattern is too long".to_string(),
        ));
    }
    let mut pilot_devices = Vec::new();
    for raw in req.pilot_devices {
        let device_id = DeviceId::from_str(raw.trim())
            .map_err(|_| ApiError::BadRequest(format!("`{raw}` is not a device id")))?;
        storage
            .get_device(&device_id)
            .await?
            .filter(|d| d.tenant_id == tenant)
            .ok_or_else(|| ApiError::BadRequest(format!("Device {device_id} not found")))?;
        pilot_devices.push(device_id);
    }
    Ok(FirmwareTarget {
        id,
        tenant_id: tenant,
        vendor,
        model_pattern,
        image_id,
        stage: req.stage.unwrap_or(RolloutStage::Pilot),
        pilot_devices,
        updated_at: SystemTime::now(),
    })
}

async fn one_target(
    state: &ApiState,
    tenant: &TenantId,
    id: &uuid::Uuid,
) -> Result<FirmwareTargetResponse, ApiError> {
    target_responses(state, tenant)
        .await?
        .into_iter()
        .find(|t| t.id == target_ref(id))
        .ok_or_else(|| ApiError::NotFound(format!("Target {id} not found")))
}

/// POST /api/v1/firmware/targets
#[utoipa::path(
    post,
    path = "/api/v1/firmware/targets",
    tag = "firmware",
    operation_id = "firmware_create_target",
    request_body = FirmwareTargetRequest,
    responses(
        (status = 201, description = "Made", body = FirmwareTargetResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_target(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<FirmwareTargetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let target = checked_target(&state, tenant.0, uuid::Uuid::new_v4(), req).await?;
    store(storage(&state)?)?
        .put_firmware_target(&target)
        .await?;
    info!(tenant_id = %tenant.0, target_id = %target.id, "Firmware target made");
    Ok((
        StatusCode::CREATED,
        Json(one_target(&state, &tenant.0, &target.id).await?),
    ))
}

/// PUT /api/v1/firmware/targets/:id — change a target: another image, the
/// pilot set, or `all` to roll it out.
#[utoipa::path(
    put,
    path = "/api/v1/firmware/targets/{id}",
    tag = "firmware",
    operation_id = "firmware_update_target",
    params(("id" = String, Path)),
    request_body = FirmwareTargetRequest,
    responses(
        (status = 200, description = "OK", body = FirmwareTargetResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_target(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<FirmwareTargetRequest>,
) -> Result<Json<FirmwareTargetResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let id = parse_ref(&id, "fwt-")
        .ok_or_else(|| ApiError::NotFound(format!("Target {id} not found")))?;
    let store = store(storage(&state)?)?;
    if !store
        .list_firmware_targets(&tenant.0)
        .await?
        .iter()
        .any(|t| t.id == id)
    {
        return Err(ApiError::NotFound(format!("Target {id} not found")));
    }
    let target = checked_target(&state, tenant.0, id, req).await?;
    store.put_firmware_target(&target).await?;
    Ok(Json(one_target(&state, &tenant.0, &id).await?))
}

/// DELETE /api/v1/firmware/targets/:id — phones keep what they run; their
/// templates stop naming an image.
#[utoipa::path(
    delete,
    path = "/api/v1/firmware/targets/{id}",
    tag = "firmware",
    operation_id = "firmware_delete_target",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_target(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let id = parse_ref(&id, "fwt-")
        .ok_or_else(|| ApiError::NotFound(format!("Target {id} not found")))?;
    if !store(storage(&state)?)?
        .delete_firmware_target(&tenant.0, &id)
        .await?
    {
        return Err(ApiError::NotFound(format!("Target {id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- What a phone is pointed at ----

/// The image a device's phone is pointed at now, and its target's version.
pub(crate) async fn image_for(
    storage: &fcp_core::DynStorage,
    device: &Device,
) -> Result<Option<FirmwareImage>, ApiError> {
    let Some(store) = storage.provisioning_store() else {
        return Ok(None);
    };
    let targets = store.list_firmware_targets(&device.tenant_id).await?;
    let Some(target) = target_for(
        &targets,
        vendor_of(device),
        device.model.as_deref(),
        &device.id,
    ) else {
        return Ok(None);
    };
    Ok(store.get_firmware_image(&target.image_id).await?)
}

/// A device's firmware against its target, for the device views.
pub(crate) fn status_of(
    device: &Device,
    targets: &[FirmwareTarget],
    images: &[FirmwareImage],
) -> Option<(String, FirmwareStatus)> {
    let target = target_for(
        targets,
        vendor_of(device),
        device.model.as_deref(),
        &device.id,
    )?;
    let image = images.iter().find(|i| i.id == target.image_id)?;
    Some((
        image.version.clone(),
        FirmwareStatus::of(device.firmware_version.as_deref(), &image.version),
    ))
}

/// GET /provision/:key/firmware/:image/:file — an image, to a phone of the
/// tenant whose key it is, while the tenant targets it.
pub(crate) async fn serve_image(
    state: &ApiState,
    tenant_id: TenantId,
    image: &str,
    file: &str,
) -> Option<Response> {
    let storage = state.storage.as_deref()?;
    let store = storage.provisioning_store()?;
    let id = uuid::Uuid::parse_str(image).ok()?;
    let targeted = store
        .list_firmware_targets(&tenant_id)
        .await
        .ok()?
        .iter()
        .any(|t| t.image_id == id);
    if !targeted {
        return None;
    }
    let image = store.get_firmware_image(&id).await.ok()??;
    if image.file_name != file {
        return None;
    }
    let path = image_path(&directory(state).ok()?, &image);
    let f = tokio::fs::File::open(&path).await.ok()?;
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(f));
    Some(
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (header::CONTENT_LENGTH, image.size.to_string()),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            body,
        )
            .into_response(),
    )
}
