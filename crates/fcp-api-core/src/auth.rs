//! API authentication middleware.

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use fcp_core::permissions::{DeploymentMode, Permission, UserRole};
use fcp_core::storage::{ApiTokenStore, TenantStore};
use fcp_core::{AccountStatus, CredentialStore, TenantId};
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
    /// JWT session token with role from claims.
    Session {
        username: String,
        role: String,
        tenant_id: Option<TenantId>,
    },
}

impl AuthIdentity {
    /// Who this is in the audit and deletion logs: a username, an API
    /// token's id, or `static_token`.
    pub fn actor(&self) -> String {
        match self {
            Self::StaticToken => "static_token".to_string(),
            Self::ApiToken { token_id, .. } => token_id.clone(),
            Self::Session { username, .. } => username.clone(),
        }
    }

    /// Returns true if the caller has admin-level access (not an end user).
    ///
    /// - **StaticToken**: always admin (it's the master config token).
    /// - **ApiToken**: admin only when permissions include `"admin"` or `"*"`.
    ///   Tokens created with empty or scoped permissions are non-admin.
    /// - **Session**: admin when the DB role is anything other than `"end_user"`.
    pub fn is_admin(&self) -> bool {
        match self {
            Self::StaticToken => true,
            Self::ApiToken { permissions, .. } => {
                permissions.iter().any(|p| p == "admin" || p == "*")
            }
            Self::Session { role, .. } => role != "end_user",
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
            Self::ApiToken { .. } => false,
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
        }
    }

    /// Returns the tenant scope carried by the authenticated identity, if any.
    pub fn tenant_id(&self) -> Option<&TenantId> {
        match self {
            Self::StaticToken => None,
            Self::ApiToken { tenant_id, .. } => Some(tenant_id),
            Self::Session { tenant_id, .. } => tenant_id.as_ref(),
        }
    }
}

