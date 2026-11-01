//! Queue/ACD management API endpoints.

use crate::{auth::TenantContext, error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_queue::{Agent, AgentKey, AgentState, QueueKey};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Helper macro to get queue_manager or return 404.
macro_rules! require_queue_manager {
    ($state:expr) => {
        $state
            .queue_manager
            .as_ref()
            .ok_or_else(|| ApiError::NotFound("Queue system is not configured".to_string()))?
    };
}

/// The tenant's queue by its name: another tenant's queue of the same name is
/// not this one.
fn ensure_queue_access(
    qm: &fcp_queue::QueueManager,
    queue_name: &str,
    tenant_id: &fcp_core::TenantId,
) -> Result<QueueKey, ApiError> {
    let key = QueueKey::new(*tenant_id, queue_name);
    if qm.get_queue_config(&key).is_none() {
        return Err(ApiError::NotFound(format!(
            "Queue '{}' not found",
            queue_name
        )));
    }
    Ok(key)
}

fn ensure_agent_access(
    qm: &fcp_queue::QueueManager,
    queue: &QueueKey,
    agent_id: &str,
) -> Result<AgentKey, ApiError> {
    let key = AgentKey::new(queue.tenant_id, agent_id);
    let agent = qm
        .get_agent(&key)
        .ok_or_else(|| ApiError::NotFound(format!("Agent '{}' not found", agent_id)))?;

    if !agent.is_in(queue) {
        return Err(ApiError::NotFound(format!(
            "Agent '{}' is not assigned to queue '{}'",
            agent_id, queue
        )));
    }

    Ok(key)
}

fn queue_response(
    name: String,
    config: fcp_queue::config::QueueConfig,
    calls_waiting: usize,
) -> QueueResponse {
    QueueResponse {
        name,
        display_name: config.name,
        extension: config.extension,
        strategy: format!("{:?}", config.strategy),
        max_wait_time_secs: config.max_wait_time_secs,
        max_queue_depth: config.max_queue_depth,
        overflow_destination: config.overflow_destination,
        agents_configured: config.agents.len(),
        calls_waiting,
    }
}

fn agent_response(agent: Agent) -> AgentResponse {
    AgentResponse {
        id: agent.id,
        extension: agent.extension,
        state: agent.state.to_string(),
        queues: agent.queues,
        calls_handled: agent.calls_handled,
        priority: agent.priority,
    }
}

// ---- Response types ----

/// Queue info response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct QueueResponse {
    pub name: String,
    pub display_name: String,
    pub extension: String,
    pub strategy: String,
    pub max_wait_time_secs: u32,
    pub max_queue_depth: usize,
    pub overflow_destination: Option<String>,
    pub agents_configured: usize,
    pub calls_waiting: usize,
}

/// Queue stats response (real-time).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct QueueStatsResponse {
    pub queue_name: String,
    pub calls_waiting: usize,
    pub agents_available: usize,
    pub agents_on_call: usize,
    pub agents_total: usize,
    pub avg_wait_time_secs: f64,
    pub longest_wait_secs: u64,
    pub calls_handled_today: u64,
    pub calls_abandoned_today: u64,
    pub service_level_pct: f64,
}

/// Agent info response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AgentResponse {
    pub id: String,
    pub extension: String,
    pub state: String,
    pub queues: Vec<String>,
    pub calls_handled: u64,
    pub priority: u32,
}

// ---- Request types ----

/// Agent login request body.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AgentLoginRequest {
    pub agent_id: String,
    pub extension: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

/// Update agent state request body.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateAgentStateRequest {
    pub state: String,
}

// ---- Handlers ----

/// GET /queues — List all configured queues.
///
/// When the queue runtime is available (call-manager), returns live state.
/// When it's not (admin server), falls back to queue configs from storage.
#[utoipa::path(
    get,
    path = "/api/v1/queues",
    tag = "queues",
    operation_id = "queues_list_queues",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<QueueResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_queues(
    State(state): State<ApiState>,
    tenant: TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    // If queue runtime is available, return live state
    if let Some(ref qm) = state.queue_manager {
        let queue_names = qm.list_queue_names_for_tenant(&tenant.0);
        let mut responses = Vec::with_capacity(queue_names.len());

        for name in &queue_names {
            let key = QueueKey::new(tenant.0, name.clone());
            if let Some(config) = qm.get_queue_config(&key) {
                responses.push(queue_response(
                    name.clone(),
                    config,
                    qm.get_queue_waiting_count(&key),
                ));
            }
        }

        return Ok(Json(crate::page::paginate(
            responses,
            &page,
            crate::page::Order::Ascending,
            |r: &QueueResponse| r.name.clone(),
        )?));
    }

    // Fallback: return queue configs from storage (admin server without runtime)
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".into()))?;

    use fcp_core::storage::QueueConfigStore;
    let configs = storage
        .list_queue_configs(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let responses: Vec<QueueResponse> = configs
        .into_iter()
        .map(|c| QueueResponse {
            name: c.name.clone(),
            display_name: c.name,
            extension: c.extension,
            strategy: c.strategy,
            max_wait_time_secs: c.max_wait_time_secs as u32,
            max_queue_depth: c.max_queue_depth as usize,
            overflow_destination: c.overflow_destination,
            agents_configured: 0,
            calls_waiting: 0,
        })
        .collect();

    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r: &QueueResponse| r.name.clone(),
    )?))
}

