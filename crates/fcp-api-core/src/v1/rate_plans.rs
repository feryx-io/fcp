//! Rate plan management API endpoints (billing MVP).
//!
//! A tenant has at most one default rate plan; answered calls are rated
//! against it at CDR creation time and the cost/currency stamped onto the
//! CDR. All routes are admin-only (registered under the admin router).

use crate::auth::TenantContext;
use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_core::billing::{BillingStore, Decimal, Rate, RatePlan};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use tracing::info;
use uuid::Uuid;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// Rate plan response.
#[derive(Debug, Serialize)]
pub struct RatePlanResponse {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub currency: String,
    pub default_rate_per_minute: Decimal,
    pub connect_fee: Decimal,
    pub billing_increment_secs: u32,
    pub is_default: bool,
    pub enabled: bool,
}

impl From<RatePlan> for RatePlanResponse {
    fn from(plan: RatePlan) -> Self {
        Self {
            id: plan.id.to_string(),
            tenant_id: plan.tenant_id.to_string(),
            name: plan.name,
            currency: plan.currency,
            default_rate_per_minute: plan.default_rate_per_minute,
            connect_fee: plan.connect_fee,
            billing_increment_secs: plan.billing_increment_secs,
            is_default: plan.is_default,
            enabled: plan.enabled,
        }
    }
}

/// Rate response.
#[derive(Debug, Serialize)]
pub struct RateResponse {
    pub id: String,
    pub rate_plan_id: String,
    pub prefix: String,
    pub rate_per_minute: Decimal,
    pub connect_fee: Option<Decimal>,
    pub description: Option<String>,
}

impl From<Rate> for RateResponse {
    fn from(rate: Rate) -> Self {
        Self {
            id: rate.id.to_string(),
            rate_plan_id: rate.rate_plan_id.to_string(),
            prefix: rate.prefix,
            rate_per_minute: rate.rate_per_minute,
            connect_fee: rate.connect_fee,
            description: rate.description,
        }
    }
}

/// Create rate plan request.
#[derive(Debug, Deserialize)]
pub struct CreateRatePlanRequest {
    pub name: String,
    #[serde(default = "default_currency")]
    pub currency: String,
    #[serde(default)]
    pub default_rate_per_minute: Decimal,
    #[serde(default)]
    pub connect_fee: Decimal,
    #[serde(default = "default_increment")]
    pub billing_increment_secs: u32,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_currency() -> String {
    "USD".to_string()
}
fn default_increment() -> u32 {
    60
}
fn default_true() -> bool {
    true
}

/// Update rate plan request (partial).
#[derive(Debug, Deserialize)]
pub struct UpdateRatePlanRequest {
    pub name: Option<String>,
    pub currency: Option<String>,
    pub default_rate_per_minute: Option<Decimal>,
    pub connect_fee: Option<Decimal>,
    pub billing_increment_secs: Option<u32>,
    pub is_default: Option<bool>,
    pub enabled: Option<bool>,
}

/// Create or replace a rate on a plan.
#[derive(Debug, Deserialize)]
pub struct UpsertRateRequest {
    pub prefix: String,
    pub rate_per_minute: Decimal,
    pub connect_fee: Option<Decimal>,
    pub description: Option<String>,
}

fn validate_money(value: Decimal, field: &str) -> Result<(), ApiError> {
    if value < Decimal::ZERO {
        return Err(ApiError::BadRequest(format!(
            "{field} must be a non-negative number"
        )));
    }
    validate_money_precision(value, field)
}

/// Money is stored as NUMERIC(18,6): at most six decimal places and twelve
/// whole digits. Refused rather than rounded by the database.
pub(crate) fn validate_money_precision(value: Decimal, field: &str) -> Result<(), ApiError> {
    if value.normalize().scale() > 6 {
        return Err(ApiError::BadRequest(format!(
            "{field} has more than 6 decimal places"
        )));
    }
    if value.abs() >= Decimal::from(1_000_000_000_000i64) {
        return Err(ApiError::BadRequest(format!("{field} is too large")));
    }
    Ok(())
}

fn validate_prefix(prefix: &str) -> Result<(), ApiError> {
    let digits = prefix.strip_prefix('+').unwrap_or(prefix);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(ApiError::BadRequest(
            "prefix must be digits, optionally starting with '+'".to_string(),
        ));
    }
    Ok(())
}

fn parse_plan_id(id: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id).map_err(|_| ApiError::NotFound(format!("Rate plan not found: {id}")))
}

fn map_storage_error(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<fcp_core::Error>() {
        Some(fcp_core::Error::Storage(fcp_core::StorageError::NotFound(msg))) => {
            ApiError::NotFound(msg.clone())
        }
        Some(fcp_core::Error::Storage(fcp_core::StorageError::Conflict(msg))) => {
            ApiError::Conflict(msg.clone())
        }
        _ => ApiError::Internal(format!("Storage error: {e}")),
    }
}

/// GET /api/v1/rate-plans — List the tenant's rate plans.
pub async fn list_rate_plans(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let plans = storage
        .list_rate_plans(&tenant.0)
        .await
        .map_err(map_storage_error)?;
    Ok(Json(
        plans
            .into_iter()
            .map(RatePlanResponse::from)
            .collect::<Vec<_>>(),
    ))
}

