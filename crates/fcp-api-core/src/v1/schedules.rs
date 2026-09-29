//! A tenant's schedules (`docs/ENTERPRISE_CALLING.md` §3.5): business
//! hours and holidays in a timezone, named by numbers, users, queues and
//! flows. `GET` answers with the state now, so a page can say "open".

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
use fcp_core::{Holiday, Schedule, ScheduleId, ScheduleStore, TenantStore, WeeklyWindow};
use fcp_events::{PlatformEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

#[derive(Debug, Serialize)]
pub struct ScheduleResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub timezone: String,
    pub weekly: Vec<WeeklyWindow>,
    pub holidays: Vec<Holiday>,
    /// `open`, `closed` or `holiday`, now.
    pub state: &'static str,
    #[serde(with = "crate::time::epoch_secs")]
    pub updated_at: u64,
}

impl From<Schedule> for ScheduleResponse {
    fn from(s: Schedule) -> Self {
        Self {
            state: s.state_now().as_str(),
            id: s.id.to_string(),
            tenant_id: s.tenant_id.to_string(),
            updated_at: s
                .updated_at
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            name: s.name,
            timezone: s.timezone,
            weekly: s.weekly,
            holidays: s.holidays,
        }
    }
}

/// What a schedule is written as.
#[derive(Debug, Deserialize)]
pub struct ScheduleRequest {
    pub name: String,
    /// IANA; unset, the tenant's timezone (§3.7), else UTC — and on an
    /// update, the schedule's own.
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub weekly: Vec<WeeklyWindow>,
    #[serde(default)]
    pub holidays: Vec<Holiday>,
}

fn parse_id(id: &str) -> Result<ScheduleId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::BadRequest(format!("Invalid schedule ID: {id}")))
}

async fn announce(state: &ApiState, schedule: &Schedule) {
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "schedule".to_string(),
            entity_id: schedule.id.to_string(),
            tenant_id: schedule.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// GET /api/v1/schedules
pub async fn list_schedules(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleRead,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);
    let schedules = storage
        .list_schedules(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(crate::page::paginate(
        schedules
            .into_iter()
            .map(ScheduleResponse::from)
            .collect::<Vec<_>>(),
        &page,
        crate::page::Order::Ascending,
        |r| format!("{}/{}", r.name, r.id),
    )?))
}

async fn owned(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    id: &str,
) -> Result<Schedule, ApiError> {
    let storage = require_storage!(state);
    storage
        .get_schedule(&parse_id(id)?)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .filter(|s| s.tenant_id == *tenant)
        .ok_or_else(|| ApiError::NotFound(format!("Schedule {id} not found")))
}

/// GET /api/v1/schedules/:id
pub async fn get_schedule(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleRead,
        state.deployment_mode,
    )?;
    Ok(Json(ScheduleResponse::from(
        owned(&state, &tenant.0, &id).await?,
    )))
}

/// POST /api/v1/schedules
pub async fn create_schedule(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Json(req): Json<ScheduleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleManage,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);
    let timezone = match req.timezone {
        Some(tz) => tz,
        None => storage
            .get_tenant(&tenant.0)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .and_then(|t| t.calling.timezone)
            .unwrap_or_else(|| "UTC".to_string()),
    };
    let mut schedule = Schedule::new(tenant.0, req.name, timezone);
    schedule.weekly = req.weekly;
    schedule.holidays = req.holidays;
    schedule.validate().map_err(ApiError::BadRequest)?;
    storage
        .put_schedule(schedule.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &schedule).await;
    Ok((StatusCode::CREATED, Json(ScheduleResponse::from(schedule))))
}

/// PUT /api/v1/schedules/:id — the whole schedule, replaced.
pub async fn update_schedule(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
    Json(req): Json<ScheduleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleManage,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);
    let mut schedule = owned(&state, &tenant.0, &id).await?;
    schedule.name = req.name;
    if let Some(tz) = req.timezone {
        schedule.timezone = tz;
    }
    schedule.weekly = req.weekly;
    schedule.holidays = req.holidays;
    schedule.updated_at = SystemTime::now();
    schedule.validate().map_err(ApiError::BadRequest)?;
    storage
        .put_schedule(schedule.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &schedule).await;
    Ok(Json(ScheduleResponse::from(schedule)))
}

/// DELETE /api/v1/schedules/:id — what named it stays open from then on.
pub async fn delete_schedule(
    State(state): State<ApiState>,
    Extension(identity): Extension<AuthIdentity>,
    tenant: crate::auth::TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::auth::check_permission(
        &identity,
        Permission::RoutingRuleManage,
        state.deployment_mode,
    )?;
    let storage = require_storage!(state);
    let schedule = owned(&state, &tenant.0, &id).await?;
    storage
        .delete_schedule(&schedule.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    announce(&state, &schedule).await;
    Ok(StatusCode::NO_CONTENT)
}

/// A schedule id given to another resource: it must be one of the
/// tenant's. Empty clears it.
pub async fn tenant_schedule(
    state: &ApiState,
    tenant: &fcp_core::TenantId,
    given: &str,
) -> Result<Option<ScheduleId>, ApiError> {
    if given.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(owned(state, tenant, given.trim()).await?.id))
}
