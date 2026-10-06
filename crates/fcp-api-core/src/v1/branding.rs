//! A tenant's brand (`docs/BRANDING.md`, `docs/SOFTPHONE.md` §2.5): made
//! at `/api/v1/branding` under `branding_manage`, served to the tenant's
//! softphones at `/brand/v1` (Anvil's contract, v1) with an ETag, and its
//! assets public by hash at `/brand/v1/assets/{sha256}`.

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use fcp_core::branding::{
    self, AssetKind, BrandAsset, BrandColors, BrandDialPlan, BrandLinks, BrandRingtone, BrandStore,
    TenantBrand,
};
use fcp_core::permissions::Permission;
use fcp_core::TenantId;
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn store(state: &ApiState) -> Result<&Arc<dyn BrandStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.brand_store())
        .ok_or_else(|| ApiError::NotFound("Branding needs the database".to_string()))
}

fn epoch(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An uploaded file.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BrandAssetResponse {
    /// Hex SHA-256: what the brand names it by.
    pub sha256: String,
    /// What the file is, from its bytes.
    pub mime: String,
    /// `image` or `audio`.
    pub kind: &'static str,
    pub size: u64,
    /// A PNG's dimensions.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub file_name: Option<String>,
    /// Where apps fetch it, without credentials.
    pub url: Option<String>,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub uploaded_at: u64,
}

impl BrandAssetResponse {
    fn of(asset: BrandAsset, base: Option<&str>) -> Self {
        Self {
            url: base.map(|b| asset_url(b, &asset.sha256)),
            kind: match asset.kind() {
                Some(AssetKind::Audio) => "audio",
                _ => "image",
            },
            uploaded_at: epoch(asset.uploaded_at),
            sha256: asset.sha256,
            mime: asset.mime,
            size: asset.size,
            width: asset.width,
            height: asset.height,
            file_name: asset.file_name,
        }
    }
}

fn asset_url(base: &str, sha256: &str) -> String {
    format!("{base}/brand/v1/assets/{sha256}")
}

/// The tenant's brand, as its administrator writes it: assets by hash.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrandRequest {
    /// The app's name, as the tenant's people see it.
    pub app_name: String,
    #[serde(default)]
    pub colors: BrandColors,
    /// The dark theme's; unset, the app derives its own.
    #[serde(default)]
    pub dark_colors: Option<BrandColors>,
    /// An uploaded image's hash.
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub logo_dark: Option<String>,
    /// An uploaded PNG, JPEG or WebP image's hash.
    #[serde(default)]
    pub icon: Option<String>,
    /// Uploaded audio, by hash; at most one the default.
    #[serde(default)]
    pub ringtones: Vec<BrandRingtone>,
    #[serde(default)]
    pub links: BrandLinks,
    #[serde(default)]
    pub dial_plan: BrandDialPlan,
}

/// The tenant's brand, with what its assets are.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BrandResponse {
    pub tenant_id: String,
    pub app_name: String,
    pub colors: BrandColors,
    pub dark_colors: Option<BrandColors>,
    pub logo: Option<String>,
    pub logo_dark: Option<String>,
    pub icon: Option<String>,
    pub ringtones: Vec<BrandRingtone>,
    pub links: BrandLinks,
    pub dial_plan: BrandDialPlan,
    /// The assets it names.
    pub assets: Vec<BrandAssetResponse>,
    /// Where the tenant's softphones fetch it.
    pub brand_url: Option<String>,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: u64,
}

