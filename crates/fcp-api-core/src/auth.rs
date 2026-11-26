//! API authentication middleware.

use axum::{extract::Request, http::header, middleware::Next, response::Response};
use fcp_core::permissions::{DeploymentMode, Permission, UserRole};
use fcp_core::storage::{ApiTokenStore, TenantStore};
use fcp_core::{CredentialStore, TenantId};
use std::sync::Arc;

/// Header that platform-admin credentials can use to target a specific tenant.
///
/// SECURITY: This override is ignored for tenant-scoped identities. Only
/// platform-admin credentials (static token / super_admin session) may select
/// a tenant this way.
pub const PLATFORM_TENANT_HEADER: &str = "x-fcp-tenant-id";

/// Identity of the authenticated caller, inserted into request extensions
/// after successful authentication by the bearer middleware.
#[derive(Clone, Debug)]
pub enum AuthIdentity {
    /// Static config token — full admin access.
    StaticToken,
    /// Database API token with stored permissions.
    ApiToken {
        /// The token's id, so what it did can be traced to it.
        token_id: String,
        tenant_id: TenantId,
        permissions: Vec<String>,
    },
    /// A personal API token: its maker's own `/me` routes and nothing
    /// else (docs/DEVELOPER_SURFACE.md §3, phase 1).
    Personal {
        token_id: String,
        username: String,
        tenant_id: TenantId,
        /// The maker's id, when the token records it.
        user_id: Option<fcp_core::UserId>,
    },
    /// A signed-in user: their username and role as they are now.
    Session {
        username: String,
        role: String,
        tenant_id: Option<TenantId>,
        /// The user's id; `None` only for a token made before sessions
        /// were rows (`docs/IDENTITY_AND_DEVICES.md` phase 1).
        user_id: Option<fcp_core::UserId>,
        /// The sign-in session the token belongs to.
        session_id: Option<fcp_core::LoginSessionId>,
        /// The roles the user's groups give them, besides `role`
        /// (`docs/USERS.md` §5).
        group_roles: Vec<UserRole>,
    },
}

impl AuthIdentity {
    /// Who this is in the audit and deletion logs: a username, an API
    /// token's id, or `static_token`.
    pub fn actor(&self) -> String {
        match self {
            Self::StaticToken => "static_token".to_string(),
            Self::ApiToken { token_id, .. } | Self::Personal { token_id, .. } => token_id.clone(),
            Self::Session { username, .. } => username.clone(),
        }
    }

    /// Returns true if the caller has admin-level access (not an end user).
    ///
    /// - **StaticToken**: always admin (it's the master config token).
    /// - **ApiToken**: always — an admin token reaches the admin API, and
    ///   each route's permission (`route_permissions`) decides the rest.
    /// - **Personal**: never; it reaches only its maker's `/me` routes.
    /// - **Session**: admin when the DB role is anything other than `"end_user"`.
    pub fn is_admin(&self) -> bool {
        match self {
            Self::StaticToken | Self::ApiToken { .. } => true,
            Self::Personal { .. } => false,
            Self::Session {
                role, group_roles, ..
            } => role != "end_user" || group_roles.iter().any(|r| *r != UserRole::EndUser),
        }
    }

    /// Whether this caller may give `role` to a user, or act on a user who
    /// holds it (`fcp_core::permissions::roles_may_assign`): the static
    /// token any role; a session what its roles allow; an admin token a
    /// role holding nothing the token lacks, never the platform's; a
    /// personal token none.
    pub fn may_assign_role(&self, role: UserRole, mode: DeploymentMode) -> bool {
        match self {
            Self::StaticToken => true,
            Self::Session { .. } => fcp_core::permissions::roles_may_assign(
                &self.session_roles().unwrap_or_default(),
                role,
                mode,
            ),
            Self::ApiToken { .. } => {
                !role.is_platform_admin()
                    && role
                        .permissions(mode)
                        .into_iter()
                        .all(|p| self.has_permission(p.as_str()))
            }
            Self::Personal { .. } => false,
        }
    }

    /// A signed-in user's roles: their own and their groups'.
    pub fn session_roles(&self) -> Option<Vec<UserRole>> {
        match self {
            Self::Session {
                role, group_roles, ..
            } => {
                let mut roles = vec![role.parse().unwrap_or(UserRole::EndUser)];
                roles.extend(group_roles.iter().copied());
                Some(roles)
            }
            _ => None,
        }
    }