/// GET /queues/:name — Get a single queue's info.
#[utoipa::path(
    get,
    path = "/api/v1/queues/{name}",
    tag = "queues",
    operation_id = "queues_get_queue",
    params(("name" = String, Path)),
    responses(
        (status = 200, description = "OK", body = QueueResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_queue(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let key = QueueKey::new(tenant.0, name.clone());
    let config = qm
        .get_queue_config(&key)
        .ok_or_else(|| ApiError::NotFound(format!("Queue '{}' not found", name)))?;

    Ok(Json(queue_response(
        name.clone(),
        config,
        qm.get_queue_waiting_count(&key),
    )))
}

/// GET /queues/:name/stats — Real-time stats for a single queue.
#[utoipa::path(
    get,
    path = "/api/v1/queues/{name}/stats",
    tag = "queues",
    operation_id = "queues_get_queue_stats",
    params(("name" = String, Path)),
    responses(
        (status = 200, description = "OK", body = QueueStatsResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_queue_stats(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let stats = qm
        .queue_stats(&QueueKey::new(tenant.0, name.clone()))
        .ok_or_else(|| ApiError::NotFound(format!("Queue '{}' not found", name)))?;

    Ok(Json(QueueStatsResponse {
        queue_name: stats.queue_name,
        calls_waiting: stats.calls_waiting,
        agents_available: stats.agents_available,
        agents_on_call: stats.agents_on_call,
        agents_total: stats.agents_total,
        avg_wait_time_secs: stats.avg_wait_time_secs,
        longest_wait_secs: stats.longest_wait_secs,
        calls_handled_today: stats.calls_handled_today,
        calls_abandoned_today: stats.calls_abandoned_today,
        service_level_pct: stats.service_level_pct,
    }))
}

/// GET /queues/stats — Real-time stats for all queues.
#[utoipa::path(
    get,
    path = "/api/v1/queues/stats",
    tag = "queues",
    operation_id = "queues_get_all_queue_stats",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<QueueStatsResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_all_queue_stats(
    State(state): State<ApiState>,
    tenant: TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let all_stats = qm.all_queue_stats_for_tenant(&tenant.0);
    let responses: Vec<QueueStatsResponse> = all_stats
        .into_iter()
        .map(|s| QueueStatsResponse {
            queue_name: s.queue_name,
            calls_waiting: s.calls_waiting,
            agents_available: s.agents_available,
            agents_on_call: s.agents_on_call,
            agents_total: s.agents_total,
            avg_wait_time_secs: s.avg_wait_time_secs,
            longest_wait_secs: s.longest_wait_secs,
            calls_handled_today: s.calls_handled_today,
            calls_abandoned_today: s.calls_abandoned_today,
            service_level_pct: s.service_level_pct,
        })
        .collect();

    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r| r.queue_name.clone(),
    )?))
}

/// GET /queues/:name/agents — List agents in a queue.
#[utoipa::path(
    get,
    path = "/api/v1/queues/{name}/agents",
    tag = "queues",
    operation_id = "queues_list_queue_agents",
    params(("name" = String, Path), crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<AgentResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_queue_agents(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(name): Path<String>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let queue = ensure_queue_access(qm, &name, &tenant.0)?;

    let agents = qm.list_agents_for_queue(&queue);
    let responses: Vec<AgentResponse> = agents.into_iter().map(agent_response).collect();

    Ok(Json(crate::page::paginate(
        responses,
        &page,
        crate::page::Order::Ascending,
        |r| r.id.clone(),
    )?))
}

/// POST /queues/:name/agents — Log an agent into a queue.
#[utoipa::path(
    post,
    path = "/api/v1/queues/{name}/agents",
    tag = "queues",
    operation_id = "queues_agent_login",
    params(("name" = String, Path)),
    request_body = AgentLoginRequest,
    responses(
        (status = 201, description = "Logged in", body = AgentResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn agent_login(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(name): Path<String>,
    Json(req): Json<AgentLoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    ensure_queue_access(qm, &name, &tenant.0)?;

    let agent_key = AgentKey::new(tenant.0, req.agent_id.clone());
    qm.agent_login(
        &agent_key,
        &req.extension,
        std::slice::from_ref(&name),
        req.skills.clone(),
    )
    .await
    .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    info!(
        agent_id = %req.agent_id,
        queue = %name,
        extension = %req.extension,
        "Agent logged in via API"
    );

    // Return the agent state
    let agent = qm
        .get_agent(&agent_key)
        .ok_or_else(|| ApiError::Internal("Agent not found after login".to_string()))?;

    Ok((StatusCode::CREATED, Json(agent_response(agent))))
}

/// DELETE /queues/:name/agents/:id — Log an agent out.
#[utoipa::path(
    delete,
    path = "/api/v1/queues/{name}/agents/{id}",
    tag = "queues",
    operation_id = "queues_agent_logout",
    params(("name" = String, Path), ("id" = String, Path)),
    responses(
        (status = 204, description = "Done"),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn agent_logout(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((name, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let queue = ensure_queue_access(qm, &name, &tenant.0)?;
    let agent_key = ensure_agent_access(qm, &queue, &id)?;

    qm.agent_logout(&agent_key)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    info!(agent_id = %id, queue = %name, "Agent logged out via API");

    Ok(StatusCode::NO_CONTENT)
}

/// POST /queues/:name/agents/:id/pause — Pause an agent.
#[utoipa::path(
    post,
    path = "/api/v1/queues/{name}/agents/{id}/pause",
    tag = "queues",
    operation_id = "queues_pause_agent",
    params(("name" = String, Path), ("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = AgentResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn pause_agent(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((name, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let queue = ensure_queue_access(qm, &name, &tenant.0)?;
    let agent_key = ensure_agent_access(qm, &queue, &id)?;

    qm.set_agent_state(&agent_key, AgentState::Paused)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    info!(agent_id = %id, queue = %name, "Agent paused via API");

    let agent = qm
        .get_agent(&agent_key)
        .ok_or_else(|| ApiError::NotFound(format!("Agent '{}' not found", id)))?;

    Ok(Json(agent_response(agent)))
}

/// POST /queues/:name/agents/:id/resume — Resume a paused agent.
#[utoipa::path(
    post,
    path = "/api/v1/queues/{name}/agents/{id}/resume",
    tag = "queues",
    operation_id = "queues_resume_agent",
    params(("name" = String, Path), ("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = AgentResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn resume_agent(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((name, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let queue = ensure_queue_access(qm, &name, &tenant.0)?;
    let agent_key = ensure_agent_access(qm, &queue, &id)?;

    qm.set_agent_state(&agent_key, AgentState::Available)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    info!(agent_id = %id, queue = %name, "Agent resumed via API");

    let agent = qm
        .get_agent(&agent_key)
        .ok_or_else(|| ApiError::NotFound(format!("Agent '{}' not found", id)))?;

    Ok(Json(agent_response(agent)))
}

/// PATCH /queues/:name/agents/:id — Update agent state.
#[utoipa::path(
    patch,
    path = "/api/v1/queues/{name}/agents/{id}",
    tag = "queues",
    operation_id = "queues_update_agent_state",
    params(("name" = String, Path), ("id" = String, Path)),
    request_body = UpdateAgentStateRequest,
    responses(
        (status = 200, description = "OK", body = AgentResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn update_agent_state(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((name, id)): Path<(String, String)>,
    Json(req): Json<UpdateAgentStateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let queue = ensure_queue_access(qm, &name, &tenant.0)?;
    let agent_key = ensure_agent_access(qm, &queue, &id)?;

    let new_state = match req.state.as_str() {
        "available" => AgentState::Available,
        "away" => AgentState::Away,
        other => {
            return Err(ApiError::BadRequest(format!(
                "Invalid agent state '{}'. Must be 'available' or 'away'",
                other
            )));
        }
    };

    qm.set_agent_state(&agent_key, new_state)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    info!(agent_id = %id, queue = %name, new_state = %req.state, "Agent state updated via API");

    let agent = qm
        .get_agent(&agent_key)
        .ok_or_else(|| ApiError::NotFound(format!("Agent '{}' not found", id)))?;

    Ok(Json(agent_response(agent)))
}
