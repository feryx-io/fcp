//! Sites: a tenant's own stacks on its own network, enrolled to this cloud
//! (`docs/HYBRID_CONTROL_PLANE.md` phase 1). A site is made here and
//! given a one-time enrollment token, shown once in the response that
//! made it; `fcp-link` at the site spends it against the admin's link
//! listener and holds a certificate from then on. What the link reports
//! is read here; what the link does is `fcp-admin`'s `site_link`.

use crate::auth::{check_permission, AuthIdentity, TenantContext};
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use base64::Engine;
use fcp_core::permissions::Permission;
use fcp_core::site::ENROLLMENT_TOKEN_TTL;
use fcp_core::{EnrollmentToken, Site, SiteId, SiteStatus, SiteStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::SystemTime;

#[derive(Debug, Deserialize)]
pub struct CreateSiteRequest {
    pub name: String,
}

/// A site as the API shows it.
#[derive(Debug, Serialize)]
pub struct SiteResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub status: SiteStatus,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrolled_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_expires_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<u64>,
    pub link_up: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_version: Option<String>,
    /// Release → node ids, from the last heartbeat.
    pub versions: std::collections::BTreeMap<String, Vec<String>>,
    /// The site's nodes as the link last found them.
    pub nodes: Vec<fcp_core::SiteNode>,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl From<Site> for SiteResponse {
    fn from(site: Site) -> Self {
        let versions = site.versions();
        Self {
            id: site.id.to_string(),
            tenant_id: site.tenant_id.to_string(),
            name: site.name,
            status: site.status,
            created_at: secs(site.created_at),
            updated_at: secs(site.updated_at),
            enrolled_at: site.enrolled_at.map(secs),
            certificate_expires_at: site.certificate_expires_at.map(secs),
            last_seen_at: site.last_seen_at.map(secs),
            link_up: site.link_up,
            link_version: site
                .last_heartbeat
                .as_ref()
                .map(|beat| beat.link_version.clone()),
            versions,
            nodes: site
                .last_heartbeat
                .map(|beat| beat.nodes)
                .unwrap_or_default(),
        }
    }
}

/// A site just made, or a new token for one: the token, shown this once.
#[derive(Debug, Serialize)]
pub struct SiteWithTokenResponse {
    #[serde(flatten)]
    pub site: SiteResponse,
    /// The one-time enrollment token; `fcp-link` spends it. Not stored,
    /// not shown again.
    pub enrollment_token: String,
    pub enrollment_token_expires_at: u64,
}

/// The token's hash, as the store keeps it and the link's enrollment
/// presents it.
pub fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A fresh token and its stored form.
fn mint_token(site_id: SiteId) -> (String, EnrollmentToken) {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let now = SystemTime::now();
    let stored = EnrollmentToken {
        site_id,
        token_hash: token_hash(&token),
        created_at: now,
        expires_at: now + ENROLLMENT_TOKEN_TTL,
        used_at: None,
    };
    (token, stored)
}

macro_rules! storage {
    ($state:expr) => {
        $state
            .storage
            .as_ref()
            .ok_or_else(|| ApiError::ServiceUnavailable("storage is not configured".to_string()))?
    };
}

async fn site_in_tenant(
    storage: &fcp_core::DynStorage,
    tenant: &TenantContext,
    id: &str,
) -> Result<Site, ApiError> {
    let id: SiteId = id
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("{id} is not a site id")))?;
    match storage.get_site(&id).await? {
        Some(site) if site.tenant_id == tenant.0 => Ok(site),
        _ => Err(ApiError::NotFound(format!("site {id} not found"))),
    }
}

/// GET /api/v1/sites
pub async fn list_sites(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
) -> Result<Json<Vec<SiteResponse>>, ApiError> {
    check_permission(&identity, Permission::SiteRead, state.deployment_mode)?;
    let storage = storage!(state);
    let sites = storage.list_sites(&tenant.0).await?;
    Ok(Json(sites.into_iter().map(SiteResponse::from).collect()))
}

/// POST /api/v1/sites — a site, and the token that enrolls it.
pub async fn create_site(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Json(req): Json<CreateSiteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    check_permission(&identity, Permission::SiteManage, state.deployment_mode)?;
    let name = req.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(ApiError::BadRequest(
            "name must be 1 to 100 characters".to_string(),
        ));
    }
    let storage = storage!(state);
    let site = Site::new(tenant.0, name.to_string());
    storage.put_site(site.clone()).await?;
    let (token, stored) = mint_token(site.id);
    storage.put_enrollment_token(stored.clone()).await?;
    Ok((
        StatusCode::CREATED,
        Json(SiteWithTokenResponse {
            site: site.into(),
            enrollment_token: token,
            enrollment_token_expires_at: secs(stored.expires_at),
        }),
    ))
}

/// GET /api/v1/sites/:id
pub async fn get_site(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<SiteResponse>, ApiError> {
    check_permission(&identity, Permission::SiteRead, state.deployment_mode)?;
    let storage = storage!(state);
    Ok(Json(site_in_tenant(storage, &tenant, &id).await?.into()))
}

/// POST /api/v1/sites/:id/enrollment-token — a new token for a site whose
/// link must enroll again (a new machine, a lost state directory). A
/// disabled site gets none.
pub async fn new_enrollment_token(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<SiteWithTokenResponse>, ApiError> {
    check_permission(&identity, Permission::SiteManage, state.deployment_mode)?;
    let storage = storage!(state);
    let site = site_in_tenant(storage, &tenant, &id).await?;
    if site.status == SiteStatus::Disabled {
        return Err(ApiError::Conflict(
            "a disabled site enrolls nothing; enable it first".to_string(),
        ));
    }
    let (token, stored) = mint_token(site.id);
    storage.put_enrollment_token(stored.clone()).await?;
    Ok(Json(SiteWithTokenResponse {
        site: site.into(),
        enrollment_token: token,
        enrollment_token_expires_at: secs(stored.expires_at),
    }))
}

async fn set_status(
    state: &ApiState,
    identity: &AuthIdentity,
    tenant: &TenantContext,
    id: &str,
    status: SiteStatus,
) -> Result<Json<SiteResponse>, ApiError> {
    check_permission(identity, Permission::SiteManage, state.deployment_mode)?;
    let storage = storage!(state);
    let mut site = site_in_tenant(storage, tenant, id).await?;
    if status == SiteStatus::Enrolled && site.certificate_serial.is_none() {
        // Never enrolled: enabling it makes it pending again.
        site.status = SiteStatus::Pending;
    } else {
        site.status = status;
    }
    if status == SiteStatus::Disabled {
        site.link_up = false;
        site.link_changed_at = Some(SystemTime::now());
    }
    site.updated_at = SystemTime::now();
    storage.put_site(site.clone()).await?;
    Ok(Json(site.into()))
}

/// POST /api/v1/sites/:id/disable — its certificate and secret are
/// refused from now on; the site's own stack keeps running.
pub async fn disable_site(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<SiteResponse>, ApiError> {
    set_status(&state, &identity, &tenant, &id, SiteStatus::Disabled).await
}

/// POST /api/v1/sites/:id/enable
pub async fn enable_site(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<SiteResponse>, ApiError> {
    set_status(&state, &identity, &tenant, &id, SiteStatus::Enrolled).await
}

/// DELETE /api/v1/sites/:id
pub async fn delete_site(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    check_permission(&identity, Permission::SiteManage, state.deployment_mode)?;
    let storage = storage!(state);
    let site = site_in_tenant(storage, &tenant, &id).await?;
    storage.delete_site(&site.id).await?;
    Ok(StatusCode::NO_CONTENT)
}
