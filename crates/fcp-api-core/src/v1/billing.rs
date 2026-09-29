//! Balance management API (billing follow-up).
//!
//! A tenant may carry a prepaid/credit balance. Settlement debits it at
//! call end; these endpoints expose the balance, its enforcement settings,
//! and the ledger, and let operators or an external billing system credit
//! or adjust it. All routes are admin-only (registered under the admin
//! router).

use crate::auth::TenantContext;
use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use fcp_billing::{
    BalanceTransaction, BalanceTransactionKind, BalanceTransactionRequest, BillingEngine,
    TenantBalance,
};
use fcp_core::billing::{BillingStore, Decimal};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::info;

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("Storage is required but not configured".to_string())
        })?
    };
}

/// Balance response.
#[derive(Debug, Serialize)]
pub struct BalanceResponse {
    pub tenant_id: String,
    pub balance: Decimal,
    pub currency: String,
    pub credit_limit: Decimal,
    /// Funds that may still be spent (balance + credit limit).
    pub available: Decimal,
    pub enforce: bool,
    pub low_balance_threshold: Option<Decimal>,
    #[serde(with = "crate::time::epoch_secs")]
    pub updated_at: u64,
}

impl From<TenantBalance> for BalanceResponse {
    fn from(b: TenantBalance) -> Self {
        Self {
            tenant_id: b.tenant_id.to_string(),
            balance: b.balance,
            currency: b.currency.clone(),
            credit_limit: b.credit_limit,
            available: b.available(),
            enforce: b.enforce,
            low_balance_threshold: b.low_balance_threshold,
            updated_at: to_epoch(b.updated_at),
        }
    }
}

/// Ledger entry response.
#[derive(Debug, Serialize)]
pub struct BalanceTransactionResponse {
    pub id: String,
    pub tenant_id: String,
    pub kind: BalanceTransactionKind,
    pub amount: Decimal,
    pub balance_after: Decimal,
    pub currency: String,
    pub call_id: Option<String>,
    pub description: Option<String>,
    pub external_ref: Option<String>,
    #[serde(with = "crate::time::epoch_secs")]
    pub created_at: u64,
}

impl From<BalanceTransaction> for BalanceTransactionResponse {
    fn from(t: BalanceTransaction) -> Self {
        Self {
            id: t.id.to_string(),
            tenant_id: t.tenant_id.to_string(),
            kind: t.kind,
            amount: t.amount,
            balance_after: t.balance_after,
            currency: t.currency,
            call_id: t.call_id.map(|c| c.to_string()),
            description: t.description,
            external_ref: t.external_ref,
            created_at: to_epoch(t.created_at),
        }
    }
}

/// Balance settings update. Every field is optional; the balance amount
/// itself is only changed through transactions.
#[derive(Debug, Deserialize)]
pub struct UpdateBalanceRequest {
    pub currency: Option<String>,
    pub credit_limit: Option<Decimal>,
    pub enforce: Option<bool>,
    /// `null` clears the threshold; omit to leave it unchanged.
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub low_balance_threshold: Option<Option<Decimal>>,
}

/// Distinguish an omitted field (`None`) from an explicit `null`
/// (`Some(None)`).
fn deserialize_double_option<'de, D>(deserializer: D) -> Result<Option<Option<Decimal>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Decimal>::deserialize(deserializer).map(Some)
}

/// Credit or adjustment request.
#[derive(Debug, Deserialize)]
pub struct CreateTransactionRequest {
    /// `top_up` (positive amount) or `adjustment` (either sign).
    /// `call_charge` entries are written only by settlement.
    pub kind: BalanceTransactionKind,
    pub amount: Decimal,
    /// Currency used when this entry creates the tenant's balance row.
    /// Defaults to USD.
    pub currency: Option<String>,
    pub description: Option<String>,
    /// Reference to the external billing system's record (invoice id,
    /// payment id, ...).
    pub external_ref: Option<String>,
}

/// Ledger listing query.
fn to_epoch(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

fn validate_currency(currency: &str) -> Result<String, ApiError> {
    let code = currency.trim().to_ascii_uppercase();
    if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApiError::BadRequest(
            "currency must be a 3-letter ISO 4217 code".to_string(),
        ));
    }
    Ok(code)
}

fn validate_finite(value: Decimal, field: &str) -> Result<(), ApiError> {
    crate::v1::rate_plans::validate_money_precision(value, field)
}

