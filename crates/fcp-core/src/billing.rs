//! Billing data model: per-tenant rate plans, prefix rates, prepaid balances
//! and the balance ledger.
//!
//! This module holds the *model* and its storage contract
//! ([`BillingStore`]), shared by the storage backends and the admin API. The billing *engine* —
//! call rating, balance admission, settlement at call end and external
//! billing hooks — lives in the dedicated `fcp-billing` crate, which
//! re-exports these types.

pub use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

/// A tenant's rate plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatePlan {
    pub id: Uuid,
    pub tenant_id: crate::TenantId,
    pub name: String,
    /// ISO 4217 currency code, e.g. "USD".
    pub currency: String,
    /// Applied when no rate prefix matches the dialed number.
    pub default_rate_per_minute: Decimal,
    /// Flat fee added once to every rated (answered) call.
    pub connect_fee: Decimal,
    /// Billing granularity in seconds: duration is rounded up to the next
    /// multiple before pricing (60 = per-minute billing, 1 = per-second).
    pub billing_increment_secs: u32,
    /// The plan used to rate this tenant's calls. At most one default per
    /// tenant (enforced by the storage layer).
    pub is_default: bool,
    pub enabled: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

/// A per-prefix rate belonging to a [`RatePlan`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rate {
    pub id: Uuid,
    pub rate_plan_id: Uuid,
    /// Dialed-number prefix (digits, optionally starting with `+`), matched
    /// longest-prefix-first, e.g. "1", "44", "4420".
    pub prefix: String,
    pub rate_per_minute: Decimal,
    /// Overrides the plan's connect fee when set.
    pub connect_fee: Option<Decimal>,
    pub description: Option<String>,
}

/// A tenant's prepaid/credit balance and its enforcement settings.
///
/// The balance is debited by the settlement step at call end (the rated
/// cost of every answered call) and credited by top-ups/adjustments through
/// the admin API. When `enforce` is set, calls whose first billing
/// increment cannot be covered by `balance + credit_limit` are rejected at
/// admission with 402 Payment Required, and established calls are cut off
/// when the affordable duration runs out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantBalance {
    pub tenant_id: crate::TenantId,
    /// Current balance in `currency`. May go negative up to `credit_limit`.
    pub balance: Decimal,
    /// ISO 4217 currency code. Expected to match the tenant's rate plan;
    /// settlement logs a warning and still debits when they differ.
    pub currency: String,
    /// How far below zero the balance may go before calls are refused
    /// (0 = strictly prepaid).
    pub credit_limit: Decimal,
    /// Whether admission control and mid-call cut-off are active. Off means
    /// the ledger is still maintained but never blocks a call.
    pub enforce: bool,
    /// Emit a `billing.balance_low` event when the balance crosses below
    /// this value.
    pub low_balance_threshold: Option<Decimal>,
    pub updated_at: SystemTime,
}

impl TenantBalance {
    /// Funds that may still be spent: balance plus the allowed overdraft.
    pub fn available(&self) -> Decimal {
        self.balance + self.credit_limit
    }
}

/// Why a balance changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceTransactionKind {
    /// Credit added by an operator or an external billing system.
    TopUp,
    /// Debit for a rated call (settlement at call end).
    CallCharge,
    /// Manual correction in either direction (refund, write-off, ...).
    Adjustment,
}

impl BalanceTransactionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TopUp => "top_up",
            Self::CallCharge => "call_charge",
            Self::Adjustment => "adjustment",
        }
    }
}

impl std::str::FromStr for BalanceTransactionKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "top_up" => Ok(Self::TopUp),
            "call_charge" => Ok(Self::CallCharge),
            "adjustment" => Ok(Self::Adjustment),
            other => Err(format!("unknown balance transaction kind: {other}")),
        }
    }
}

/// One ledger entry: a signed change to a tenant's balance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceTransaction {
    pub id: Uuid,
    pub tenant_id: crate::TenantId,
    pub kind: BalanceTransactionKind,
    /// Signed amount: positive credits, negative debits.
    pub amount: Decimal,
    /// Balance after this entry was applied.
    pub balance_after: Decimal,
    pub currency: String,
    /// The call this charge settles, for `CallCharge` entries.
    pub call_id: Option<crate::CallId>,
    /// Free-form note (top-up reference, adjustment reason, ...).
    pub description: Option<String>,
    /// Opaque reference to the external billing system's record, if any.
    pub external_ref: Option<String>,
    pub created_at: SystemTime,
}

