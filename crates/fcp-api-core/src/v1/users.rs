//! User management API endpoints.

use crate::{auth::AuthIdentity, error::ApiError, state::ApiState, validation};
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
    Extension, Json,
};
use fcp_core::{
    permissions::Permission, storage::password, AccountStatus, CredentialStore, EmergencyContact,
    SiteLocationStore, User, UserRole,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// User response (excludes password hash).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
    pub enabled: bool,
    pub role: String, // Role name (e.g., "super_admin", "telecom_admin")
    pub department_id: Option<String>, // For department-scoped admins
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: std::time::SystemTime,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: std::time::SystemTime,

    // Enterprise fields
    pub email: Option<String>,
    #[serde(default, with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
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
    #[serde(default, with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_login: Option<std::time::SystemTime>,
    pub failed_login_attempts: i32,
    #[serde(default, with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub account_locked_until: Option<std::time::SystemTime>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
    /// The name the user's phones register and authenticate as: their
    /// username, or the one they had before a rename (`docs/USERS.md` §6).
    pub sip_username: String,
    /// When the user was deleted; they are purged the tenant's
    /// `offboarding.grace_days` later unless restored.
    #[serde(default, with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub deprovisioned_at: Option<std::time::SystemTime>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        // Serialize role to proper snake_case string using serde (e.g., "telecom_admin", not "telecomadmin")
        let role_str = serde_json::to_string(&user.role)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "end_user".to_string());
        let sip_username = user.sip_name().to_string();
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
            sip_username,
            deprovisioned_at: user.deprovisioned_at,
        }
    }
}

/// Create user request.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateUserRequest {
    pub username: String,
    /// The web password. Left out, the user has none and signs in only
    /// through the tenant's identity provider (`docs/IDENTITY_AND_DEVICES.md`
    /// phase 2); their SIP password is then generated.
    #[serde(default)]
    pub password: Option<String>,
    /// Optional SIP password retained for device auto-provisioning templates.
    ///
    /// When set, this becomes the user's SIP digest password and is exposed by
    /// provisioning as `${sip_password}` for vendors that cannot use HA1.
    pub sip_provisioning_password: Option<String>,
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
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateUserRequest {
    /// A new username: the user is renamed (`docs/USERS.md` §6). Their
    /// phones keep registering under their SIP name, which stays.
    #[serde(default)]
    pub username: Option<String>,
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
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePasswordRequest {
    pub password: String,
}

/// Update digest password request (for Cisco-style separate SIP credentials).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateDigestPasswordRequest {
    pub digest_password: String,
    /// Retain the plaintext SIP password for auto-provisioning templates.
    ///
    /// Defaults to true because most desk-phone provisioning templates require
    /// a raw SIP password rather than HA1/precomputed digest material.
    #[serde(default = "default_true")]
    pub store_for_provisioning: bool,
}

/// List all users.
///
/// GET /api/v1/users
#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "users",
    operation_id = "users_list_users",
    params(crate::page::PageQuery, UserListQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<UserResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_users(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
    axum::extract::Query(filter): axum::extract::Query<UserListQuery>,
) -> Result<Json<crate::page::Page<UserResponse>>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let filter = fcp_core::storage::UserFilter {
        search: filter.q.filter(|q| !q.trim().is_empty()),
        status: filter
            .status
            .as_deref()
            .map(parse_account_status)
            .transpose()?,
    };
    let users = storage.list_users_in_tenant(&tenant.0, &filter).await?;
    let responses: Vec<UserResponse> = users.into_iter().map(UserResponse::from).collect();
    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.username, r.id),
    )?))
}

/// Get a specific user.
///
/// GET /api/v1/users/:username
#[utoipa::path(
    get,
    path = "/api/v1/users/{username}",
    tag = "users",
    operation_id = "users_get_user",
    params(("username" = String, Path)),
    responses(
        (status = 200, description = "OK", body = UserResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserRead, state.deployment_mode)?;
    validation::validate_username(&username)?;

    let storage = require_storage!(state);
    let user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;

    Ok(Json(UserResponse::from(user)))
}

