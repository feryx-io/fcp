//! Flow API endpoints.
//!
//! RESTful API for managing call flows (programmable voice).

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use fcp_flow::{FlowDefinition, FlowQuery, FlowStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

/// Helper to get flow store from state or return error.
fn get_flow_store(state: &ApiState) -> Result<Arc<dyn FlowStore>, ApiError> {
    state
        .flow_store
        .clone()
        .ok_or_else(|| ApiError::Internal("Flow store not available".to_string()))
}

fn ensure_flow_in_tenant(
    flow: FlowDefinition,
    tenant_id: &str,
    id: &str,
) -> Result<FlowDefinition, ApiError> {
    if flow.tenant_id.as_deref() == Some(tenant_id) {
        Ok(flow)
    } else {
        Err(ApiError::NotFound(format!("Flow not found: {}", id)))
    }
}

/// Flow response model.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FlowResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: u32,
    pub initial_node: String,
    pub nodes: serde_json::Value,     // FlowNode map as JSON
    pub variables: serde_json::Value, // Variables map as JSON
    pub tenant_id: Option<String>,
    pub enabled: bool,
    pub created_at: Option<String>, // ISO 8601
    pub updated_at: Option<String>, // ISO 8601
}

impl From<FlowDefinition> for FlowResponse {
    fn from(flow: FlowDefinition) -> Self {
        // Serialize nodes and variables to JSON
        let nodes = serde_json::to_value(&flow.nodes).unwrap_or_else(|_| serde_json::json!({}));
        let variables =
            serde_json::to_value(&flow.variables).unwrap_or_else(|_| serde_json::json!({}));

        // Convert timestamps to ISO 8601
        let created_at = flow.created_at.map(|t| t.to_rfc3339());
        let updated_at = flow.updated_at.map(|t| t.to_rfc3339());

        Self {
            id: flow.id,
            name: flow.name,
            description: flow.description,
            version: flow.version,
            initial_node: flow.initial_node,
            nodes,
            variables,
            tenant_id: flow.tenant_id,
            enabled: flow.enabled,
            created_at,
            updated_at,
        }
    }
}

/// Create flow request model.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateFlowRequest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub initial_node: String,
    pub nodes: serde_json::Value, // FlowNode map as JSON
    #[serde(default)]
    pub variables: serde_json::Value, // Variables map as JSON
    pub tenant_id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Update flow request model.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateFlowRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub initial_node: Option<String>,
    pub nodes: Option<serde_json::Value>,
    pub variables: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

/// Query parameters for listing flows.
#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListFlowsQuery {
    /// Filter by tenant ID
    pub tenant_id: Option<String>,

    /// Filter by enabled status
    pub enabled: Option<bool>,

    /// Search by name (partial match)
    pub name: Option<String>,

    /// Search by description (partial match)
    pub description: Option<String>,
}

