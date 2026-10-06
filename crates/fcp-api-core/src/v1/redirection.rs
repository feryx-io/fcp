//! A tenant's accounts with vendors' redirection services, and each
//! device's registration with its vendor's (`docs/PROVISIONING.md` §10,
//! `docs/IDENTITY_AND_DEVICES.md` phase 8).

use crate::redirection::{connector, PhoneRegistration, RPS_MAX_PASSWORD};
use crate::{auth::AuthIdentity, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::provisioning::{
    DeviceRedirection, RedirectionAccount, RedirectionService, RedirectionState,
};
use fcp_core::storage::DeviceStore;
use fcp_core::{Device, DeviceId, TenantStore};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::SystemTime;
use tracing::{info, warn};

fn storage(state: &ApiState) -> Result<&std::sync::Arc<fcp_core::DynStorage>, ApiError> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Storage is required but not configured".to_string()))
}

fn store(
    storage: &fcp_core::DynStorage,
) -> Result<&std::sync::Arc<dyn fcp_core::provisioning::ProvisioningStore>, ApiError> {
    storage
        .provisioning_store()
        .ok_or_else(|| ApiError::NotFound("Provisioning is not available here".to_string()))
}

fn service_of(name: &str) -> Result<RedirectionService, ApiError> {
    RedirectionService::from_str(name).map_err(ApiError::NotFound)
}

/// The service a device's phone asks, by its vendor.
fn service_for(device: &Device) -> Option<RedirectionService> {
    let vendor = device.vendor.or_else(|| {
        device
            .mac_address
            .as_deref()
            .and_then(fcp_core::provisioning::vendor_of_mac)
    })?;
    RedirectionService::for_vendor(vendor)
}

/// The lock each device's syncs take, so they run one at a time.
fn sync_lock(device: &fcp_core::DeviceId) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        dashmap::DashMap<fcp_core::DeviceId, std::sync::Arc<tokio::sync::Mutex<()>>>,
    > = std::sync::OnceLock::new();
    LOCKS
        .get_or_init(dashmap::DashMap::new)
        .entry(*device)
        .or_default()
        .clone()
}

