//! API endpoints for tenant management

use crate::auth::AuthIdentity;
use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::{Tenant, TenantId, TenantStatus, TenantStore};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::SystemTime;

/// Require a platform-level admin identity for tenant lifecycle operations.
///
/// Tenant CRUD (create/delete) is strictly a super_admin / static-token
/// operation. Tenant admins may view and update their own tenant, but must
/// never be able to create sibling tenants or delete a tenant — that would
/// let a tenant-scoped admin operate outside their scope.
fn require_platform(identity: Option<&AuthIdentity>) -> Result<(), ApiError> {
    if let Some(id) = identity {
        if !id.is_platform_admin() {
            return Err(ApiError::Forbidden(
                "Platform admin access required".to_string(),
            ));
        }
    }
    Ok(())
}

/// Require that the caller can see the given tenant.
///
/// Platform admins see every tenant. Any other identity (tenant_admin,
/// telecom_admin, ...) may only act on their own tenant. Callers without
/// an identity at all (auth disabled in dev) are allowed through.
fn require_tenant_access(
    identity: Option<&AuthIdentity>,
    target: &TenantId,
) -> Result<(), ApiError> {
    let Some(id) = identity else { return Ok(()) };
    if id.is_platform_admin() {
        return Ok(());
    }
    match id.tenant_id() {
        Some(own) if own == target => Ok(()),
        _ => Err(ApiError::Forbidden(
            "Cannot access resources in another tenant".to_string(),
        )),
    }
}

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Tenant management requires registrar to be enabled".to_string())
        })?
    };
}

/// Response for tenant API calls
#[derive(Debug, Serialize)]
pub struct TenantResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
    /// SIP domains claimed by this tenant. See `Tenant::sip_domains`.
    pub sip_domains: Vec<String>,
}

impl From<Tenant> for TenantResponse {
    fn from(tenant: Tenant) -> Self {
        Self {
            id: tenant.id.to_string(),
            name: tenant.name,
            description: tenant.description,
            status: tenant.status.to_string().to_lowercase(),
            contact_email: tenant.contact_email,
            contact_phone: tenant.contact_phone,
            max_users: tenant.max_users,
            max_phone_numbers: tenant.max_phone_numbers,
            max_concurrent_calls: tenant.max_concurrent_calls,
            billing_id: tenant.billing_id,
            created_at: tenant
                .created_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            updated_at: tenant
                .updated_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            trial_expires_at: tenant.trial_expires_at.and_then(|t| {
                t.duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs())
            }),
            metadata: tenant.metadata,
            sip_domains: tenant.sip_domains,
        }
    }
}

/// Request to create a new tenant
#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
    /// SIP domains to claim on behalf of the new tenant.
    #[serde(default)]
    pub sip_domains: Vec<String>,
}

/// Request to update a tenant
#[derive(Debug, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
    /// Full replacement for the tenant's SIP domain list. Use `None` to
    /// leave the list alone; use `Some(vec![])` to clear it.
    pub sip_domains: Option<Vec<String>>,
}

/// Normalize and validate a list of user-supplied SIP domains.
///
/// Returns a deduplicated, lowercased, trimmed list. Rejects entries that
/// are numeric IP literals, `localhost`, or contain characters that clearly
/// can't appear in a hostname — these belong on the compatibility fallback
/// path, not in the allowlist.
fn normalize_sip_domains(input: &[String]) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = Vec::with_capacity(input.len());
    for raw in input {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();

        // Reject host literals — the resolver handles IP/localhost via
        // the compatibility fallback and listing them here would shadow
        // the real allowlist semantics.
        if lower == "localhost" || lower.parse::<std::net::IpAddr>().is_ok() {
            return Err(ApiError::BadRequest(format!(
                "'{}' is a host literal and cannot be added to sip_domains. \
                 IP-addressed devices are handled by the server's default tenant fallback.",
                raw
            )));
        }

        // Very light syntactic check: no whitespace, no SIP separators.
        if lower
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '@' | ':' | '/' | '<' | '>' | ',' | ';'))
        {
            return Err(ApiError::BadRequest(format!(
                "'{}' is not a valid SIP domain (whitespace or SIP separator characters are not allowed)",
                raw
            )));
        }

        if !out.contains(&lower) {
            out.push(lower);
        }
    }
    Ok(out)
}

