//! Registration viewing API endpoints.

use crate::{
    auth::{can_access_all_tenants, AuthIdentity, TenantContext, PLATFORM_TENANT_HEADER},
    error::ApiError,
    state::ApiState,
    validation,
};
use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use fcp_core::storage::DeviceStore;
use fcp_core::{CredentialStore, Registration, RegistrationStore, TenantId};
use fcp_events::{PlatformEvent, RegistrationEvent};
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
        .list_users_in_tenant(tenant_id, &fcp_core::storage::UserFilter::default())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list users: {e}")))?
        .into_iter()
    {
        visible.insert(format!(
            "{}@{}",
            user.username.to_ascii_lowercase(),
            state.realm.to_ascii_lowercase()
        ));
    }

    Ok(visible)
}

/// Cross-tenant access applies to platform admins only while they have not
/// selected a tenant with the override header.
fn platform_wide(identity: Option<&AuthIdentity>, headers: &HeaderMap) -> bool {
    can_access_all_tenants(identity) && !headers.contains_key(PLATFORM_TENANT_HEADER)
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
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RegistrationResponse {
    pub aor: String,
    pub contact: String,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: std::time::SystemTime,
    pub call_id: String,
    pub cseq: u32,
    pub user_agent: Option<String>,
    pub q: Option<f32>,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub registered_at: std::time::SystemTime,
    pub expires_in_seconds: u64,
    /// The node that took the registration (its `[service] node_id`);
    /// null for one written before v0.25.639 that has not re-registered.
    pub node_id: Option<String>,
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
            node_id: reg.node_id,
        }
    }
}

/// List all active registrations.
///
/// GET /api/v1/registrations
#[utoipa::path(
    get,
    path = "/api/v1/registrations",
    tag = "registrations",
    operation_id = "registrations_list_registrations",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<RegistrationResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_registrations(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<RegistrationResponse>>, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Registrar not enabled".to_string()))?;

    // Platform admins see every tenant unless they selected one with the
    // override header (then `tenant` already carries it).
    let allow_all_tenants = platform_wide(identity.as_ref().map(|ext| &ext.0), &headers);
    let registrations = if allow_all_tenants {
        storage.get_all_registrations().await?
    } else {
        storage.get_all_registrations_in_tenant(&tenant.0).await?
    };

    let now = std::time::SystemTime::now();
    let active: Vec<RegistrationResponse> = registrations
        .into_iter()
        .filter(|r| r.expires_at > now)
        .map(RegistrationResponse::from)
        .collect();

    Ok(Json(crate::page::paginate(
        active,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.aor, r.contact),
    )?))
}

/// List registrations for a specific AOR.
///
/// GET /api/v1/registrations/:aor
#[utoipa::path(
    get,
    path = "/api/v1/registrations/{aor}",
    tag = "registrations",
    operation_id = "registrations_get_registrations",
    params(("aor" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<RegistrationResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_registrations(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Path(aor): Path<String>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<Json<crate::page::Page<RegistrationResponse>>, ApiError> {
    validation::validate_aor(&aor)?;

    if !platform_wide(identity.as_ref().map(|ext| &ext.0), &headers)
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

    let registrations = if platform_wide(identity.as_ref().map(|ext| &ext.0), &headers) {
        storage.get_registrations(&aor).await?
    } else {
        storage.get_registrations_in_tenant(&tenant.0, &aor).await?
    };

    // Filter out expired registrations
    let now = std::time::SystemTime::now();
    let active: Vec<RegistrationResponse> = registrations
        .into_iter()
        .filter(|r| r.expires_at > now)
        .map(RegistrationResponse::from)
        .collect();

    Ok(Json(crate::page::paginate(
        active,
        &page,
        crate::page::Order::Ascending,
        |r| r.contact.clone(),
    )?))
}

/// Delete all registrations for a specific AOR.
///
/// DELETE /api/v1/registrations/:aor
#[utoipa::path(
    delete,
    path = "/api/v1/registrations/{aor}",
    tag = "registrations",
    operation_id = "registrations_delete_registrations",
    params(("aor" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_registrations(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Path(aor): Path<String>,
) -> Result<StatusCode, ApiError> {
    validation::validate_aor(&aor)?;

    if !platform_wide(identity.as_ref().map(|ext| &ext.0), &headers)
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

    let tenant_id = if platform_wide(identity.as_ref().map(|ext| &ext.0), &headers) {
        storage.delete_all_registrations(&aor).await?;
        TenantId::nil()
    } else {
        storage
            .delete_all_registrations_in_tenant(&tenant.0, &aor)
            .await?;
        tenant.0
    };
    announce_removed(&state, tenant_id, &aor, "*").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Say that an administrator removed a registration, as the registrar says
/// a deregistration: a call manager's registration cache forgets the
/// address on it rather than ringing the removed contact until its TTL
/// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 2). A nil tenant — a
/// platform-wide removal — is forgotten in every tenant.
pub(crate) async fn announce_removed(
    state: &ApiState,
    tenant_id: TenantId,
    aor: &str,
    contact: &str,
) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::Registration(
            RegistrationEvent::Unregistered {
                aor: aor.to_string(),
                contact: contact.to_string(),
                tenant_id,
                reason: "admin".to_string(),
                timestamp: std::time::SystemTime::now(),
            },
        ))
        .await;
}

/// Delete a specific registration.
///
/// DELETE /api/v1/registrations/:aor/:contact
#[utoipa::path(
    delete,
    path = "/api/v1/registrations/{aor}/{contact}",
    tag = "registrations",
    operation_id = "registrations_delete_registration",
    params(("aor" = String, Path), ("contact" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_registration(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Path((aor, contact)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    validation::validate_aor(&aor)?;
    validation::validate_contact(&contact)?;

    if !platform_wide(identity.as_ref().map(|ext| &ext.0), &headers)
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

    let tenant_id = if platform_wide(identity.as_ref().map(|ext| &ext.0), &headers) {
        storage.delete_registration(&aor, &contact).await?;
        TenantId::nil()
    } else {
        storage
            .delete_registration_in_tenant(&tenant.0, &aor, &contact)
            .await?;
        tenant.0
    };
    announce_removed(&state, tenant_id, &aor, &contact).await;
    Ok(StatusCode::NO_CONTENT)
}
