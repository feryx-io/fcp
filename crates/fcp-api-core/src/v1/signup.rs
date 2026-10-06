//! Tenant self-service signup endpoint.
//!
//! POST /auth/signup — Create a new tenant and admin user when public signup
//! is explicitly enabled by the deployment.

use crate::error::ApiError;
use crate::state::ApiState;
use crate::v1::auth_session;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use fcp_core::storage::password;
use fcp_core::{
    AccountStatus, CredentialStore, EmergencyContact, MediaPreferences, Tenant, TenantId,
    TenantStatus, TenantStore, User, UserId, UserRole,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SignupRequest {
    pub company_name: String,
    pub username: String,
    pub password: String,
    pub email: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SignupResponse {
    pub token: String,
    pub tenant_id: String,
    pub tenant_name: String,
    pub username: String,
}

async fn cleanup_failed_signup(
    storage: &Arc<fcp_core::DynStorage>,
    tenant_id: &TenantId,
    username: &str,
    user_created: bool,
) {
    if user_created {
        if let Err(error) = storage.delete_user_in_tenant(username, tenant_id).await {
            warn!(
                tenant_id = %tenant_id,
                username = %username,
                %error,
                "Failed to clean up signup user after partial failure"
            );
        }
    }

    if let Err(error) = storage.delete_tenant(tenant_id).await {
        warn!(
            tenant_id = %tenant_id,
            username = %username,
            %error,
            "Failed to clean up signup tenant after partial failure"
        );
    }
}

/// POST /auth/signup — Create a new tenant and admin user when enabled.
#[utoipa::path(
    post,
    path = "/api/v1/auth/signup",
    tag = "signup",
    operation_id = "signup_signup",
    request_body = SignupRequest,
    responses(
        (status = 201, description = "OK", body = SignupResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn signup(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<SignupRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".into()))?;

    // Validate inputs
    if req.company_name.trim().is_empty() {
        return Err(ApiError::BadRequest("Company name is required".into()));
    }
    if req.username.trim().is_empty() || req.username.len() < 3 {
        return Err(ApiError::BadRequest(
            "Username must be at least 3 characters".into(),
        ));
    }
    if req.password.len() < 8 {
        return Err(ApiError::BadRequest(
            "Password must be at least 8 characters".into(),
        ));
    }
    if !req.email.contains('@') {
        return Err(ApiError::BadRequest("Valid email is required".into()));
    }

    // Fail early if signing in is impossible — avoids orphaned rows
    if state.session_keys.is_none() {
        return Err(ApiError::Internal("Sign-in is not configured".to_string()));
    }

    // Hash the password (constant-time, same pattern as create_user)
    let password_hash = password::hash_password(&req.password)
        .map_err(|e| ApiError::BadRequest(format!("Invalid password: {}", e)))?;

    // Compute SIP HA1 hash for digest authentication
    let sip_ha1_hash = password::compute_ha1(&req.username, &state.realm, &req.password);

    // Create tenant
    let tenant_id = TenantId::new();
    let now = SystemTime::now();

    let tenant = Tenant {
        id: tenant_id,
        name: req.company_name.trim().to_string(),
        description: None,
        status: TenantStatus::Trial,
        contact_email: Some(req.email.clone()),
        contact_phone: None,
        max_users: Some(50),
        max_phone_numbers: Some(10),
        max_concurrent_calls: Some(20),
        billing_id: None,
        sip_domains: vec![],
        // A new tenant calls out on the platform's trunks until it has its
        // own (`docs/TENANT_TRUNKS.md` §5).
        platform_trunk_fallback: true,
        calling: Default::default(),
        login_domains: Vec::new(),
        sign_in: Default::default(),
        offboarding: Default::default(),
        provisioning: Default::default(),
        directory: Default::default(),
        created_at: now,
        updated_at: now,
        trial_expires_at: Some(now + Duration::from_secs(14 * 24 * 3600)),
        metadata: None,
    };

    storage.put_tenant(tenant.clone()).await.map_err(|e| {
        if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
            ApiError::Conflict(format!(
                "Company name '{}' is already taken",
                req.company_name
            ))
        } else {
            ApiError::Internal(e.to_string())
        }
    })?;

    // Create admin user (same pattern as users.rs create_user)
    let user = User {
        id: UserId::new(),
        username: req.username.clone(),
        password: password_hash,
        digest_password: None,
        sip_ha1_hash: Some(sip_ha1_hash),
        sip_provisioning_password: None,
        display_name: Some(req.company_name.trim().to_string()),
        tenant_id,
        enabled: true,
        role: UserRole::TenantAdmin,
        department_id: None,
        created_at: now,
        updated_at: now,
        media_preferences: MediaPreferences::default(),
        email: Some(req.email),
        password_last_reset: Some(now),
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
        last_login: Some(now),
        emergency_contact: EmergencyContact {
            name: None,
            phone: None,
        },
        primary_phone_number_id: None,
        voicemail_box_id: None,
        profile_picture: None,
        deprovisioned_at: None,
        sip_username: None,
        scim_managed: false,
        scim_external_id: None,
        failed_login_attempts: 0,
        account_locked_until: None,
    };

    if let Err(error) = storage.put_user(user.clone()).await {
        cleanup_failed_signup(storage, &tenant_id, &user.username, false).await;
        return Err(ApiError::Internal(format!(
            "Failed to create admin user: {}",
            error
        )));
    }

    // Signed in as the new tenant's admin.
    let issued = match auth_session::begin_session(
        &state,
        &user,
        fcp_core::session::method::SIGNUP,
        &headers,
        None,
    )
    .await
    {
        Ok(issued) => issued,
        Err(error) => {
            cleanup_failed_signup(storage, &tenant_id, &user.username, true).await;
            return Err(error);
        }
    };

    info!(
        event = "tenant_signup",
        tenant_id = %tenant_id,
        username = %user.username,
        company_name = %tenant.name,
        "New tenant signed up"
    );

    let mut response = (
        StatusCode::CREATED,
        Json(SignupResponse {
            token: issued.token,
            tenant_id: tenant_id.to_string(),
            tenant_name: tenant.name,
            username: user.username,
        }),
    )
        .into_response();
    if let Some(cookie) = issued.cookie {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    Ok(response)
}
