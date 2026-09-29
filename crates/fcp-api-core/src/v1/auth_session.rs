//! Auth session endpoints for the admin UI.
//!
//! Provides JWT-based authentication:
//! - POST /api/v1/auth/login  — authenticate with username/password, returns JWT
//! - POST /api/v1/auth/logout — client-side token discard
//! - GET  /api/v1/auth/me     — get current user info from JWT
//! - POST /api/v1/auth/change-expired-password — change an expired password

use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use fcp_core::storage::password;
use fcp_core::{AccountStatus, CredentialStore};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

use fcp_core::UserRole;

const JWT_EXPIRY_HOURS: i64 = 24;

/// Serialize a UserRole to its serde snake_case string (e.g. EndUser -> "end_user").
pub fn role_to_string(role: &UserRole) -> String {
    serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| format!("{:?}", role).to_lowercase())
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub token: String,
    pub user: AuthUserResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_expired: Option<bool>,
}

#[derive(Debug, Serialize, Clone, utoipa::ToSchema)]
pub struct AuthUserResponse {
    pub username: String,
    pub role: String,
    pub tenant_id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Claims {
    pub sub: String, // username
    pub role: String,
    pub tenant_id: String,
    pub display_name: Option<String>,
    pub exp: usize, // expiry timestamp
    pub iat: usize, // issued at
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ChangeExpiredPasswordRequest {
    pub username: String,
    pub current_password: String,
    pub new_password: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

fn parse_tenant_id(tenant_id: Option<&str>) -> Result<Option<fcp_core::TenantId>, ApiError> {
    tenant_id
        .filter(|tenant_id| !tenant_id.trim().is_empty())
        .map(|tenant_id| {
            tenant_id
                .parse::<fcp_core::TenantId>()
                .map_err(|e| ApiError::BadRequest(format!("Invalid tenant_id: {}", e)))
        })
        .transpose()
}

async fn load_user_for_login(
    storage: &std::sync::Arc<fcp_core::DynStorage>,
    username: &str,
    tenant_id: Option<&fcp_core::TenantId>,
) -> Result<Option<fcp_core::User>, ApiError> {
    if let Some(tenant_id) = tenant_id {
        return storage
            .get_user_in_tenant(username, tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()));
    }

    let users = storage
        .list_users_by_username(username)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    match users.len() {
        0 => Ok(None),
        1 => Ok(users.into_iter().next()),
        _ => Err(ApiError::BadRequest(
            "tenant_id is required for usernames that exist in multiple tenants".to_string(),
        )),
    }
}

fn claims_tenant_id(claims: &Claims) -> Result<fcp_core::TenantId, ApiError> {
    claims
        .tenant_id
        .parse::<fcp_core::TenantId>()
        .map_err(|e| ApiError::Unauthorized(format!("Invalid tenant in token: {}", e)))
}

/// POST /api/v1/auth/login
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth_session",
    operation_id = "auth_session_login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "OK", body = LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn login(
    State(state): State<ApiState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;

    // Get user first (we need to check lock status before password verification)
    let requested_tenant = parse_tenant_id(req.tenant_id.as_deref())?;

    let mut user =
        match load_user_for_login(storage, &req.username, requested_tenant.as_ref()).await? {
            Some(u) => u,
            None => {
                // Constant-time: still do a bcrypt check to prevent timing attacks
                let _ = password::verify_password(
                    &req.password,
                    "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewY5jtRKm5T6fHGC",
                );
                return Err(ApiError::Unauthorized(
                    "Invalid username or password".to_string(),
                ));
            }
        };

    let now = SystemTime::now();
    let policy = &state.security_policy;

    // Check if account is locked
    if user.account_status == AccountStatus::Locked {
        if let Some(locked_until) = user.account_locked_until {
            if now < locked_until {
                // Still locked — do constant-time bcrypt to prevent timing attacks
                let _ = password::verify_password(&req.password, &user.password);
                return Err(ApiError::Forbidden(
                    "Account is locked. Try again later.".to_string(),
                ));
            }
            // Lock expired — auto-unlock
            user.account_status = AccountStatus::Active;
            user.failed_login_attempts = 0;
            user.account_locked_until = None;
            user.updated_at = now;
            let _ = storage.put_user(user.clone()).await;
            tracing::info!(
                event = "account_auto_unlocked",
                username = %user.username,
                "Account auto-unlocked after lockout expiry"
            );
        } else {
            // Locked without expiry (admin-locked) — still do bcrypt for timing
            let _ = password::verify_password(&req.password, &user.password);
            return Err(ApiError::Forbidden(
                "Account is locked. Contact an administrator.".to_string(),
            ));
        }
    }

    // Check account is enabled
    if !user.enabled {
        let _ = password::verify_password(&req.password, &user.password);
        return Err(ApiError::Forbidden("Account is disabled".to_string()));
    }

    // Check account is not suspended
    if user.account_status == AccountStatus::Suspended {
        let _ = password::verify_password(&req.password, &user.password);
        return Err(ApiError::Forbidden("Account is suspended".to_string()));
    }

    // Verify password
    let valid = password::verify_password(&req.password, &user.password)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if !valid {
        // Increment failed attempts
        user.failed_login_attempts += 1;
        user.updated_at = now;

        if user.failed_login_attempts >= policy.max_failed_attempts as i32 {
            // Lock the account
            user.account_status = AccountStatus::Locked;
            user.account_locked_until =
                Some(now + Duration::from_secs(policy.lockout_duration_secs));
            tracing::warn!(
                event = "account_locked",
                username = %user.username,
                attempts = user.failed_login_attempts,
                lockout_secs = policy.lockout_duration_secs,
                "Account locked after too many failed login attempts"
            );
        }

        let _ = storage.put_user(user).await;
        // Return generic error — don't reveal lockout details to attacker
        return Err(ApiError::Unauthorized(
            "Invalid username or password".to_string(),
        ));
    }

    // Password is correct — reset failed attempts and update last_login
    user.failed_login_attempts = 0;
    user.last_login = Some(now);
    user.updated_at = now;
    let _ = storage.put_user(user.clone()).await;

    // Check password expiration
    let password_expired = if let Some(expiration_days) = policy.password_expiration_days {
        if expiration_days > 0 {
            if let Some(last_reset) = user.password_last_reset {
                let expiry = last_reset + Duration::from_secs(expiration_days as u64 * 86400);
                now > expiry
            } else {
                // No password_last_reset set — consider expired to force initial password set
                true
            }
        } else {
            false
        }
    } else {
        false
    };

    // Build JWT claims
    let epoch_secs = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as usize;

    let claims = Claims {
        sub: user.username.clone(),
        role: role_to_string(&user.role),
        tenant_id: user.tenant_id.to_string(),
        display_name: user.display_name.clone(),
        exp: epoch_secs + (JWT_EXPIRY_HOURS as usize * 3600),
        iat: epoch_secs,
    };

    // Encode JWT using HMAC-SHA256
    let jwt_secret = get_jwt_secret(&state)?;
    let token = encode_jwt(&claims, jwt_secret)?;

    let auth_user = AuthUserResponse {
        username: user.username,
        role: claims.role,
        tenant_id: claims.tenant_id,
        display_name: user.display_name,
    };

    tracing::info!(
        username = %auth_user.username,
        tenant_id = %auth_user.tenant_id,
        password_expired = password_expired,
        "Login successful"
    );

    Ok((
        StatusCode::OK,
        Json(LoginResponse {
            token,
            user: auth_user,
            password_expired: if password_expired { Some(true) } else { None },
        }),
    ))
}

/// POST /api/v1/auth/logout
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth_session",
    operation_id = "auth_session_logout",
    responses(
        (status = 204, description = "Logged out: the client discards its token"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn logout() -> impl IntoResponse {
    // JWT is stateless — client discards the token.
    // In the future, we could maintain a token blacklist.
    StatusCode::NO_CONTENT
}

/// GET /api/v1/auth/me
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "auth_session",
    operation_id = "auth_session_me",
    responses(
        (status = 200, description = "OK", body = AuthUserResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn me(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let tenant_id = claims_tenant_id(&claims)?;

    // Optionally refresh user data from storage
    if let Some(storage) = &state.storage {
        if let Ok(Some(user)) = storage.get_user_in_tenant(&claims.sub, &tenant_id).await {
            return Ok(Json(AuthUserResponse {
                username: user.username,
                role: role_to_string(&user.role),
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
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    tag = "auth_session",
    operation_id = "auth_session_refresh",
    responses(
        (status = 200, description = "OK", body = LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn refresh(
    State(state): State<ApiState>,
    req: axum::extract::Request,
) -> Result<impl IntoResponse, ApiError> {
    let claims = extract_claims(&req, &state)?;
    let tenant_id = claims_tenant_id(&claims)?;

    // Verify user still exists and is active
    if let Some(storage) = &state.storage {
        let user = storage
            .get_user_in_tenant(&claims.sub, &tenant_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::Unauthorized("User no longer exists".to_string()))?;

        if !user.enabled {
            return Err(ApiError::Forbidden("Account is disabled".to_string()));
        }

        // Check lock status on refresh too
        if user.account_status == AccountStatus::Locked {
            return Err(ApiError::Forbidden("Account is locked".to_string()));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;

        let new_claims = Claims {
            sub: user.username.clone(),
            role: role_to_string(&user.role),
            tenant_id: user.tenant_id.to_string(),
            display_name: user.display_name.clone(),
            exp: now + (JWT_EXPIRY_HOURS as usize * 3600),
            iat: now,
        };

        let jwt_secret = get_jwt_secret(&state)?;
        let token = encode_jwt(&new_claims, jwt_secret)?;

        return Ok(Json(LoginResponse {
            token,
            user: AuthUserResponse {
                username: user.username,
                role: new_claims.role,
                tenant_id: new_claims.tenant_id,
                display_name: user.display_name,
            },
            password_expired: None,
        }));
    }

    Err(ApiError::Internal("Storage not configured".to_string()))
}

/// POST /api/v1/auth/change-expired-password
///
/// Allows a user with an expired password to set a new password.
/// Requires current credentials for verification.
#[utoipa::path(
    post,
    path = "/api/v1/auth/change-expired-password",
    tag = "auth_session",
    operation_id = "auth_session_change_expired_password",
    request_body = ChangeExpiredPasswordRequest,
    responses(
        (status = 200, description = "OK", body = LoginResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn change_expired_password(
    State(state): State<ApiState>,
    Json(req): Json<ChangeExpiredPasswordRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;

    // Validate new password is not empty
    if req.new_password.is_empty() {
        return Err(ApiError::BadRequest(
            "New password cannot be empty".to_string(),
        ));
    }

    // Validate new password is different from current
    if req.new_password == req.current_password {
        return Err(ApiError::BadRequest(
            "New password must be different from current password".to_string(),
        ));
    }

    // Get user (with constant-time bcrypt on miss to prevent timing attacks)
    let requested_tenant = parse_tenant_id(req.tenant_id.as_deref())?;

    let mut user =
        match load_user_for_login(storage, &req.username, requested_tenant.as_ref()).await? {
            Some(u) => u,
            None => {
                let _ = password::verify_password(
                    &req.current_password,
                    "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewY5jtRKm5T6fHGC",
                );
                return Err(ApiError::Unauthorized(
                    "Invalid username or password".to_string(),
                ));
            }
        };

    let now = SystemTime::now();
    let policy = &state.security_policy;

    // Check if account is locked
    if user.account_status == AccountStatus::Locked {
        if let Some(locked_until) = user.account_locked_until {
            if now < locked_until {
                let _ = password::verify_password(&req.current_password, &user.password);
                return Err(ApiError::Forbidden(
                    "Account is locked. Try again later.".to_string(),
                ));
            }
            // Lock expired — auto-unlock
            user.account_status = AccountStatus::Active;
            user.failed_login_attempts = 0;
            user.account_locked_until = None;
            user.updated_at = now;
            let _ = storage.put_user(user.clone()).await;
            tracing::info!(
                event = "account_auto_unlocked",
                username = %user.username,
                "Account auto-unlocked after lockout expiry"
            );
        } else {
            let _ = password::verify_password(&req.current_password, &user.password);
            return Err(ApiError::Forbidden(
                "Account is locked. Contact an administrator.".to_string(),
            ));
        }
    }

    // Check account is enabled
    if !user.enabled {
        let _ = password::verify_password(&req.current_password, &user.password);
        return Err(ApiError::Forbidden("Account is disabled".to_string()));
    }

    // Check account is not suspended
    if user.account_status == AccountStatus::Suspended {
        let _ = password::verify_password(&req.current_password, &user.password);
        return Err(ApiError::Forbidden("Account is suspended".to_string()));
    }

    // Verify current password
    let valid = password::verify_password(&req.current_password, &user.password)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if !valid {
        // Increment failed attempts
        user.failed_login_attempts += 1;
        user.updated_at = now;

        if user.failed_login_attempts >= policy.max_failed_attempts as i32 {
            user.account_status = AccountStatus::Locked;
            user.account_locked_until =
                Some(now + Duration::from_secs(policy.lockout_duration_secs));
            tracing::warn!(
                event = "account_locked",
                username = %user.username,
                attempts = user.failed_login_attempts,
                lockout_secs = policy.lockout_duration_secs,
                "Account locked after too many failed password change attempts"
            );
        }

        let _ = storage.put_user(user).await;
        return Err(ApiError::Unauthorized(
            "Invalid username or password".to_string(),
        ));
    }

    // Password correct — reset failed attempts
    user.failed_login_attempts = 0;

    // Verify the password is actually expired (prevent using this as a
    // general password-change path that bypasses the /me/password flow)
    if let Some(expiration_days) = state.security_policy.password_expiration_days {
        if expiration_days > 0 {
            let expired = if let Some(last_reset) = user.password_last_reset {
                let expiry = last_reset + Duration::from_secs(expiration_days as u64 * 86400);
                now > expiry
            } else {
                true // No reset date → treat as expired
            };
            if !expired {
                return Err(ApiError::BadRequest(
                    "Password is not expired. Use the regular password change endpoint."
                        .to_string(),
                ));
            }
        } else {
            return Err(ApiError::BadRequest(
                "Password expiration is not enabled".to_string(),
            ));
        }
    } else {
        return Err(ApiError::BadRequest(
            "Password expiration is not enabled".to_string(),
        ));
    }

    // Hash new password
    let password_hash = password::hash_password(&req.new_password)
        .map_err(|e| ApiError::Internal(format!("Failed to hash password: {}", e)))?;

    // Update user
    user.password = password_hash;
    if user.digest_password.is_none() {
        user.sip_ha1_hash = Some(password::compute_ha1(
            &req.username,
            &state.realm,
            &req.new_password,
        ));
        user.sip_provisioning_password = None;
    }
    user.password_last_reset = Some(now);
    user.last_login = Some(now);
    user.updated_at = now;

    storage
        .put_user(user.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    tracing::warn!(
        event = "expired_password_changed",
        username = %user.username,
        "User changed expired password"
    );

    // Issue JWT token so user can proceed
    let epoch_secs = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as usize;

    let claims = Claims {
        sub: user.username.clone(),
        role: role_to_string(&user.role),
        tenant_id: user.tenant_id.to_string(),
        display_name: user.display_name.clone(),
        exp: epoch_secs + (JWT_EXPIRY_HOURS as usize * 3600),
        iat: epoch_secs,
    };

    let jwt_secret = get_jwt_secret(&state)?;
    let token = encode_jwt(&claims, jwt_secret)?;

    let auth_user = AuthUserResponse {
        username: user.username,
        role: claims.role,
        tenant_id: claims.tenant_id,
        display_name: user.display_name,
    };

    Ok((
        StatusCode::OK,
        Json(LoginResponse {
            token,
            user: auth_user,
            password_expired: None,
        }),
    ))
}

// ---- JWT helpers ----

/// Get the JWT secret from ApiState, or return an error.
fn get_jwt_secret(state: &ApiState) -> Result<&[u8], ApiError> {
    state
        .jwt_secret
        .as_deref()
        .ok_or_else(|| ApiError::Internal("JWT secret not configured".to_string()))
}

pub(crate) fn encode_jwt(claims: &Claims, secret: &[u8]) -> Result<String, ApiError> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let header = base64url_encode(b"{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
    let payload = base64url_encode(
        &serde_json::to_vec(claims).map_err(|e| ApiError::Internal(e.to_string()))?,
    );

    let message = format!("{}.{}", header, payload);

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|e| ApiError::Internal(e.to_string()))?;
    mac.update(message.as_bytes());
    let signature = base64url_encode(&mac.finalize().into_bytes());

    Ok(format!("{}.{}", message, signature))
}

pub(crate) fn decode_jwt(token: &str, secret: &[u8]) -> Result<Claims, ApiError> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(ApiError::Unauthorized("Invalid token format".to_string()));
    }

    // Verify signature
    let message = format!("{}.{}", parts[0], parts[1]);
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|e| ApiError::Internal(e.to_string()))?;
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

pub fn extract_claims(req: &axum::extract::Request, state: &ApiState) -> Result<Claims, ApiError> {
    extract_claims_from_headers(req.headers(), state)
}

/// The caller of a `/me` route, as the claims a session carries: the
/// session's own, or — for a personal API token (docs/DEVELOPER_SURFACE.md
/// §3, phase 1) — its maker, as an end user. A personal token has no expiry
/// in these claims; the bearer layer has already judged it.
pub struct Me(pub Claims);

#[axum::async_trait]
impl axum::extract::FromRequestParts<ApiState> for Me {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &ApiState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(crate::auth::AuthIdentity::Personal {
            username,
            tenant_id,
            ..
        }) = parts.extensions.get::<crate::auth::AuthIdentity>()
        {
            return Ok(Me(Claims {
                sub: username.clone(),
                role: "end_user".to_string(),
                tenant_id: tenant_id.to_string(),
                display_name: None,
                exp: 0,
                iat: 0,
            }));
        }
        extract_claims_from_headers(&parts.headers, state).map(Me)
    }
}

/// Extract JWT claims from a `HeaderMap` (useful when Request is not available).
pub fn extract_claims_from_headers(
    headers: &axum::http::HeaderMap,
    state: &ApiState,
) -> Result<Claims, ApiError> {
    let secret = get_jwt_secret(state)?;
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("Missing Authorization header".to_string()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::Unauthorized("Invalid Authorization format".to_string()))?;

    decode_jwt(token, secret)
}

fn base64url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn base64url_decode(data: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data)
}
