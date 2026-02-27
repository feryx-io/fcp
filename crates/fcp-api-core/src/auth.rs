//! API authentication middleware.

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use fcp_core::storage::ApiTokenStore;
use std::sync::Arc;

/// Authentication middleware state.
#[derive(Clone)]
pub struct AuthConfig {
    pub token: String,
    pub api_token_store: Option<Arc<dyn ApiTokenStore>>,
}

impl AuthConfig {
    pub fn new(token: String) -> Self {
        Self {
            token,
            api_token_store: None,
        }
    }

    /// Attach an API token store for `fcp_...` bearer token validation.
    pub fn with_api_token_store(mut self, store: Arc<dyn ApiTokenStore>) -> Self {
        self.api_token_store = Some(store);
        self
    }
}

/// Extract and validate the Bearer token from the Authorization header.
///
/// Expected format: "Authorization: Bearer <token>"
///
/// Check order:
/// 1. No Authorization header → 401
/// 2. Static token matches → allow (fast path)
/// 3. Token starts with `fcp_` and an `ApiTokenStore` is configured:
///    hash with SHA-256, look up in DB, check expiry, update `last_used_at`
/// 4. Otherwise → 401
pub async fn validate_bearer_token(
    auth_config: AuthConfig,
    req: Request,
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
            // Fast path: constant-time comparison against static config token
            if constant_time_compare(token, &auth_config.token) {
                return Ok(next.run(req).await);
            }

            // Check API tokens (fcp_... prefix) if a store is configured
            if token.starts_with("fcp_") {
                if let Some(ref store) = auth_config.api_token_store {
                    if validate_api_token(token, store).await {
                        return Ok(next.run(req).await);
                    }
                }
            }
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        "Invalid or missing authentication token".to_string(),
    ))
}

/// Validate an `fcp_...` API token against the database.
///
/// Returns `true` if the token is valid (exists, not expired).
/// On success, spawns a background task to update `last_used_at`.
async fn validate_api_token(token: &str, store: &Arc<dyn ApiTokenStore>) -> bool {
    use sha2::{Digest, Sha256};

    // Hash the presented token
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = format!("{:x}", hasher.finalize());

    // Look up in DB
    let api_token = match store.get_api_token_by_hash(&token_hash).await {
        Ok(Some(t)) => t,
        Ok(None) => return false,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to look up API token");
            return false;
        }
    };

    // Check expiry
    if let Some(expires_at) = api_token.expires_at {
        if std::time::SystemTime::now() > expires_at {
            return false;
        }
    }

    // Update last_used_at in the background (non-blocking)
    let store = Arc::clone(store);
    let token_id = api_token.id.clone();
    tokio::spawn(async move {
        if let Err(e) = store.update_last_used(&token_id).await {
            tracing::debug!(error = %e, "Failed to update API token last_used_at");
        }
    });

    true
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

/// Tenant context extracted from request headers.
///
/// Checks the `X-Tenant-Id` header first, then falls back to the
/// default tenant configured in [`crate::state::ApiState`].
/// Forward-compatible with Phase 2 JWT-based tenant extraction.
pub struct TenantContext(pub fcp_core::TenantId);

#[axum::async_trait]
impl axum::extract::FromRequestParts<crate::state::ApiState> for TenantContext {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &crate::state::ApiState,
    ) -> Result<Self, Self::Rejection> {
        // Try X-Tenant-Id header first
        if let Some(value) = parts.headers.get("X-Tenant-Id") {
            if let Ok(s) = value.to_str() {
                if let Ok(tid) = s.parse::<fcp_core::TenantId>() {
                    return Ok(TenantContext(tid));
                }
            }
        }

        // Fall back to default tenant
        Ok(TenantContext(state.default_tenant_id.clone()))
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
        async fn get_api_token_by_hash(&self, token_hash: &str) -> anyhow::Result<Option<ApiToken>> {
            Ok(self.tokens.iter().find(|t| t.token_hash == token_hash).cloned())
        }
        async fn delete_api_token(&self, _id: &ApiTokenId) -> anyhow::Result<()> {
            Ok(())
        }
        async fn update_last_used(&self, id: &ApiTokenId) -> anyhow::Result<()> {
            self.last_used_calls.lock().unwrap().push(id.clone());
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

        let auth = AuthConfig::new("static-secret".into())
            .with_api_token_store(store.clone());

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

        let auth = AuthConfig::new("static-secret".into())
            .with_api_token_store(store);

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

        let auth = AuthConfig::new("static-secret".into())
            .with_api_token_store(store.clone());

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
        assert!(calls.is_empty(), "expired token should not update last_used");
    }

    #[tokio::test]
    async fn non_fcp_prefix_skips_store_lookup() {
        // A token that doesn't start with "fcp_" should not hit the store,
        // even if the store happens to contain a matching hash.
        let secret = "not_fcp_prefix_token";
        let token = make_token(secret, None);
        let store = Arc::new(MockApiTokenStore::new(vec![token]));

        let auth = AuthConfig::new("static-secret".into())
            .with_api_token_store(store);

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

        let auth = AuthConfig::new("static-secret".into())
            .with_api_token_store(store);

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