/// A user by id (`docs/IDENTITY_AND_DEVICES.md` phase 1): what a token's
/// maker, a session or an event names.
///
/// GET /api/v1/users/by-id/:id
#[utoipa::path(
    get,
    path = "/api/v1/users/by-id/{id}",
    tag = "users",
    operation_id = "users_get_user_by_id",
    params(("id" = String, Path, description = "`user-<uuid>`")),
    responses(
        (status = 200, description = "OK", body = UserResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_user_by_id(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<UserResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserRead, state.deployment_mode)?;
    let not_found = || ApiError::NotFound(format!("User '{id}' not found"));
    let user_id = id.parse::<fcp_core::UserId>().map_err(|_| not_found())?;
    let user = require_storage!(state)
        .get_user_by_id(&user_id)
        .await?
        .filter(|user| user.tenant_id == tenant.0)
        .ok_or_else(not_found)?;
    Ok(Json(UserResponse::from(user)))
}

/// Create a new user.
///
/// POST /api/v1/users
///
/// **Security Note**: This endpoint uses constant-time operations to prevent
/// user enumeration attacks. Even if a user exists, we perform password hashing
/// before returning an error to match the timing of successful creation.
#[utoipa::path(
    post,
    path = "/api/v1/users",
    tag = "users",
    operation_id = "users_create_user",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "OK", body = UserResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserCreate, state.deployment_mode)?;
    let storage = require_storage!(state);

    validation::validate_username(&req.username)?;
    refuse_id_shaped(&req.username)?;
    validation::validate_display_name(&req.display_name)?;
    if let Some(ref web_password) = req.password {
        validation::reject_crlf("password", web_password)?;
    }
    if let Some(ref sip_password) = req.sip_provisioning_password {
        validation::reject_crlf("sip_provisioning_password", sip_password)?;
    }

    // Always hash a password first (constant-time operation regardless of user existence)
    // This prevents timing attacks that could reveal which usernames exist
    let password_hash = match &req.password {
        Some(web_password) => password::hash_password(web_password)
            .map_err(|e| ApiError::BadRequest(format!("Invalid password: {}", e)))?,
        // No web password: nothing a password can match.
        None => {
            let _ = password::hash_password(&generated_sip_password());
            String::new()
        }
    };
    // A user without a web password still has a phone: a SIP password of
    // their own, generated, as a separate digest password.
    let sip_password = req
        .sip_provisioning_password
        .clone()
        .or_else(|| req.password.is_none().then(generated_sip_password));

    let (digest_password_hash, sip_ha1_hash, sip_provisioning_password) =
        if let Some(ref sip_password) = sip_password {
            let digest_hash = password::hash_password(sip_password).map_err(|e| {
                ApiError::BadRequest(format!("Invalid SIP provisioning password: {}", e))
            })?;
            (
                Some(digest_hash),
                password::compute_ha1(&req.username, &state.realm, sip_password),
                Some(sip_password.clone()),
            )
        } else {
            (
                None,
                password::compute_ha1(
                    &req.username,
                    &state.realm,
                    req.password.as_deref().unwrap_or_default(),
                ),
                None,
            )
        };

    ensure_room_for_a_user(&state, &tenant.0).await?;

    // Check if user already exists, or their phones' name is taken: a
    // renamed user keeps theirs (`docs/USERS.md` §6), and a device may have
    // its own.
    if storage
        .get_user_in_tenant(&req.username, &tenant.0)
        .await?
        .is_some()
        || sip_name_taken(storage, &tenant.0, &req.username, None).await?
    {
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
        parse_settable_status(status_str)?
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
        digest_password: digest_password_hash, // Can be set separately via API for Cisco-style security
        sip_ha1_hash: Some(sip_ha1_hash),
        sip_provisioning_password,
        display_name: req.display_name,
        tenant_id: tenant.0,
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
        deprovisioned_at: None,
        sip_username: None,
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
#[utoipa::path(
    put,
    path = "/api/v1/users/{username}",
    tag = "users",
    operation_id = "users_update_user",
    params(("username" = String, Path)),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "OK", body = UserResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserUpdate, state.deployment_mode)?;
    validation::validate_username(&username)?;
    validation::validate_display_name(&req.display_name)?;

    // Get existing user
    let storage = require_storage!(state);
    let mut user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;
    refuse_deprovisioned(&user)?;
    let before = user.clone();

    // A new username first: the rest is written to the user as renamed.
    if let Some(new_username) = req.username.as_deref().filter(|n| *n != user.username) {
        user = rename(&state, storage, &user, new_username).await?;
    }

    // Update fields
    if let Some(display_name) = req.display_name {
        user.display_name = Some(display_name);
    }
    // `enabled` is the suspension, said the old way (one release).
    match req.enabled {
        Some(false) => user.account_status = AccountStatus::Suspended,
        Some(true) if user.account_status == AccountStatus::Suspended => {
            user.account_status = AccountStatus::Active
        }
        _ => {}
    }
    user.enabled = user.account_status != AccountStatus::Suspended;
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
        user.account_status = parse_settable_status(&account_status_str)?;
    }
    user.normalize_status();
    if let Some(emergency_name) = req.emergency_contact_name {
        user.emergency_contact.name = Some(emergency_name);
    }
    if let Some(emergency_phone) = req.emergency_contact_phone {
        user.emergency_contact.phone = Some(emergency_phone);
    }

    user.updated_at = std::time::SystemTime::now();

    // Save user
    require_storage!(state).put_user(user.clone()).await?;

    // What the change of status does: signed out, phones dropped, and
    // the world told.
    status_changed(&state, &before, &user).await;

    // Publish user updated event
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Updated {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            changed_fields: changed_fields(&before, &user),
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
#[utoipa::path(
    put,
    path = "/api/v1/users/{username}/password",
    tag = "users",
    operation_id = "users_update_password",
    params(("username" = String, Path)),
    request_body = UpdatePasswordRequest,
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_password(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Json(req): Json<UpdatePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::UserSetPassword,
        state.deployment_mode,
    )?;
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

    // Update web/API password. Only update SIP credentials when this user does
    // not have a separate digest password configured.
    user.password = password_hash;
    if user.digest_password.is_none() {
        user.sip_ha1_hash = Some(password::compute_ha1(
            user.sip_name(),
            &state.realm,
            &req.password,
        ));
        user.sip_provisioning_password = None;
    }
    user.updated_at = std::time::SystemTime::now();

    let tenant_id = user.tenant_id;
    let user_id = user.id;
    let user_id_str = user.id.to_string();

    // Save user
    require_storage!(state).put_user(user).await?;
    // A password set by an administrator signs the user out everywhere.
    crate::v1::auth_session::revoke_sessions(&state, &user_id, None, "password reset").await;

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
#[utoipa::path(
    put,
    path = "/api/v1/users/{username}/digest-password",
    tag = "users",
    operation_id = "users_update_digest_password",
    params(("username" = String, Path)),
    request_body = UpdateDigestPasswordRequest,
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_digest_password(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
    Json(req): Json<UpdateDigestPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::UserSetPassword,
        state.deployment_mode,
    )?;
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
    // Over the SIP name, which a rename keeps (`docs/USERS.md` §6).
    let sip_ha1_hash = password::compute_ha1(user.sip_name(), &state.realm, &req.digest_password);

    // Update digest password and HA1 hash. Optionally retain the plaintext SIP
    // password only for device auto-provisioning templates.
    user.digest_password = Some(digest_password_hash);
    user.sip_ha1_hash = Some(sip_ha1_hash);
    user.sip_provisioning_password = if req.store_for_provisioning {
        Some(req.digest_password)
    } else {
        None
    };
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
#[utoipa::path(
    delete,
    path = "/api/v1/users/{username}/digest-password",
    tag = "users",
    operation_id = "users_delete_digest_password",
    params(("username" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_digest_password(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserDelete, state.deployment_mode)?;
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
    user.sip_provisioning_password = None;

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
#[utoipa::path(
    post,
    path = "/api/v1/users/{username}/unlock",
    tag = "users",
    operation_id = "users_unlock_user",
    params(("username" = String, Path)),
    responses(
        (status = 200, description = "OK", body = UserResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn unlock_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserUpdate, state.deployment_mode)?;
    validation::validate_username(&username)?;

    let storage = require_storage!(state);
    let mut user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;
    refuse_deprovisioned(&user)?;

    let before = user.clone();
    user.failed_login_attempts = 0;
    user.account_locked_until = None;
    user.account_status = AccountStatus::Active;
    user.normalize_status();
    user.updated_at = std::time::SystemTime::now();

    storage.put_user(user.clone()).await?;
    status_changed(&state, &before, &user).await;
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Updated {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            changed_fields: changed_fields(&before, &user),
            timestamp: std::time::SystemTime::now(),
        }))
        .await;
    // The call managers' credential caches hear of it.
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: user.id.to_string(),
            tenant_id: user.tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;

    warn!(
        event = "account_unlocked",
        username = %username,
        "User account unlocked by admin"
    );

    Ok(Json(UserResponse::from(user)))
}

/// Delete a user: they are suspended at once and purged when the tenant's
/// grace period (`offboarding.grace_days`) runs out, unless restored
/// (`docs/USERS.md` §4).
///
/// DELETE /api/v1/users/:username
#[utoipa::path(
    delete,
    path = "/api/v1/users/{username}",
    tag = "users",
    operation_id = "users_delete_user",
    params(("username" = String, Path)),
    responses(
        (status = 204, description = "Deprovisioned; purged after the grace period"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserDelete, state.deployment_mode)?;
    validation::validate_username(&username)?;

    let storage = require_storage!(state);
    let user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;
    crate::offboarding::deprovision(&state, user, &identity.actor()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Restore a deleted user before their purge: active again, with what
/// they had.
///
/// POST /api/v1/users/:username/restore
#[utoipa::path(
    post,
    path = "/api/v1/users/{username}/restore",
    tag = "users",
    operation_id = "users_restore_user",
    params(("username" = String, Path)),
    responses(
        (status = 200, description = "OK", body = UserResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn restore_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<Json<UserResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserDelete, state.deployment_mode)?;
    validation::validate_username(&username)?;

    let storage = require_storage!(state);
    let user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;
    let user = crate::offboarding::restore(&state, user).await?;
    Ok(Json(UserResponse::from(user)))
}

/// What a purge did.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PurgeResponse {
    pub user_id: String,
    pub username: String,
    pub report: fcp_core::offboarding::PurgeReport,
}

/// Purge a deleted user now, without waiting for the grace period. A
/// platform admin's.
///
/// POST /api/v1/users/:username/purge
#[utoipa::path(
    post,
    path = "/api/v1/users/{username}/purge",
    tag = "users",
    operation_id = "users_purge_user",
    params(("username" = String, Path)),
    responses(
        (status = 200, description = "Purged", body = PurgeResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn purge_user(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(username): Path<String>,
) -> Result<Json<PurgeResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::UserDelete, state.deployment_mode)?;
    if !identity.is_platform_admin() {
        return Err(ApiError::Forbidden(
            "Purging a user before the grace period ends is a platform admin's".to_string(),
        ));
    }
    validation::validate_username(&username)?;

    let storage = require_storage!(state);
    let user = storage
        .get_user_in_tenant(&username, &tenant.0)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", username)))
        .and_then(|user| ensure_user_in_tenant(user, &tenant.0, &username))?;
    if !user.is_deprovisioned() {
        return Err(ApiError::Conflict(format!(
            "User '{username}' is not deleted; delete them first"
        )));
    }
    match crate::offboarding::purge(&state, &user, None, &identity.actor()).await? {
        fcp_core::offboarding::PurgeOutcome::Purged { user, report } => Ok(Json(PurgeResponse {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            report,
        })),
        fcp_core::offboarding::PurgeOutcome::Gone => {
            Err(ApiError::NotFound(format!("User '{username}' not found")))
        }
        fcp_core::offboarding::PurgeOutcome::ErasureOpen => Err(ApiError::Conflict(format!(
            "An erasure request for '{username}' is open; it runs first"
        ))),
        fcp_core::offboarding::PurgeOutcome::NotDeprovisioned
        | fcp_core::offboarding::PurgeOutcome::NotDue => Err(ApiError::Conflict(format!(
            "User '{username}' is not deleted; delete them first"
        ))),
    }
}

/// A user is named by id (`user-<uuid>`) where a token or session records
/// them; a username of that shape could be mistaken for one.
pub(crate) fn refuse_id_shaped(username: &str) -> Result<(), ApiError> {
    if username.starts_with("user-") && username.parse::<fcp_core::UserId>().is_ok() {
        return Err(ApiError::BadRequest(
            "A username cannot have the form of a user id (user-<uuid>)".to_string(),
        ));
    }
    Ok(())
}

/// Rename `user` to `new_username` (`docs/USERS.md` §6): checked, then
/// renamed in the store with what names them by username; announced as
/// `user.renamed`. Their SIP name stays.
pub(crate) async fn rename(
    state: &ApiState,
    storage: &fcp_core::DynStorage,
    user: &User,
    new_username: &str,
) -> Result<User, ApiError> {
    validation::validate_username(new_username)?;
    refuse_id_shaped(new_username)?;
    if storage
        .get_user_in_tenant(new_username, &user.tenant_id)
        .await?
        .is_some_and(|u| u.id != user.id)
        || sip_name_taken(storage, &user.tenant_id, new_username, Some(&user.id)).await?
    {
        return Err(ApiError::Conflict(format!(
            "The username '{new_username}' is taken"
        )));
    }
    let renamed = storage.rename_user(user, new_username).await?;
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Renamed {
            user_id: renamed.id.to_string(),
            old_username: user.username.clone(),
            username: renamed.username.clone(),
            tenant_id: renamed.tenant_id,
            timestamp: std::time::SystemTime::now(),
        }))
        .await;
    warn!(
        event = "user_renamed",
        from = %user.username,
        to = %renamed.username,
        tenant_id = %renamed.tenant_id,
        "User renamed"
    );
    Ok(renamed)
}