async fn respond(
    store: &Arc<dyn BrandStore>,
    brand: TenantBrand,
    base: Option<&str>,
) -> Result<BrandResponse, ApiError> {
    let mut assets = Vec::new();
    for hash in brand.asset_hashes() {
        if let Some(asset) = store.get_brand_asset(&brand.tenant_id, hash).await? {
            assets.push(BrandAssetResponse::of(asset, base));
        }
    }
    Ok(BrandResponse {
        tenant_id: brand.tenant_id.to_string(),
        brand_url: base.map(|b| format!("{b}/brand/v1")),
        updated_at: epoch(brand.updated_at),
        app_name: brand.app_name,
        colors: brand.colors,
        dark_colors: brand.dark_colors,
        logo: brand.logo,
        logo_dark: brand.logo_dark,
        icon: brand.icon,
        ringtones: brand.ringtones,
        links: brand.links,
        dial_plan: brand.dial_plan,
        assets,
    })
}

fn check_permission(state: &ApiState, identity: &AuthIdentity) -> Result<(), ApiError> {
    crate::auth::check_permission(identity, Permission::BrandingManage, state.deployment_mode)
}

async fn announce(state: &ApiState, tenant: &TenantId) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "branding".to_string(),
            entity_id: tenant.to_string(),
            tenant_id: *tenant,
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// GET /api/v1/branding
#[utoipa::path(
    get,
    path = "/api/v1/branding",
    tag = "branding",
    operation_id = "branding_get_brand",
    responses(
        (status = 200, description = "OK", body = BrandResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_brand(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: HeaderMap,
) -> Result<Json<BrandResponse>, ApiError> {
    check_permission(&state, &identity)?;
    let store = store(&state)?;
    let brand = store
        .get_brand(&tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound("The tenant has no brand".to_string()))?;
    let base = crate::v1::provisioning::public_base(&state, &headers);
    Ok(Json(respond(store, brand, base.as_deref()).await?))
}

/// The brand as asked for, checked against the tenant's assets.
async fn checked(
    store: &Arc<dyn BrandStore>,
    tenant: &TenantId,
    req: BrandRequest,
) -> Result<TenantBrand, ApiError> {
    let bad = |m: String| Err(ApiError::BadRequest(m));
    let app_name = req.app_name.trim().to_string();
    if app_name.is_empty() || app_name.chars().count() > 64 {
        return bad("`app_name` is 1 to 64 characters".to_string());
    }
    for (theme, colors) in [
        ("colors", Some(&req.colors)),
        ("dark_colors", req.dark_colors.as_ref()),
    ] {
        if let Some(slot) = colors.and_then(BrandColors::invalid) {
            return bad(format!("`{theme}.{slot}` is not #RRGGBB or #RRGGBBAA"));
        }
    }
    if let Some(name) = req.links.invalid() {
        return bad(format!("`links.{name}` is not an https:// URL"));
    }
    let dialable = |s: &str| {
        !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_digit() || "+*#".contains(c))
    };
    if req.dial_plan.prefix_strip.len() > 16
        || !req.dial_plan.prefix_strip.iter().all(|p| dialable(p))
    {
        return bad(
            "`dial_plan.prefix_strip` is at most 16 prefixes of digits, +, * and #".to_string(),
        );
    }
    if req.dial_plan.short_codes.len() > 200 {
        return bad("`dial_plan.short_codes` is at most 200 codes".to_string());
    }
    for (code, target) in &req.dial_plan.short_codes {
        let uri = ["sip:", "sips:", "tel:"]
            .iter()
            .any(|s| target.len() > s.len() && target.starts_with(s))
            && target.len() <= 256
            && !target.chars().any(char::is_whitespace);
        if !dialable(code) || !uri {
            return bad(format!(
                "short code `{code}`: a code of digits, * and #, to a sip:, sips: or tel: URI"
            ));
        }
    }
    if req.ringtones.len() > branding::MAX_RINGTONES {
        return bad(format!("at most {} ringtones", branding::MAX_RINGTONES));
    }
    if req.ringtones.iter().filter(|r| r.default).count() > 1 {
        return bad("at most one ringtone is the default".to_string());
    }
    for r in &req.ringtones {
        if r.name.trim().is_empty() || r.name.chars().count() > 64 {
            return bad("a ringtone's name is 1 to 64 characters".to_string());
        }
    }
    // Every asset named is one of the tenant's, of the kind its slot takes.
    let slots = [
        ("logo", req.logo.as_deref(), false),
        ("logo_dark", req.logo_dark.as_deref(), false),
        ("icon", req.icon.as_deref(), true),
    ];
    for (slot, hash, raster) in slots {
        let Some(hash) = hash else { continue };
        let asset = owned_asset(store, tenant, slot, hash).await?;
        if asset.kind() != Some(AssetKind::Image) || (raster && asset.mime == "image/svg+xml") {
            return bad(format!(
                "`{slot}` is {}, not {}",
                asset.mime,
                if raster {
                    "a PNG, JPEG or WebP image"
                } else {
                    "an image"
                }
            ));
        }
    }
    for r in &req.ringtones {
        let asset = owned_asset(store, tenant, "ringtones", &r.sha256).await?;
        if asset.kind() != Some(AssetKind::Audio) {
            return bad(format!(
                "ringtone `{}` is {}, not audio",
                r.name, asset.mime
            ));
        }
    }
    Ok(TenantBrand {
        tenant_id: *tenant,
        app_name,
        colors: req.colors,
        dark_colors: req.dark_colors,
        logo: req.logo,
        logo_dark: req.logo_dark,
        icon: req.icon,
        ringtones: req
            .ringtones
            .into_iter()
            .map(|r| BrandRingtone {
                name: r.name.trim().to_string(),
                ..r
            })
            .collect(),
        links: req.links,
        dial_plan: req.dial_plan,
        updated_at: SystemTime::now(),
    })
}

async fn owned_asset(
    store: &Arc<dyn BrandStore>,
    tenant: &TenantId,
    slot: &str,
    hash: &str,
) -> Result<BrandAsset, ApiError> {
    if !branding::valid_hash(hash) {
        return Err(ApiError::BadRequest(format!(
            "`{slot}`: `{hash}` is not an asset's SHA-256"
        )));
    }
    store
        .get_brand_asset(tenant, hash)
        .await?
        .ok_or_else(|| ApiError::BadRequest(format!("`{slot}`: no asset {hash}; upload it first")))
}

/// PUT /api/v1/branding — set the tenant's brand.
#[utoipa::path(
    put,
    path = "/api/v1/branding",
    tag = "branding",
    operation_id = "branding_put_brand",
    request_body = BrandRequest,
    responses(
        (status = 200, description = "OK", body = BrandResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn put_brand(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: HeaderMap,
    Json(req): Json<BrandRequest>,
) -> Result<Json<BrandResponse>, ApiError> {
    check_permission(&state, &identity)?;
    let store = store(&state)?;
    let brand = checked(store, &tenant.0, req).await?;
    store.put_brand(&brand).await?;
    announce(&state, &tenant.0).await;
    let base = crate::v1::provisioning::public_base(&state, &headers);
    Ok(Json(respond(store, brand, base.as_deref()).await?))
}

/// DELETE /api/v1/branding — no brand: the apps show their own.
#[utoipa::path(
    delete,
    path = "/api/v1/branding",
    tag = "branding",
    operation_id = "branding_delete_brand",
    responses(
        (status = 204, description = "Removed"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_brand(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<StatusCode, ApiError> {
    check_permission(&state, &identity)?;
    if !store(&state)?.delete_brand(&tenant.0).await? {
        return Err(ApiError::NotFound("The tenant has no brand".to_string()));
    }
    announce(&state, &tenant.0).await;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/branding/assets
#[utoipa::path(
    get,
    path = "/api/v1/branding/assets",
    tag = "branding",
    operation_id = "branding_list_assets",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<BrandAssetResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_assets(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: HeaderMap,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    check_permission(&state, &identity)?;
    let base = crate::v1::provisioning::public_base(&state, &headers);
    let assets: Vec<BrandAssetResponse> = store(&state)?
        .list_brand_assets(&tenant.0)
        .await?
        .into_iter()
        .map(|a| BrandAssetResponse::of(a, base.as_deref()))
        .collect();
    Ok(Json(crate::page::paginate(
        assets,
        &page,
        crate::page::Order::Ascending,
        |a| format!("{:020}/{}", a.uploaded_at, a.sha256),
    )?))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UploadQuery {
    /// The file's name, for the admin's page.
    pub name: Option<String>,
}

/// POST /api/v1/branding/assets — upload a file as the body: a PNG, JPEG,
/// WebP or SVG image, or Ogg, MP3 or WAV audio, at most 5 MB. What it is
/// comes from its bytes. The same file again is the same asset.
#[utoipa::path(
    post,
    path = "/api/v1/branding/assets",
    tag = "branding",
    operation_id = "branding_upload_asset",
    params(UploadQuery),
    request_body(content_type = "application/octet-stream", content = String),
    responses(
        (status = 201, description = "Kept", body = BrandAssetResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn upload_asset(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: HeaderMap,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<BrandAssetResponse>), ApiError> {
    check_permission(&state, &identity)?;
    let store = store(&state)?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("The file is empty".to_string()));
    }
    if body.len() > branding::MAX_ASSET_BYTES {
        return Err(ApiError::BadRequest(format!(
            "The file is over {} MB",
            branding::MAX_ASSET_BYTES / (1024 * 1024)
        )));
    }
    let mime = branding::sniff_mime(&body).ok_or_else(|| {
        ApiError::BadRequest(
            "Not a PNG, JPEG, WebP or SVG image, nor Ogg, MP3 or WAV audio".to_string(),
        )
    })?;
    let sha256 = format!("{:x}", Sha256::digest(&body));
    let (width, height) = branding::png_dimensions(&body).unzip();
    let file_name = query
        .name
        .map(|n| n.trim().chars().take(128).collect::<String>())
        .filter(|n| !n.is_empty());
    let asset = BrandAsset {
        tenant_id: tenant.0,
        sha256: sha256.clone(),
        mime: mime.to_string(),
        size: body.len() as u64,
        width,
        height,
        file_name,
        uploaded_at: SystemTime::now(),
    };
    store.put_brand_asset(&asset, &body).await?;
    let kept = store
        .get_brand_asset(&tenant.0, &sha256)
        .await?
        .unwrap_or(asset);
    let base = crate::v1::provisioning::public_base(&state, &headers);
    Ok((
        StatusCode::CREATED,
        Json(BrandAssetResponse::of(kept, base.as_deref())),
    ))
}

/// DELETE /api/v1/branding/assets/{sha256} — one the brand names is
/// refused (409).
#[utoipa::path(
    delete,
    path = "/api/v1/branding/assets/{sha256}",
    tag = "branding",
    operation_id = "branding_delete_asset",
    params(("sha256" = String, Path, description = "The asset's SHA-256")),
    responses(
        (status = 204, description = "Removed"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_asset(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(sha256): Path<String>,
) -> Result<StatusCode, ApiError> {
    check_permission(&state, &identity)?;
    let store = store(&state)?;
    if let Some(brand) = store.get_brand(&tenant.0).await? {
        if brand.asset_hashes().contains(&sha256.as_str()) {
            return Err(ApiError::Conflict(
                "The brand names this asset; change the brand first".to_string(),
            ));
        }
    }
    if !store.delete_brand_asset(&tenant.0, &sha256).await? {
        return Err(ApiError::NotFound(format!("No asset {sha256}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// What `/brand/v1` serves (`docs/BRANDING.md` §5), without its `etag`.
async fn profile(
    store: &Arc<dyn BrandStore>,
    brand: &TenantBrand,
    base: &str,
) -> Result<Value, ApiError> {
    let mut assets = std::collections::HashMap::new();
    for hash in brand.asset_hashes() {
        if let Some(a) = store.get_brand_asset(&brand.tenant_id, hash).await? {
            assets.insert(hash.to_string(), a);
        }
    }
    let asset = |hash: &Option<String>| -> Value {
        match hash.as_deref().and_then(|h| assets.get(h)) {
            None => Value::Null,
            Some(a) => {
                let mut v = json!({
                    "url": asset_url(base, &a.sha256),
                    "mime": a.mime,
                    "sha256": a.sha256,
                });
                if let (Some(w), Some(h)) = (a.width, a.height) {
                    v["width"] = json!(w);
                    v["height"] = json!(h);
                }
                v
            }
        }
    };
    let ringtones: Vec<Value> = brand
        .ringtones
        .iter()
        .filter_map(|r| {
            let a = asset(&Some(r.sha256.clone()));
            (!a.is_null()).then(|| json!({"name": r.name, "default": r.default, "asset": a}))
        })
        .collect();
    let mut body = json!({
        "schema_version": branding::SCHEMA_VERSION,
        "tenant_id": brand.tenant_id.to_string(),
        "app_name": brand.app_name,
        "colors": brand.colors,
        "links": brand.links,
        "dial_plan": brand.dial_plan,
        "ringtones": ringtones,
    });
    if let Some(dark) = &brand.dark_colors {
        body["dark_colors"] = json!(dark);
    }
    for (field, hash) in [
        ("logo", &brand.logo),
        ("logo_dark", &brand.logo_dark),
        ("icon", &brand.icon),
    ] {
        let a = asset(hash);
        if !a.is_null() {
            body[field] = a;
        }
    }
    Ok(body)
}

/// GET /brand/v1 — the signed-in person's tenant's brand
/// (`docs/BRANDING.md` §3), for any of its tokens: an app's session above
/// all. `aor`, `locale` and `theme` are taken and not needed: the token
/// names the tenant, and both themes are always there. 404 when the
/// tenant has no brand; 304 when `If-None-Match` is what it would send.
pub async fn serve_brand(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let store = store(&state)?;
    let brand = store
        .get_brand(&tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound("The tenant has no brand".to_string()))?;
    let base = crate::v1::provisioning::public_base(&state, &headers).ok_or_else(|| {
        ApiError::ServiceUnavailable("No public URL to serve the brand's assets from".to_string())
    })?;
    let mut body = profile(store, &brand, &base).await?;
    let digest = Sha256::digest(serde_json::to_vec(&body).unwrap_or_default());
    let tag = format!("{:x}", digest)[..32].to_string();
    let etag = format!("\"{tag}\"");
    body["etag"] = json!(tag);
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .map(|t| t.trim().trim_start_matches("W/"))
                .any(|t| t == etag || t == "*")
        });
    let etag_header = HeaderValue::from_str(&etag).expect("hex is a header value");
    if matches {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag_header)]).into_response());
    }
    Ok((
        [
            (header::ETAG, etag_header),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-cache"),
            ),
        ],
        Json(body),
    )
        .into_response())
}

/// GET /brand/v1/assets/{sha256} — an asset, public: the hash is its name
/// and it never changes.
pub async fn serve_asset(
    State(state): State<ApiState>,
    Path(sha256): Path<String>,
) -> Result<Response, ApiError> {
    if !branding::valid_hash(&sha256) {
        return Err(ApiError::NotFound("No such asset".to_string()));
    }
    let (asset, bytes) = store(&state)?
        .brand_asset_bytes(&sha256)
        .await?
        .ok_or_else(|| ApiError::NotFound("No such asset".to_string()))?;
    let mime = HeaderValue::from_str(&asset.mime)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));
    let mut headers = vec![
        (header::CONTENT_TYPE, mime),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ),
        (
            header::HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ),
    ];
    // An SVG opened directly runs no script and loads nothing.
    if asset.mime == "image/svg+xml" {
        headers.push((
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
        ));
    }
    let mut response = bytes.into_response();
    for (name, value) in headers {
        response.headers_mut().insert(name, value);
    }
    Ok(response)
}
