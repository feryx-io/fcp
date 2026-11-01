//! API token management (docs/DEVELOPER_SURFACE.md §3, phase 1).
//!
//! An admin token is made by a user holding `api_token_manage`, with
//! permissions each of which its maker holds (`UserRole::token_permissions`:
//! the maker's own, less anything a tenant admin lacks); `admin` asks for
//! all of them. It names its maker, who stays answerable for it: a disabled
//! or deleted maker's tokens are refused, and a demoted maker's lose what
//! the maker lost. Revoking keeps the row; rotating gives a new secret and
//! honours the old one for a grace period.
//!
//! A personal token is made by any user for themself at `/me/api-tokens`
//! and reaches only that user's own `/me` routes.

use crate::{
    auth::{AuthIdentity, TenantContext},
    error::ApiError,
    state::ApiState,
    v1::auth_session::Me,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::{DeploymentMode, Permission, UserRole};
use fcp_core::{ApiToken, ApiTokenScope, ApiTokenStore};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a rotated-out secret still opens its token, unless asked.
const DEFAULT_ROTATION_GRACE: Duration = Duration::from_secs(24 * 3600);
/// The longest grace a rotation may give the old secret.
const MAX_ROTATION_GRACE: Duration = Duration::from_secs(7 * 24 * 3600);

fn rfc3339(ts: SystemTime) -> String {
    let d = ts.duration_since(UNIX_EPOCH).unwrap_or_default();
    chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// Response model for an API token (never includes a hash).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ApiTokenResponse {
    pub id: String,
    pub name: String,
    pub prefix: String,
    /// `admin` (the admin API under its permissions) or `personal` (its
    /// maker's own `/me` routes).
    pub scope: String,
    /// The maker's username; empty for a token made with the static token or
    /// before makers were recorded.
    pub created_by: String,
    pub tenant_id: String,
    pub permissions: Vec<String>,
    /// A token made before permissions were checked, carrying `admin` or
    /// `*`. Since v0.25.498 it reaches what a tenant administrator may (or
    /// its maker, when it has one), never system configuration: make a new
    /// one with the permissions it needs.
    pub legacy_full_access: bool,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    pub revoked_by: Option<String>,
    pub rotated_at: Option<String>,
    /// Until when the secret a rotation replaced still opens the token.
    pub previous_secret_valid_until: Option<String>,
}

impl From<ApiToken> for ApiTokenResponse {
    fn from(t: ApiToken) -> Self {
        let legacy_full_access = t.permissions.iter().any(|p| p == "admin" || p == "*");
        let now = SystemTime::now();
        Self {
            id: t.id.to_string(),
            name: t.name,
            prefix: t.prefix,
            scope: t.scope.as_str().to_string(),
            created_by: t.user_id,
            tenant_id: t.tenant_id.to_string(),
            permissions: t.permissions,
            legacy_full_access,
            expires_at: t.expires_at.map(rfc3339),
            created_at: rfc3339(t.created_at),
            last_used_at: t.last_used_at.map(rfc3339),
            revoked_at: t.revoked_at.map(rfc3339),
            revoked_by: t.revoked_by,
            rotated_at: t.rotated_at.map(rfc3339),
            previous_secret_valid_until: t
                .previous_expires_at
                .filter(|until| *until > now)
                .map(rfc3339),
        }
    }
}

/// Request body for creating a new admin API token.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateApiTokenRequest {
    pub name: String,
    /// Permission names (`Permission::as_str`), or `admin` for everything
    /// the maker may grant.
    #[serde(default)]
    pub permissions: Vec<String>,
    pub expires_at: Option<String>,
}

/// Request body for creating a personal API token.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreatePersonalTokenRequest {
    pub name: String,
    pub expires_at: Option<String>,
}

/// Request body for rotating a token's secret.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct RotateApiTokenRequest {
    /// How long the old secret keeps working, in seconds (default a day,
    /// at most a week; 0 retires it at once).
    pub grace_seconds: Option<u64>,
}

/// Response body after creating a token (includes the one-time secret).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CreateApiTokenResponse {
    pub token: ApiTokenResponse,
    pub secret: String,
}

/// A permission the caller may put on a token.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GrantablePermission {
    pub name: &'static str,
    pub label: &'static str,
    pub group: &'static str,
}