/// Ensure no other tenant already claims any of `domains`. Pass the
/// tenant id being updated (or `None` for creates) to exclude self-matches.
async fn ensure_sip_domains_unique<S: fcp_core::TenantStore + ?Sized>(
    storage: &S,
    domains: &[String],
    exclude: Option<&TenantId>,
) -> Result<(), ApiError> {
    for d in domains {
        if let Some(existing) = storage.get_tenant_by_sip_domain(d).await? {
            if exclude != Some(&existing.id) {
                return Err(ApiError::Conflict(format!(
                    "SIP domain '{}' is already claimed by tenant '{}'",
                    d, existing.name
                )));
            }
        }
    }
    Ok(())
}

/// List all tenants
///
/// Platform admins see every tenant. Tenant-scoped identities (including
/// the new tenant_admin role) only see their own tenant so they can render
/// their own tenant card in the admin UI without learning about siblings.
pub async fn list_tenants(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<Json<Vec<TenantResponse>>, ApiError> {
    let storage = require_storage!(state);
    let identity_ref = identity.as_ref().map(|ext| &ext.0);

    let tenants = if identity_ref.is_none_or(|id| id.is_platform_admin()) {
        storage.list_tenants().await?
    } else {
        // Tenant-scoped identities can only see themselves.
        match identity_ref.and_then(|id| id.tenant_id()) {
            Some(own) => match storage.get_tenant(own).await? {
                Some(t) => vec![t],
                None => vec![],
            },
            None => vec![],
        }
    };

    let response: Vec<TenantResponse> = tenants.into_iter().map(|t| t.into()).collect();
    Ok(Json(response))
}

/// Get a specific tenant by ID
///
/// Tenant-scoped identities may only load their own tenant.
pub async fn get_tenant(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    Path(id): Path<String>,
) -> Result<Json<TenantResponse>, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = TenantId::from_str(&id)
        .map_err(|_| ApiError::BadRequest("Invalid tenant ID format".to_string()))?;
    require_tenant_access(identity.as_ref().map(|ext| &ext.0), &tenant_id)?;
    let tenant = storage
        .get_tenant(&tenant_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Tenant {} not found", id)))?;
    Ok(Json(tenant.into()))
}

/// Create a new tenant (platform admin only)
pub async fn create_tenant(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    Json(req): Json<CreateTenantRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_platform(identity.as_ref().map(|ext| &ext.0))?;
    let storage = require_storage!(state);

    // Check if tenant name already exists
    if storage.get_tenant_by_name(&req.name).await?.is_some() {
        return Err(ApiError::Conflict(format!(
            "Tenant with name '{}' already exists",
            req.name
        )));
    }

    // Normalize and uniqueness-check any SIP domains the create provided.
    let sip_domains = normalize_sip_domains(&req.sip_domains)?;
    ensure_sip_domains_unique(storage.as_ref(), &sip_domains, None).await?;

    let now = SystemTime::now();
    let status = if let Some(status_str) = &req.status {
        match status_str.to_lowercase().as_str() {
            "active" => TenantStatus::Active,
            "suspended" => TenantStatus::Suspended,
            "trial" => TenantStatus::Trial,
            "closed" => TenantStatus::Closed,
            _ => {
                return Err(ApiError::BadRequest(format!(
                    "Invalid status '{}'. Must be one of: active, suspended, trial, closed",
                    status_str
                )))
            }
        }
    } else {
        TenantStatus::Active
    };

    let trial_expires_at = req
        .trial_expires_at
        .map(|secs| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs));

    let tenant = Tenant {
        id: TenantId::new(),
        name: req.name,
        description: req.description,
        status,
        contact_email: req.contact_email,
        contact_phone: req.contact_phone,
        max_users: req.max_users,
        max_phone_numbers: req.max_phone_numbers,
        max_concurrent_calls: req.max_concurrent_calls,
        billing_id: req.billing_id,
        created_at: now,
        updated_at: now,
        trial_expires_at,
        metadata: req.metadata,
        sip_domains,
    };

    storage.put_tenant(tenant.clone()).await?;

    let response: TenantResponse = tenant.into();
    Ok((StatusCode::CREATED, Json(response)))
}

