//! Usage and billing API endpoints.
//!
//! Provides aggregated CDR usage statistics and billing summaries for tenants.
//!
//! - GET /usage — aggregate CDR totals for a tenant
//! - GET /usage/by-number — per-number CDR breakdown
//! - GET /billing/summary — combined usage + resource counts

use crate::{auth::TenantContext, error::ApiError, state::ApiState};
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use fcp_core::billing::{BillingStore, Decimal};
use fcp_core::storage::{CdrStore, DeviceStore, PhoneNumberStore};
use fcp_core::CredentialStore;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::debug;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Usage data requires storage to be enabled".to_string())
        })?
    };
}

/// Query parameters for usage endpoints.
#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UsageQuery {
    /// Predefined period: "current_month" or "last_month".
    pub period: Option<String>,
    /// Custom start date (ISO 8601 date, e.g. "2026-01-01").
    pub from: Option<String>,
    /// Custom end date (ISO 8601 date, e.g. "2026-02-01").
    pub to: Option<String>,
}

/// Usage aggregation response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UsageResponse {
    pub period_start: u64,
    pub period_end: u64,
    pub total_calls: u64,
    pub answered_calls: u64,
    pub total_duration_ms: u64,
    pub inbound_calls: u64,
    pub outbound_calls: u64,
    /// Sum of rated call costs in the period, in the tenant's rate-plan
    /// currency (0 when billing is not configured).
    #[schema(value_type = f64)]
    pub total_cost: Decimal,
}

/// Per-number usage response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NumberUsageResponse {
    pub number: String,
    pub total_calls: u64,
    pub answered_calls: u64,
    pub total_duration_ms: u64,
    pub inbound_calls: u64,
    pub outbound_calls: u64,
}

/// Billing summary response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BillingSummaryResponse {
    pub period_start: u64,
    pub period_end: u64,
    pub usage: UsageResponse,
    pub resource_counts: ResourceCounts,
    /// The tenant's prepaid/credit balance, when one is configured.
    pub balance: Option<super::billing::BalanceResponse>,
}

/// Counts of provisioned resources for the tenant.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ResourceCounts {
    pub phone_numbers: usize,
    pub users: usize,
    pub devices: usize,
}

/// Resolve the time range from query parameters.
///
/// If `period` is set, it overrides `from`/`to`. If neither is set,
/// defaults to the current calendar month.
fn resolve_time_range(params: &UsageQuery) -> Result<(SystemTime, SystemTime), ApiError> {
    if let Some(ref period) = params.period {
        let now = SystemTime::now();
        let now_secs = now.duration_since(UNIX_EPOCH).unwrap().as_secs();
        // Approximate month boundaries using chrono-free arithmetic.
        // For billing purposes, use calendar month boundaries.
        match period.as_str() {
            "current_month" => {
                let (from, to) = current_month_range(now_secs);
                Ok((
                    UNIX_EPOCH + Duration::from_secs(from),
                    UNIX_EPOCH + Duration::from_secs(to),
                ))
            }
            "last_month" => {
                let (from, to) = last_month_range(now_secs);
                Ok((
                    UNIX_EPOCH + Duration::from_secs(from),
                    UNIX_EPOCH + Duration::from_secs(to),
                ))
            }
            other => Err(ApiError::BadRequest(format!(
                "Invalid period '{}'. Use 'current_month' or 'last_month'.",
                other
            ))),
        }
    } else if params.from.is_some() || params.to.is_some() {
        let from = params
            .from
            .as_deref()
            .map(parse_date)
            .transpose()?
            .unwrap_or(UNIX_EPOCH);
        let to = params
            .to
            .as_deref()
            .map(parse_date)
            .transpose()?
            .unwrap_or_else(SystemTime::now);
        if from > to {
            return Err(ApiError::BadRequest(
                "'from' must be before 'to'".to_string(),
            ));
        }
        Ok((from, to))
    } else {
        // Default: current month
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let (from, to) = current_month_range(now_secs);
        Ok((
            UNIX_EPOCH + Duration::from_secs(from),
            UNIX_EPOCH + Duration::from_secs(to),
        ))
    }
}

/// Parse an ISO 8601 date string ("YYYY-MM-DD") into a SystemTime at midnight UTC.
fn parse_date(s: &str) -> Result<SystemTime, ApiError> {
    use chrono::NaiveDate;
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| ApiError::BadRequest(format!("Invalid date '{}': {}", s, e)))?;
    let dt = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| ApiError::BadRequest(format!("Invalid date '{}'", s)))?;
    let secs = dt.and_utc().timestamp() as u64;
    Ok(UNIX_EPOCH + Duration::from_secs(secs))
}

/// Compute start-of-current-month and start-of-next-month in epoch seconds.
fn current_month_range(now_secs: u64) -> (u64, u64) {
    use chrono::{DateTime, Datelike, Months, Utc};
    let dt = DateTime::<Utc>::from_timestamp(now_secs as i64, 0).unwrap_or_default();
    let first = dt.date_naive().with_day(1).unwrap_or(dt.date_naive());
    let start = first.and_hms_opt(0, 0, 0).unwrap_or_default();
    let next = first.checked_add_months(Months::new(1)).unwrap_or(first);
    let end = next.and_hms_opt(0, 0, 0).unwrap_or_default();
    (
        start.and_utc().timestamp() as u64,
        end.and_utc().timestamp() as u64,
    )
}

