//! Routing rule management API endpoints.
//!
//! A tenant's rules decide its calls on every call manager
//! (`docs/TENANT_TRUNKS.md` §12, phase 6b): managing them takes the same
//! permissions as managing its trunks, a rule is checked here the way a
//! node compiles it, and every change is announced so nodes re-read the
//! rules at once.

use crate::auth::AuthIdentity;
use crate::error::ApiError;
use crate::state::ApiState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use fcp_core::permissions::Permission;
use fcp_core::storage::{RoutingRuleStore, TrunkStore};
use fcp_core::{RoutingAction, RoutingRule, RoutingRuleId, TenantId, TrunkTarget};
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use tracing::info;

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

#[derive(Debug, Serialize)]
pub struct RoutingRuleResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub priority: i32,
    pub enabled: bool,
    pub destination_pattern: Option<String>,
    pub source_pattern: Option<String>,
    pub source_ip_ranges: Vec<String>,
    pub time_window_days: Vec<String>,
    pub time_window_start: Option<String>,
    pub time_window_end: Option<String>,
    pub time_window_timezone: String,
    pub action: String,
    pub target_trunk_id: Option<String>,
    pub target_trunk_group: Option<String>,
    pub reason: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl From<RoutingRule> for RoutingRuleResponse {
    fn from(r: RoutingRule) -> Self {
        Self {
            id: r.id.to_string(),
            tenant_id: r.tenant_id.to_string(),
            name: r.name,
            description: r.description,
            priority: r.priority,
            enabled: r.enabled,
            destination_pattern: r.destination_pattern,
            source_pattern: r.source_pattern,
            source_ip_ranges: r.source_ip_ranges,
            time_window_days: r.time_window_days,
            time_window_start: r.time_window_start,
            time_window_end: r.time_window_end,
            time_window_timezone: r.time_window_timezone,
            action: r.action.to_string(),
            target_trunk_id: r.target_trunk_id,
            target_trunk_group: r.target_trunk_group,
            reason: r.reason,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateRoutingRuleRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub destination_pattern: Option<String>,
    pub source_pattern: Option<String>,
    #[serde(default)]
    pub source_ip_ranges: Vec<String>,
    #[serde(default)]
    pub time_window_days: Vec<String>,
    pub time_window_start: Option<String>,
    pub time_window_end: Option<String>,
    #[serde(default = "default_tz")]
    pub time_window_timezone: String,
    #[serde(default = "default_action")]
    pub action: String,
    pub target_trunk_id: Option<String>,
    pub target_trunk_group: Option<String>,
    pub reason: Option<String>,
}

fn default_priority() -> i32 {
    100
}
fn default_true() -> bool {
    true
}
fn default_tz() -> String {
    "UTC".to_string()
}
fn default_action() -> String {
    "allow".to_string()
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoutingRuleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
    pub destination_pattern: Option<String>,
    pub source_pattern: Option<String>,
    pub source_ip_ranges: Option<Vec<String>>,
    pub time_window_days: Option<Vec<String>>,
    pub time_window_start: Option<String>,
    pub time_window_end: Option<String>,
    pub time_window_timezone: Option<String>,
    pub action: Option<String>,
    pub target_trunk_id: Option<String>,
    pub target_trunk_group: Option<String>,
    pub reason: Option<String>,
}

pub async fn list_routing_rules(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let rules = storage
        .list_routing_rules(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list routing rules: {}", e)))?;
    let response: Vec<RoutingRuleResponse> =
        rules.into_iter().map(RoutingRuleResponse::from).collect();
    Ok(Json(response))
}

pub async fn get_routing_rule(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkRead, state.deployment_mode)?;
    let storage = require_storage!(state);
    let rule = storage
        .get_routing_rule(&RoutingRuleId::from_string(id))
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get routing rule: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Routing rule not found".to_string()))?;
    ensure_rule_visible_to_tenant(&rule, &tenant)?;
    Ok(Json(RoutingRuleResponse::from(rule)))
}

pub async fn create_routing_rule(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<CreateRoutingRuleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);

    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("Rule name is required".to_string()));
    }

    // Validate regex patterns
    if let Some(ref pat) = req.destination_pattern {
        regex::Regex::new(pat)
            .map_err(|e| ApiError::BadRequest(format!("Invalid destination pattern: {}", e)))?;
    }
    if let Some(ref pat) = req.source_pattern {
        regex::Regex::new(pat)
            .map_err(|e| ApiError::BadRequest(format!("Invalid source pattern: {}", e)))?;
    }

    let action = req
        .action
        .parse::<RoutingAction>()
        .map_err(ApiError::BadRequest)?;

    let mut rule = RoutingRule::new(req.name, tenant.0);
    rule.description = req.description;
    rule.priority = req.priority;
    rule.enabled = req.enabled;
    rule.destination_pattern = req.destination_pattern;
    rule.source_pattern = req.source_pattern;
    rule.source_ip_ranges = req.source_ip_ranges;
    rule.time_window_days = req.time_window_days;
    rule.time_window_start = req.time_window_start;
    rule.time_window_end = req.time_window_end;
    rule.time_window_timezone = req.time_window_timezone;
    rule.action = action;
    rule.target_trunk_id = req.target_trunk_id;
    rule.target_trunk_group = req.target_trunk_group;
    rule.reason = req.reason;

    check_rule(storage.as_ref(), &rule).await?;

    info!(rule_name = %rule.name, rule_id = %rule.id, "Creating routing rule");

    storage
        .put_routing_rule(rule.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create routing rule: {}", e)))?;
    announce_rule_change(&state, &rule.id.to_string(), rule.tenant_id).await;

    Ok((StatusCode::CREATED, Json(RoutingRuleResponse::from(rule))))
}