    /// Returns true when the caller is a platform-level administrator with
    /// cross-tenant visibility and the right to edit system-wide settings.
    ///
    /// This is strictly narrower than [`is_admin`]: a tenant admin is an
    /// "admin" inside a single tenant but must not be treated as a platform
    /// admin. Only [`StaticToken`] and [`Session`] carrying the `super_admin`
    /// role qualify. API tokens are scoped by construction and therefore
    /// never qualify.
    ///
    /// Endpoints that must stay off-limits to tenant admins (tenant CRUD,
    /// system-scoped config, cluster topology, etc.) should check this
    /// instead of [`is_admin`].
    pub fn is_platform_admin(&self) -> bool {
        match self {
            Self::StaticToken => true,
            Self::ApiToken { .. } | Self::Personal { .. } => false,
            Self::Session { role, .. } => role == "super_admin",
        }
    }

    /// Check whether this identity holds a specific permission.
    ///
    /// Returns `true` for static tokens (unrestricted) and for API tokens
    /// whose permissions list contains `permission`, `"admin"`, or `"*"`.
    /// JWT sessions always return `true` (role-gated separately).
    pub fn has_permission(&self, permission: &str) -> bool {
        match self {
            Self::StaticToken | Self::Session { .. } => true,
            Self::ApiToken { permissions, .. } => permissions
                .iter()
                .any(|p| p == permission || p == "admin" || p == "*"),
            Self::Personal { .. } => false,
        }
    }

    /// Returns the tenant scope carried by the authenticated identity, if any.
    pub fn tenant_id(&self) -> Option<&TenantId> {
        match self {
            Self::StaticToken => None,
            Self::ApiToken { tenant_id, .. } | Self::Personal { tenant_id, .. } => Some(tenant_id),
            Self::Session { tenant_id, .. } => tenant_id.as_ref(),
        }
    }
}

/// Check that the authenticated caller has a specific permission.
///
/// - **StaticToken**: always allowed (unrestricted admin).
/// - **ApiToken**: checks if the `permissions` array contains the permission's
///   snake_case name, `"admin"`, or `"*"` (only a token made before makers
///   were recorded still carries those two).
/// - **Personal**: never.
/// - **Session**: parses the role string and checks `role.has_permission(perm, mode)`.
///
/// Returns `Ok(())` on success, or `Err(ApiError::Forbidden)` if denied.
pub fn check_permission(
    identity: &AuthIdentity,
    perm: Permission,
    mode: DeploymentMode,
) -> Result<(), crate::error::ApiError> {
    match identity {
        AuthIdentity::StaticToken => Ok(()),
        AuthIdentity::ApiToken { permissions, .. } => {
            let perm_str = perm.as_str();
            if permissions
                .iter()
                .any(|p| p == perm_str || p == "admin" || p == "*")
            {
                Ok(())
            } else {
                Err(crate::error::ApiError::Forbidden(
                    "Insufficient permissions".into(),
                ))
            }
        }
        AuthIdentity::Personal { .. } => Err(crate::error::ApiError::Forbidden(
            "A personal token reaches only /me routes".into(),
        )),
        AuthIdentity::Session { .. } => {
            let roles = identity.session_roles().unwrap_or_default();
            if fcp_core::permissions::roles_have_permission(&roles, perm, mode) {
                Ok(())
            } else {
                Err(crate::error::ApiError::Forbidden(
                    "Insufficient permissions".into(),
                ))
            }
        }
    }
}

/// Returns true when the caller is allowed to operate outside tenant scope.
///
/// Both static config tokens and JWT sessions carrying the `super_admin`
/// role are treated as platform-admin credentials and can see/modify
/// resources in every tenant. Tenant-scoped roles (`tenant_admin`,
/// `telecom_admin`, ...) are rejected so that list/read endpoints clamp
/// to their own tenant_id.
pub fn can_access_all_tenants(identity: Option<&AuthIdentity>) -> bool {
    match identity {
        Some(id) => id.is_platform_admin(),
        None => false,
    }
}