/// The engine used by the API: settlement is not involved here, but the
/// engine emits the `billing.balance_*` events for credits/adjustments.
fn engine(state: &ApiState) -> Result<BillingEngine, ApiError> {
    let storage = require_storage!(state);
    let events: Arc<dyn fcp_events::EventPublisher> = state.event_bus.clone();
    Ok(BillingEngine::new(Arc::clone(storage)).with_events(events))
}

/// GET /api/v1/billing/balance — The tenant's balance and settings.
pub async fn get_balance(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let balance = storage
        .get_balance(&tenant.0)
        .await
        .map_err(map_storage_error)?
        .ok_or_else(|| ApiError::NotFound("No balance configured for tenant".to_string()))?;
    Ok(Json(BalanceResponse::from(balance)))
}

/// PUT /api/v1/billing/balance — Create or update balance settings.
///
/// Creates the balance row (at zero) when the tenant has none. The amount
/// is never changed here; use transactions.
pub async fn update_balance(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<UpdateBalanceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let existing = storage
        .get_balance(&tenant.0)
        .await
        .map_err(map_storage_error)?;
    let created = existing.is_none();
    let mut balance = existing.unwrap_or_else(|| TenantBalance {
        tenant_id: tenant.0,
        balance: Decimal::ZERO,
        currency: "USD".to_string(),
        credit_limit: Decimal::ZERO,
        enforce: false,
        low_balance_threshold: None,
        updated_at: SystemTime::now(),
    });

    if let Some(currency) = req.currency {
        balance.currency = validate_currency(&currency)?;
    }
    if let Some(limit) = req.credit_limit {
        validate_finite(limit, "credit_limit")?;
        if limit < Decimal::ZERO {
            return Err(ApiError::BadRequest(
                "credit_limit must not be negative".to_string(),
            ));
        }
        balance.credit_limit = limit;
    }
    if let Some(enforce) = req.enforce {
        balance.enforce = enforce;
    }
    if let Some(threshold) = req.low_balance_threshold {
        if let Some(t) = threshold {
            validate_finite(t, "low_balance_threshold")?;
        }
        balance.low_balance_threshold = threshold;
    }
    balance.updated_at = SystemTime::now();

    storage
        .upsert_balance(balance.clone())
        .await
        .map_err(map_storage_error)?;
    info!(
        tenant_id = %tenant.0,
        enforce = balance.enforce,
        credit_limit = %balance.credit_limit,
        "Balance settings updated"
    );

    // Re-read so the response reflects the stored amount for existing rows.
    let stored = storage
        .get_balance(&tenant.0)
        .await
        .map_err(map_storage_error)?
        .unwrap_or(balance);
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(BalanceResponse::from(stored))))
}

/// POST /api/v1/billing/balance/transactions — Credit or adjust the balance.
pub async fn create_transaction(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<CreateTransactionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_finite(req.amount, "amount")?;
    if req.amount.is_zero() {
        return Err(ApiError::BadRequest("amount must not be zero".to_string()));
    }
    match req.kind {
        BalanceTransactionKind::TopUp if req.amount < Decimal::ZERO => {
            return Err(ApiError::BadRequest(
                "top_up amount must be positive; use an adjustment to debit".to_string(),
            ));
        }
        BalanceTransactionKind::CallCharge => {
            return Err(ApiError::BadRequest(
                "call_charge entries are created by settlement only".to_string(),
            ));
        }
        _ => {}
    }
    let currency = validate_currency(req.currency.as_deref().unwrap_or("USD"))?;

    let engine = engine(&state)?;
    let txn = engine
        .apply_transaction(BalanceTransactionRequest {
            tenant_id: tenant.0,
            kind: req.kind,
            amount: req.amount,
            currency,
            call_id: None,
            description: req.description,
            external_ref: req.external_ref,
        })
        .await
        .map_err(map_storage_error)?;
    info!(
        tenant_id = %tenant.0,
        kind = txn.kind.as_str(),
        amount = %txn.amount,
        balance_after = %txn.balance_after,
        "Balance transaction applied"
    );
    Ok((
        StatusCode::CREATED,
        Json(BalanceTransactionResponse::from(txn)),
    ))
}

/// GET /api/v1/billing/balance/transactions — The ledger, newest first.
pub async fn list_transactions(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let limit = page.limit()?;
    let offset = page.offset()?;
    // One more than the page, to know whether there is another.
    let entries = storage
        .list_balance_transactions(&tenant.0, limit + 1, offset)
        .await
        .map_err(map_storage_error)?;
    let more = entries.len() > limit;
    let data: Vec<BalanceTransactionResponse> = entries
        .into_iter()
        .take(limit)
        .map(BalanceTransactionResponse::from)
        .collect();
    Ok(Json(crate::page::offset_page(data, offset, more, None)))
}