/// Whether `name` is already some phone's SIP name in the tenant: another
/// user's (than `except`), or a device's own.
pub(crate) async fn sip_name_taken(
    storage: &fcp_core::DynStorage,
    tenant: &fcp_core::TenantId,
    name: &str,
    except: Option<&fcp_core::UserId>,
) -> Result<bool, ApiError> {
    use fcp_core::storage::DeviceStore;
    if storage
        .get_user_by_sip_name(name, tenant)
        .await?
        .is_some_and(|u| Some(&u.id) != except)
    {
        return Ok(true);
    }
    // A device that authenticates as itself; one on its owner's account
    // (its URI naming the owner) takes no name of its own.
    Ok(storage
        .get_device_by_sip_name(tenant, name)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .is_some_and(|d| {
            d.sip_username
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        }))
}

/// Refuse a new user past the tenant's `max_users`.
pub(crate) async fn ensure_room_for_a_user(
    state: &ApiState,
    tenant_id: &fcp_core::TenantId,
) -> Result<(), ApiError> {
    use fcp_core::TenantStore;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let Some(max) = storage
        .get_tenant(tenant_id)
        .await?
        .and_then(|t| t.max_users)
    else {
        return Ok(());
    };
    if storage.count_users_in_tenant(tenant_id).await? >= u64::from(max) {
        return Err(ApiError::Conflict(format!(
            "The tenant has its {max} users (max_users)"
        )));
    }
    Ok(())
}