/// Authentication middleware state.
#[derive(Clone)]
pub struct AuthConfig {
    pub token: String,
    pub api_token_store: Option<Arc<dyn ApiTokenStore>>,
    /// The keys session tokens are verified with. When set,
    /// `validate_token` also accepts a signed-in user's access token, so the
    /// admin UI can use it for both REST and WebSocket requests.
    pub session_keys: Option<fcp_core::session::SessionKeys>,
    /// Where sign-in sessions are kept: a token naming a session is refused
    /// once the session is revoked.
    pub login_sessions: Option<Arc<dyn fcp_core::session::LoginSessionStore>>,
    /// Credential store for checking user status (enabled/locked) on JWT auth.
    /// When set, JWT tokens for disabled/locked users are rejected at the
    /// middleware level without waiting for token expiry.
    pub credential_store: Option<Arc<dyn CredentialStore>>,
    /// The deployment mode, which decides what a token's maker may do and
    /// so what the token may.
    pub mode: DeploymentMode,
}

impl AuthConfig {
    pub fn new(token: String) -> Self {
        Self {
            token,
            api_token_store: None,
            session_keys: None,
            login_sessions: None,
            credential_store: None,
            mode: DeploymentMode::default(),
        }
    }

    /// Set the deployment mode tokens are judged under.
    pub fn with_deployment_mode(mut self, mode: DeploymentMode) -> Self {
        self.mode = mode;
        self
    }

    /// Attach an API token store for `fcp_...` bearer token validation.
    pub fn with_api_token_store(mut self, store: Arc<dyn ApiTokenStore>) -> Self {
        self.api_token_store = Some(store);
        self
    }

    /// Attach the session keys so the bearer middleware also accepts a
    /// signed-in user's access token.
    pub fn with_session_keys(mut self, keys: fcp_core::session::SessionKeys) -> Self {
        self.session_keys = Some(keys);
        self
    }

    /// Attach the sign-in session store.
    pub fn with_login_sessions(
        mut self,
        store: Arc<dyn fcp_core::session::LoginSessionStore>,
    ) -> Self {
        self.login_sessions = Some(store);
        self
    }

    /// Attach a credential store for per-request user status checks on JWT auth.
    pub fn with_credential_store(mut self, store: Arc<dyn CredentialStore>) -> Self {
        self.credential_store = Some(store);
        self
    }
}

/// Validate a bearer token string against the auth config.
///
/// Used by both the HTTP middleware and the WebSocket first-message auth.
/// Returns the caller's identity if the token is valid, `None` otherwise.
pub async fn validate_token(token: &str, config: &AuthConfig) -> Option<AuthIdentity> {
    // Fast path: constant-time comparison against static config token
    if fcp_core::secret::constant_time_eq(token, &config.token) {
        return Some(AuthIdentity::StaticToken);
    }

    // Check API tokens (fcp_... prefix) if a store is configured
    if token.starts_with("fcp_") {
        if let Some(ref store) = config.api_token_store {
            return validate_api_token(token, store, config).await;
        }
    }

    // A signed-in user's access token (`docs/IDENTITY_AND_DEVICES.md`
    // phase 1): the session live, the user active, the role as it is now.
    if let (Some(keys), Some(users)) = (&config.session_keys, &config.credential_store) {
        let principal = fcp_core::session::authenticate_session(
            token,
            keys,
            users.as_ref(),
            config.login_sessions.as_deref(),
            std::time::SystemTime::now(),
        )
        .await?;
        let user = principal.user;
        return Some(AuthIdentity::Session {
            group_roles: principal.group_roles,
            role: crate::v1::auth_session::role_to_string(&user.role),
            tenant_id: Some(user.tenant_id),
            user_id: Some(user.id),
            session_id: principal.session,
            username: user.username,
        });
    }

    None
}