/// Bring a device's registration with its vendor in line with the device:
/// removed from a service it no longer belongs on (its MAC changed, the
/// device went, the account is off), registered where it belongs and is
/// not yet — or again, with `force`. `token` is the device's provisioning
/// token when the caller has it in hand; without it, a phone the service
/// must give credentials to gets a new one.
pub(crate) async fn sync_device(
    state: &ApiState,
    device: &Device,
    base: Option<&str>,
    token: Option<&str>,
    force: bool,
) -> Result<Option<DeviceRedirection>, ApiError> {
    // One device's syncs one at a time: a device made and then changed at
    // once (its template set) starts two, and two registering together
    // left the service holding one's credentials and the device the
    // other's. The second, once the first is done, sees its registration.
    let lock = sync_lock(&device.id);
    let _held = lock.lock().await;
    let storage = storage(state)?;
    let store = store(storage)?;
    let prior = store.get_device_redirection(&device.id).await?;
    // A token in hand is the phone's only while the device still has it:
    // another sync may have given the phone a new one meanwhile.
    let current = storage.get_device(&device.id).await?;
    let token = token.filter(|t| {
        current.as_ref().is_some_and(|d| {
            d.provisioning_token_hash.as_deref()
                == Some(crate::v1::devices::hash_device_token(t).as_str())
        })
    });
    let mac = device
        .mac_address
        .as_deref()
        .and_then(fcp_core::provisioning::mac_of);
    let target = match (service_for(device), &mac) {
        (Some(service), Some(_)) if device.enabled => store
            .get_redirection_account(&device.tenant_id, service)
            .await?
            .filter(|a| a.enabled),
        _ => None,
    };

    // Off a service it no longer belongs on.
    if let Some(prior) = &prior {
        let stays = target.as_ref().is_some_and(|a| {
            a.service == prior.service && mac.as_deref() == Some(prior.mac_address.as_str())
        });
        if !stays {
            if let Some(account) = store
                .get_redirection_account(&prior.tenant_id, prior.service)
                .await?
            {
                if let Err(e) = connector(&state.redirection, &account)
                    .unregister(&prior.mac_address, prior.vendor_ref.as_deref())
                    .await
                {
                    warn!(device_id = %device.id, error = %e, "Could not remove a phone from its redirection service");
                }
            }
            store.delete_device_redirection(&device.id).await?;
        }
    }
    let (Some(account), Some(mac)) = (target, mac) else {
        return Ok(None);
    };
    if let Some(prior) = &prior {
        let same = prior.service == account.service
            && prior.mac_address == mac
            && prior.state == RedirectionState::Registered;
        if same && !force && token.is_none() {
            return Ok(Some(prior.clone()));
        }
    }

    let failed = |detail: String| DeviceRedirection {
        tenant_id: device.tenant_id,
        device_id: device.id,
        service: account.service,
        mac_address: mac.clone(),
        vendor_ref: None,
        state: RedirectionState::Failed,
        detail: Some(detail),
        with_credentials: false,
        at: SystemTime::now(),
    };
    let key = store
        .get_provisioning_key(&device.tenant_id)
        .await?
        .map(|k| k.key);
    let url = match (base, key) {
        (Some(base), Some(key)) => format!("{}/provision/{key}/", base.trim_end_matches('/')),
        (None, _) => {
            let record = failed(
                "the admin's public address is unknown: set [api.provisioning] public_url"
                    .to_string(),
            );
            store.put_device_redirection(&record).await?;
            return Ok(Some(record));
        }
        (_, None) => {
            let record = failed("the tenant has no provisioning URL yet".to_string());
            store.put_device_redirection(&record).await?;
            return Ok(Some(record));
        }
    };

    let service = connector(&state.redirection, &account);
    // The phone's own credentials, where the service carries them: the
    // token in hand if it fits, else a new one when the tenant wants them.
    let requires = storage
        .get_tenant(&device.tenant_id)
        .await?
        .is_some_and(|t| t.provisioning.require_device_credentials);
    let password = if !service.carries_credentials() {
        None
    } else {
        match token.filter(|t| t.len() <= RPS_MAX_PASSWORD) {
            Some(t) => Some(t.to_string()),
            None if requires || token.is_some() => {
                let fresh = crate::v1::devices::generate_provisioning_token();
                if let Some(mut current) = storage.get_device(&device.id).await? {
                    current.provisioning_token_hash =
                        Some(crate::v1::devices::hash_device_token(&fresh));
                    current.updated_at = SystemTime::now();
                    storage.put_device(current).await?;
                }
                Some(fresh)
            }
            None => None,
        }
    };
    let phone = PhoneRegistration {
        mac: mac.clone(),
        url,
        username: mac.clone(),
        password: password.clone(),
        remark: device.name.clone(),
    };
    let record = match service.register(&phone).await {
        Ok(vendor_ref) => DeviceRedirection {
            tenant_id: device.tenant_id,
            device_id: device.id,
            service: account.service,
            mac_address: mac,
            vendor_ref,
            state: RedirectionState::Registered,
            detail: None,
            with_credentials: password.is_some(),
            at: SystemTime::now(),
        },
        Err(e) => failed(e.0),
    };
    store.put_device_redirection(&record).await?;
    info!(
        device_id = %device.id,
        service = record.service.as_str(),
        state = ?record.state,
        "A phone's redirection registration"
    );
    Ok(Some(record))
}

/// [`sync_device`] on a task of its own: a vendor's service does not hold
/// up the request that changed the device.
pub(crate) fn device_changed(
    state: &ApiState,
    device: &Device,
    headers: &HeaderMap,
    token: Option<String>,
) {
    let has_store = state
        .storage
        .as_ref()
        .is_some_and(|s| s.provisioning_store().is_some());
    if !has_store {
        return;
    }
    let state = state.clone();
    let device = device.clone();
    let base = crate::v1::provisioning::public_base(&state, headers);
    tokio::spawn(async move {
        if let Err(e) = sync_device(&state, &device, base.as_deref(), token.as_deref(), false).await
        {
            warn!(device_id = %device.id, error = e.message(), "Could not sync a phone with its redirection service");
        }
    });
}

/// A deleted device's phone, taken off its vendor's service.
pub(crate) fn device_removed(state: &ApiState, device: &Device) {
    let mut gone = device.clone();
    gone.enabled = false;
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = sync_device(&state, &gone, None, None, false).await {
            warn!(device_id = %gone.id, error = e.message(), "Could not remove a phone from its redirection service");
        }
    });
}

// ---- The tenant's accounts ----

/// A tenant's account with a redirection service; the secret is never shown.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RedirectionAccountResponse {
    pub service: RedirectionService,
    pub configured: bool,
    pub enabled: bool,
    /// Yealink: the AccessKey ID; Poly: the API user id.
    pub key_id: Option<String>,
    /// Poly: the Reference ID devices are associated under.
    pub account_id: Option<String>,
    /// Poly: the ZTP profile naming the tenant's provisioning URL.
    pub profile: Option<String>,
    /// Whether the service hands phones their own credentials.
    pub carries_credentials: bool,
    #[serde(default, with = "crate::time::rfc3339_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub updated_at: Option<SystemTime>,
}

