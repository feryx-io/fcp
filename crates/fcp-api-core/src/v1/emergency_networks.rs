//! A tenant's networks, and the sites they are at
//! (`docs/E911_BLOCK_2.md` §6.1).
//!
//! A device that moves — a softphone, a mobile app, a browser — is at the
//! site of the network it is seen on, not the site it was provisioned at.
//! A tenant maps its ranges here; the most specific match wins, and
//! `applies_to` says whether desk phones are placed this way too.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::e911::{EmergencyNetwork, NetworkApplies};
use fcp_core::{EmergencyNetworkId, LocationId};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::SystemTime;

use crate::auth::TenantContext;
use crate::error::ApiError;
use crate::state::ApiState;

fn require_store(
    state: &ApiState,
) -> Result<&std::sync::Arc<dyn fcp_core::e911::EmergencyStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.emergency_store())
        .ok_or_else(|| ApiError::NotFound("Emergency storage is not configured".to_string()))
}

#[derive(Debug, Serialize)]
pub struct NetworkResponse {
    pub id: String,
    pub tenant_id: String,
    pub cidr: String,
    pub location_id: String,
    pub name: Option<String>,
    pub priority: i32,
    /// `nomadic` (the default) or `all`.
    pub applies_to: String,
    #[serde(with = "crate::time::epoch_secs")]
    pub created_at: u64,
    #[serde(with = "crate::time::epoch_secs")]
    pub updated_at: u64,
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl From<EmergencyNetwork> for NetworkResponse {
    fn from(n: EmergencyNetwork) -> Self {
        Self {
            id: n.id.to_string(),
            tenant_id: n.tenant_id.to_string(),
            cidr: n.cidr.to_string(),
            location_id: n.location_id.to_string(),
            name: n.name,
            priority: n.priority,
            applies_to: n.applies_to.to_string(),
            created_at: unix_secs(n.created_at),
            updated_at: unix_secs(n.updated_at),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateNetworkRequest {
    /// A range (`10.1.2.0/24`) or a single address.
    pub cidr: String,
    /// The site an address on this network is at.
    pub location_id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    /// `nomadic` (the default) or `all`.
    #[serde(default)]
    pub applies_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNetworkRequest {
    pub cidr: Option<String>,
    pub location_id: Option<String>,
    pub name: Option<String>,
    pub priority: Option<i32>,
    pub applies_to: Option<String>,
}

/// A range, or a single address as a host range.
fn parse_cidr(value: &str) -> Result<ipnetwork::IpNetwork, ApiError> {
    let value = value.trim();
    value
        .parse::<ipnetwork::IpNetwork>()
        .or_else(|_| {
            value
                .parse::<std::net::IpAddr>()
                .map(ipnetwork::IpNetwork::from)
        })
        .map_err(|_| ApiError::BadRequest(format!("cidr: '{value}' is not an address or range")))
}

fn parse_applies_to(value: &str) -> Result<NetworkApplies, ApiError> {
    value.parse().map_err(ApiError::BadRequest)
}

/// The site must be one of this tenant's.
async fn require_location(
    state: &ApiState,
    tenant: &TenantContext,
    location_id: &str,
) -> Result<LocationId, ApiError> {
    use fcp_core::storage::SiteLocationStore;

    let id = LocationId::from_str(location_id)
        .map_err(|e| ApiError::BadRequest(format!("location_id: {e}")))?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Storage is not configured".to_string()))?;
    let site = storage
        .get_location(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    match site {
        Some(site) if site.tenant_id == tenant.0 => Ok(id),
        _ => Err(ApiError::BadRequest(format!(
            "location {location_id} is not a site of this tenant"
        ))),
    }
}

/// A network of this tenant, by id.
async fn require_network(
    state: &ApiState,
    tenant: &TenantContext,
    id: &str,
) -> Result<EmergencyNetwork, ApiError> {
    let network_id = EmergencyNetworkId::from_str(id)
        .map_err(|e| ApiError::BadRequest(format!("network id: {e}")))?;
    let network = require_store(state)?
        .get_emergency_network(&network_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    match network {
        Some(network) if network.tenant_id == tenant.0 => Ok(network),
        // A network of another tenant is not this tenant's to see.
        _ => Err(ApiError::NotFound(format!("Network {id} not found"))),
    }
}

pub async fn list_networks(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let networks = require_store(&state)?
        .list_emergency_networks(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let out: Vec<NetworkResponse> = networks.into_iter().map(Into::into).collect();
    Ok(Json(out))
}

pub async fn get_network(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<Json<NetworkResponse>, ApiError> {
    let network = require_network(&state, &tenant, &id).await?;
    Ok(Json(network.into()))
}

pub async fn create_network(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<CreateNetworkRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let cidr = parse_cidr(&req.cidr)?;
    let location_id = require_location(&state, &tenant, &req.location_id).await?;
    let mut network = EmergencyNetwork::new(tenant.0, cidr, location_id);
    if let Some(name) = req.name {
        crate::validation::reject_crlf("name", &name)?;
        network.name = Some(name);
    }
    if let Some(priority) = req.priority {
        network.priority = priority;
    }
    if let Some(applies_to) = req.applies_to {
        network.applies_to = parse_applies_to(&applies_to)?;
    }
    require_store(&state)?
        .put_emergency_network(&network)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tracing::info!(
        tenant_id = %tenant.0,
        cidr = %network.cidr,
        location_id = %network.location_id,
        applies_to = %network.applies_to,
        "Emergency network mapped"
    );
    Ok((StatusCode::CREATED, Json(NetworkResponse::from(network))))
}

pub async fn update_network(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateNetworkRequest>,
) -> Result<Json<NetworkResponse>, ApiError> {
    let mut network = require_network(&state, &tenant, &id).await?;
    if let Some(cidr) = req.cidr {
        network.cidr = parse_cidr(&cidr)?;
    }
    if let Some(location_id) = req.location_id {
        network.location_id = require_location(&state, &tenant, &location_id).await?;
    }
    if let Some(name) = req.name {
        crate::validation::reject_crlf("name", &name)?;
        network.name = Some(name);
    }
    if let Some(priority) = req.priority {
        network.priority = priority;
    }
    if let Some(applies_to) = req.applies_to {
        network.applies_to = parse_applies_to(&applies_to)?;
    }
    network.updated_at = SystemTime::now();
    require_store(&state)?
        .put_emergency_network(&network)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(network.into()))
}

pub async fn delete_network(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let network = require_network(&state, &tenant, &id).await?;
    require_store(&state)?
        .delete_emergency_network(&network.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}
