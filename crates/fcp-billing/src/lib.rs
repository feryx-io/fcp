//! Billing engine for the Ferrous Communications Platform.
//!
//! The data model (rate plans, prefix rates, tenant balances, the balance
//! ledger) lives in `fcp_core::billing` because the storage contract refers
//! to it; this crate re-exports it and adds the behaviour:
//!
//! - [`rating`]: pure call rating (longest-prefix match, billing increments,
//!   connect fees) and dialed-digit extraction.
//! - [`balance`]: pure admission math — can the tenant afford this
//!   destination, and for how long.
//! - [`hooks`]: the [`BillingHook`] trait through which every billing
//!   outcome (rated call, balance change, low/depleted balance, rejected or
//!   cut-off call) is delivered to external billing systems. The default
//!   hook publishes [`fcp_events::BillingEvent`]s on the platform event bus,
//!   which the webhook dispatcher forwards to subscribed endpoints.
//! - [`engine`]: [`BillingEngine`], the async facade used by the call path
//!   (admit before routing, settle at CDR creation) and the admin API
//!   (credits/adjustments).

pub mod balance;
pub mod engine;
pub mod hooks;
pub mod rating;

pub use balance::{check_admission, AdmissionDecision};
pub use engine::{BillingEngine, BillingStore, Settlement};
pub use fcp_core::billing::{
    BalanceTransaction, BalanceTransactionKind, BalanceTransactionRequest, Rate, RatePlan,
    TenantBalance,
};
pub use hooks::{BillingHook, CompositeHook, EventBusHook, NoopHook};
pub use rating::{dialed_digits_from_uri, rate_call, RatedCall, SelectedRate};
