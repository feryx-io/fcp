//! Registration viewing API endpoints.

use crate::{
    auth::{can_access_all_tenants, AuthIdentity, TenantContext},
    error::ApiError,
    state::ApiState,
    validation,
};
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use fcp_core::storage::DeviceStore;
use fcp_core::{CredentialStore, Registration, RegistrationStore, TenantId};
use serde::Serialize;
use std::collections::HashSet;

fn normalize_aor(aor: &str) -> String {
    fcp_sip::presence::extract_aor(aor)
        .unwrap_or_else(|| {
            aor.trim_start_matches("sip:")
                .trim_start_matches("sips:")
                .to_string()
        })
        .to_ascii_lowercase()
}

async fn tenant_visible_aors(
    state: &ApiState,
    tenant_id: &TenantId,
) -> Result<HashSet<String>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    let mut visible = HashSet::new();

    for device in storage
        .list_devices(tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list tenant devices: {e}")))?
    {
        if let Some(sip_uri) = device.sip_uri {
            visible.insert(normalize_aor(&sip_uri));
        }
    }

    for user in storage
        .list_users()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list users: {e}")))?
        .into_iter()
        .filter(|user| user.tenant_id == *tenant_id)
    {
        visible.insert(format!(
            "{}@{}",
            user.username.to_ascii_lowercase(),
            state.realm.to_ascii_lowercase()
        ));
    }

    Ok(visible)
}

async fn registration_visible_to_tenant(
    state: &ApiState,
    tenant_id: &TenantId,
    aor: &str,
) -> Result<bool, ApiError> {
    Ok(tenant_visible_aors(state, tenant_id)
        .await?
        .contains(&normalize_aor(aor)))
}

/// Registration response.
#[derive(Debug, Serialize)]
pub struct RegistrationResponse {
    pub aor: String,
    pub contact: String,
    pub expires_at: std::time::SystemTime,
    pub call_id: String,
    pub cseq: u32,
    pub user_agent: Option<String>,
    pub q: Option<f32>,
    pub registered_at: std::time::SystemTime,
    pub expires_in_seconds: u64,
}

impl From<Registration> for RegistrationResponse {
    fn from(reg: Registration) -> Self {
        let expires_in_seconds = reg
            .expires_at
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default()
            .as_secs();

        Self {
            aor: reg.aor,
            contact: reg.contact,
            expires_at: reg.expires_at,
            call_id: reg.call_id,
            cseq: reg.cseq,
            user_agent: reg.user_agent,
            q: reg.q,
            registered_at: reg.registered_at,
            expires_in_seconds,
        }
    }
}

/// List all active registrations.
///
/// GET /api/v1/registrations
pub async fn list_registrations(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<Json<Vec<RegistrationResponse>>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    let registrations = storage.get_all_registrations().await?;
    let allow_all_tenants = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0));
    let visible_aors = if allow_all_tenants {
        None
    } else {
        Some(tenant_visible_aors(&state, &tenant.0).await?)
    };

    let now = std::time::SystemTime::now();
    let active: Vec<RegistrationResponse> = registrations
        .into_iter()
        .filter(|r| {
            visible_aors
                .as_ref()
                .map(|allowed| allowed.contains(&normalize_aor(&r.aor)))
                .unwrap_or(true)
        })
        .filter(|r| r.expires_at > now)
        .map(RegistrationResponse::from)
        .collect();

    Ok(Json(active))
}

/// List registrations for a specific AOR.
///
/// GET /api/v1/registrations/:aor
pub async fn get_registrations(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(aor): Path<String>,
) -> Result<Json<Vec<RegistrationResponse>>, ApiError> {
    validation::validate_aor(&aor)?;

    if !can_access_all_tenants(identity.as_ref().map(|ext| &ext.0))
        && !registration_visible_to_tenant(&state, &tenant.0, &aor).await?
    {
        return Err(ApiError::NotFound(format!(
            "Registrations not found for '{}'",
            aor
        )));
    }

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    let registrations = storage.get_registrations(&aor).await?;

    // Filter out expired registrations
    let now = std::time::SystemTime::now();
    let active: Vec<RegistrationResponse> = registrations
        .into_iter()
        .filter(|r| r.expires_at > now)
        .map(RegistrationResponse::from)
        .collect();

    Ok(Json(active))
}

/// Delete all registrations for a specific AOR.
///
/// DELETE /api/v1/registrations/:aor
pub async fn delete_registrations(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(aor): Path<String>,
) -> Result<StatusCode, ApiError> {
    validation::validate_aor(&aor)?;

    if !can_access_all_tenants(identity.as_ref().map(|ext| &ext.0))
        && !registration_visible_to_tenant(&state, &tenant.0, &aor).await?
    {
        return Err(ApiError::NotFound(format!(
            "Registrations not found for '{}'",
            aor
        )));
    }

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    storage.delete_all_registrations(&aor).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a specific registration.
///
/// DELETE /api/v1/registrations/:aor/:contact
pub async fn delete_registration(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path((aor, contact)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    validation::validate_aor(&aor)?;
    validation::validate_contact(&contact)?;

    if !can_access_all_tenants(identity.as_ref().map(|ext| &ext.0))
        && !registration_visible_to_tenant(&state, &tenant.0, &aor).await?
    {
        return Err(ApiError::NotFound(format!(
            "Registration not found for '{}'",
            aor
        )));
    }

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    storage.delete_registration(&aor, &contact).await?;
    Ok(StatusCode::NO_CONTENT)
}
