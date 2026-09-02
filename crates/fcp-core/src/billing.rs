//! Billing data model: per-tenant rate plans, prefix rates, prepaid balances
//! and the balance ledger.
//!
//! This module holds only the *model* shared by the storage contract
//! ([`crate::storage::CdrStore`]) and the admin API. The billing *engine* —
//! call rating, balance admission, settlement at call end and external
//! billing hooks — lives in the dedicated `fcp-billing` crate, which
//! re-exports these types.

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
    pub default_rate_per_minute: f64,
    /// Flat fee added once to every rated (answered) call.
    pub connect_fee: f64,
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
    pub rate_per_minute: f64,
    /// Overrides the plan's connect fee when set.
    pub connect_fee: Option<f64>,
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
    pub balance: f64,
    /// ISO 4217 currency code. Expected to match the tenant's rate plan;
    /// settlement logs a warning and still debits when they differ.
    pub currency: String,
    /// How far below zero the balance may go before calls are refused
    /// (0 = strictly prepaid).
    pub credit_limit: f64,
    /// Whether admission control and mid-call cut-off are active. Off means
    /// the ledger is still maintained but never blocks a call.
    pub enforce: bool,
    /// Emit a `billing.balance_low` event when the balance crosses below
    /// this value.
    pub low_balance_threshold: Option<f64>,
    pub updated_at: SystemTime,
}

impl TenantBalance {
    /// Funds that may still be spent: balance plus the allowed overdraft.
    pub fn available(&self) -> f64 {
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
    pub amount: f64,
    /// Balance after this entry was applied.
    pub balance_after: f64,
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
/// [`crate::storage::CdrStore::apply_balance_transaction`]).
#[derive(Debug, Clone)]
pub struct BalanceTransactionRequest {
    pub tenant_id: crate::TenantId,
    pub kind: BalanceTransactionKind,
    /// Signed amount: positive credits, negative debits.
    pub amount: f64,
    /// Currency used to create the balance row when the tenant has none yet.
    pub currency: String,
    pub call_id: Option<crate::CallId>,
    pub description: Option<String>,
    pub external_ref: Option<String>,
}
