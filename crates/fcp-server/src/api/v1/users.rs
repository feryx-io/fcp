//! User management API endpoints.

use crate::api::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::{storage::password, CredentialStore, User};
use serde::{Deserialize, Serialize};

/// User response (excludes password hash).
#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub username: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
    pub enabled: bool,
    pub created_at: std::time::SystemTime,
    pub updated_at: std::time::SystemTime,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            username: user.username,
            display_name: user.display_name,
            tenant_id: user.tenant_id.to_string(),
            enabled: user.enabled,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// Create user request.
#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: fcp_core::TenantId,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_tenant_id() -> fcp_core::TenantId {
    fcp_core::TenantId::nil()
}

fn default_true() -> bool {
    true
}

/// Update user request.
#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub enabled: Option<bool>,
}

/// Update password request.
#[derive(Debug, Deserialize)]
pub struct UpdatePasswordRequest {
    pub password: String,
}

/// List all users.
///
/// GET /api/v1/users
pub async fn list_users(
    State(state): State<ApiState>,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    let users = state.storage.list_users().await?;
    let responses: Vec<UserResponse> = users.into_iter().map(UserResponse::from).collect();
    Ok(Json(responses))
}

/// Get a specific user.
///
/// GET /api/v1/users/:username
pub async fn get_user(
    State(state): State<ApiState>,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state
        .storage
        .get_user(&username)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))?;

    Ok(Json(UserResponse::from(user)))
}

/// Create a new user.
///
/// POST /api/v1/users
pub async fn create_user(
    State(state): State<ApiState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Check if user already exists
    if let Some(_) = state.storage.get_user(&req.username).await? {
        return Err(ApiError::Conflict(format!(
            "User '{}' already exists",
            req.username
        )));
    }

    // Hash the password
    let password_hash = password::hash_password(&req.password)
        .map_err(|e| ApiError::Internal(format!("Failed to hash password: {}", e)))?;

    // Create user
    let user = User {
        username: req.username.clone(),
        password: password_hash,
        display_name: req.display_name,
        tenant_id: req.tenant_id,
        enabled: req.enabled,
        created_at: std::time::SystemTime::now(),
        updated_at: std::time::SystemTime::now(),
    };

    state.storage.put_user(user.clone()).await?;

    Ok((StatusCode::CREATED, Json(UserResponse::from(user))))
}

/// Update a user.
///
/// PUT /api/v1/users/:username
pub async fn update_user(
    State(state): State<ApiState>,
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    // Get existing user
    let mut user = state
        .storage
        .get_user(&username)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))?;

    // Update fields
    if let Some(display_name) = req.display_name {
        user.display_name = Some(display_name);
    }
    if let Some(enabled) = req.enabled {
        user.enabled = enabled;
    }
    user.updated_at = std::time::SystemTime::now();

    // Save user
    state.storage.put_user(user.clone()).await?;

    Ok(Json(UserResponse::from(user)))
}

/// Update a user's password.
///
/// PUT /api/v1/users/:username/password
pub async fn update_password(
    State(state): State<ApiState>,
    Path(username): Path<String>,
    Json(req): Json<UpdatePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    // Get existing user
    let mut user = state
        .storage
        .get_user(&username)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))?;

    // Hash new password
    let password_hash = password::hash_password(&req.password)
        .map_err(|e| ApiError::Internal(format!("Failed to hash password: {}", e)))?;

    // Update password
    user.password = password_hash;
    user.updated_at = std::time::SystemTime::now();

    // Save user
    state.storage.put_user(user).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Delete a user.
///
/// DELETE /api/v1/users/:username
pub async fn delete_user(
    State(state): State<ApiState>,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    // Check if user exists
    if state.storage.get_user(&username).await?.is_none() {
        return Err(ApiError::NotFound(format!(
            "User '{}' not found",
            username
        )));
    }

    // Delete user
    state.storage.delete_user(&username).await?;

    Ok(StatusCode::NO_CONTENT)
}