/// What a users listing is narrowed by.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UserListQuery {
    /// A word in the username, display name or email.
    #[serde(default)]
    pub q: Option<String>,
    /// `active`, `suspended` or `locked`.
    #[serde(default)]
    pub status: Option<String>,
}

/// An account status from a request: `active`, `suspended` or `locked`
/// (`disabled`, the old word, is a suspension); anything else refused,
/// where it used to become Active.
pub(crate) fn parse_account_status(s: &str) -> Result<AccountStatus, ApiError> {
    AccountStatus::from_str(s).map_err(|_| {
        ApiError::BadRequest(format!(
            "account_status must be active, suspended, locked or deprovisioned, not '{s}'"
        ))
    })
}

/// A status a create or an update may set: not `deprovisioned`, which only
/// a delete sets and only a restore clears.
pub(crate) fn parse_settable_status(s: &str) -> Result<AccountStatus, ApiError> {
    match parse_account_status(s)? {
        AccountStatus::Deprovisioned => Err(ApiError::BadRequest(
            "account_status 'deprovisioned' is set by deleting the user".to_string(),
        )),
        status => Ok(status),
    }
}

/// A deleted user waits for their purge or a restore; nothing else
/// changes them.
pub(crate) fn refuse_deprovisioned(user: &User) -> Result<(), ApiError> {
    if user.is_deprovisioned() {
        return Err(ApiError::Conflict(format!(
            "User '{}' is deleted and waiting to be purged; restore them first",
            user.username
        )));
    }
    Ok(())
}

