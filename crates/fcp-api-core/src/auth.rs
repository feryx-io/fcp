//! API authentication middleware.

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};

/// Authentication middleware state.
#[derive(Clone)]
pub struct AuthConfig {
    pub token: String,
}

impl AuthConfig {
    pub fn new(token: String) -> Self {
        Self { token }
    }
}

/// Extract and validate the Bearer token from the Authorization header.
///
/// Expected format: "Authorization: Bearer <token>"
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
            // Constant-time comparison to prevent timing attacks
            if constant_time_compare(token, &auth_config.token) {
                return Ok(next.run(req).await);
            }
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        "Invalid or missing authentication token".to_string(),
    ))
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
}