/// Compute start-of-last-month and start-of-current-month in epoch seconds.
fn last_month_range(now_secs: u64) -> (u64, u64) {
    use chrono::{DateTime, Datelike, Months, Utc};
    let dt = DateTime::<Utc>::from_timestamp(now_secs as i64, 0).unwrap_or_default();
    let first_this = dt.date_naive().with_day(1).unwrap_or(dt.date_naive());
    let first_last = first_this
        .checked_sub_months(Months::new(1))
        .unwrap_or(first_this);
    let start = first_last.and_hms_opt(0, 0, 0).unwrap_or_default();
    let end = first_this.and_hms_opt(0, 0, 0).unwrap_or_default();
    (
        start.and_utc().timestamp() as u64,
        end.and_utc().timestamp() as u64,
    )
}

fn to_epoch(st: SystemTime) -> u64 {
    st.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// GET /api/v1/usage — Aggregate CDR totals for a tenant.
#[utoipa::path(
    get,
    path = "/api/v1/usage",
    tag = "usage",
    operation_id = "usage_get_usage",
    params(UsageQuery),
    responses(
        (status = 200, description = "OK", body = UsageResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_usage(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(params): Query<UsageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let (from, to) = resolve_time_range(&params)?;

    debug!(
        tenant_id = %tenant.0,
        from = ?from,
        to = ?to,
        "Querying usage aggregation"
    );

    let agg = storage
        .aggregate_usage(&tenant.0, from, to)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to aggregate usage: {}", e)))?;

    Ok(Json(UsageResponse {
        period_start: to_epoch(from),
        period_end: to_epoch(to),
        total_calls: agg.total_calls,
        answered_calls: agg.answered_calls,
        total_duration_ms: agg.total_duration_ms,
        inbound_calls: agg.inbound_calls,
        outbound_calls: agg.outbound_calls,
        total_cost: agg.total_cost,
    }))
}

/// GET /api/v1/usage/by-number — Per-number CDR breakdown.
#[utoipa::path(
    get,
    path = "/api/v1/usage/by-number",
    tag = "usage",
    operation_id = "usage_get_usage_by_number",
    params(UsageQuery, crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<NumberUsageResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_usage_by_number(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(params): Query<UsageQuery>,
    axum::extract::Query(page): axum::extract::Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let (from, to) = resolve_time_range(&params)?;

    debug!(
        tenant_id = %tenant.0,
        from = ?from,
        to = ?to,
        "Querying per-number usage"
    );

    let numbers = storage
        .aggregate_usage_by_number(&tenant.0, from, to)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to aggregate by number: {}", e)))?;

    let response: Vec<NumberUsageResponse> = numbers
        .into_iter()
        .map(|n| NumberUsageResponse {
            number: n.number,
            total_calls: n.total_calls,
            answered_calls: n.answered_calls,
            total_duration_ms: n.total_duration_ms,
            inbound_calls: n.inbound_calls,
            outbound_calls: n.outbound_calls,
        })
        .collect();

    Ok(Json(crate::page::paginate(
        response,
        &page,
        crate::page::Order::Ascending,
        |r| r.number.clone(),
    )?))
}

/// GET /api/v1/billing/summary — Combined usage + resource counts.
#[utoipa::path(
    get,
    path = "/api/v1/billing/summary",
    tag = "usage",
    operation_id = "usage_billing_summary",
    params(UsageQuery),
    responses(
        (status = 200, description = "OK", body = BillingSummaryResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn billing_summary(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(params): Query<UsageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let (from, to) = resolve_time_range(&params)?;

    debug!(
        tenant_id = %tenant.0,
        from = ?from,
        to = ?to,
        "Querying billing summary"
    );

    // Run usage aggregation and resource counts
    let agg = storage
        .aggregate_usage(&tenant.0, from, to)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to aggregate usage: {}", e)))?;

    let phone_numbers = storage
        .list_phone_numbers(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list phone numbers: {}", e)))?;

    let users = storage
        .list_users()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list users: {}", e)))?;

    let devices = storage
        .list_devices(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list devices: {}", e)))?;

    // Filter users to this tenant
    let user_count = users.iter().filter(|u| u.tenant_id == tenant.0).count();

    let balance = storage
        .get_balance(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to load balance: {}", e)))?
        .map(super::billing::BalanceResponse::from);

    Ok(Json(BillingSummaryResponse {
        period_start: to_epoch(from),
        period_end: to_epoch(to),
        usage: UsageResponse {
            period_start: to_epoch(from),
            period_end: to_epoch(to),
            total_calls: agg.total_calls,
            answered_calls: agg.answered_calls,
            total_duration_ms: agg.total_duration_ms,
            inbound_calls: agg.inbound_calls,
            outbound_calls: agg.outbound_calls,
            total_cost: agg.total_cost,
        },
        resource_counts: ResourceCounts {
            phone_numbers: phone_numbers.len(),
            users: user_count,
            devices: devices.len(),
        },
        balance,
    }))
}
