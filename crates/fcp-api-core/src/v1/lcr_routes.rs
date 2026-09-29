//! A tenant's least-cost routes (`docs/TENANT_TRUNKS.md` §12, phase 6c).
//!
//! Each route prices a dialled-number prefix on one of the tenant's own
//! trunks; every call manager orders that tenant's trunks by them. They
//! take the permissions a tenant's trunks take, a route must name one of
//! the tenant's own trunks, and every change is announced so nodes re-read
//! them at once.

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::billing::Decimal;
use fcp_core::permissions::Permission;
use fcp_core::storage::TrunkStore;
use fcp_core::{LcrRoute, LcrRouteId, TenantId, TrunkId};
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

#[derive(Debug, Serialize)]
pub struct LcrRouteResponse {
    pub id: String,
    pub tenant_id: String,
    pub prefix: String,
    pub trunk_id: String,
    pub cost_per_minute: Decimal,
    pub priority: u32,
    pub enabled: bool,
    pub description: Option<String>,
    #[serde(with = "crate::time::epoch_secs")]
    pub created_at: u64,
    #[serde(with = "crate::time::epoch_secs")]
    pub updated_at: u64,
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl From<LcrRoute> for LcrRouteResponse {
    fn from(r: LcrRoute) -> Self {
        Self {
            id: r.id.to_string(),
            tenant_id: r.tenant_id.to_string(),
            prefix: r.prefix,
            trunk_id: r.trunk_id.to_string(),
            cost_per_minute: r.cost_per_minute,
            priority: r.priority,
            enabled: r.enabled,
            description: r.description,
            created_at: unix_secs(r.created_at),
            updated_at: unix_secs(r.updated_at),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListLcrRoutesQuery {
    /// Only the routes on this trunk.
    pub trunk_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLcrRouteRequest {
    pub prefix: String,
    pub trunk_id: String,
    pub cost_per_minute: Decimal,
    pub priority: Option<u32>,
    pub enabled: Option<bool>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLcrRouteRequest {
    pub prefix: Option<String>,
    pub trunk_id: Option<String>,
    pub cost_per_minute: Option<Decimal>,
    pub priority: Option<u32>,
    pub enabled: Option<bool>,
    /// An empty string clears it.
    pub description: Option<String>,
}

pub async fn list_lcr_routes(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Query(query): Query<ListLcrRoutesQuery>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let routes = storage
        .list_lcr_routes(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list least-cost routes: {e}")))?;
    let response: Vec<LcrRouteResponse> = routes
        .into_iter()
        .filter(|r| {
            query
                .trunk_id
                .as_deref()
                .is_none_or(|trunk| r.trunk_id.as_str() == trunk)
        })
        .map(LcrRouteResponse::from)
        .collect();
    Ok(Json(crate::page::paginate(
        response,
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.prefix, r.id),
    )?))
}

pub async fn get_lcr_route(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let route = visible_route(storage.as_ref(), &tenant.0, id).await?;
    Ok(Json(LcrRouteResponse::from(route)))
}

pub async fn create_lcr_route(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateLcrRouteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);

    let mut route = LcrRoute::new(
        tenant.0,
        req.prefix.trim(),
        TrunkId::from_string(req.trunk_id.trim().to_string()),
        req.cost_per_minute,
    );
    if let Some(priority) = req.priority {
        route.priority = priority;
    }
    if let Some(enabled) = req.enabled {
        route.enabled = enabled;
    }
    route.description = req.description.filter(|d| !d.trim().is_empty());
    check_route(storage.as_ref(), &route).await?;
    check_unique(storage.as_ref(), &route).await?;

    info!(route_id = %route.id, prefix = %route.prefix, trunk_id = %route.trunk_id, "Creating least-cost route");
    storage
        .put_lcr_route(route.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create least-cost route: {e}")))?;
    announce_route_change(&state, route.id.as_str(), route.tenant_id).await;
    Ok((StatusCode::CREATED, Json(LcrRouteResponse::from(route))))
}

pub async fn update_lcr_route(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateLcrRouteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let mut route = visible_route(storage.as_ref(), &tenant.0, id).await?;

    if let Some(prefix) = req.prefix {
        route.prefix = prefix.trim().to_string();
    }
    if let Some(trunk) = req.trunk_id {
        route.trunk_id = TrunkId::from_string(trunk.trim().to_string());
    }
    if let Some(cost) = req.cost_per_minute {
        route.cost_per_minute = cost;
    }
    if let Some(priority) = req.priority {
        route.priority = priority;
    }
    if let Some(enabled) = req.enabled {
        route.enabled = enabled;
    }
    if let Some(description) = req.description {
        route.description = Some(description).filter(|d| !d.trim().is_empty());
    }
    route.updated_at = SystemTime::now();
    check_route(storage.as_ref(), &route).await?;
    check_unique(storage.as_ref(), &route).await?;

    info!(route_id = %route.id, prefix = %route.prefix, trunk_id = %route.trunk_id, "Updating least-cost route");
    storage
        .put_lcr_route(route.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update least-cost route: {e}")))?;
    announce_route_change(&state, route.id.as_str(), route.tenant_id).await;
    Ok(Json(LcrRouteResponse::from(route)))
}

pub async fn delete_lcr_route(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let route = visible_route(storage.as_ref(), &tenant.0, id).await?;

    info!(route_id = %route.id, prefix = %route.prefix, "Deleting least-cost route");
    storage
        .delete_lcr_route(&route.id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete least-cost route: {e}")))?;
    announce_route_change(&state, route.id.as_str(), route.tenant_id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The route `id`, if it is this tenant's; another tenant's reads as
/// missing.
async fn visible_route<S>(storage: &S, tenant: &TenantId, id: String) -> Result<LcrRoute, ApiError>
where
    S: TrunkStore + ?Sized,
{
    storage
        .get_lcr_route(&LcrRouteId::from_string(id))
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get least-cost route: {e}")))?
        .filter(|r| &r.tenant_id == tenant)
        .ok_or_else(|| ApiError::NotFound("Least-cost route not found".to_string()))
}

/// A route says something a node can use, about a trunk of the tenant's
/// own: a route may not price another tenant's carrier, nor the
/// platform's, which the node's own `[lcr]` prices.
async fn check_route<S>(storage: &S, route: &LcrRoute) -> Result<(), ApiError>
where
    S: TrunkStore + ?Sized,
{
    route.validate().map_err(ApiError::BadRequest)?;
    let owned = storage
        .get_trunk(&route.trunk_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read trunk: {e}")))?
        .is_some_and(|trunk| trunk.tenant_id == Some(route.tenant_id));
    if !owned {
        return Err(ApiError::BadRequest(format!(
            "trunk_id '{}' is not one of this tenant's trunks",
            route.trunk_id
        )));
    }
    Ok(())
}

/// One route per prefix and trunk: a second is a conflict, not a tie.
async fn check_unique<S>(storage: &S, route: &LcrRoute) -> Result<(), ApiError>
where
    S: TrunkStore + ?Sized,
{
    let existing = storage
        .list_lcr_routes(&route.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list least-cost routes: {e}")))?;
    if existing
        .iter()
        .any(|r| r.id != route.id && r.prefix == route.prefix && r.trunk_id == route.trunk_id)
    {
        return Err(ApiError::Conflict(format!(
            "a route for prefix '{}' on trunk '{}' already exists",
            route.prefix, route.trunk_id
        )));
    }
    Ok(())
}

/// Tell every node that a route changed, so the ones routing calls re-read
/// the tenants' trunks and routes now rather than at the next backstop tick.
async fn announce_route_change(state: &ApiState, entity_id: &str, tenant_id: TenantId) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "lcr_route".to_string(),
            entity_id: entity_id.to_string(),
            tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}