fn account_response(
    service: RedirectionService,
    account: Option<&RedirectionAccount>,
) -> RedirectionAccountResponse {
    RedirectionAccountResponse {
        service,
        configured: account.is_some(),
        enabled: account.is_some_and(|a| a.enabled),
        key_id: account.map(|a| a.key_id.clone()),
        account_id: account.and_then(|a| a.account_id.clone()),
        profile: account.and_then(|a| a.profile.clone()),
        carries_credentials: service == RedirectionService::YealinkRps,
        updated_at: account.map(|a| a.updated_at),
    }
}

/// GET /api/v1/provisioning/redirection — each service and the tenant's
/// account with it.
#[utoipa::path(
    get,
    path = "/api/v1/provisioning/redirection",
    tag = "provisioning",
    operation_id = "provisioning_list_redirection",
    responses(
        (status = 200, description = "OK", body = crate::page::Page<RedirectionAccountResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_accounts(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let accounts = store(storage(&state)?)?
        .list_redirection_accounts(&tenant.0)
        .await?;
    let rows: Vec<RedirectionAccountResponse> = RedirectionService::ALL
        .into_iter()
        .map(|s| account_response(s, accounts.iter().find(|a| a.service == s)))
        .collect();
    Ok(Json(crate::page::paginate(
        rows,
        &crate::page::PageQuery::default(),
        crate::page::Order::Ascending,
        |r| r.service.as_str().to_string(),
    )?))
}

/// An account, made or changed.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PutRedirectionAccountRequest {
    /// Unset: on.
    pub enabled: Option<bool>,
    pub key_id: String,
    /// Required to make the account; unset on a change keeps it.
    pub secret: Option<String>,
    pub account_id: Option<String>,
    pub profile: Option<String>,
}

fn field(name: &str, value: &str, max: usize) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(format!(
            "{name} must be 1 to {max} characters"
        )));
    }
    Ok(value.to_string())
}

/// PUT /api/v1/provisioning/redirection/:service — the tenant's account.
#[utoipa::path(
    put,
    path = "/api/v1/provisioning/redirection/{service}",
    tag = "provisioning",
    operation_id = "provisioning_put_redirection",
    params(("service" = String, Path)),
    request_body = PutRedirectionAccountRequest,
    responses(
        (status = 200, description = "OK", body = RedirectionAccountResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn put_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(service): Path<String>,
    Json(req): Json<PutRedirectionAccountRequest>,
) -> Result<Json<RedirectionAccountResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let service = service_of(&service)?;
    let store = store(storage(&state)?)?;
    let existing = store.get_redirection_account(&tenant.0, service).await?;
    let secret = match (req.secret.as_deref(), &existing) {
        (Some(s), _) => field("secret", s, 256)?,
        (None, Some(existing)) => existing.secret.clone(),
        (None, None) => {
            return Err(ApiError::BadRequest("secret is required".to_string()));
        }
    };
    let optional = |name: &str, v: Option<String>| -> Result<Option<String>, ApiError> {
        match v.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(v) => field(name, v, 128).map(Some),
        }
    };
    let account = RedirectionAccount {
        tenant_id: tenant.0,
        service,
        enabled: req.enabled.unwrap_or(true),
        key_id: field("key_id", &req.key_id, 128)?,
        secret,
        account_id: optional("account_id", req.account_id)?,
        profile: optional("profile", req.profile)?,
        updated_at: SystemTime::now(),
    };
    if service == RedirectionService::PolyZtp
        && (account.account_id.is_none() || account.profile.is_none())
    {
        return Err(ApiError::BadRequest(
            "Poly ZTP needs account_id (the Reference ID) and profile".to_string(),
        ));
    }
    store.put_redirection_account(&account).await?;
    info!(tenant_id = %tenant.0, service = service.as_str(), "Redirection account saved");
    Ok(Json(account_response(service, Some(&account))))
}