/// The fields a change touched, by name (secrets named, never shown).
pub(crate) fn changed_fields(before: &User, after: &User) -> Vec<String> {
    let (Ok(serde_json::Value::Object(a)), Ok(serde_json::Value::Object(b))) =
        (serde_json::to_value(before), serde_json::to_value(after))
    else {
        return Vec::new();
    };
    let mut changed: Vec<String> = b
        .iter()
        .filter(|(k, v)| k.as_str() != "updated_at" && a.get(*k) != Some(*v))
        .map(|(k, _)| k.clone())
        .collect();
    changed.sort();
    changed
}

/// What a change of a user's status does (`docs/IDENTITY_AND_DEVICES.md`
/// phase 5). Suspended: every session ended, every registration of theirs
/// dropped (their name, their extension, the devices they own), their
/// hoteling session ended, `user.disabled` published. Resumed:
/// `user.enabled`. Locked (the web lockout): signed out of the web only.
pub(crate) async fn status_changed(state: &ApiState, before: &User, after: &User) {
    let signed_out =
        fcp_core::session::user_is_active(before) && !fcp_core::session::user_is_active(after);
    if signed_out {
        crate::v1::auth_session::revoke_sessions(
            state,
            &after.id,
            None,
            "account suspended or locked",
        )
        .await;
    }
    let now = std::time::SystemTime::now();
    if !before.is_suspended() && after.is_suspended() {
        if let Some(storage) = state.storage.as_ref() {
            drop_registrations(state, storage, after).await;
            end_hoteling(state, storage, after).await;
        }
        let _ = state
            .event_bus
            .publish(PlatformEvent::User(UserEvent::Disabled {
                user_id: after.id.to_string(),
                username: after.username.clone(),
                tenant_id: after.tenant_id,
                timestamp: now,
            }))
            .await;
        info!(event = "user_suspended", username = %after.username, tenant_id = %after.tenant_id, "User suspended");
    } else if before.is_suspended() && !after.is_suspended() {
        let _ = state
            .event_bus
            .publish(PlatformEvent::User(UserEvent::Enabled {
                user_id: after.id.to_string(),
                username: after.username.clone(),
                tenant_id: after.tenant_id,
                timestamp: now,
            }))
            .await;
        info!(event = "user_resumed", username = %after.username, tenant_id = %after.tenant_id, "User resumed");
    }
}

