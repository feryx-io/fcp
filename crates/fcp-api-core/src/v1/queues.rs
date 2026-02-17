//! Queue/ACD management API endpoints.

use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_queue::AgentState;
use serde::{Deserialize, Serialize};
use tracing::info;

/// Helper macro to get queue_manager or return 404.
macro_rules! require_queue_manager {
    ($state:expr) => {
        $state.queue_manager.as_ref().ok_or_else(|| {
            ApiError::NotFound("Queue system is not configured".to_string())
        })?
    };
}

// ---- Response types ----

/// Queue info response.
#[derive(Debug, Serialize)]
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
#[derive(Debug, Serialize)]
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
#[derive(Debug, Serialize)]
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
#[derive(Debug, Deserialize)]
pub struct AgentLoginRequest {
    pub agent_id: String,
    pub extension: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

/// Update agent state request body.
#[derive(Debug, Deserialize)]
pub struct UpdateAgentStateRequest {
    pub state: String,
}

// ---- Handlers ----

/// GET /queues — List all configured queues.
pub async fn list_queues(
    State(state): State<ApiState>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let queue_names = qm.list_queue_names();
    let mut responses = Vec::with_capacity(queue_names.len());

    for name in &queue_names {
        if let Some(config) = qm.get_queue_config(name) {
            responses.push(QueueResponse {
                name: name.clone(),
                display_name: config.name,
                extension: config.extension,
                strategy: format!("{:?}", config.strategy),
                max_wait_time_secs: config.max_wait_time_secs,
                max_queue_depth: config.max_queue_depth,
                overflow_destination: config.overflow_destination,
                agents_configured: config.agents.len(),
                calls_waiting: qm.get_queue_waiting_count(name),
            });
        }
    }

    Ok(Json(responses))
}

/// GET /queues/:name — Get a single queue's info.
pub async fn get_queue(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let config = qm
        .get_queue_config(&name)
        .ok_or_else(|| ApiError::NotFound(format!("Queue '{}' not found", name)))?;

    Ok(Json(QueueResponse {
        name: name.clone(),
        display_name: config.name,
        extension: config.extension,
        strategy: format!("{:?}", config.strategy),
        max_wait_time_secs: config.max_wait_time_secs,
        max_queue_depth: config.max_queue_depth,
        overflow_destination: config.overflow_destination,
        agents_configured: config.agents.len(),
        calls_waiting: qm.get_queue_waiting_count(&name),
    }))
}

/// GET /queues/:name/stats — Real-time stats for a single queue.
pub async fn get_queue_stats(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let stats = qm
        .queue_stats(&name)
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
pub async fn get_all_queue_stats(
    State(state): State<ApiState>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    let all_stats = qm.all_queue_stats();
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

    Ok(Json(responses))
}

/// GET /queues/:name/agents — List agents in a queue.
pub async fn list_queue_agents(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    // Verify queue exists
    if qm.get_queue_config(&name).is_none() {
        return Err(ApiError::NotFound(format!("Queue '{}' not found", name)));
    }

    let agents = qm.list_agents_for_queue(&name);
    let responses: Vec<AgentResponse> = agents
        .into_iter()
        .map(|a| AgentResponse {
            id: a.id,
            extension: a.extension,
            state: a.state.to_string(),
            queues: a.queues,
            calls_handled: a.calls_handled,
            priority: a.priority,
        })
        .collect();

    Ok(Json(responses))
}

/// POST /queues/:name/agents — Log an agent into a queue.
pub async fn agent_login(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    Json(req): Json<AgentLoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    // Verify queue exists
    if qm.get_queue_config(&name).is_none() {
        return Err(ApiError::NotFound(format!("Queue '{}' not found", name)));
    }

    let tenant_id = state.default_tenant_id.clone();

    qm.agent_login(&req.agent_id, &req.extension, &[name.clone()], tenant_id, req.skills.clone())
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
        .get_agent(&req.agent_id)
        .ok_or_else(|| ApiError::Internal("Agent not found after login".to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(AgentResponse {
            id: agent.id,
            extension: agent.extension,
            state: agent.state.to_string(),
            queues: agent.queues,
            calls_handled: agent.calls_handled,
            priority: agent.priority,
        }),
    ))
}

/// DELETE /queues/:name/agents/:id — Log an agent out.
pub async fn agent_logout(
    State(state): State<ApiState>,
    Path((name, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    // Verify queue exists
    if qm.get_queue_config(&name).is_none() {
        return Err(ApiError::NotFound(format!("Queue '{}' not found", name)));
    }

    qm.agent_logout(&id)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    info!(agent_id = %id, queue = %name, "Agent logged out via API");

    Ok(StatusCode::NO_CONTENT)
}

/// PATCH /queues/:name/agents/:id — Update agent state.
pub async fn update_agent_state(
    State(state): State<ApiState>,
    Path((name, id)): Path<(String, String)>,
    Json(req): Json<UpdateAgentStateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let qm = require_queue_manager!(state);

    // Verify queue exists
    if qm.get_queue_config(&name).is_none() {
        return Err(ApiError::NotFound(format!("Queue '{}' not found", name)));
    }

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

    qm.set_agent_state(&id, new_state)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    info!(agent_id = %id, queue = %name, new_state = %req.state, "Agent state updated via API");

    let agent = qm
        .get_agent(&id)
        .ok_or_else(|| ApiError::NotFound(format!("Agent '{}' not found", id)))?;

    Ok(Json(AgentResponse {
        id: agent.id,
        extension: agent.extension,
        state: agent.state.to_string(),
        queues: agent.queues,
        calls_handled: agent.calls_handled,
        priority: agent.priority,
    }))
}