/// List flows with filters and pagination.
///
/// GET /api/v1/flows
#[utoipa::path(
    get,
    path = "/api/v1/flows",
    tag = "flows",
    operation_id = "flows_list_flows",
    params(ListFlowsQuery, crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<FlowResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_flows(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Query(params): Query<ListFlowsQuery>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    debug!("Listing flows with filters: {:?}", params);

    let store = get_flow_store(&state)?;
    let limit = page.limit()?;
    let offset = page.offset()?;

    // Build FlowQuery from request parameters
    let query = FlowQuery {
        tenant_id: Some(tenant.0.to_string()),
        enabled: params.enabled,
        name_contains: params.name.clone(),
        description_contains: params.description.clone(),
        updated_after: None,
        updated_before: None,
        limit,
        offset,
        sort_by: Some(fcp_flow::storage::SortField::UpdatedAt),
        sort_order: fcp_flow::storage::SortOrder::Desc,
    };

    // Execute query
    let result = store
        .query_flows(&query)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to query flows: {}", e)))?;

    // Convert to response
    let flows: Vec<FlowResponse> = result.flows.into_iter().map(FlowResponse::from).collect();

    let more = offset + flows.len() < result.total;
    Ok(Json(crate::page::offset_page(
        flows,
        offset,
        more,
        Some(result.total as u64),
    )))
}

/// Get a flow by ID.
///
/// GET /api/v1/flows/:id
#[utoipa::path(
    get,
    path = "/api/v1/flows/{id}",
    tag = "flows",
    operation_id = "flows_get_flow",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = FlowResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_flow(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    debug!(flow_id = %id, "Getting flow");

    let store = get_flow_store(&state)?;

    let flow = store
        .get_flow(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get flow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Flow not found: {}", id)))
        .and_then(|flow| ensure_flow_in_tenant(flow, &tenant.0.to_string(), &id))?;

    Ok(Json(FlowResponse::from(flow)))
}

/// Create a new flow.
///
/// POST /api/v1/flows
#[utoipa::path(
    post,
    path = "/api/v1/flows",
    tag = "flows",
    operation_id = "flows_create_flow",
    request_body = CreateFlowRequest,
    responses(
        (status = 201, description = "OK", body = FlowResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_flow(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateFlowRequest>,
) -> Result<impl IntoResponse, ApiError> {
    info!(flow_id = %req.id, flow_name = %req.name, "Creating flow");

    let store = get_flow_store(&state)?;

    // Deserialize nodes and variables from JSON
    let nodes = serde_json::from_value(req.nodes)
        .map_err(|e| ApiError::BadRequest(format!("Invalid nodes: {}", e)))?;

    // `variables` is optional: left out, it is `null`, and no variables.
    let variables = match req.variables {
        serde_json::Value::Null => Default::default(),
        variables => serde_json::from_value(variables)
            .map_err(|e| ApiError::BadRequest(format!("Invalid variables: {}", e)))?,
    };

    // Create flow definition
    let flow = FlowDefinition {
        id: req.id.clone(),
        name: req.name,
        description: req.description,
        version: 1,
        initial_node: req.initial_node,
        nodes,
        variables,
        tenant_id: Some(tenant.0.to_string()),
        enabled: req.enabled,
        created_at: None, // Will be set by storage
        updated_at: None,
    };

    // Insert flow
    store
        .insert_flow(flow.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create flow: {}", e)))?;

    info!(flow_id = %req.id, "Flow created successfully");

    Ok((
        axum::http::StatusCode::CREATED,
        Json(FlowResponse::from(flow)),
    ))
}

/// Update a flow.
///
/// PUT /api/v1/flows/:id
#[utoipa::path(
    put,
    path = "/api/v1/flows/{id}",
    tag = "flows",
    operation_id = "flows_update_flow",
    params(("id" = String, Path)),
    request_body = UpdateFlowRequest,
    responses(
        (status = 200, description = "OK", body = FlowResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_flow(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateFlowRequest>,
) -> Result<impl IntoResponse, ApiError> {
    info!(flow_id = %id, "Updating flow");

    let store = get_flow_store(&state)?;

    // Get existing flow
    let mut flow = store
        .get_flow(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get flow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Flow not found: {}", id)))
        .and_then(|flow| ensure_flow_in_tenant(flow, &tenant.0.to_string(), &id))?;

    // Update fields
    if let Some(name) = req.name {
        flow.name = name;
    }
    if let Some(description) = req.description {
        flow.description = Some(description);
    }
    if let Some(initial_node) = req.initial_node {
        flow.initial_node = initial_node;
    }
    if let Some(nodes) = req.nodes {
        flow.nodes = serde_json::from_value(nodes)
            .map_err(|e| ApiError::BadRequest(format!("Invalid nodes: {}", e)))?;
    }
    if let Some(variables) = req.variables {
        flow.variables = serde_json::from_value(variables)
            .map_err(|e| ApiError::BadRequest(format!("Invalid variables: {}", e)))?;
    }
    if let Some(enabled) = req.enabled {
        flow.enabled = enabled;
    }

    // Update flow (a new version), and answer with what was stored: its
    // version number is the store's to give.
    store
        .update_flow(flow)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update flow: {}", e)))?;
    let flow = store
        .get_flow(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get flow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Flow not found: {}", id)))?;

    info!(flow_id = %id, version = flow.version, "Flow updated successfully");

    Ok(Json(FlowResponse::from(flow)))
}

/// Delete a flow.
///
/// DELETE /api/v1/flows/:id
#[utoipa::path(
    delete,
    path = "/api/v1/flows/{id}",
    tag = "flows",
    operation_id = "flows_delete_flow",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = serde_json::Value),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_flow(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    info!(flow_id = %id, "Deleting flow");

    let store = get_flow_store(&state)?;

    store
        .get_flow(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get flow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Flow not found: {}", id)))
        .and_then(|flow| ensure_flow_in_tenant(flow, &tenant.0.to_string(), &id))?;

    store
        .delete_flow(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete flow: {}", e)))?;

    info!(flow_id = %id, "Flow deleted successfully");

    Ok(Json(serde_json::json!({
        "message": "Flow deleted successfully",
        "id": id
    })))
}

/// Get all versions of a flow.
///
/// GET /api/v1/flows/:id/versions
#[utoipa::path(
    get,
    path = "/api/v1/flows/{id}/versions",
    tag = "flows",
    operation_id = "flows_list_flow_versions",
    params(("id" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<FlowResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_flow_versions(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    debug!(flow_id = %id, "Listing flow versions");

    let store = get_flow_store(&state)?;

    let versions = store
        .list_flow_versions(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list flow versions: {}", e)))?;
    let tenant_id = tenant.0.to_string();
    let responses: Vec<FlowResponse> = versions
        .into_iter()
        .filter(|flow| flow.tenant_id.as_deref() == Some(tenant_id.as_str()))
        .map(FlowResponse::from)
        .collect();

    if responses.is_empty() {
        return Err(ApiError::NotFound(format!("Flow not found: {}", id)));
    }

    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Descending,
        |r| format!("{:010}", r.version),
    )?))
}

/// One version of a flow: what a call that ran it ran (a flow trace names
/// the version).
///
/// GET /api/v1/flows/:id/versions/:version
#[utoipa::path(
    get,
    path = "/api/v1/flows/{id}/versions/{version}",
    tag = "flows",
    operation_id = "flows_get_flow_version",
    params(("id" = String, Path), ("version" = u32, Path)),
    responses(
        (status = 200, description = "OK", body = FlowResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_flow_version(
    State(state): State<ApiState>,
    tenant: crate::auth::TenantContext,
    Path((id, version)): Path<(String, u32)>,
) -> Result<impl IntoResponse, ApiError> {
    let store = get_flow_store(&state)?;
    let flow = store
        .get_flow_version(&id, version)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get flow version: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Flow version not found: {id} v{version}")))
        .and_then(|flow| ensure_flow_in_tenant(flow, &tenant.0.to_string(), &id))?;
    Ok(Json(FlowResponse::from(flow)))
}