/// Every registration a suspended user's phones hold: under their name,
/// their extension, or the SIP name of a device they own.
async fn drop_registrations(state: &ApiState, storage: &fcp_core::DynStorage, user: &User) {
    use fcp_core::storage::{DeviceStore, RegistrationStore};
    let mut names: Vec<String> = vec![user.username.to_lowercase(), user.sip_name().to_lowercase()];
    if let Some(ext) = user.extension.as_deref().filter(|e| !e.is_empty()) {
        names.push(ext.to_lowercase());
    }
    if let Ok(devices) = storage.list_devices_by_user(&user.id).await {
        names.extend(
            devices
                .into_iter()
                .filter_map(|d| d.sip_username)
                .map(|n| n.to_lowercase()),
        );
    }
    let registrations = match storage
        .get_all_registrations_in_tenant(&user.tenant_id)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(username = %user.username, error = %e, "Could not read registrations to drop");
            return;
        }
    };
    let mut aors: Vec<String> = registrations
        .into_iter()
        .map(|r| r.aor)
        .filter(|aor| {
            let user_part = aor
                .trim_start_matches("sips:")
                .trim_start_matches("sip:")
                .split('@')
                .next()
                .unwrap_or_default()
                .to_lowercase();
            names.contains(&user_part)
        })
        .collect();
    aors.sort();
    aors.dedup();
    for aor in aors {
        match storage
            .delete_all_registrations_in_tenant(&user.tenant_id, &aor)
            .await
        {
            Ok(_) => {
                crate::v1::registrations::announce_removed(state, user.tenant_id, &aor, "*").await;
            }
            Err(e) => warn!(aor, error = %e, "Could not drop a suspended user's registration"),
        }
    }
}