/// Extract and validate the Bearer token from the Authorization header.
///
/// On success, inserts [`AuthIdentity`] into request extensions so downstream
/// middleware and handlers can check the caller's role/permissions.
///
/// Expected format: "Authorization: Bearer <token>"
///
/// Check order:
/// 1. No Authorization header → 401
/// 2. Static token matches → allow (full admin access)
/// 3. Token starts with `fcp_` and an `ApiTokenStore` is configured:
///    hash with SHA-256, look up in DB, check expiry, update `last_used_at`
/// 4. JWT session token — validate signature, expiry, and user status
/// 5. Otherwise → 401
pub async fn validate_bearer_token(
    auth_config: AuthConfig,
    mut req: Request,
    next: Next,
) -> Result<Response, crate::error::ApiError> {
    // Get the Authorization header
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    if let Some(auth_value) = auth_header {
        // Check if it's a Bearer token
        if let Some(token) = auth_value.strip_prefix("Bearer ") {
            if let Some(identity) = validate_token(token, &auth_config).await {
                req.extensions_mut().insert(identity);
                return Ok(next.run(req).await);
            }
        }
    }

    Err(crate::error::ApiError::Unauthorized(
        "Invalid or missing authentication token".to_string(),
    ))
}

/// Middleware that requires admin-level access.
///
/// Checks the [`AuthIdentity`] inserted by [`validate_bearer_token`] and
/// rejects callers where [`AuthIdentity::is_admin()`] returns false:
/// - JWT sessions with `role: "end_user"`
/// - API tokens without `"admin"` or `"*"` in their permissions
///
/// Static config tokens are always admin-level.
///
/// When no `AuthIdentity` is present (e.g. auth is disabled), the request
/// is allowed through.
pub async fn require_admin(req: Request, next: Next) -> Result<Response, crate::error::ApiError> {
    if let Some(identity) = req.extensions().get::<AuthIdentity>() {
        if !identity.is_admin() {
            return Err(crate::error::ApiError::Forbidden(
                "Admin access required".to_string(),
            ));
        }
    }
    Ok(next.run(req).await)
}

/// Middleware that requires platform-level (cross-tenant) admin access.
///
/// Strictly narrower than [`require_admin`]: a tenant admin is rejected.
/// Use this to gate endpoints that operate on platform-wide resources
/// such as tenant CRUD, system-scope configuration, cluster membership,
/// and any operation that a tenant-scoped administrator should not be
/// able to perform on behalf of the whole platform.
///
/// Callers:
/// - `StaticToken`  — allowed (master config token)
/// - `Session { role: "super_admin", .. }` — allowed
/// - `Session` with any other role, including `tenant_admin` — rejected
/// - `ApiToken` — rejected (API tokens are tenant-scoped by construction)
/// - no identity — allowed (auth disabled)
pub async fn require_platform_admin(
    req: Request,
    next: Next,
) -> Result<Response, crate::error::ApiError> {
    if let Some(identity) = req.extensions().get::<AuthIdentity>() {
        if !identity.is_platform_admin() {
            return Err(crate::error::ApiError::Forbidden(
                "Platform admin access required".to_string(),
            ));
        }
    }
    // What a platform-only router does is the platform's: its audit rows
    // have no tenant (docs/TENANT_SCOPE.md §3.3).
    let mut response = next.run(req).await;
    if response
        .extensions()
        .get::<crate::audit::AuditScope>()
        .is_none()
    {
        response
            .extensions_mut()
            .insert(crate::audit::AuditScope::Platform);
    }
    Ok(response)
}

/// Validate an `fcp_...` API token against the database.
///
/// Returns the caller's identity if the token opens (exists, not revoked,
/// not expired, the current secret or a rotated one inside its grace) and
/// its maker is still an active user (`fcp_core::authenticate_api_token`).
/// An admin token acts with its permissions less anything its maker can no
/// longer do; a personal token as its maker on `/me`. On success, spawns a
/// background task to update `last_used_at`.
async fn validate_api_token(
    token: &str,
    store: &Arc<dyn ApiTokenStore>,
    config: &AuthConfig,
) -> Option<AuthIdentity> {
    let users = config.credential_store.as_deref();
    let fcp_core::AuthenticatedApiToken {
        token,
        maker_role,
        maker_group_roles,
        maker,
    } = fcp_core::authenticate_api_token(store.as_ref(), users, token).await?;

    let store = Arc::clone(store);
    let token_id = token.id;
    tokio::spawn(async move {
        if let Err(e) = store.update_last_used(&token_id).await {
            tracing::debug!(error = %e, "Failed to update API token last_used_at");
        }
    });

    match token.scope {
        fcp_core::ApiTokenScope::Personal => {
            if token.user_id.is_empty() {
                return None;
            }
            let (user_id, username) = match maker {
                Some((id, username)) => (Some(id), username),
                None => (None, token.user_id),
            };
            Some(AuthIdentity::Personal {
                token_id: token.id.to_string(),
                username,
                tenant_id: token.tenant_id,
                user_id,
            })
        }
        fcp_core::ApiTokenScope::Admin => Some(AuthIdentity::ApiToken {
            token_id: token.id.to_string(),
            tenant_id: token.tenant_id,
            permissions: effective_permissions(
                &token.permissions,
                maker_role
                    .map(|own| {
                        let mut roles = vec![own];
                        roles.extend(maker_group_roles.iter().copied());
                        roles
                    })
                    .as_deref(),
                config.mode,
            ),
        }),
    }
}

