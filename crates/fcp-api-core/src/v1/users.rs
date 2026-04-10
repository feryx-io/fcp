//! User management API endpoints.

use crate::{error::ApiError, state::ApiState, validation};
use fcp_events::{PlatformEvent, SystemEvent, UserEvent};
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
use fcp_core::{
    storage::password, AccountStatus, CredentialStore, EmergencyContact, SiteLocationStore, User,
    UserRole,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// User response (excludes password hash).
#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
    pub enabled: bool,
    pub role: String, // Role name (e.g., "super_admin", "telecom_admin")
    pub department_id: Option<String>, // For department-scoped admins
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
    pub location_id: Option<String>,
    pub language: String,
    pub timezone: String,
    pub account_status: String,
    pub last_login: Option<std::time::SystemTime>,
    pub failed_login_attempts: i32,
    pub account_locked_until: Option<std::time::SystemTime>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        // Serialize role to proper snake_case string using serde (e.g., "telecom_admin", not "telecomadmin")
        let role_str = serde_json::to_string(&user.role)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "end_user".to_string());
        Self {
            id: user.id.to_string(),
            username: user.username,
            display_name: user.display_name,
            tenant_id: user.tenant_id.to_string(),
            enabled: user.enabled,
            role: role_str,
            department_id: user.department_id,
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
            location_id: user.location_id.map(|id| id.to_string()),
            language: user.language,
            timezone: user.timezone,
            account_status: user.account_status.to_string(),
            last_login: user.last_login,
            failed_login_attempts: user.failed_login_attempts,
            account_locked_until: user.account_locked_until,
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
    pub role: Option<String>, // Role name (e.g., "super_admin", "telecom_admin")
    pub department_id: Option<String>, // For department-scoped admins
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
    pub location_id: Option<String>,
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

fn ensure_user_in_tenant(
    user: User,
    tenant_id: &fcp_core::TenantId,
    username: &str,
) -> Result<User, ApiError> {
    if &user.tenant_id == tenant_id {
        Ok(user)
    } else {
        Err(ApiError::NotFound(format!("User '{}' not found", username)))
    }
}

/// Parse role string to UserRole enum.
fn parse_user_role(role_str: Option<&str>) -> Result<UserRole, ApiError> {
    match role_str {
        Some(s) => {
            // Try deserializing with serde first (handles snake_case like "telecom_admin")
            if let Ok(role) = serde_json::from_str::<UserRole>(&format!("\"{}\"", s)) {
                return Ok(role);
            }

            // Fallback to manual matching for backward compatibility
            match s.to_lowercase().as_str() {
                "superadmin" | "super_admin" | "admin" => Ok(UserRole::SuperAdmin),
                "tenantadmin" | "tenant_admin" => Ok(UserRole::TenantAdmin),
                "telecomadmin" | "telecom_admin" => Ok(UserRole::TelecomAdmin),
                "useradmin" | "user_admin" => Ok(UserRole::UserAdmin),
                "departmentadmin" | "department_admin" => Ok(UserRole::DepartmentAdmin),
                "readonly" | "read_only" => Ok(UserRole::ReadOnly),
                "enduser" | "end_user" | "user" => Ok(UserRole::EndUser),
                _ => Err(ApiError::BadRequest(format!("Invalid role: {}", s))),
            }
        }
        None => Ok(UserRole::EndUser), // Default to EndUser if not specified
    }
}

/// Update user request.
#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub enabled: Option<bool>,
    pub role: Option<String>,
    pub department_id: Option<String>,

    // Enterprise fields
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub mobile_number: Option<String>,
    pub pin: Option<String>,
    pub extension: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    pub location: Option<String>,
    pub location_id: Option<String>,
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

/// Update digest password request (for Cisco-style separate SIP credentials).
#[derive(Debug, Deserialize)]
pub struct UpdateDigestPasswordRequest {
    pub digest_password: String,
}

/// List all users.
///
/// GET /api/v1/users
pub async fn list_users(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    let storage = require_storage!(state);
    let users = storage
        .list_users()
        .await?
        .into_iter()
        .filter(|user| user.tenant_id == tenant.0)
        .collect::<Vec<_>>();
    let responses: Vec<UserResponse> = users.into_iter().map(UserResponse::from).collect();
    Ok(Json(responses))
}

/// Get a specific user.
///
/// GET /api/v1/users/:username
pub async fn get_user(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, ApiError> {
    validation::validate_username(&username)?;

    let storage = require_storage!(state);
    let user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;

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
    tenant: crate::auth::TenantContext,
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

    // Compute SIP HA1 hash for digest authentication
    let sip_ha1_hash = password::compute_ha1(&req.username, &state.realm, &req.password);

    // Check if user already exists
    if let Some(_) = storage.get_user_in_tenant(&req.username, &tenant.0).await? {
        // Use generic error message to avoid user enumeration
        return Err(ApiError::Conflict(
            "Unable to create user with provided credentials".to_string(),
        ));
    }

    // Hash PIN if provided
    let pin_hash = if let Some(ref pin) = req.pin {
        Some(
            password::hash_pin(pin)
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

    // Parse role from request
    let role = parse_user_role(req.role.as_deref())?;

    // Create user
    let user = User {
        id: fcp_core::UserId::new(),
        username: req.username.clone(),
        password: password_hash,
        digest_password: None, // Can be set separately via API for Cisco-style security
        sip_ha1_hash: Some(sip_ha1_hash),
        display_name: req.display_name,
        tenant_id: tenant.0.clone(),
        enabled: req.enabled,
        role,                             // NEW: User role for RBAC
        department_id: req.department_id, // NEW: Department scoping for department admins
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
        location_id: if let Some(location_id_str) = req.location_id {
            let location_id = fcp_core::LocationId::from_str(&location_id_str)
                .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {}", e)))?;
            let location = storage
                .get_location(&location_id)
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to get location: {}", e)))?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("Location {} not found", location_id))
                })?;
            if location.tenant_id != tenant.0 {
                return Err(ApiError::BadRequest(format!(
                    "Location {} is not in the authenticated tenant",
                    location_id
                )));
            }
            Some(location_id)
        } else {
            None
        },
        language: req.language.unwrap_or_else(|| "en-US".to_string()),
        timezone: req.timezone.unwrap_or_else(|| "UTC".to_string()),
        account_status,
        last_login: None,
        emergency_contact,
        primary_phone_number_id: None,
        voicemail_box_id: None,
        profile_picture: None,
        failed_login_attempts: 0,
        account_locked_until: None,
    };

    storage.put_user(user.clone()).await?;

    // Publish user created event
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Created {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: user.id.to_string(),
            tenant_id: user.tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

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
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    validation::validate_username(&username)?;
    validation::validate_display_name(&req.display_name)?;

    // Get existing user
    let storage = require_storage!(state);
    let mut user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;

    // Update fields
    if let Some(display_name) = req.display_name {
        user.display_name = Some(display_name);
    }
    if let Some(enabled) = req.enabled {
        user.enabled = enabled;
    }
    if let Some(role_str) = &req.role {
        let new_role = parse_user_role(Some(role_str.as_str()))?;
        let old_role = user.role;
        user.role = new_role;

        // Security audit log for role changes
        if old_role != new_role {
            warn!(
                event = "role_changed",
                username = %username,
                old_role = ?old_role,
                new_role = ?new_role,
                "User role changed"
            );
        }
    }
    if let Some(department_id) = req.department_id {
        user.department_id = if department_id.is_empty() {
            None
        } else {
            Some(department_id)
        };
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
        let pin_hash = password::hash_pin(&pin)
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
    if let Some(location_id_str) = req.location_id {
        user.location_id = if location_id_str.is_empty() {
            None
        } else {
            let location_id = fcp_core::LocationId::from_str(&location_id_str)
                .map_err(|e| ApiError::BadRequest(format!("Invalid location ID: {}", e)))?;
            let location = storage
                .get_location(&location_id)
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to get location: {}", e)))?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("Location {} not found", location_id))
                })?;
            if location.tenant_id != tenant.0 {
                return Err(ApiError::BadRequest(format!(
                    "Location {} is not in the authenticated tenant",
                    location_id
                )));
            }
            Some(location_id)
        };
    }
    if let Some(language) = req.language {
        user.language = language;
    }
    if let Some(timezone) = req.timezone {
        user.timezone = timezone;
    }
    if let Some(account_status_str) = req.account_status {
        user.account_status =
            AccountStatus::from_str(&account_status_str).unwrap_or(AccountStatus::Active);
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

    // Publish user updated event
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Updated {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            changed_fields: vec![], // TODO: track which fields changed
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: user.id.to_string(),
            tenant_id: user.tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

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
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Json(req): Json<UpdatePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    validation::validate_username(&username)?;
    validation::reject_crlf("password", &req.password)?;

    // Get existing user
    let storage = require_storage!(state);
    let mut user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;

    // Hash new password
    let password_hash = password::hash_password(&req.password)
        .map_err(|e| ApiError::Internal(format!("Failed to hash password: {}", e)))?;

    // Compute SIP HA1 hash for digest authentication
    let sip_ha1_hash = password::compute_ha1(&username, &state.realm, &req.password);

    // Update password and HA1 hash
    user.password = password_hash;
    user.sip_ha1_hash = Some(sip_ha1_hash);
    user.updated_at = std::time::SystemTime::now();

    let tenant_id = user.tenant_id;
    let user_id_str = user.id.to_string();

    // Save user
    require_storage!(state).put_user(user).await?;

    // Publish ConfigChanged event for cache invalidation (credentials changed)
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: user_id_str,
            tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

    // Security audit log
    warn!(
        event = "password_changed",
        username = %username,
        "User password changed"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Update digest password (Cisco-style separate SIP credentials).
///
/// PUT /api/v1/users/:username/digest-password
///
/// This sets a separate password for SIP authentication, providing security isolation
/// between API/web access and SIP phone registration (Cisco CUCM style).
///
/// **Security Benefits:**
/// - Compromised SIP credentials don't expose API/admin access
/// - Different password policies for phones vs admin users
/// - Separate rotation schedules for SIP and API credentials
///
/// **Note:** If digest_password is not set (NULL), the main password is used for SIP.
pub async fn update_digest_password(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Json(req): Json<UpdateDigestPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    validation::validate_username(&username)?;
    validation::reject_crlf("digest_password", &req.digest_password)?;

    // Get existing user
    let storage = require_storage!(state);
    let mut user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;

    // Hash new digest password for audit/display purposes
    let digest_password_hash = password::hash_password(&req.digest_password)
        .map_err(|e| ApiError::Internal(format!("Failed to hash digest password: {}", e)))?;

    // Compute SIP HA1 hash from digest password (NOT main password) - legacy compatibility
    let sip_ha1_hash = password::compute_ha1(&username, &state.realm, &req.digest_password);

    // Update digest password and HA1 hash (no plaintext storage for security)
    user.digest_password = Some(digest_password_hash);
    user.sip_ha1_hash = Some(sip_ha1_hash);
    user.updated_at = std::time::SystemTime::now();

    let tenant_id = user.tenant_id;
    let user_id_str = user.id.to_string();

    // Save user
    require_storage!(state).put_user(user).await?;

    // Publish ConfigChanged event for cache invalidation (credentials changed)
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: user_id_str,
            tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

    // Security audit log
    info!(
        event = "digest_password_set",
        username = %username,
        "User digest password set (Cisco-style separate SIP credentials)"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Delete digest password (revert to using main password for SIP).
///
/// DELETE /api/v1/users/:username/digest-password
///
/// Removes the separate digest password and reverts to using the main password
/// for SIP authentication. The HA1 hash will be recomputed from the main password.
pub async fn delete_digest_password(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    validation::validate_username(&username)?;

    // Get existing user
    let storage = require_storage!(state);
    let mut user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;

    // Check if digest password is set
    if user.digest_password.is_none() {
        return Err(ApiError::NotFound(
            "User does not have a separate digest password".to_string(),
        ));
    }

    // Remove digest password
    user.digest_password = None;

    // Note: We cannot recompute HA1 hash from main password because we only have the bcrypt hash
    // The HA1 hash will need to be recomputed when the user next updates their main password
    // or we need to prompt for the plaintext password
    // For now, clear the HA1 hash to force re-authentication setup
    user.sip_ha1_hash = None;
    user.updated_at = std::time::SystemTime::now();

    let tenant_id = user.tenant_id;
    let user_id_str = user.id.to_string();

    // Save user
    require_storage!(state).put_user(user).await?;

    // Publish ConfigChanged event for cache invalidation (credentials changed)
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: user_id_str,
            tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

    // Security audit log
    warn!(
        event = "digest_password_removed",
        username = %username,
        "User digest password removed, reverted to main password for SIP (HA1 hash cleared)"
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Unlock a user account.
///
/// POST /api/v1/users/:username/unlock
///
/// Resets failed login attempts, clears lockout, and sets account status to Active.
pub async fn unlock_user(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, ApiError> {
    validation::validate_username(&username)?;

    let storage = require_storage!(state);
    let mut user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;

    user.failed_login_attempts = 0;
    user.account_locked_until = None;
    user.account_status = AccountStatus::Active;
    user.updated_at = std::time::SystemTime::now();

    storage.put_user(user.clone()).await?;

    warn!(
        event = "account_unlocked",
        username = %username,
        "User account unlocked by admin"
    );

    Ok(Json(UserResponse::from(user)))
}

/// Delete a user.
///
/// DELETE /api/v1/users/:username
pub async fn delete_user(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    validation::validate_username(&username)?;

    // Get user before deletion for event data
    let storage = require_storage!(state);
    let user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;

    // Delete user
    storage.delete_user_in_tenant(&username, &tenant.0).await?;

    // Publish user deleted event
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Deleted {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

    // Publish ConfigChanged event for cache invalidation
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: user.id.to_string(),
            tenant_id: user.tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

    // Security audit log
    warn!(
        event = "user_deleted",
        username = %username,
        "User account deleted"
    );

    Ok(StatusCode::NO_CONTENT)
}
