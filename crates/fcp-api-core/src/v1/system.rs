//! System status API endpoints.

use crate::{
    auth::{can_access_all_tenants, AuthIdentity, TenantContext},
    error::ApiError,
    state::ApiState,
};
use axum::{
    extract::{Extension, State},
    Json,
};
use fcp_core::storage::DeviceStore;
use fcp_core::{CredentialStore, RegistrationStore, TenantId};
use serde::Serialize;
use std::collections::HashSet;
use std::time::SystemTime;

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// System status response.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub users_count: usize,
    pub active_registrations_count: usize,
}

/// Health check endpoint.
///
/// GET /api/v1/health
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// System status endpoint.
///
/// GET /api/v1/status
pub async fn status(State(state): State<ApiState>) -> Result<Json<StatusResponse>, ApiError> {
    let default_tenant_id = state.default_tenant_id;
    status_with_scope(state, TenantContext(default_tenant_id), None).await
}

async fn status_with_scope(
    state: ApiState,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<Json<StatusResponse>, ApiError> {
    let allow_all_tenants = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0));

    // Count users if storage is available
    let users_count = if let Some(storage) = &state.storage {
        let users = storage.list_users().await?;
        if allow_all_tenants {
            users.len()
        } else {
            users
                .into_iter()
                .filter(|user| user.tenant_id == tenant.0)
                .count()
        }
    } else {
        0
    };

    // Count active registrations across all AORs
    let active_registrations_count = if let Some(storage) = &state.storage {
        if allow_all_tenants {
            storage.registration_count().await.unwrap_or(0)
        } else {
            let visible_aors = tenant_visible_aors(&state, &tenant.0).await?;
            let now = SystemTime::now();
            storage
                .get_all_registrations()
                .await?
                .into_iter()
                .filter(|registration| registration.expires_at > now)
                .filter(|registration| visible_aors.contains(&normalize_aor(&registration.aor)))
                .count()
        }
    } else {
        0
    };

    Ok(Json(StatusResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        users_count,
        active_registrations_count,
    }))
}

pub async fn status_scoped(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<Json<StatusResponse>, ApiError> {
    status_with_scope(state, tenant, identity).await
}

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