/// Check that the authenticated caller has a specific permission.
///
/// - **StaticToken**: always allowed (unrestricted admin).
/// - **ApiToken**: checks if the `permissions` array contains the permission's
///   snake_case name, `"admin"`, or `"*"`.
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
        AuthIdentity::Session { role, .. } => {
            let user_role: UserRole = role.parse().unwrap_or(UserRole::EndUser);
            if user_role.has_permission(perm, mode) {
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
    /// JWT secret for validating session tokens in the bearer middleware.
    /// When set, `validate_token` also accepts valid JWT session tokens,
    /// so the admin UI can use its JWT for both REST and WebSocket requests.
    pub jwt_secret: Option<Vec<u8>>,
    /// Credential store for checking user status (enabled/locked) on JWT auth.
    /// When set, JWT tokens for disabled/locked users are rejected at the
    /// middleware level without waiting for token expiry.
    pub credential_store: Option<Arc<dyn CredentialStore>>,
}

impl AuthConfig {
    pub fn new(token: String) -> Self {
        Self {
            token,
            api_token_store: None,
            jwt_secret: None,
            credential_store: None,
        }
    }

    /// Attach an API token store for `fcp_...` bearer token validation.
    pub fn with_api_token_store(mut self, store: Arc<dyn ApiTokenStore>) -> Self {
        self.api_token_store = Some(store);
        self
    }

    /// Attach a JWT secret so the bearer middleware also accepts JWT session tokens.
    pub fn with_jwt_secret(mut self, secret: Vec<u8>) -> Self {
        self.jwt_secret = Some(secret);
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
    if constant_time_compare(token, &config.token) {
        return Some(AuthIdentity::StaticToken);
    }

    // Check API tokens (fcp_... prefix) if a store is configured
    if token.starts_with("fcp_") {
        if let Some(ref store) = config.api_token_store {
            return validate_api_token(token, store).await;
        }
    }

    // Check JWT session tokens (admin UI sends these after login)
    if let Some(ref jwt_secret) = config.jwt_secret {
        if let Ok(claims) = crate::v1::auth_session::decode_jwt(token, jwt_secret) {
            // Verify user is still active (not disabled/locked/deleted) and
            // fetch current role from DB so demotions take effect immediately
            // (not after JWT expiry).
            if let Some(ref cred_store) = config.credential_store {
                let tenant_id = match claims.tenant_id.parse::<TenantId>() {
                    Ok(tenant_id) => tenant_id,
                    Err(_) => return None,
                };

                match cred_store.get_user_in_tenant(&claims.sub, &tenant_id).await {
                    Ok(Some(user)) => {
                        if !user.enabled
                            || user.account_status == AccountStatus::Locked
                            || user.account_status == AccountStatus::Suspended
                        {
                            tracing::debug!(
                                username = %claims.sub,
                                status = ?user.account_status,
                                enabled = user.enabled,
                                "JWT rejected: user account inactive"
                            );
                            return None;
                        }
                        // Use current DB role, not the (possibly stale) JWT claim
                        let current_role = crate::v1::auth_session::role_to_string(&user.role);
                        return Some(AuthIdentity::Session {
                            username: claims.sub,
                            role: current_role,
                            tenant_id: Some(tenant_id),
                        });
                    }
                    Ok(None) => {
                        tracing::debug!(username = %claims.sub, "JWT rejected: user deleted");
                        return None;
                    }
                    Err(e) => {
                        // Storage error — reject to fail closed
                        tracing::warn!(error = %e, "JWT user status check failed");
                        return None;
                    }
                }
            }

            return Some(AuthIdentity::Session {
                username: claims.sub,
                role: claims.role,
                tenant_id: claims.tenant_id.parse().ok(),
            });
        }
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
) -> Result<Response, (StatusCode, String)> {
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

    Err((
        StatusCode::UNAUTHORIZED,
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
pub async fn require_admin(req: Request, next: Next) -> Result<Response, (StatusCode, String)> {
    if let Some(identity) = req.extensions().get::<AuthIdentity>() {
        if !identity.is_admin() {
            return Err((StatusCode::FORBIDDEN, "Admin access required".to_string()));
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
) -> Result<Response, (StatusCode, String)> {
    if let Some(identity) = req.extensions().get::<AuthIdentity>() {
        if !identity.is_platform_admin() {
            return Err((
                StatusCode::FORBIDDEN,
                "Platform admin access required".to_string(),
            ));
        }
    }
    Ok(next.run(req).await)
}

/// Validate an `fcp_...` API token against the database.
///
/// Returns the caller's identity if the token is valid (exists, not expired).
/// On success, spawns a background task to update `last_used_at`.
async fn validate_api_token(token: &str, store: &Arc<dyn ApiTokenStore>) -> Option<AuthIdentity> {
    use sha2::{Digest, Sha256};

    // Hash the presented token
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = format!("{:x}", hasher.finalize());

    // Look up in DB
    let api_token = match store.get_api_token_by_hash(&token_hash).await {
        Ok(Some(t)) => t,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to look up API token");
            return None;
        }
    };

    // Check expiry
    if let Some(expires_at) = api_token.expires_at {
        if std::time::SystemTime::now() > expires_at {
            return None;
        }
    }

    let permissions = api_token.permissions.clone();
    let tenant_id = api_token.tenant_id;

    // Update last_used_at in the background (non-blocking)
    let store = Arc::clone(store);
    let token_id = api_token.id;
    let identity_token_id = token_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = store.update_last_used(&token_id).await {
            tracing::debug!(error = %e, "Failed to update API token last_used_at");
        }
    });

    Some(AuthIdentity::ApiToken {
        token_id: identity_token_id,
        tenant_id,
        permissions,
    })
}

/// Constant-time string comparison to prevent timing attacks.
///
/// This ensures that the comparison always takes the same amount of time
/// regardless of where the first difference occurs.
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (byte_a, byte_b) in a.bytes().zip(b.bytes()) {
        result |= byte_a ^ byte_b;
    }

    result == 0
}

/// Tenant context extracted from the authenticated session.
///
/// SECURITY: Does NOT trust the raw X-Tenant-Id header. In a
/// single-tenant deployment, always uses the server's default tenant.
/// Multi-tenant deployments should extract tenant from JWT claims.
pub struct TenantContext(pub fcp_core::TenantId);

#[axum::async_trait]
impl axum::extract::FromRequestParts<crate::state::ApiState> for TenantContext {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &crate::state::ApiState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(identity) = parts.extensions.get::<AuthIdentity>() {
            if let Some(tenant_id) = identity.tenant_id() {
                return Ok(TenantContext(*tenant_id));
            }

            if identity.is_platform_admin() {
                if let Some(raw_tenant_id) = parts
                    .headers
                    .get(PLATFORM_TENANT_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    let tenant_id = raw_tenant_id.parse::<TenantId>().map_err(|e| {
                        (
                            StatusCode::BAD_REQUEST,
                            format!("Invalid {} header: {}", PLATFORM_TENANT_HEADER, e),
                        )
                    })?;

                    if let Some(storage) = state.storage.as_ref() {
                        match storage.get_tenant(&tenant_id).await {
                            Ok(Some(_)) => {}
                            Ok(None) => {
                                return Err((
                                    StatusCode::NOT_FOUND,
                                    format!("Tenant '{}' not found", tenant_id),
                                ));
                            }
                            Err(e) => {
                                return Err((
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    format!("Failed to resolve tenant override: {}", e),
                                ));
                            }
                        }
                    }

                    return Ok(TenantContext(tenant_id));
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
    use axum::{body::Body, http::Request as HttpRequest, routing::get, Router};
    use fcp_core::storage::{ApiToken, ApiTokenStore};
    use fcp_core::{ApiTokenId, TenantId};
    use sha2::{Digest, Sha256};
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};
    use tower::ServiceExt; // for oneshot

    #[test]
    fn test_constant_time_compare() {
        // Same strings
        assert!(constant_time_compare("secret123", "secret123"));

        // Different strings (same length)
        assert!(!constant_time_compare("secret123", "secret124"));

        // Different lengths
        assert!(!constant_time_compare("secret", "secret123"));
        assert!(!constant_time_compare("secret123", "secret"));

        // Empty strings
        assert!(constant_time_compare("", ""));
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
}
