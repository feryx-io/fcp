//! Flow execution trace API endpoints.

use crate::{
    auth::{can_access_all_tenants, AuthIdentity, TenantContext},
    error::ApiError,
    state::ApiState,
};
use axum::{
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
    Json,
};
use fcp_core::flow_trace::{FlowTraceEventType, FlowTraceQuery};
use fcp_core::CdrStore;
use serde::{Deserialize, Serialize};
use std::time::UNIX_EPOCH;
use tracing::{debug, warn};

async fn call_is_visible_to_tenant(
    state: &ApiState,
    tenant: &TenantContext,
    call_id: &str,
) -> Result<bool, ApiError> {
    if let Some(active_call) = state
        .call_manager
        .all_calls()
        .into_iter()
        .find(|call| call.id.to_string() == call_id)
    {
        return Ok(active_call.tenant_id == tenant.0);
    }

    let Some(storage) = &state.storage else {
        return Ok(false);
    };

    let parsed_call_id = match call_id.parse::<fcp_core::CallId>() {
        Ok(call_id) => call_id,
        Err(_) => return Ok(false),
    };

    Ok(storage
        .get_cdr(&parsed_call_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to resolve flow trace tenant: {e}")))?
        .map(|cdr| cdr.tenant_id == tenant.0)
        .unwrap_or(false))
}

/// Flow trace response model.
#[derive(Debug, Serialize)]
pub struct FlowTraceResponse {
    pub id: i64,
    pub call_id: String,
    pub flow_id: String,
    pub flow_name: Option<String>,
    pub node_id: Option<String>,
    pub event_type: String,
    pub timestamp: u64, // Unix timestamp in milliseconds
    pub detail: Option<String>,
    pub flow_version: Option<u32>,
    pub elapsed_ms: Option<i64>,
}

impl From<fcp_core::flow_trace::FlowTraceEntry> for FlowTraceResponse {
    fn from(entry: fcp_core::flow_trace::FlowTraceEntry) -> Self {
        let ts_ms = entry
            .timestamp
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            id: entry.id,
            call_id: entry.call_id,
            flow_id: entry.flow_id,
            flow_name: entry.flow_name,
            node_id: entry.node_id,
            event_type: entry.event_type.to_string(),
            timestamp: ts_ms,
            detail: entry.detail,
            flow_version: entry.flow_version,
            elapsed_ms: entry.elapsed_ms,
        }
    }
}

/// Query parameters for listing flow traces.
#[derive(Debug, Deserialize)]
pub struct ListFlowTracesQuery {
    /// Filter by call ID
    pub call_id: Option<String>,
    /// Filter by flow ID
    pub flow_id: Option<String>,
    /// Filter by event type (started, node_executed, waiting_for_input, completed, failed)
    pub event_type: Option<String>,
    /// Filter by timestamp start (Unix timestamp in seconds)
    pub time_from: Option<String>,
    /// Filter by timestamp end (Unix timestamp in seconds)
    pub time_to: Option<String>,
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

/// Paginated list of flow traces.
#[derive(Debug, Serialize)]
pub struct ListFlowTracesResponse {
    pub traces: Vec<FlowTraceResponse>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

/// List flow execution traces with filters and pagination.
///
/// GET /api/v1/flow-traces
pub async fn list_flow_traces(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Query(params): Query<ListFlowTracesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Storage not configured".to_string()))?;

    let trace_store = storage
        .flow_trace_store()
        .ok_or_else(|| ApiError::NotFound("Flow trace store not configured".to_string()))?;

    let limit = params.limit.clamp(1, 1000);

    let mut query = FlowTraceQuery {
        call_id: params.call_id,
        flow_id: params.flow_id,
        event_type: None,
        time_from: None,
        time_to: None,
        limit: Some(limit),
        offset: Some(params.offset),
    };

    // Parse event_type
    if let Some(ref et) = params.event_type {
        query.event_type = Some(match et.as_str() {
            "started" => FlowTraceEventType::Started,
            "node_executed" => FlowTraceEventType::NodeExecuted,
            "waiting_for_input" => FlowTraceEventType::WaitingForInput,
            "completed" => FlowTraceEventType::Completed,
            "failed" => FlowTraceEventType::Failed,
            _ => {
                return Err(ApiError::BadRequest(format!(
                    "Invalid event_type '{}'. Valid: started, node_executed, waiting_for_input, completed, failed",
                    et
                )));
            }
        });
    }

    // Parse timestamps
    if let Some(ref ts) = params.time_from {
        let secs = ts
            .parse::<u64>()
            .map_err(|e| ApiError::BadRequest(format!("Invalid time_from: {}", e)))?;
        query.time_from = Some(UNIX_EPOCH + std::time::Duration::from_secs(secs));
    }
    if let Some(ref ts) = params.time_to {
        let secs = ts
            .parse::<u64>()
            .map_err(|e| ApiError::BadRequest(format!("Invalid time_to: {}", e)))?;
        query.time_to = Some(UNIX_EPOCH + std::time::Duration::from_secs(secs));
    }

    debug!(
        call_id = ?query.call_id,
        flow_id = ?query.flow_id,
        event_type = ?query.event_type,
        limit = limit,
        offset = params.offset,
        "Querying flow traces"
    );

    let (entries, total) = trace_store.query_flow_traces(&query).await.map_err(|e| {
        warn!(error = %e, "Failed to query flow traces");
        ApiError::Internal("Failed to query flow traces".to_string())
    })?;

    let allow_all_tenants = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0));
    let mut scoped_entries = Vec::new();
    for entry in entries {
        if allow_all_tenants || call_is_visible_to_tenant(&state, &tenant, &entry.call_id).await? {
            scoped_entries.push(entry);
        }
    }

    let traces: Vec<FlowTraceResponse> = scoped_entries
        .into_iter()
        .map(FlowTraceResponse::from)
        .collect();
    let scoped_total = traces.len();

    Ok(Json(ListFlowTracesResponse {
        traces,
        total: if allow_all_tenants {
            total
        } else {
            scoped_total
        },
        limit,
        offset: params.offset,
    }))
}

/// Get all flow traces for a specific call, ordered by timestamp.
///
/// GET /api/v1/flow-traces/:call_id
pub async fn get_call_flow_traces(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Path(call_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if !can_access_all_tenants(identity.as_ref().map(|ext| &ext.0))
        && !call_is_visible_to_tenant(&state, &tenant, &call_id).await?
    {
        return Err(ApiError::NotFound("Flow traces not found".to_string()));
    }

    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Storage not configured".to_string()))?;

    let trace_store = storage
        .flow_trace_store()
        .ok_or_else(|| ApiError::NotFound("Flow trace store not configured".to_string()))?;

    debug!(call_id = %call_id, "Retrieving flow traces for call");

    let entries = trace_store
        .get_flow_traces_by_call(&call_id)
        .await
        .map_err(|e| {
            warn!(call_id = %call_id, error = %e, "Failed to retrieve flow traces");
            ApiError::Internal("Failed to retrieve flow traces".to_string())
        })?;

    let traces: Vec<FlowTraceResponse> = entries.into_iter().map(FlowTraceResponse::from).collect();

    Ok(Json(traces))
}