/// What an admin token may do now: its permissions, less anything its maker
/// can no longer grant (a maker demoted since takes the token down with
/// them). A token without a maker — made with the static token, or before
/// makers were recorded — is held to a tenant administrator's reach, as a
/// made token is (`docs/TENANT_SCOPE.md` §3.7): never system configuration
/// or another tenant's lifecycle, whatever it carries.
pub fn effective_permissions(
    stored: &[String],
    maker_roles: Option<&[UserRole]>,
    mode: DeploymentMode,
) -> Vec<String> {
    let roles = maker_roles.unwrap_or(&[UserRole::TenantAdmin]);
    let allowed: Vec<&str> = fcp_core::permissions::roles_token_permissions(roles, mode)
        .iter()
        .map(|p| p.as_str())
        .collect();
    if stored.iter().any(|p| p == "admin" || p == "*") {
        return allowed.into_iter().map(String::from).collect();
    }
    stored
        .iter()
        .filter(|p| allowed.contains(&p.as_str()))
        .cloned()
        .collect()
}

/// The tenant a request works in (`docs/TENANT_SCOPE.md` §3.1).
///
/// A platform administrator (a super_admin session, the static token) works
/// in the tenant the `x-fcp-tenant-id` header names — the admin UI's picker
/// — and without one in their own tenant (the default tenant for the
/// static token). Anyone else works in their own tenant: their header is
/// ignored, so no one leaves their tenant by naming another.
pub struct TenantContext(pub fcp_core::TenantId);

#[axum::async_trait]
impl axum::extract::FromRequestParts<crate::state::ApiState> for TenantContext {
    type Rejection = crate::error::ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &crate::state::ApiState,
    ) -> Result<Self, Self::Rejection> {
        let tenant = Self::resolve(parts, state).await?;
        // The request's lines name the tenant it acts on
        // (`docs/LOG_VIEWER.md` L2).
        tracing::Span::current().record("tenant_id", tracing::field::display(tenant.0));
        Ok(tenant)
    }
}

impl TenantContext {
    async fn resolve(
        parts: &mut axum::http::request::Parts,
        state: &crate::state::ApiState,
    ) -> Result<Self, crate::error::ApiError> {
        if let Some(identity) = parts.extensions.get::<AuthIdentity>() {
            if !identity.is_platform_admin() {
                if let Some(tenant_id) = identity.tenant_id() {
                    return Ok(TenantContext(*tenant_id));
                }
            } else {
                if let Some(raw_tenant_id) = parts
                    .headers
                    .get(PLATFORM_TENANT_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    let tenant_id = raw_tenant_id.parse::<TenantId>().map_err(|e| {
                        crate::error::ApiError::BadRequest(format!(
                            "Invalid {} header: {}",
                            PLATFORM_TENANT_HEADER, e
                        ))
                    })?;

                    // The default tenant is a tenant whether or not it has a
                    // row: requests without a header already work in it.
                    if let Some(storage) = state.storage.as_ref().filter(|_| {
                        tenant_id != state.default_tenant_id && tenant_id != TenantId::nil()
                    }) {
                        match storage.get_tenant(&tenant_id).await {
                            Ok(Some(_)) => {}
                            Ok(None) => {
                                return Err(crate::error::ApiError::NotFound(format!(
                                    "Tenant '{}' not found",
                                    tenant_id
                                )));
                            }
                            Err(e) => {
                                return Err(crate::error::ApiError::Internal(format!(
                                    "Failed to resolve tenant override: {}",
                                    e
                                )));
                            }
                        }
                    }

                    return Ok(TenantContext(tenant_id));
                }
                if let Some(tenant_id) = identity.tenant_id() {
                    return Ok(TenantContext(*tenant_id));
                }
            }
        }

        // Fall back to the server-configured tenant for static tokens and
        // single-tenant deployments where the auth layer carries no tenant scope.
        Ok(TenantContext(state.default_tenant_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request as HttpRequest, StatusCode},
        routing::get,
        Router,
    };
    use fcp_core::storage::{ApiToken, ApiTokenStore};
    use fcp_core::{ApiTokenId, TenantId};
    use sha2::{Digest, Sha256};
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};
    use tower::ServiceExt; // for oneshot