fn token_store(state: &ApiState) -> Result<Arc<dyn ApiTokenStore>, ApiError> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?
        .api_token_store()
        .cloned()
        .ok_or_else(|| ApiError::Internal("API token store not configured".to_string()))
}

/// What `identity` may put on a token it makes or rotates. A session gives
/// what its role's tokens may carry; the static token (the platform) what a
/// tenant admin's may; an API token what it carries itself.
fn grantable(identity: &AuthIdentity, mode: DeploymentMode) -> Vec<Permission> {
    match identity {
        AuthIdentity::StaticToken => UserRole::TenantAdmin.token_permissions(mode),
        AuthIdentity::Session { .. } => fcp_core::permissions::roles_token_permissions(
            &identity.session_roles().unwrap_or_default(),
            mode,
        ),
        AuthIdentity::ApiToken { permissions, .. } => {
            let full = permissions.iter().any(|p| p == "admin" || p == "*");
            UserRole::TenantAdmin
                .token_permissions(mode)
                .into_iter()
                .filter(|p| full || permissions.iter().any(|q| q == p.as_str()))
                .collect()
        }
        AuthIdentity::Personal { .. } => Vec::new(),
    }
}

/// The permissions a request asks for, checked against what the caller may
/// grant: `admin` (or `*`) is everything it may; an unknown name is a bad
/// request; a known one it may not grant is forbidden.
fn resolve_permissions(
    requested: &[String],
    grantable: &[Permission],
) -> Result<Vec<String>, ApiError> {
    if requested.is_empty() {
        return Err(ApiError::BadRequest(
            "A token needs at least one permission".to_string(),
        ));
    }
    if requested.iter().any(|p| p == "admin" || p == "*") {
        if grantable.is_empty() {
            return Err(ApiError::Forbidden(
                "You may not grant any permission".to_string(),
            ));
        }
        return Ok(grantable.iter().map(|p| p.as_str().to_string()).collect());
    }
    let mut out: Vec<String> = Vec::new();
    let mut refused = Vec::new();
    for name in requested {
        let permission: Permission = name
            .parse()
            .map_err(|_| ApiError::BadRequest(format!("Unknown permission '{name}'")))?;
        if !grantable.contains(&permission) {
            refused.push(name.clone());
        } else if !out.contains(name) {
            out.push(name.clone());
        }
    }
    if !refused.is_empty() {
        return Err(ApiError::Forbidden(format!(
            "You may not grant: {}",
            refused.join(", ")
        )));
    }
    Ok(out)
}

fn parse_expiry(raw: Option<&str>) -> Result<Option<SystemTime>, ApiError> {
    // Accept both RFC3339 ("2026-03-15T00:00:00Z") and a plain date
    // ("2026-03-15") from HTML date inputs.
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let secs = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        dt.timestamp()
    } else if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        date.and_hms_opt(23, 59, 59)
            .expect("valid time")
            .and_utc()
            .timestamp()
    } else {
        return Err(ApiError::BadRequest(format!(
            "Invalid expires_at: expected YYYY-MM-DD or RFC3339, got '{raw}'"
        )));
    };
    Ok(Some(UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)))
}

fn check_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::BadRequest(
            "Name must be between 1 and 64 characters".to_string(),
        ));
    }
    Ok(())
}