/// POST /api/v1/rate-plans — Create a rate plan.
pub async fn create_rate_plan(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<CreateRatePlanRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".to_string()));
    }
    validate_money(req.default_rate_per_minute, "default_rate_per_minute")?;
    validate_money(req.connect_fee, "connect_fee")?;
    if req.billing_increment_secs == 0 {
        return Err(ApiError::BadRequest(
            "billing_increment_secs must be at least 1".to_string(),
        ));
    }

    let now = SystemTime::now();
    let plan = RatePlan {
        id: Uuid::new_v4(),
        tenant_id: tenant.0,
        name: req.name.trim().to_string(),
        currency: req.currency,
        default_rate_per_minute: req.default_rate_per_minute,
        connect_fee: req.connect_fee,
        billing_increment_secs: req.billing_increment_secs,
        is_default: req.is_default,
        enabled: req.enabled,
        created_at: now,
        updated_at: now,
    };
    storage
        .create_rate_plan(plan.clone())
        .await
        .map_err(map_storage_error)?;
    info!(tenant_id = %tenant.0, plan_id = %plan.id, name = %plan.name, "Rate plan created");
    Ok((StatusCode::CREATED, Json(RatePlanResponse::from(plan))))
}

/// GET /api/v1/rate-plans/:id — Get one rate plan.
pub async fn get_rate_plan(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let plan_id = parse_plan_id(&id)?;
    let plan = storage
        .get_rate_plan(&tenant.0, plan_id)
        .await
        .map_err(map_storage_error)?
        .ok_or_else(|| ApiError::NotFound(format!("Rate plan not found: {id}")))?;
    Ok(Json(RatePlanResponse::from(plan)))
}

/// PATCH /api/v1/rate-plans/:id — Update a rate plan.
pub async fn update_rate_plan(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateRatePlanRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let plan_id = parse_plan_id(&id)?;
    let mut plan = storage
        .get_rate_plan(&tenant.0, plan_id)
        .await
        .map_err(map_storage_error)?
        .ok_or_else(|| ApiError::NotFound(format!("Rate plan not found: {id}")))?;

    if let Some(name) = req.name {
        if name.trim().is_empty() {
            return Err(ApiError::BadRequest("name must not be empty".to_string()));
        }
        plan.name = name.trim().to_string();
    }
    if let Some(currency) = req.currency {
        plan.currency = currency;
    }
    if let Some(rate) = req.default_rate_per_minute {
        validate_money(rate, "default_rate_per_minute")?;
        plan.default_rate_per_minute = rate;
    }
    if let Some(fee) = req.connect_fee {
        validate_money(fee, "connect_fee")?;
        plan.connect_fee = fee;
    }
    if let Some(increment) = req.billing_increment_secs {
        if increment == 0 {
            return Err(ApiError::BadRequest(
                "billing_increment_secs must be at least 1".to_string(),
            ));
        }
        plan.billing_increment_secs = increment;
    }
    if let Some(is_default) = req.is_default {
        plan.is_default = is_default;
    }
    if let Some(enabled) = req.enabled {
        plan.enabled = enabled;
    }
    plan.updated_at = SystemTime::now();

    storage
        .update_rate_plan(plan.clone())
        .await
        .map_err(map_storage_error)?;
    info!(tenant_id = %tenant.0, plan_id = %plan.id, "Rate plan updated");
    Ok(Json(RatePlanResponse::from(plan)))
}

/// DELETE /api/v1/rate-plans/:id — Delete a rate plan and its rates.
pub async fn delete_rate_plan(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let plan_id = parse_plan_id(&id)?;
    storage
        .delete_rate_plan(&tenant.0, plan_id)
        .await
        .map_err(map_storage_error)?;
    info!(tenant_id = %tenant.0, plan_id = %plan_id, "Rate plan deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/rate-plans/:id/rates — List a plan's rates.
pub async fn list_rates(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let plan_id = parse_plan_id(&id)?;
    // Distinguish "plan has no rates" from "plan does not exist".
    storage
        .get_rate_plan(&tenant.0, plan_id)
        .await
        .map_err(map_storage_error)?
        .ok_or_else(|| ApiError::NotFound(format!("Rate plan not found: {id}")))?;
    let rates = storage
        .list_rates(&tenant.0, plan_id)
        .await
        .map_err(map_storage_error)?;
    Ok(Json(
        rates
            .into_iter()
            .map(RateResponse::from)
            .collect::<Vec<_>>(),
    ))
}

/// POST /api/v1/rate-plans/:id/rates — Add or replace a rate (upsert on prefix).
pub async fn upsert_rate(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpsertRateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let plan_id = parse_plan_id(&id)?;
    validate_prefix(&req.prefix)?;
    validate_money(req.rate_per_minute, "rate_per_minute")?;
    if let Some(fee) = req.connect_fee {
        validate_money(fee, "connect_fee")?;
    }

    let rate = Rate {
        id: Uuid::new_v4(),
        rate_plan_id: plan_id,
        prefix: req.prefix,
        rate_per_minute: req.rate_per_minute,
        connect_fee: req.connect_fee,
        description: req.description,
    };
    storage
        .upsert_rate(&tenant.0, rate.clone())
        .await
        .map_err(map_storage_error)?;
    info!(tenant_id = %tenant.0, plan_id = %plan_id, prefix = %rate.prefix, "Rate upserted");
    Ok((StatusCode::CREATED, Json(RateResponse::from(rate))))
}

/// DELETE /api/v1/rate-plans/:id/rates/:rate_id — Delete one rate.
pub async fn delete_rate(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Path((id, rate_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let plan_id = parse_plan_id(&id)?;
    let rate_id = Uuid::parse_str(&rate_id)
        .map_err(|_| ApiError::NotFound(format!("Rate not found: {rate_id}")))?;
    storage
        .delete_rate(&tenant.0, plan_id, rate_id)
        .await
        .map_err(map_storage_error)?;
    info!(tenant_id = %tenant.0, plan_id = %plan_id, rate_id = %rate_id, "Rate deleted");
    Ok(StatusCode::NO_CONTENT)
}