    #[test]
    fn test_constant_time_compare() {
        // Same strings
        assert!(fcp_core::secret::constant_time_eq("secret123", "secret123"));

        // Different strings (same length)
        assert!(!fcp_core::secret::constant_time_eq(
            "secret123",
            "secret124"
        ));

        // Different lengths
        assert!(!fcp_core::secret::constant_time_eq("secret", "secret123"));
        assert!(!fcp_core::secret::constant_time_eq("secret123", "secret"));

        // Empty strings
        assert!(fcp_core::secret::constant_time_eq("", ""));
    }

    #[test]
    fn test_bearer_token_parsing() {
        // Valid bearer token
        let auth_header = "Bearer my-secret-token";
        let token = auth_header.strip_prefix("Bearer ");
        assert_eq!(token, Some("my-secret-token"));

        // Invalid format
        let auth_header = "Basic dXNlcjpwYXNz";
        let token = auth_header.strip_prefix("Bearer ");
        assert_eq!(token, None);

        // No prefix
        let auth_header = "my-secret-token";
        let token = auth_header.strip_prefix("Bearer ");
        assert_eq!(token, None);
    }

    // -- Mock ApiTokenStore --------------------------------------------------

    /// In-memory mock that stores tokens and tracks `update_last_used` calls.
    struct MockApiTokenStore {
        tokens: Vec<ApiToken>,
        last_used_calls: Mutex<Vec<ApiTokenId>>,
    }