/// A fresh secret: `fcp_` and 32 random bytes, base64url.
fn new_secret() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!(
        "fcp_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

/// The first characters of a secret, shown to tell tokens apart.
fn display_prefix(secret: &str) -> String {
    secret.chars().take(8).collect()
}

fn identity_of(identity: Option<Extension<AuthIdentity>>) -> AuthIdentity {
    // Without the bearer layer (authentication off) every caller is the
    // platform, as `lib.rs` inserts.
    identity
        .map(|Extension(id)| id)
        .unwrap_or(AuthIdentity::StaticToken)
}

/// Show each token's maker by username: a token records its maker by id
/// (`docs/IDENTITY_AND_DEVICES.md` phase 1), or, made before, by username.
async fn named(state: &ApiState, mut tokens: Vec<ApiTokenResponse>) -> Vec<ApiTokenResponse> {
    let Some(storage) = state.storage.as_ref() else {
        return tokens;
    };
    let mut names: std::collections::HashMap<String, Option<String>> = Default::default();
    for token in &mut tokens {
        let Ok(id) = token.created_by.parse::<fcp_core::UserId>() else {
            continue;
        };
        if !names.contains_key(&token.created_by) {
            let name = fcp_core::CredentialStore::get_user_by_id(storage.as_ref(), &id)
                .await
                .ok()
                .flatten()
                .map(|u| u.username);
            names.insert(token.created_by.clone(), name);
        }
        if let Some(Some(name)) = names.get(&token.created_by) {
            token.created_by = name.clone();
        }
    }
    tokens
}

/// A made or rotated token's answer, its maker by username.
async fn named_one(
    state: &ApiState,
    mut response: CreateApiTokenResponse,
) -> CreateApiTokenResponse {
    let token = response.token;
    response.token = named(state, vec![token]).await.remove(0);
    response
}

/// Whether `token` is the caller's own personal token: made by their id,
/// or, before makers were ids, by their username.
fn owned_by(token: &ApiToken, me: &Me) -> bool {
    token.scope == ApiTokenScope::Personal
        && (me.0.user_id.as_deref() == Some(token.user_id.as_str()) || token.user_id == me.0.sub)
}

async fn find_in_tenant(
    store: &dyn ApiTokenStore,
    id: &str,
    tenant: &fcp_core::TenantId,
) -> Result<ApiToken, ApiError> {
    let token_id: fcp_core::ApiTokenId = id
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid token ID".to_string()))?;
    let token = store
        .get_api_token(&token_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to look up API token: {e}")))?
        .ok_or_else(|| ApiError::NotFound("API token not found".to_string()))?;
    if token.tenant_id != *tenant {
        return Err(ApiError::NotFound("API token not found".to_string()));
    }
    Ok(token)
}

async fn store_new_token(
    store: &dyn ApiTokenStore,
    name: String,
    maker: String,
    tenant_id: fcp_core::TenantId,
    scope: ApiTokenScope,
    permissions: Vec<String>,
    expires_at: Option<SystemTime>,
) -> Result<CreateApiTokenResponse, ApiError> {
    let secret = new_secret();
    let token = ApiToken {
        id: fcp_core::ApiTokenId::new(),
        name,
        token_hash: ApiToken::hash_secret(&secret),
        prefix: display_prefix(&secret),
        user_id: maker,
        tenant_id,
        permissions,
        expires_at,
        created_at: SystemTime::now(),
        last_used_at: None,
        scope,
        revoked_at: None,
        revoked_by: None,
        previous_token_hash: None,
        previous_expires_at: None,
        rotated_at: None,
    };
    store
        .create_api_token(token.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create API token: {e}")))?;
    Ok(CreateApiTokenResponse {
        token: token.into(),
        secret,
    })
}

async fn rotate(
    store: &dyn ApiTokenStore,
    token: ApiToken,
    req: RotateApiTokenRequest,
) -> Result<CreateApiTokenResponse, ApiError> {
    if token.revoked_at.is_some() {
        return Err(ApiError::Conflict(
            "A revoked token cannot be rotated".to_string(),
        ));
    }
    let grace = req
        .grace_seconds
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_ROTATION_GRACE);
    if grace > MAX_ROTATION_GRACE {
        return Err(ApiError::BadRequest(format!(
            "grace_seconds may be at most {}",
            MAX_ROTATION_GRACE.as_secs()
        )));
    }
    let secret = new_secret();
    store
        .rotate_api_token(
            &token.id,
            &ApiToken::hash_secret(&secret),
            &display_prefix(&secret),
            SystemTime::now() + grace,
        )
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to rotate API token: {e}")))?;
    let rotated = store
        .get_api_token(&token.id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to look up API token: {e}")))?
        .ok_or_else(|| ApiError::NotFound("API token not found".to_string()))?;
    Ok(CreateApiTokenResponse {
        token: rotated.into(),
        secret,
    })
}

// ---------------------------------------------------------------------------
// Admin tokens: /api/v1/api-tokens (under `api_token_manage`)
// ---------------------------------------------------------------------------

/// List the tenant's tokens, admin and personal, revoked ones included.
///
/// GET /api/v1/api-tokens
#[utoipa::path(
    get,
    path = "/api/v1/api-tokens",
    tag = "api_tokens",
    operation_id = "api_tokens_list_api_tokens",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<ApiTokenResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_api_tokens(
    State(state): State<ApiState>,
    tenant: TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let tokens = token_store(&state)?
        .list_api_tokens(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list API tokens: {e}")))?;
    let response = named(&state, tokens.into_iter().map(Into::into).collect()).await;
    Ok(Json(crate::page::paginate(
        response,
        &page,
        crate::page::Order::Descending,
        |r| crate::page::rfc3339_key(&r.created_at, &r.id),
    )?))
}

/// The permissions the caller may put on a token, grouped by area.
///
/// GET /api/v1/api-tokens/permissions
#[utoipa::path(
    get,
    path = "/api/v1/api-tokens/permissions",
    tag = "api_tokens",
    operation_id = "api_tokens_grantable_permissions",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<GrantablePermission>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn grantable_permissions(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = identity_of(identity);
    let response: Vec<GrantablePermission> = grantable(&identity, state.deployment_mode)
        .into_iter()
        .map(|p| GrantablePermission {
            name: p.as_str(),
            label: p.name(),
            group: p.group(),
        })
        .collect();
    Ok(Json(crate::page::paginate(
        response,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.group, r.name),
    )?))
}

/// Create an admin token. Only a user (or the static token) makes one; a
/// token cannot make another.
///
/// POST /api/v1/api-tokens
#[utoipa::path(
    post,
    path = "/api/v1/api-tokens",
    tag = "api_tokens",
    operation_id = "api_tokens_create_api_token",
    request_body = CreateApiTokenRequest,
    responses(
        (status = 201, description = "Made; the secret is shown once", body = CreateApiTokenResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_api_token(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Json(req): Json<CreateApiTokenRequest>,
) -> Result<impl IntoResponse, ApiError> {
    check_name(&req.name)?;
    let identity = identity_of(identity);
    let maker = match &identity {
        AuthIdentity::StaticToken => String::new(),
        // The maker by id, so a user later made with the same name is not.
        AuthIdentity::Session {
            user_id, username, ..
        } => user_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| username.clone()),
        AuthIdentity::ApiToken { .. } | AuthIdentity::Personal { .. } => {
            return Err(ApiError::Forbidden(
                "An API token cannot make API tokens; sign in to make one".to_string(),
            ))
        }
    };
    let permissions = resolve_permissions(
        &req.permissions,
        &grantable(&identity, state.deployment_mode),
    )?;
    let expires_at = parse_expiry(req.expires_at.as_deref())?;
    let store = token_store(&state)?;
    let response = store_new_token(
        store.as_ref(),
        req.name,
        maker,
        tenant.0,
        ApiTokenScope::Admin,
        permissions,
        expires_at,
    )
    .await?;
    let response = named_one(&state, response).await;
    Ok((StatusCode::CREATED, Json(response)))
}

/// Give a token a new secret; the old one works for the grace period. The
/// caller must be able to grant everything the token carries, and a
/// personal token is rotated only by its maker, at `/me/api-tokens`.
///
/// POST /api/v1/api-tokens/:id/rotate
#[utoipa::path(
    post,
    path = "/api/v1/api-tokens/{id}/rotate",
    tag = "api_tokens",
    operation_id = "api_tokens_rotate_api_token",
    params(("id" = String, Path)),
    request_body = RotateApiTokenRequest,
    responses(
        (status = 200, description = "Rotated; the new secret is shown once", body = CreateApiTokenResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn rotate_api_token(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(id): Path<String>,
    body: Option<Json<RotateApiTokenRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = identity_of(identity);
    let store = token_store(&state)?;
    let token = find_in_tenant(store.as_ref(), &id, &tenant.0).await?;
    if token.scope == ApiTokenScope::Personal {
        return Err(ApiError::Forbidden(
            "A personal token is rotated only by its maker".to_string(),
        ));
    }
    let may_grant = grantable(&identity, state.deployment_mode);
    let exceeds = if token.permissions.iter().any(|p| p == "admin" || p == "*") {
        UserRole::TenantAdmin
            .token_permissions(state.deployment_mode)
            .iter()
            .any(|p| !may_grant.contains(p))
    } else {
        token.permissions.iter().any(|name| {
            name.parse::<Permission>()
                .map(|p| !may_grant.contains(&p))
                .unwrap_or(false)
        })
    };
    if exceeds {
        return Err(ApiError::Forbidden(
            "This token carries permissions you may not grant".to_string(),
        ));
    }
    let response = rotate(store.as_ref(), token, body.map(|b| b.0).unwrap_or_default()).await?;
    let response = named_one(&state, response).await;
    Ok(Json(response))
}

/// Revoke a token: kept, marked, and refused from now on.
///
/// DELETE /api/v1/api-tokens/:id
#[utoipa::path(
    delete,
    path = "/api/v1/api-tokens/{id}",
    tag = "api_tokens",
    operation_id = "api_tokens_delete_api_token",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_api_token(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = identity_of(identity);
    let store = token_store(&state)?;
    let token = find_in_tenant(store.as_ref(), &id, &tenant.0).await?;
    store
        .revoke_api_token(&token.id, &identity.actor())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to revoke API token: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Personal tokens: /api/v1/me/api-tokens
// ---------------------------------------------------------------------------

/// A personal token is made, rotated and listed from a signed-in session,
/// not from another token.
fn require_session(identity: &AuthIdentity) -> Result<(), ApiError> {
    match identity {
        AuthIdentity::Personal { .. } | AuthIdentity::ApiToken { .. } => Err(ApiError::Forbidden(
            "Sign in to manage your API tokens".to_string(),
        )),
        _ => Ok(()),
    }
}

async fn my_token(store: &dyn ApiTokenStore, id: &str, me: &Me) -> Result<ApiToken, ApiError> {
    let tenant: fcp_core::TenantId =
        me.0.tenant_id
            .parse()
            .map_err(|_| ApiError::Unauthorized("Invalid session tenant".to_string()))?;
    let token = find_in_tenant(store, id, &tenant).await?;
    if !owned_by(&token, me) {
        return Err(ApiError::NotFound("API token not found".to_string()));
    }
    Ok(token)
}

/// The caller's own personal tokens.
///
/// GET /api/v1/me/api-tokens
#[utoipa::path(
    get,
    path = "/api/v1/me/api-tokens",
    tag = "api_tokens",
    operation_id = "api_tokens_list_my_api_tokens",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<ApiTokenResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_my_api_tokens(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    me: Me,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    require_session(&identity_of(identity))?;
    let tenant: fcp_core::TenantId =
        me.0.tenant_id
            .parse()
            .map_err(|_| ApiError::Unauthorized("Invalid session tenant".to_string()))?;
    let tokens = token_store(&state)?
        .list_api_tokens(&tenant)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list API tokens: {e}")))?;
    let response: Vec<ApiTokenResponse> = tokens
        .into_iter()
        .filter(|t| owned_by(t, &me))
        .map(Into::into)
        .collect();
    let response = named(&state, response).await;
    Ok(Json(crate::page::paginate(
        response,
        &page,
        crate::page::Order::Descending,
        |r| crate::page::rfc3339_key(&r.created_at, &r.id),
    )?))
}

/// Make a personal token: the caller's own `/me` routes, nothing else.
///
/// POST /api/v1/me/api-tokens
#[utoipa::path(
    post,
    path = "/api/v1/me/api-tokens",
    tag = "api_tokens",
    operation_id = "api_tokens_create_my_api_token",
    request_body = CreatePersonalTokenRequest,
    responses(
        (status = 201, description = "Made; the secret is shown once", body = CreateApiTokenResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_my_api_token(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    me: Me,
    Json(req): Json<CreatePersonalTokenRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_session(&identity_of(identity))?;
    check_name(&req.name)?;
    let tenant: fcp_core::TenantId =
        me.0.tenant_id
            .parse()
            .map_err(|_| ApiError::Unauthorized("Invalid session tenant".to_string()))?;
    let expires_at = parse_expiry(req.expires_at.as_deref())?;
    let store = token_store(&state)?;
    let response = store_new_token(
        store.as_ref(),
        req.name,
        me.0.user_id.clone().unwrap_or_else(|| me.0.sub.clone()),
        tenant,
        ApiTokenScope::Personal,
        Vec::new(),
        expires_at,
    )
    .await?;
    let response = named_one(&state, response).await;
    Ok((StatusCode::CREATED, Json(response)))
}

/// Rotate one of the caller's personal tokens.
///
/// POST /api/v1/me/api-tokens/:id/rotate
#[utoipa::path(
    post,
    path = "/api/v1/me/api-tokens/{id}/rotate",
    tag = "api_tokens",
    operation_id = "api_tokens_rotate_my_api_token",
    params(("id" = String, Path)),
    request_body = RotateApiTokenRequest,
    responses(
        (status = 200, description = "Rotated; the new secret is shown once", body = CreateApiTokenResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn rotate_my_api_token(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    me: Me,
    Path(id): Path<String>,
    body: Option<Json<RotateApiTokenRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    require_session(&identity_of(identity))?;
    let store = token_store(&state)?;
    let token = my_token(store.as_ref(), &id, &me).await?;
    let response = rotate(store.as_ref(), token, body.map(|b| b.0).unwrap_or_default()).await?;
    let response = named_one(&state, response).await;
    Ok(Json(response))
}

/// Revoke one of the caller's personal tokens. A personal token may revoke
/// itself or its siblings, so a leaked one can be killed with itself.
///
/// DELETE /api/v1/me/api-tokens/:id
#[utoipa::path(
    delete,
    path = "/api/v1/me/api-tokens/{id}",
    tag = "api_tokens",
    operation_id = "api_tokens_revoke_my_api_token",
    params(("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn revoke_my_api_token(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    me: Me,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = identity_of(identity);
    let store = token_store(&state)?;
    let token = my_token(store.as_ref(), &id, &me).await?;
    store
        .revoke_api_token(&token.id, &identity.actor())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to revoke API token: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ps: &[&str]) -> Vec<String> {
        ps.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn read_only_cannot_grant_what_it_does_not_hold() {
        let mode = DeploymentMode::ServiceProvider;
        let grantable = UserRole::ReadOnly.token_permissions(mode);
        let err = resolve_permissions(&names(&["user_create"]), &grantable).unwrap_err();
        assert!(matches!(err, ApiError::Forbidden(_)), "{err:?}");
        // `admin` means everything the maker may grant, which for a
        // read-only user is reading.
        let all = resolve_permissions(&names(&["admin"]), &grantable).unwrap();
        assert!(all.contains(&"user_read".to_string()));
        assert!(!all
            .iter()
            .any(|p| p.ends_with("_create") || p.ends_with("_manage")));
        assert!(!all.contains(&"api_token_manage".to_string()));
    }

    #[test]
    fn unknown_permissions_and_empty_lists_are_bad_requests() {
        let grantable = UserRole::TenantAdmin.token_permissions(DeploymentMode::Enterprise);
        assert!(matches!(
            resolve_permissions(&names(&["read"]), &grantable),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            resolve_permissions(&[], &grantable),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[test]
    fn a_tenant_admin_grants_within_the_tenant_and_no_further() {
        let mode = DeploymentMode::ServiceProvider;
        let grantable = UserRole::TenantAdmin.token_permissions(mode);
        let ok = resolve_permissions(&names(&["cdr_read", "cdr_read", "flow_manage"]), &grantable)
            .unwrap();
        assert_eq!(ok, names(&["cdr_read", "flow_manage"]));
        assert!(matches!(
            resolve_permissions(&names(&["tenant_create"]), &grantable),
            Err(ApiError::Forbidden(_))
        ));
        // A super admin's tokens are a tenant admin's.
        assert_eq!(grantable, UserRole::SuperAdmin.token_permissions(mode),);
    }

    #[test]
    fn a_token_grants_only_what_it_carries() {
        let mode = DeploymentMode::Enterprise;
        let identity = AuthIdentity::ApiToken {
            token_id: "t".into(),
            tenant_id: fcp_core::TenantId::new(),
            permissions: names(&["cdr_read", "api_token_manage"]),
        };
        let grantable = grantable(&identity, mode);
        assert_eq!(grantable.len(), 2);
        assert!(grantable.contains(&Permission::CdrRead));
    }
}
