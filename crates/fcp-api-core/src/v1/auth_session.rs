//! Auth session endpoints for the admin UI.
//!
//! Provides JWT-based authentication:
//! - POST /api/v1/auth/login  — authenticate with username/password, returns JWT
//! - POST /api/v1/auth/logout — client-side token discard
//! - GET  /api/v1/auth/me     — get current user info from JWT

use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use fcp_core::CredentialStore;
use serde::{Deserialize, Serialize};

/// JWT secret — in production this should come from config/env.
/// For now we use the same auth_token or a derived key.
const JWT_SECRET: &[u8] = b"fcp-admin-jwt-secret-change-in-production";
const JWT_EXPIRY_HOURS: i64 = 24;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: AuthUserResponse,
}

#[derive(Debug, Serialize, Clone)]
pub struct AuthUserResponse {
    pub username: String,
    pub role: String,
    pub tenant_id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,       // username
    role: String,
    tenant_id: String,
    display_name: Option<String>,
    exp: usize,        // expiry timestamp
    iat: usize,        // issued at
}

/// POST /api/v1/auth/login
pub async fn login(
    State(state): State<ApiState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state.storage.as_ref().ok_or_else(|| {
        ApiError::Internal("Storage not configured".to_string())
    })?;

    // Verify credentials using the existing CredentialStore
    let valid = storage
        .verify_credentials(&req.username, &req.password)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if !valid {
        return Err(ApiError::Unauthorized("Invalid username or password".to_string()));
    }

    // Get user details
    let user = storage
        .get_user(&req.username)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("User not found".to_string()))?;

    // Check account is active
    if !user.enabled {
        return Err(ApiError::Forbidden("Account is disabled".to_string()));
    }

    // Build JWT claims
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let claims = Claims {
        sub: user.username.clone(),
        role: format!("{:?}", user.role).to_lowercase(),
        tenant_id: user.tenant_id.to_string(),
        display_name: user.display_name.clone(),
        exp: now + (JWT_EXPIRY_HOURS as usize * 3600),
        iat: now,
    };

    // Encode JWT using HMAC-SHA256
    let token = encode_jwt(&claims)?;

    let auth_user = AuthUserResponse {
        username: user.username,
        role: claims.role,
        tenant_id: claims.tenant_id,
        display_name: user.display_name,
    };

    tracing::info!(
        username = %auth_user.username,
        tenant_id = %auth_user.tenant_id,
        "Admin UI login successful"
    );

    Ok((StatusCode::OK, Json(LoginResponse { token, user: auth_user })))
}

/// POST /api/v1/auth/logout
pub async fn logout() -> impl IntoResponse {
    // JWT is stateless — client discards the token.
    // In the future, we could maintain a token blacklist.
    StatusCode::NO_CONTENT
}

/// GET /api/v1/auth/me
pub async fn me(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req)?;

    // Optionally refresh user data from storage
    if let Some(storage) = &state.storage {
        if let Ok(Some(user)) = storage.get_user(&claims.sub).await {
            return Ok(Json(AuthUserResponse {
                username: user.username,
                role: format!("{:?}", user.role).to_lowercase(),
                tenant_id: user.tenant_id.to_string(),
                display_name: user.display_name,
            }));
        }
    }

    // Fallback to claims data
    Ok(Json(AuthUserResponse {
        username: claims.sub,
        role: claims.role,
        tenant_id: claims.tenant_id,
        display_name: claims.display_name,
    }))
}

/// POST /api/v1/auth/refresh
pub async fn refresh(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req)?;

    // Verify user still exists and is active
    if let Some(storage) = &state.storage {
        let user = storage
            .get_user(&claims.sub)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::Unauthorized("User no longer exists".to_string()))?;

        if !user.enabled {
            return Err(ApiError::Forbidden("Account is disabled".to_string()));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;

        let new_claims = Claims {
            sub: user.username.clone(),
            role: format!("{:?}", user.role).to_lowercase(),
            tenant_id: user.tenant_id.to_string(),
            display_name: user.display_name.clone(),
            exp: now + (JWT_EXPIRY_HOURS as usize * 3600),
            iat: now,
        };

        let token = encode_jwt(&new_claims)?;

        return Ok(Json(LoginResponse {
            token,
            user: AuthUserResponse {
                username: user.username,
                role: new_claims.role,
                tenant_id: new_claims.tenant_id,
                display_name: user.display_name,
            },
        }));
    }

    Err(ApiError::Internal("Storage not configured".to_string()))
}

// ---- JWT helpers ----

fn encode_jwt(claims: &Claims) -> Result<String, ApiError> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let header = base64url_encode(b"{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
    let payload = base64url_encode(
        &serde_json::to_vec(claims).map_err(|e| ApiError::Internal(e.to_string()))?,
    );

    let message = format!("{}.{}", header, payload);

    let mut mac = Hmac::<Sha256>::new_from_slice(JWT_SECRET)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    mac.update(message.as_bytes());
    let signature = base64url_encode(&mac.finalize().into_bytes());

    Ok(format!("{}.{}", message, signature))
}

fn decode_jwt(token: &str) -> Result<Claims, ApiError> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(ApiError::Unauthorized("Invalid token format".to_string()));
    }

    // Verify signature
    let message = format!("{}.{}", parts[0], parts[1]);
    let mut mac = Hmac::<Sha256>::new_from_slice(JWT_SECRET)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    mac.update(message.as_bytes());

    let signature = base64url_decode(parts[2])
        .map_err(|_| ApiError::Unauthorized("Invalid token signature".to_string()))?;

    mac.verify_slice(&signature)
        .map_err(|_| ApiError::Unauthorized("Invalid token signature".to_string()))?;

    // Decode payload
    let payload_bytes = base64url_decode(parts[1])
        .map_err(|_| ApiError::Unauthorized("Invalid token payload".to_string()))?;

    let claims: Claims = serde_json::from_slice(&payload_bytes)
        .map_err(|_| ApiError::Unauthorized("Invalid token claims".to_string()))?;

    // Check expiry
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    if claims.exp < now {
        return Err(ApiError::Unauthorized("Token expired".to_string()));
    }

    Ok(claims)
}

fn extract_claims(req: &axum::extract::Request) -> Result<Claims, ApiError> {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("Missing Authorization header".to_string()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::Unauthorized("Invalid Authorization format".to_string()))?;

    decode_jwt(token)
}

fn base64url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn base64url_decode(data: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data)
}