    impl MockApiTokenStore {
        fn new(tokens: Vec<ApiToken>) -> Self {
            Self {
                tokens,
                last_used_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ApiTokenStore for MockApiTokenStore {
        async fn create_api_token(&self, _token: ApiToken) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_api_tokens(&self, _tenant_id: &TenantId) -> anyhow::Result<Vec<ApiToken>> {
            Ok(self.tokens.clone())
        }
        async fn get_api_token(&self, id: &ApiTokenId) -> anyhow::Result<Option<ApiToken>> {
            Ok(self.tokens.iter().find(|t| t.id == *id).cloned())
        }
        async fn get_api_token_by_hash(
            &self,
            token_hash: &str,
        ) -> anyhow::Result<Option<ApiToken>> {
            Ok(self
                .tokens
                .iter()
                .find(|t| t.token_hash == token_hash)
                .cloned())
        }
        async fn delete_api_token(&self, _id: &ApiTokenId) -> anyhow::Result<()> {
            Ok(())
        }
        async fn revoke_api_token(&self, _id: &ApiTokenId, _by: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn rotate_api_token(
            &self,
            _id: &ApiTokenId,
            _new_hash: &str,
            _new_prefix: &str,
            _previous_valid_until: SystemTime,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn update_last_used(&self, id: &ApiTokenId) -> anyhow::Result<()> {
            self.last_used_calls.lock().unwrap().push(*id);
            Ok(())
        }
    }

    // -- Helpers -------------------------------------------------------------

    /// Hash a plaintext secret the same way the middleware does.
    fn hash_secret(secret: &str) -> String {
        let mut h = Sha256::new();
        h.update(secret.as_bytes());
        format!("{:x}", h.finalize())
    }

    /// Build a minimal axum app protected by `validate_bearer_token`.
    fn test_app(auth_config: AuthConfig) -> Router {
        Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(move |req, next| {
                let cfg = auth_config.clone();
                validate_bearer_token(cfg, req, next)
            }))
    }

    /// Create a test ApiToken with the given plaintext secret and optional expiry.
    fn make_token(secret: &str, expires_at: Option<SystemTime>) -> ApiToken {
        ApiToken {
            id: ApiTokenId::new(),
            name: "test".into(),
            token_hash: hash_secret(secret),
            prefix: secret.chars().take(8).collect(),
            user_id: String::new(),
            tenant_id: TenantId::nil(),
            permissions: vec![],
            expires_at,
            created_at: SystemTime::now(),
            last_used_at: None,
            scope: Default::default(),
            revoked_at: None,
            revoked_by: None,
            previous_token_hash: None,
            previous_expires_at: None,
            rotated_at: None,
        }
    }

    // -- Tests ---------------------------------------------------------------

    #[tokio::test]
    async fn static_token_still_works() {
        let app = test_app(AuthConfig::new("static-secret".into()));
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .header("Authorization", "Bearer static-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_header_returns_401() {
        let app = test_app(AuthConfig::new("static-secret".into()));
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_static_token_returns_401() {
        let app = test_app(AuthConfig::new("static-secret".into()));
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .header("Authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_api_token_accepted() {
        let secret = "fcp_test_valid_token_abc123";
        let token = make_token(secret, None);
        let store = Arc::new(MockApiTokenStore::new(vec![token]));

        let auth = AuthConfig::new("static-secret".into()).with_api_token_store(store.clone());

        let app = test_app(auth);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .header("Authorization", format!("Bearer {}", secret))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Give the background spawn a moment to run
        tokio::time::sleep(Duration::from_millis(50)).await;
        let calls = store.last_used_calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "update_last_used should have been called");
    }

    #[tokio::test]
    async fn unknown_api_token_rejected() {
        let store = Arc::new(MockApiTokenStore::new(vec![])); // empty store

        let auth = AuthConfig::new("static-secret".into()).with_api_token_store(store);

        let app = test_app(auth);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .header("Authorization", "Bearer fcp_does_not_exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn expired_api_token_rejected() {
        let secret = "fcp_expired_token_xyz";
        let expired = SystemTime::now() - Duration::from_secs(3600); // 1 hour ago
        let token = make_token(secret, Some(expired));
        let store = Arc::new(MockApiTokenStore::new(vec![token]));

        let auth = AuthConfig::new("static-secret".into()).with_api_token_store(store.clone());

        let app = test_app(auth);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .header("Authorization", format!("Bearer {}", secret))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // update_last_used should NOT have been called
        tokio::time::sleep(Duration::from_millis(50)).await;
        let calls = store.last_used_calls.lock().unwrap();
        assert!(
            calls.is_empty(),
            "expired token should not update last_used"
        );
    }

    #[tokio::test]
    async fn non_fcp_prefix_skips_store_lookup() {
        // A token that doesn't start with "fcp_" should not hit the store,
        // even if the store happens to contain a matching hash.
        let secret = "not_fcp_prefix_token";
        let token = make_token(secret, None);
        let store = Arc::new(MockApiTokenStore::new(vec![token]));

        let auth = AuthConfig::new("static-secret".into()).with_api_token_store(store);

        let app = test_app(auth);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .header("Authorization", format!("Bearer {}", secret))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn future_expiry_accepted() {
        let secret = "fcp_future_expiry_token";
        let future = SystemTime::now() + Duration::from_secs(86400); // +24h
        let token = make_token(secret, Some(future));
        let store = Arc::new(MockApiTokenStore::new(vec![token]));

        let auth = AuthConfig::new("static-secret".into()).with_api_token_store(store);

        let app = test_app(auth);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .header("Authorization", format!("Bearer {}", secret))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    async fn identity_for(secret: &str, tokens: Vec<ApiToken>) -> Option<AuthIdentity> {
        let store: Arc<dyn ApiTokenStore> = Arc::new(MockApiTokenStore::new(tokens));
        let auth = AuthConfig::new("static-secret".into()).with_api_token_store(store);
        validate_token(secret, &auth).await
    }

    #[tokio::test]
    async fn a_revoked_token_is_refused() {
        let secret = "fcp_revoked";
        let mut token = make_token(secret, None);
        token.revoked_at = Some(SystemTime::now());
        assert!(identity_for(secret, vec![token]).await.is_none());
    }

    #[tokio::test]
    async fn a_rotated_out_secret_works_through_its_grace_and_not_after() {
        let old = "fcp_old_secret";
        let new = "fcp_new_secret";
        let mut token = make_token(new, None);
        token.previous_token_hash = Some(hash_secret(old));
        token.previous_expires_at = Some(SystemTime::now() + Duration::from_secs(60));
        assert!(identity_for(new, vec![token.clone()]).await.is_some());
        // The store finds the row by either hash; the token decides.
        assert!(token.accepts(&hash_secret(old), SystemTime::now()));
        token.previous_expires_at = Some(SystemTime::now() - Duration::from_secs(1));
        assert!(!token.accepts(&hash_secret(old), SystemTime::now()));
        assert!(token.accepts(&hash_secret(new), SystemTime::now()));
    }

    #[tokio::test]
    async fn a_personal_token_is_its_maker_and_not_an_admin() {
        let secret = "fcp_personal";
        let mut token = make_token(secret, None);
        token.scope = fcp_core::ApiTokenScope::Personal;
        token.user_id = "alice".into();
        let identity = identity_for(secret, vec![token]).await.expect("opens");
        assert!(matches!(
            &identity,
            AuthIdentity::Personal { username, .. } if username == "alice"
        ));
        assert!(!identity.is_admin());
        assert!(
            check_permission(&identity, Permission::UserRead, DeploymentMode::Enterprise).is_err()
        );
    }

    #[test]
    fn a_makers_groups_add_to_what_their_token_may_carry() {
        let mode = DeploymentMode::ServiceProvider;
        let stored = vec!["user_create".to_string(), "queue_manage".to_string()];
        // Neither role alone holds both; together they do.
        let both = [UserRole::TelecomAdmin, UserRole::UserAdmin];
        assert_eq!(
            effective_permissions(&stored, Some(&both[..]), mode),
            stored
        );
        assert_eq!(
            effective_permissions(&stored, Some(&[UserRole::TelecomAdmin][..]), mode),
            vec!["queue_manage".to_string()]
        );
        let session = AuthIdentity::Session {
            username: "u".into(),
            role: "end_user".into(),
            tenant_id: None,
            user_id: None,
            session_id: None,
            group_roles: vec![UserRole::UserAdmin],
        };
        assert!(session.is_admin());
        assert!(!session.is_platform_admin());
        assert!(check_permission(&session, Permission::UserCreate, mode).is_ok());
        assert!(check_permission(&session, Permission::QueueManage, mode).is_err());
    }

    #[test]
    fn a_demoted_maker_takes_the_token_down_with_them() {
        let mode = DeploymentMode::ServiceProvider;
        let stored = vec!["user_read".to_string(), "user_create".to_string()];
        assert_eq!(effective_permissions(&stored, None, mode), stored);
        // Without a maker, a tenant administrator's reach: a legacy `admin`
        // token, or one carrying system configuration, holds none of it.
        for stored in [
            vec!["admin".to_string()],
            vec!["system_config".to_string(), "user_read".to_string()],
        ] {
            let reach = effective_permissions(&stored, None, mode);
            assert!(
                !reach.contains(&"system_config".to_string()),
                "{stored:?}: {reach:?}"
            );
            assert!(
                !reach.contains(&"tenant_create".to_string()),
                "{stored:?}: {reach:?}"
            );
        }
        assert!(effective_permissions(&["admin".to_string()], None, mode)
            .contains(&"user_create".to_string()));
        assert_eq!(
            effective_permissions(&stored, Some(&[UserRole::TenantAdmin][..]), mode),
            stored
        );
        assert_eq!(
            effective_permissions(&stored, Some(&[UserRole::ReadOnly][..]), mode),
            vec!["user_read".to_string()]
        );
        // A legacy full-access token with a maker is the maker's reach.
        let legacy = vec!["admin".to_string()];
        let reach = effective_permissions(&legacy, Some(&[UserRole::ReadOnly][..]), mode);
        assert!(reach.contains(&"user_read".to_string()));
        assert!(!reach.contains(&"user_create".to_string()));
    }
}
