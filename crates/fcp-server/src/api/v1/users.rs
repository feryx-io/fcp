//! User management API endpoints.

use crate::api::{error::ApiError, state::ApiState, validation};
use tracing::{info, warn};

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("User management requires registrar to be enabled".to_string())
        })?
    };
}
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::{storage::password, AccountStatus, CredentialStore, EmergencyContact, User};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// User response (excludes password hash).
#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub username: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
    pub enabled: bool,
    pub created_at: std::time::SystemTime,
    pub updated_at: std::time::SystemTime,

    // Enterprise fields
    pub email: Option<String>,
    pub password_last_reset: Option<std::time::SystemTime>,
    pub phone_number: Option<String>,
    pub mobile_number: Option<String>,
    pub extension: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    pub location: Option<String>,
    pub language: String,
    pub timezone: String,
    pub account_status: String,
    pub last_login: Option<std::time::SystemTime>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
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
            // Enterprise fields
            email: user.email,
            password_last_reset: user.password_last_reset,
            phone_number: user.phone_number,
            mobile_number: user.mobile_number,
            extension: user.extension,
            department: user.department,
            job_title: user.job_title,
            location: user.location,
            language: user.language,
            timezone: user.timezone,
            account_status: user.account_status.to_string(),
            last_login: user.last_login,
            emergency_contact_name: user.emergency_contact.name,
            emergency_contact_phone: user.emergency_contact.phone,
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
    #[serde(default)]
    pub media_preferences: fcp_core::MediaPreferences,

    // Enterprise fields
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub mobile_number: Option<String>,
    pub pin: Option<String>,
    pub extension: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    pub location: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
    pub account_status: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
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

    // Enterprise fields
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub mobile_number: Option<String>,
    pub pin: Option<String>,
    pub extension: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    pub location: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
    pub account_status: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
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
    let storage = require_storage!(state);
    let users = storage.list_users().await?;
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
    validation::validate_username(&username)?;

    let storage = require_storage!(state);
    let user = storage
        .get_user(&username)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))?;

    Ok(Json(UserResponse::from(user)))
}

/// Create a new user.
///
/// POST /api/v1/users
///
/// **Security Note**: This endpoint uses constant-time operations to prevent
/// user enumeration attacks. Even if a user exists, we perform password hashing
/// before returning an error to match the timing of successful creation.
pub async fn create_user(
    State(state): State<ApiState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    validation::validate_username(&req.username)?;
    validation::validate_display_name(&req.display_name)?;
    validation::reject_crlf("password", &req.password)?;

    // Always hash the password first (constant-time operation regardless of user existence)
    // This prevents timing attacks that could reveal which usernames exist
    let password_hash = password::hash_password(&req.password)
        .map_err(|e| ApiError::BadRequest(format!("Invalid password: {}", e)))?;

    // Check if user already exists
    if let Some(_) = storage.get_user(&req.username).await? {
        // Use generic error message to avoid user enumeration
        return Err(ApiError::Conflict(
            "Unable to create user with provided credentials".to_string(),
        ));
    }

    // Hash PIN if provided
    let pin_hash = if let Some(ref pin) = req.pin {
        Some(
            password::hash_password(pin)
                .map_err(|e| ApiError::BadRequest(format!("Invalid PIN: {}", e)))?,
        )
    } else {
        None
    };

    // Parse account status
    let account_status = if let Some(status_str) = &req.account_status {
        AccountStatus::from_str(status_str).unwrap_or(AccountStatus::Active)
    } else {
        AccountStatus::Active
    };

    // Create emergency contact
    let emergency_contact = EmergencyContact {
        name: req.emergency_contact_name,
        phone: req.emergency_contact_phone,
    };

    // Create user
    let user = User {
        username: req.username.clone(),
        password: password_hash,
        display_name: req.display_name,
        tenant_id: req.tenant_id,
        enabled: req.enabled,
        created_at: std::time::SystemTime::now(),
        updated_at: std::time::SystemTime::now(),
        media_preferences: req.media_preferences,
        email: req.email,
        password_last_reset: None,
        phone_number: req.phone_number,
        mobile_number: req.mobile_number,
        pin: pin_hash,
        extension: req.extension,
        department: req.department,
        job_title: req.job_title,
        location: req.location,
        language: req.language.unwrap_or_else(|| "en-US".to_string()),
        timezone: req.timezone.unwrap_or_else(|| "UTC".to_string()),
        account_status,
        last_login: None,
        emergency_contact,
    };

    storage.put_user(user.clone()).await?;

    // Security audit log
    info!(
        event = "user_created",
        username = %user.username,
        tenant_id = %user.tenant_id,
        enabled = user.enabled,
        "User account created"
    );

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
    validation::validate_username(&username)?;
    validation::validate_display_name(&req.display_name)?;

    // Get existing user
    let storage = require_storage!(state);
    let mut user = storage
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

    // Update enterprise fields
    if let Some(email) = req.email {
        user.email = Some(email);
    }
    if let Some(phone_number) = req.phone_number {
        user.phone_number = Some(phone_number);
    }
    if let Some(mobile_number) = req.mobile_number {
        user.mobile_number = Some(mobile_number);
    }
    if let Some(pin) = req.pin {
        // Hash the PIN
        let pin_hash = password::hash_password(&pin)
            .map_err(|e| ApiError::BadRequest(format!("Invalid PIN: {}", e)))?;
        user.pin = Some(pin_hash);
    }
    if let Some(extension) = req.extension {
        user.extension = Some(extension);
    }
    if let Some(department) = req.department {
        user.department = Some(department);
    }
    if let Some(job_title) = req.job_title {
        user.job_title = Some(job_title);
    }
    if let Some(location) = req.location {
        user.location = Some(location);
    }
    if let Some(language) = req.language {
        user.language = language;
    }
    if let Some(timezone) = req.timezone {
        user.timezone = timezone;
    }
    if let Some(account_status_str) = req.account_status {
        user.account_status = AccountStatus::from_str(&account_status_str)
            .unwrap_or(AccountStatus::Active);
    }
    if let Some(emergency_name) = req.emergency_contact_name {
        user.emergency_contact.name = Some(emergency_name);
    }
    if let Some(emergency_phone) = req.emergency_contact_phone {
        user.emergency_contact.phone = Some(emergency_phone);
    }

    user.updated_at = std::time::SystemTime::now();

    // Save user
    require_storage!(state).put_user(user.clone()).await?;

    // Security audit log
    info!(
        event = "user_updated",
        username = %username,
        "User account updated"
    );

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
    validation::validate_username(&username)?;
    validation::reject_crlf("password", &req.password)?;

    // Get existing user
    let storage = require_storage!(state);
    let mut user = storage
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
    require_storage!(state).put_user(user).await?;

    // Security audit log
    warn!(
        event = "password_changed",
        username = %username,
        "User password changed"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Delete a user.
///
/// DELETE /api/v1/users/:username
pub async fn delete_user(
    State(state): State<ApiState>,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    validation::validate_username(&username)?;

    // Check if user exists
    if require_storage!(state).get_user(&username).await?.is_none() {
        return Err(ApiError::NotFound(format!("User '{}' not found", username)));
    }

    // Delete user
    require_storage!(state).delete_user(&username).await?;

    // Security audit log
    warn!(
        event = "user_deleted",
        username = %username,
        "User account deleted"
    );

    Ok(StatusCode::NO_CONTENT)
}
