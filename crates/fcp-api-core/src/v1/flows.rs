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
use tracing::{debug, info, warn};

/// Helper to get flow store from state or return error.
fn get_flow_store(state: &ApiState) -> Result<Arc<dyn FlowStore>, ApiError> {
    state
        .flow_store
        .clone()
        .ok_or_else(|| ApiError::Internal("Flow store not available".to_string()))
}

/// Flow response model.
#[derive(Debug, Serialize, Deserialize)]
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
    pub webhook_url: Option<String>,
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
            webhook_url: flow.webhook_url,
            created_at,
            updated_at,
        }
    }
}

/// Create flow request model.
#[derive(Debug, Deserialize)]
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
    pub webhook_url: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Update flow request model.
#[derive(Debug, Deserialize)]
pub struct UpdateFlowRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub initial_node: Option<String>,
    pub nodes: Option<serde_json::Value>,
    pub variables: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub webhook_url: Option<String>,
}

/// Query parameters for listing flows.
#[derive(Debug, Deserialize)]
pub struct ListFlowsQuery {
    /// Filter by tenant ID
    pub tenant_id: Option<String>,

    /// Filter by enabled status
    pub enabled: Option<bool>,

    /// Search by name (partial match)
    pub name: Option<String>,

    /// Search by description (partial match)
    pub description: Option<String>,

    /// Maximum number of results (default: 100, max: 1000)
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Offset for pagination (default: 0)
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    100
}

/// Paginated list of flows.
#[derive(Debug, Serialize)]
pub struct ListFlowsResponse {
    pub flows: Vec<FlowResponse>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

/// List flows with filters and pagination.
///
/// GET /api/v1/flows
pub async fn list_flows(
    State(state): State<ApiState>,
    Query(params): Query<ListFlowsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    debug!("Listing flows with filters: {:?}", params);

    let store = get_flow_store(&state)?;

    // Validate and cap limit
    let limit = if params.limit > 1000 {
        warn!(
            requested_limit = params.limit,
            "Requested limit exceeds maximum, capping at 1000"
        );
        1000
    } else if params.limit == 0 {
        default_limit()
    } else {
        params.limit
    };

    // Build FlowQuery from request parameters
    let query = FlowQuery {
        tenant_id: params.tenant_id.clone(),
        enabled: params.enabled,
        name_contains: params.name.clone(),
        description_contains: params.description.clone(),
        updated_after: None,
        updated_before: None,
        limit,
        offset: params.offset,
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

    Ok(Json(ListFlowsResponse {
        flows,
        total: result.total,
        limit: result.limit,
        offset: result.offset,
    }))
}

/// Get a flow by ID.
///
/// GET /api/v1/flows/:id
pub async fn get_flow(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    debug!(flow_id = %id, "Getting flow");

    let store = get_flow_store(&state)?;

    let flow = store
        .get_flow(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get flow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Flow not found: {}", id)))?;

    Ok(Json(FlowResponse::from(flow)))
}

/// Create a new flow.
///
/// POST /api/v1/flows
pub async fn create_flow(
    State(state): State<ApiState>,
    Json(req): Json<CreateFlowRequest>,
) -> Result<impl IntoResponse, ApiError> {
    info!(flow_id = %req.id, flow_name = %req.name, "Creating flow");

    let store = get_flow_store(&state)?;

    // Deserialize nodes and variables from JSON
    let nodes = serde_json::from_value(req.nodes)
        .map_err(|e| ApiError::BadRequest(format!("Invalid nodes: {}", e)))?;

    let variables = serde_json::from_value(req.variables)
        .map_err(|e| ApiError::BadRequest(format!("Invalid variables: {}", e)))?;

    // Create flow definition
    let flow = FlowDefinition {
        id: req.id.clone(),
        name: req.name,
        description: req.description,
        version: 1,
        initial_node: req.initial_node,
        nodes,
        variables,
        tenant_id: req.tenant_id,
        enabled: req.enabled,
        webhook_url: req.webhook_url,
        created_at: None, // Will be set by storage
        updated_at: None,
    };

    // Insert flow
    store
        .insert_flow(flow.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create flow: {}", e)))?;

    info!(flow_id = %req.id, "Flow created successfully");

    Ok(Json(FlowResponse::from(flow)))
}

/// Update a flow.
///
/// PUT /api/v1/flows/:id
pub async fn update_flow(
    State(state): State<ApiState>,
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
        .ok_or_else(|| ApiError::NotFound(format!("Flow not found: {}", id)))?;

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
    if req.webhook_url.is_some() {
        flow.webhook_url = req.webhook_url;
    }

    // Update flow (increments version)
    store
        .update_flow(flow.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update flow: {}", e)))?;

    info!(flow_id = %id, version = flow.version + 1, "Flow updated successfully");

    Ok(Json(FlowResponse::from(flow)))
}

/// Delete a flow.
///
/// DELETE /api/v1/flows/:id
pub async fn delete_flow(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    info!(flow_id = %id, "Deleting flow");

    let store = get_flow_store(&state)?;

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
pub async fn list_flow_versions(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    debug!(flow_id = %id, "Listing flow versions");

    let store = get_flow_store(&state)?;

    let versions = store
        .list_flow_versions(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list flow versions: {}", e)))?;

    let responses: Vec<FlowResponse> = versions.into_iter().map(FlowResponse::from).collect();

    Ok(Json(serde_json::json!({
        "id": id,
        "versions": responses
    })))
}