/// Update an existing tenant
///
/// Tenant admins may update their own tenant (name, contact info, etc.)
/// but not cross-tenant. Platform admins may update any tenant.
pub async fn update_tenant(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTenantRequest>,
) -> Result<Json<TenantResponse>, ApiError> {
    let storage = require_storage!(state);
    let tenant_id = TenantId::from_str(&id)
        .map_err(|_| ApiError::BadRequest("Invalid tenant ID format".to_string()))?;
    require_tenant_access(identity.as_ref().map(|ext| &ext.0), &tenant_id)?;

    // Get existing tenant
    let mut tenant = storage
        .get_tenant(&tenant_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Tenant {} not found", id)))?;

    // Update fields if provided
    if let Some(name) = req.name {
        // Check if new name conflicts with another tenant
        if let Some(existing) = storage.get_tenant_by_name(&name).await? {
            if existing.id != tenant.id {
                return Err(ApiError::Conflict(format!(
                    "Tenant with name '{}' already exists",
                    name
                )));
            }
        }
        tenant.name = name;
    }

    if let Some(description) = req.description {
        tenant.description = Some(description);
    }

    if let Some(status_str) = req.status {
        tenant.status = match status_str.to_lowercase().as_str() {
            "active" => TenantStatus::Active,
            "suspended" => TenantStatus::Suspended,
            "trial" => TenantStatus::Trial,
            "closed" => TenantStatus::Closed,
            _ => {
                return Err(ApiError::BadRequest(format!(
                    "Invalid status '{}'. Must be one of: active, suspended, trial, closed",
                    status_str
                )))
            }
        };
    }

    if let Some(contact_email) = req.contact_email {
        tenant.contact_email = Some(contact_email);
    }

    if let Some(contact_phone) = req.contact_phone {
        tenant.contact_phone = Some(contact_phone);
    }

    if let Some(max_users) = req.max_users {
        tenant.max_users = Some(max_users);
    }

    if let Some(max_phone_numbers) = req.max_phone_numbers {
        tenant.max_phone_numbers = Some(max_phone_numbers);
    }

    if let Some(max_concurrent_calls) = req.max_concurrent_calls {
        tenant.max_concurrent_calls = Some(max_concurrent_calls);
    }

    if let Some(billing_id) = req.billing_id {
        tenant.billing_id = Some(billing_id);
    }

    if let Some(trial_expires_at_secs) = req.trial_expires_at {
        tenant.trial_expires_at =
            Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(trial_expires_at_secs));
    }

    if let Some(metadata) = req.metadata {
        tenant.metadata = Some(metadata);
    }

    if let Some(raw_domains) = req.sip_domains {
        let normalized = normalize_sip_domains(&raw_domains)?;
        ensure_sip_domains_unique(storage.as_ref(), &normalized, Some(&tenant.id)).await?;
        tenant.sip_domains = normalized;
    }

    tenant.updated_at = SystemTime::now();

    storage.put_tenant(tenant.clone()).await?;

    Ok(Json(tenant.into()))
}

/// Delete a tenant (platform admin only)
pub async fn delete_tenant(
    State(state): State<ApiState>,
    identity: Option<Extension<AuthIdentity>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_platform(identity.as_ref().map(|ext| &ext.0))?;

    let storage = require_storage!(state);
    let tenant_id = TenantId::from_str(&id)
        .map_err(|_| ApiError::BadRequest("Invalid tenant ID format".to_string()))?;

    // Check if tenant exists
    if storage.get_tenant(&tenant_id).await?.is_none() {
        return Err(ApiError::NotFound(format!("Tenant {} not found", id)));
    }

    storage.delete_tenant(&tenant_id).await?;

    Ok(StatusCode::NO_CONTENT)
}