pub async fn update_routing_rule(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateRoutingRuleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let rule_id = RoutingRuleId::from_string(id);

    let mut rule = storage
        .get_routing_rule(&rule_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get routing rule: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Routing rule not found".to_string()))?;
    ensure_rule_visible_to_tenant(&rule, &tenant)?;

    if let Some(name) = req.name {
        rule.name = name;
    }
    if let Some(desc) = req.description {
        rule.description = Some(desc);
    }
    if let Some(pri) = req.priority {
        rule.priority = pri;
    }
    if let Some(en) = req.enabled {
        rule.enabled = en;
    }
    if let Some(pat) = req.destination_pattern {
        if !pat.is_empty() {
            regex::Regex::new(&pat)
                .map_err(|e| ApiError::BadRequest(format!("Invalid destination pattern: {}", e)))?;
        }
        rule.destination_pattern = Some(pat).filter(|s| !s.is_empty());
    }
    if let Some(pat) = req.source_pattern {
        if !pat.is_empty() {
            regex::Regex::new(&pat)
                .map_err(|e| ApiError::BadRequest(format!("Invalid source pattern: {}", e)))?;
        }
        rule.source_pattern = Some(pat).filter(|s| !s.is_empty());
    }
    if let Some(ips) = req.source_ip_ranges {
        rule.source_ip_ranges = ips;
    }
    if let Some(days) = req.time_window_days {
        rule.time_window_days = days;
    }
    if let Some(start) = req.time_window_start {
        rule.time_window_start = Some(start).filter(|s| !s.is_empty());
    }
    if let Some(end) = req.time_window_end {
        rule.time_window_end = Some(end).filter(|s| !s.is_empty());
    }
    if let Some(tz) = req.time_window_timezone {
        rule.time_window_timezone = tz;
    }
    if let Some(action) = req.action {
        rule.action = action
            .parse::<RoutingAction>()
            .map_err(ApiError::BadRequest)?;
    }
    if let Some(tid) = req.target_trunk_id {
        rule.target_trunk_id = Some(tid).filter(|s| !s.is_empty());
    }
    if let Some(tg) = req.target_trunk_group {
        rule.target_trunk_group = Some(tg).filter(|s| !s.is_empty());
    }
    if let Some(reason) = req.reason {
        rule.reason = Some(reason).filter(|s| !s.is_empty());
    }

    rule.updated_at = SystemTime::now();
    check_rule(storage.as_ref(), &rule).await?;

    info!(rule_name = %rule.name, rule_id = %rule.id, "Updating routing rule");

    storage
        .put_routing_rule(rule.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to update routing rule: {}", e)))?;
    announce_rule_change(&state, &rule.id.to_string(), rule.tenant_id).await;

    Ok(Json(RoutingRuleResponse::from(rule)))
}

pub async fn delete_routing_rule(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(&identity, Permission::TrunkUpdate, state.deployment_mode)?;
    let storage = require_storage!(state);
    let rule_id = RoutingRuleId::from_string(id);

    let rule = storage
        .get_routing_rule(&rule_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get routing rule: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("Routing rule not found".to_string()))?;
    ensure_rule_visible_to_tenant(&rule, &tenant)?;

    info!(rule_name = %rule.name, rule_id = %rule.id, "Deleting routing rule");

    storage
        .delete_routing_rule(&rule_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete routing rule: {}", e)))?;
    announce_rule_change(&state, &rule.id.to_string(), rule.tenant_id).await;

    Ok(StatusCode::NO_CONTENT)
}

fn ensure_rule_visible_to_tenant(
    rule: &RoutingRule,
    tenant: &crate::auth::TenantContext,
) -> Result<(), ApiError> {
    if rule.tenant_id == tenant.0 {
        Ok(())
    } else {
        Err(ApiError::NotFound("Routing rule not found".to_string()))
    }
}

/// Check a rule the way every node will compile it, and that a trunk it
/// names is this tenant's own: a rule may not send a tenant's calls out on
/// another tenant's carrier, nor name the platform's trunks one by one
/// (a trunk group is how a rule reaches those).
async fn check_rule<S>(storage: &S, rule: &RoutingRule) -> Result<(), ApiError>
where
    S: TrunkStore + ?Sized,
{
    fcp_policy::tenant_rules::validate(rule).map_err(ApiError::BadRequest)?;
    if let Some(TrunkTarget::Trunk(id)) = rule.trunk_target() {
        let owned = storage
            .get_trunk(&id)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to read trunk: {}", e)))?
            .is_some_and(|trunk| trunk.tenant_id == Some(rule.tenant_id));
        if !owned {
            return Err(ApiError::BadRequest(format!(
                "target_trunk_id '{id}' is not one of this tenant's trunks"
            )));
        }
    }
    Ok(())
}

/// Tell every node that a rule changed, so the ones deciding calls re-read
/// the rules now rather than at the next backstop tick.
async fn announce_rule_change(state: &ApiState, entity_id: &str, tenant_id: TenantId) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "routing_rule".to_string(),
            entity_id: entity_id.to_string(),
            tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}