/// DELETE /api/v1/provisioning/redirection/:service — forget the account;
/// phones registered with it stay registered at the vendor.
#[utoipa::path(
    delete,
    path = "/api/v1/provisioning/redirection/{service}",
    tag = "provisioning",
    operation_id = "provisioning_delete_redirection",
    params(("service" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(service): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let service = service_of(&service)?;
    if !store(storage(&state)?)?
        .delete_redirection_account(&tenant.0, service)
        .await?
    {
        return Err(ApiError::NotFound(format!(
            "No {} account",
            service.as_str()
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Whether an account's credentials work.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RedirectionCheckResponse {
    pub ok: bool,
    pub detail: Option<String>,
}

/// POST /api/v1/provisioning/redirection/:service/check — ask the service
/// whether the account's credentials work.
#[utoipa::path(
    post,
    path = "/api/v1/provisioning/redirection/{service}/check",
    tag = "provisioning",
    operation_id = "provisioning_check_redirection",
    params(("service" = String, Path)),
    responses(
        (status = 200, description = "OK", body = RedirectionCheckResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn check_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(service): Path<String>,
) -> Result<Json<RedirectionCheckResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let service = service_of(&service)?;
    let account = store(storage(&state)?)?
        .get_redirection_account(&tenant.0, service)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("No {} account", service.as_str())))?;
    let outcome = connector(&state.redirection, &account).check().await;
    Ok(Json(RedirectionCheckResponse {
        ok: outcome.is_ok(),
        detail: outcome.err().map(|e| e.0),
    }))
}

/// What a sync did.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RedirectionSyncResponse {
    /// The tenant's enabled devices of the service's vendor with a MAC.
    pub devices: u64,
    pub registered: u64,
    pub failed: u64,
}

/// POST /api/v1/provisioning/redirection/:service/sync — register every
/// phone of the service's vendor not yet registered (an account made after
/// its devices).
#[utoipa::path(
    post,
    path = "/api/v1/provisioning/redirection/{service}/sync",
    tag = "provisioning",
    operation_id = "provisioning_sync_redirection",
    params(("service" = String, Path)),
    responses(
        (status = 200, description = "OK", body = RedirectionSyncResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn sync_account(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: HeaderMap,
    Path(service): Path<String>,
) -> Result<Json<RedirectionSyncResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let service = service_of(&service)?;
    let storage = storage(&state)?;
    let base = crate::v1::provisioning::public_base(&state, &headers);
    let (mut devices, mut registered, mut failed) = (0, 0, 0);
    for device in storage.list_devices(&tenant.0).await? {
        if !device.enabled || device.mac_address.is_none() || service_for(&device) != Some(service)
        {
            continue;
        }
        devices += 1;
        match sync_device(&state, &device, base.as_deref(), None, false).await? {
            Some(r) if r.state == RedirectionState::Registered => registered += 1,
            _ => failed += 1,
        }
    }
    Ok(Json(RedirectionSyncResponse {
        devices,
        registered,
        failed,
    }))
}

// ---- A device's registration ----

/// Where a device stands with its vendor's service.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeviceRedirectionResponse {
    pub service: RedirectionService,
    pub mac_address: String,
    pub state: RedirectionState,
    pub detail: Option<String>,
    /// Whether the phone was given its own credentials through the service.
    pub with_credentials: bool,
    #[serde(with = "crate::time::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub at: SystemTime,
}

impl From<DeviceRedirection> for DeviceRedirectionResponse {
    fn from(r: DeviceRedirection) -> Self {
        Self {
            service: r.service,
            mac_address: r.mac_address,
            state: r.state,
            detail: r.detail,
            with_credentials: r.with_credentials,
            at: r.at,
        }
    }
}

async fn tenant_device(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    id: &str,
) -> Result<Device, ApiError> {
    let id = DeviceId::from_str(id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid device ID: {e}")))?;
    storage(state)?
        .get_device(&id)
        .await?
        .filter(|d| d.tenant_id == *tenant)
        .ok_or_else(|| ApiError::NotFound(format!("Device {id} not found")))
}

/// GET /api/v1/devices/:id/redirection
#[utoipa::path(
    get,
    path = "/api/v1/devices/{id}/redirection",
    tag = "provisioning",
    operation_id = "provisioning_device_redirection",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = DeviceRedirectionResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn device_redirection(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<Json<DeviceRedirectionResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceRead, state.deployment_mode)?;
    let device = tenant_device(&state, &tenant.0, &id).await?;
    let record = store(storage(&state)?)?
        .get_device_redirection(&device.id)
        .await?
        .ok_or_else(|| {
            ApiError::NotFound(
                "The device is not registered with a redirection service".to_string(),
            )
        })?;
    Ok(Json(record.into()))
}

/// POST /api/v1/devices/:id/redirection — register the device's phone
/// with its vendor's service now (again, if it was).
#[utoipa::path(
    post,
    path = "/api/v1/devices/{id}/redirection",
    tag = "provisioning",
    operation_id = "provisioning_register_device",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = DeviceRedirectionResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn register_device(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<DeviceRedirectionResponse>, ApiError> {
    crate::auth::check_permission(&identity, Permission::DeviceUpdate, state.deployment_mode)?;
    let device = tenant_device(&state, &tenant.0, &id).await?;
    let base = crate::v1::provisioning::public_base(&state, &headers);
    let record = sync_device(&state, &device, base.as_deref(), None, true)
        .await?
        .ok_or_else(|| {
            ApiError::BadRequest(
                "No redirection account is on for this device's vendor, or it has no MAC"
                    .to_string(),
            )
        })?;
    Ok(Json(record.into()))
}