/// A suspended user's hoteling session, ended: the host phone goes back
/// to its owner.
async fn end_hoteling(state: &ApiState, storage: &fcp_core::DynStorage, user: &User) {
    use fcp_core::HotelingStore;
    if let Ok(Some(session)) = storage.hoteling_session_for_user(&user.id).await {
        if let Err(e) =
            crate::v1::hoteling::end_session(state, storage, &session, fcp_core::SessionEnd::Admin)
                .await
        {
            warn!(username = %user.username, error = ?e, "Could not end a suspended user's hoteling session");
        }
    }
}

/// A user with no web password, as single sign-on creates one just in time
/// (`docs/IDENTITY_AND_DEVICES.md` phase 3): their SIP password generated,
/// everything else as `create_user` defaults it.
pub(crate) fn user_without_password(
    realm: &str,
    tenant_id: fcp_core::TenantId,
    username: &str,
    display_name: Option<String>,
    email: Option<String>,
    role: UserRole,
) -> Result<User, ApiError> {
    let sip_password = generated_sip_password();
    let digest = password::hash_password(&sip_password)
        .map_err(|e| ApiError::Internal(format!("hashing a generated password: {e}")))?;
    let now = std::time::SystemTime::now();
    Ok(User {
        id: fcp_core::UserId::new(),
        username: username.to_string(),
        // No web password: nothing a password can match.
        password: String::new(),
        digest_password: Some(digest),
        sip_ha1_hash: Some(password::compute_ha1(username, realm, &sip_password)),
        sip_provisioning_password: Some(sip_password),
        display_name,
        tenant_id,
        enabled: true,
        role,
        department_id: None,
        created_at: now,
        updated_at: now,
        media_preferences: Default::default(),
        email,
        password_last_reset: None,
        phone_number: None,
        mobile_number: None,
        pin: None,
        extension: None,
        department: None,
        job_title: None,
        location: None,
        location_id: None,
        language: "en-US".to_string(),
        timezone: "UTC".to_string(),
        account_status: AccountStatus::Active,
        last_login: None,
        emergency_contact: EmergencyContact {
            name: None,
            phone: None,
        },
        primary_phone_number_id: None,
        voicemail_box_id: None,
        profile_picture: None,
        deprovisioned_at: None,
        sip_username: None,
        failed_login_attempts: 0,
        account_locked_until: None,
    })
}

/// A SIP password for a user who has no web password to derive one from:
/// 24 random bytes, with the classes the password rules ask for.
fn generated_sip_password() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!(
        "Aa1-{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}