/// Request to append one ledger entry (see
/// [`BillingStore::apply_balance_transaction`]).
#[derive(Debug, Clone)]
pub struct BalanceTransactionRequest {
    pub tenant_id: crate::TenantId,
    pub kind: BalanceTransactionKind,
    /// Signed amount: positive credits, negative debits.
    pub amount: Decimal,
    /// Currency used to create the balance row when the tenant has none yet.
    pub currency: String,
    pub call_id: Option<crate::CallId>,
    pub description: Option<String>,
    pub external_ref: Option<String>,
}

/// Storage for rate plans, rates, balances and the ledger. Split from
/// [`crate::storage::CdrStore`] in phase 2 of the CDR pipeline
/// (`docs/CDR_PIPELINE.md` §4.4): the records and the money are different
/// concerns with different callers. Every method is defaulted so backends
/// without billing keep compiling; the defaults fail loudly rather than
/// pretending to persist anything, except the default-plan lookup, which
/// answers "no plan".
#[async_trait::async_trait]
pub trait BillingStore: Send + Sync {
    /// Create a rate plan. When `plan.is_default` is set, any previous
    /// default plan for the tenant loses its default flag.
    async fn create_rate_plan(&self, plan: RatePlan) -> anyhow::Result<()> {
        let _ = plan;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Get a rate plan by id, scoped to a tenant.
    async fn get_rate_plan(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> anyhow::Result<Option<RatePlan>> {
        let _ = (tenant_id, plan_id);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// List a tenant's rate plans.
    async fn list_rate_plans(&self, tenant_id: &crate::TenantId) -> anyhow::Result<Vec<RatePlan>> {
        let _ = tenant_id;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Replace a rate plan's mutable fields (name, currency, rates config,
    /// default/enabled flags). When `plan.is_default` is set, any previous
    /// default plan for the tenant loses its default flag.
    async fn update_rate_plan(&self, plan: RatePlan) -> anyhow::Result<()> {
        let _ = plan;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Delete a rate plan and its rates.
    async fn delete_rate_plan(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        let _ = (tenant_id, plan_id);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Add or replace a rate on a plan (upsert on `(rate_plan_id, prefix)`).
    async fn upsert_rate(&self, tenant_id: &crate::TenantId, rate: Rate) -> anyhow::Result<()> {
        let _ = (tenant_id, rate);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// List a plan's rates.
    async fn list_rates(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> anyhow::Result<Vec<Rate>> {
        let _ = (tenant_id, plan_id);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Delete one rate from a plan.
    async fn delete_rate(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
        rate_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        let _ = (tenant_id, plan_id, rate_id);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// The tenant's enabled default rate plan with its rates, if any — the
    /// single lookup the CDR rating path performs at call end.
    async fn get_default_rate_plan_with_rates(
        &self,
        tenant_id: &crate::TenantId,
    ) -> anyhow::Result<Option<(RatePlan, Vec<Rate>)>> {
        // Default: billing not configured — rating is skipped silently.
        let _ = tenant_id;
        Ok(None)
    }

    // ── Balances (billing follow-up) ─────────────────────────────────────

    /// The tenant's balance row, if one has been created. `None` means no
    /// balance is tracked for the tenant: settlement skips the debit and
    /// admission never blocks.
    async fn get_balance(
        &self,
        tenant_id: &crate::TenantId,
    ) -> anyhow::Result<Option<TenantBalance>> {
        let _ = tenant_id;
        Ok(None)
    }

    /// Create or replace the balance *settings* (currency, credit limit,
    /// enforcement, low-balance threshold). The stored `balance` amount is
    /// preserved for an existing row and taken from `balance.balance` only
    /// when the row is created.
    async fn upsert_balance(&self, balance: TenantBalance) -> anyhow::Result<()> {
        let _ = balance;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "balances are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Atomically apply a signed amount to the tenant's balance and append
    /// the ledger entry, returning that entry (with `balance_after`). When
    /// the tenant has no balance row yet, one is created with default
    /// settings (enforcement off) in the request's currency.
    async fn apply_balance_transaction(
        &self,
        request: BalanceTransactionRequest,
    ) -> anyhow::Result<BalanceTransaction> {
        let _ = request;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "balances are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// The tenant's ledger, newest first.
    async fn list_balance_transactions(
        &self,
        tenant_id: &crate::TenantId,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<Vec<BalanceTransaction>> {
        let _ = (tenant_id, limit, offset);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "balances are not supported by this storage backend".to_string(),
            ),
        )))
    }
}
