//! The billing engine: async facade over storage + hooks used by the call
//! path and the admin API.

use crate::balance::{check_admission, AdmissionDecision};
use crate::hooks::{BillingHook, EventBusHook, NoopHook};
use crate::rating::{dialed_digits_from_uri, rate_call, RatedCall};
use async_trait::async_trait;
use fcp_core::billing::{
    BalanceTransaction, BalanceTransactionKind, BalanceTransactionRequest, Rate, RatePlan,
    TenantBalance,
};
use fcp_core::storage::CdrStore;
use fcp_core::{CallId, TenantId};
use fcp_events::{BillingEvent, EventPublisher};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// The slice of storage the engine needs. Blanket-implemented for every
/// [`CdrStore`] (so `Arc<DynStorage>` works directly); tests supply an
/// in-memory implementation.
#[async_trait]
pub trait BillingStore: Send + Sync {
    async fn default_rate_plan(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<(RatePlan, Vec<Rate>)>>;

    async fn balance(&self, tenant_id: &TenantId) -> anyhow::Result<Option<TenantBalance>>;

    async fn apply_transaction(
        &self,
        request: BalanceTransactionRequest,
    ) -> anyhow::Result<BalanceTransaction>;
}

#[async_trait]
impl<T: CdrStore + ?Sized> BillingStore for T {
    async fn default_rate_plan(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<(RatePlan, Vec<Rate>)>> {
        self.get_default_rate_plan_with_rates(tenant_id).await
    }

    async fn balance(&self, tenant_id: &TenantId) -> anyhow::Result<Option<TenantBalance>> {
        self.get_balance(tenant_id).await
    }

    async fn apply_transaction(
        &self,
        request: BalanceTransactionRequest,
    ) -> anyhow::Result<BalanceTransaction> {
        self.apply_balance_transaction(request).await
    }
}

/// Outcome of settling one call at CDR creation.
#[derive(Debug, Clone, PartialEq)]
pub struct Settlement {
    pub rated: RatedCall,
    /// The rate plan the cost came from.
    pub rate_plan_id: Option<String>,
    /// Dialed digits the call was rated for.
    pub destination: String,
    /// Balance after the charge, when the tenant tracks a balance and the
    /// debit was applied.
    pub balance_after: Option<f64>,
}

/// Billing engine. Cheap to clone (all state is shared).
#[derive(Clone)]
pub struct BillingEngine {
    store: Arc<dyn BillingStore>,
    hook: Arc<dyn BillingHook>,
}

impl BillingEngine {
    /// Engine with no external hook (events are dropped). Accepts any
    /// store, typically `Arc<DynStorage>`.
    pub fn new<S: BillingStore + 'static>(store: Arc<S>) -> Self {
        Self::from_dyn(store)
    }

    /// Engine over an already type-erased store.
    pub fn from_dyn(store: Arc<dyn BillingStore>) -> Self {
        Self {
            store,
            hook: Arc::new(NoopHook),
        }
    }

    /// Replace the hook.
    pub fn with_hook(mut self, hook: Arc<dyn BillingHook>) -> Self {
        self.hook = hook;
        self
    }

    /// Deliver billing events to the platform event bus.
    pub fn with_events(self, events: Arc<dyn EventPublisher>) -> Self {
        self.with_hook(Arc::new(EventBusHook::new(events)))
    }

    /// Balance admission for a call from `tenant_id` to `to_uri`.
    ///
    /// Allows when the destination is not billable (no dialed digits), the
    /// tenant has no rate plan or no balance row, or the balance is not
    /// enforced. Storage failures fail *open* with an error log: a billing
    /// outage must not take signaling down, and settlement will still be
    /// attempted at call end.
    pub async fn admit(&self, tenant_id: &TenantId, to_uri: &str) -> AdmissionDecision {
        let allow = AdmissionDecision::Allow { max_duration: None };
        let Some(dialed) = dialed_digits_from_uri(to_uri) else {
            return allow;
        };

        let balance = match self.store.balance(tenant_id).await {
            Ok(Some(b)) if b.enforce => b,
            Ok(_) => return allow,
            Err(e) => {
                tracing::error!(tenant_id = %tenant_id, error = %e, "Balance lookup failed; admitting call");
                return allow;
            }
        };

        let (plan, rates) = match self.store.default_rate_plan(tenant_id).await {
            Ok(Some(p)) => p,
            Ok(None) => return allow,
            Err(e) => {
                tracing::error!(tenant_id = %tenant_id, error = %e, "Rate plan lookup failed; admitting call");
                return allow;
            }
        };

        let decision = check_admission(&plan, &rates, &balance, &dialed);
        match &decision {
            AdmissionDecision::Allow { max_duration } => {
                debug!(
                    tenant_id = %tenant_id,
                    destination = %dialed,
                    max_duration_secs = ?max_duration.map(|d| d.as_secs()),
                    "Balance admission allowed"
                );
            }
            AdmissionDecision::Deny {
                available,
                required,
                currency,
            } => {
                info!(
                    tenant_id = %tenant_id,
                    destination = %dialed,
                    available,
                    required,
                    currency = %currency,
                    "Balance admission denied"
                );
                self.hook
                    .on_event(BillingEvent::CallRejected {
                        tenant_id: *tenant_id,
                        destination: dialed.clone(),
                        available: *available,
                        required: *required,
                        currency: currency.clone(),
                        timestamp: SystemTime::now(),
                    })
                    .await;
            }
        }
        decision
    }

    /// Rate a finished call and debit the tenant's balance.
    ///
    /// Returns `None` when billing is not configured for the tenant or the
    /// destination is not billable; otherwise the rating for the CDR plus the
    /// balance after the charge. Failures are logged, never fatal — the CDR
    /// is written regardless.
    pub async fn settle(
        &self,
        tenant_id: &TenantId,
        call_id: CallId,
        to_uri: &str,
        duration_ms: u64,
    ) -> Option<Settlement> {
        let dialed = dialed_digits_from_uri(to_uri)?;

        let (plan, rates) = match self.store.default_rate_plan(tenant_id).await {
            Ok(Some(p)) => p,
            Ok(None) => return None,
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Rate plan lookup failed - CDR will carry no cost");
                return None;
            }
        };

        let rated = rate_call(&plan, &rates, &dialed, duration_ms);
        debug!(
            call_id = %call_id,
            cost = rated.cost,
            currency = %rated.currency,
            matched_prefix = ?rated.matched_prefix,
            billable_secs = rated.billable_secs,
            "Call rated"
        );

        let balance_after = if rated.cost > 0.0 {
            self.charge(tenant_id, call_id, &rated).await
        } else {
            None
        };

        self.hook
            .on_event(BillingEvent::CallRated {
                tenant_id: *tenant_id,
                call_id,
                cost: rated.cost,
                currency: rated.currency.clone(),
                matched_prefix: rated.matched_prefix.clone(),
                billable_secs: rated.billable_secs,
                destination: dialed.clone(),
                balance_after,
                timestamp: SystemTime::now(),
            })
            .await;

        Some(Settlement {
            rated,
            rate_plan_id: Some(plan.id.to_string()),
            destination: dialed,
            balance_after,
        })
    }

    /// Debit a rated call against the tenant's balance (when one exists).
    async fn charge(
        &self,
        tenant_id: &TenantId,
        call_id: CallId,
        rated: &RatedCall,
    ) -> Option<f64> {
        let before = match self.store.balance(tenant_id).await {
            Ok(Some(b)) => b,
            Ok(None) => return None,
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Balance lookup failed - call not debited");
                return None;
            }
        };
        if before.currency != rated.currency {
            warn!(
                tenant_id = %tenant_id,
                balance_currency = %before.currency,
                plan_currency = %rated.currency,
                "Balance and rate plan currencies differ; debiting nominal amount"
            );
        }

        let request = BalanceTransactionRequest {
            tenant_id: *tenant_id,
            kind: BalanceTransactionKind::CallCharge,
            amount: -rated.cost,
            currency: before.currency.clone(),
            call_id: Some(call_id),
            description: Some(format!(
                "call charge: {}s{}",
                rated.billable_secs,
                rated
                    .matched_prefix
                    .as_deref()
                    .map(|p| format!(" @ prefix {p}"))
                    .unwrap_or_default()
            )),
            external_ref: None,
        };

        match self.store.apply_transaction(request).await {
            Ok(txn) => {
                self.emit_balance_events(&before, &txn).await;
                Some(txn.balance_after)
            }
            Err(e) if is_conflict(&e) => {
                info!(call_id = %call_id, "Call already settled; skipping duplicate charge");
                Some(before.balance)
            }
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "Balance debit failed - call not debited");
                None
            }
        }
    }

    /// Apply a credit or adjustment (admin API / external billing system)
    /// and emit the resulting events.
    pub async fn apply_transaction(
        &self,
        request: BalanceTransactionRequest,
    ) -> anyhow::Result<BalanceTransaction> {
        let before = self.store.balance(&request.tenant_id).await?;
        let txn = self.store.apply_transaction(request).await?;
        match before {
            Some(before) => self.emit_balance_events(&before, &txn).await,
            None => {
                // First entry created the balance row: only the change event.
                self.hook.on_event(balance_changed(&txn)).await;
            }
        }
        Ok(txn)
    }

    /// Report that an established call was hung up because its affordable
    /// duration ran out.
    pub async fn report_cut_off(
        &self,
        tenant_id: &TenantId,
        call_id: CallId,
        max_duration: Duration,
    ) {
        warn!(
            tenant_id = %tenant_id,
            call_id = %call_id,
            max_duration_secs = max_duration.as_secs(),
            "Call cut off: affordable duration exhausted"
        );
        self.hook
            .on_event(BillingEvent::CallCutOff {
                tenant_id: *tenant_id,
                call_id,
                max_duration_secs: max_duration.as_secs(),
                timestamp: SystemTime::now(),
            })
            .await;
    }

    /// `BalanceChanged` for every entry, plus `BalanceLow` / `BalanceDepleted`
    /// when the entry crosses the respective line downwards.
    async fn emit_balance_events(&self, before: &TenantBalance, txn: &BalanceTransaction) {
        self.hook.on_event(balance_changed(txn)).await;

        let after = txn.balance_after;
        if txn.amount >= 0.0 {
            return;
        }
        if let Some(threshold) = before.low_balance_threshold {
            if before.balance >= threshold && after < threshold {
                self.hook
                    .on_event(BillingEvent::BalanceLow {
                        tenant_id: txn.tenant_id,
                        balance: after,
                        threshold,
                        currency: txn.currency.clone(),
                        timestamp: txn.created_at,
                    })
                    .await;
            }
        }
        if before.enforce && before.available() > 0.0 && after + before.credit_limit <= 0.0 {
            self.hook
                .on_event(BillingEvent::BalanceDepleted {
                    tenant_id: txn.tenant_id,
                    balance: after,
                    credit_limit: before.credit_limit,
                    currency: txn.currency.clone(),
                    timestamp: txn.created_at,
                })
                .await;
        }
    }
}

fn balance_changed(txn: &BalanceTransaction) -> BillingEvent {
    BillingEvent::BalanceChanged {
        tenant_id: txn.tenant_id,
        transaction_id: txn.id.to_string(),
        kind: txn.kind.as_str().to_string(),
        amount: txn.amount,
        balance_after: txn.balance_after,
        currency: txn.currency.clone(),
        call_id: txn.call_id,
        description: txn.description.clone(),
        external_ref: txn.external_ref.clone(),
        timestamp: txn.created_at,
    }
}

fn is_conflict(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<fcp_core::Error>(),
        Some(fcp_core::Error::Storage(fcp_core::StorageError::Conflict(
            _
        )))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::RecordingHook;
    use crate::rating::test_support::{plan, rate};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    /// In-memory store: one plan per tenant, one balance row per tenant, and
    /// a ledger with the same one-charge-per-call rule as Postgres.
    #[derive(Default)]
    struct MemStore {
        plans: Mutex<HashMap<TenantId, (RatePlan, Vec<Rate>)>>,
        balances: Mutex<HashMap<TenantId, TenantBalance>>,
        ledger: Mutex<Vec<BalanceTransaction>>,
    }

    #[async_trait]
    impl BillingStore for MemStore {
        async fn default_rate_plan(
            &self,
            tenant_id: &TenantId,
        ) -> anyhow::Result<Option<(RatePlan, Vec<Rate>)>> {
            Ok(self.plans.lock().unwrap().get(tenant_id).cloned())
        }

        async fn balance(&self, tenant_id: &TenantId) -> anyhow::Result<Option<TenantBalance>> {
            Ok(self.balances.lock().unwrap().get(tenant_id).cloned())
        }

        async fn apply_transaction(
            &self,
            request: BalanceTransactionRequest,
        ) -> anyhow::Result<BalanceTransaction> {
            let mut ledger = self.ledger.lock().unwrap();
            if request.kind == BalanceTransactionKind::CallCharge
                && ledger.iter().any(|t| {
                    t.kind == BalanceTransactionKind::CallCharge && t.call_id == request.call_id
                })
            {
                return Err(anyhow::Error::new(fcp_core::Error::Storage(
                    fcp_core::StorageError::Conflict("already settled".into()),
                )));
            }
            let mut balances = self.balances.lock().unwrap();
            let row = balances
                .entry(request.tenant_id)
                .or_insert_with(|| TenantBalance {
                    tenant_id: request.tenant_id,
                    balance: 0.0,
                    currency: request.currency.clone(),
                    credit_limit: 0.0,
                    enforce: false,
                    low_balance_threshold: None,
                    updated_at: SystemTime::now(),
                });
            row.balance += request.amount;
            let txn = BalanceTransaction {
                id: Uuid::new_v4(),
                tenant_id: request.tenant_id,
                kind: request.kind,
                amount: request.amount,
                balance_after: row.balance,
                currency: row.currency.clone(),
                call_id: request.call_id,
                description: request.description,
                external_ref: request.external_ref,
                created_at: SystemTime::now(),
            };
            ledger.push(txn.clone());
            Ok(txn)
        }
    }

    fn setup(
        balance: f64,
        enforce: bool,
        threshold: Option<f64>,
    ) -> (TenantId, Arc<MemStore>, Arc<RecordingHook>, BillingEngine) {
        let mut p = plan();
        let tenant = p.tenant_id;
        let rates = vec![rate(p.id, "44", 0.03)];
        p.default_rate_per_minute = 0.02;
        let store = Arc::new(MemStore::default());
        store.plans.lock().unwrap().insert(tenant, (p, rates));
        store.balances.lock().unwrap().insert(
            tenant,
            TenantBalance {
                tenant_id: tenant,
                balance,
                currency: "USD".into(),
                credit_limit: 0.0,
                enforce,
                low_balance_threshold: threshold,
                updated_at: SystemTime::now(),
            },
        );
        let hook = Arc::new(RecordingHook::default());
        let engine = BillingEngine::new(store.clone()).with_hook(hook.clone());
        (tenant, store, hook, engine)
    }

    fn names(hook: &RecordingHook) -> Vec<&'static str> {
        hook.events()
            .into_iter()
            .map(|e| fcp_events::PlatformEvent::Billing(e).event_type_name())
            .collect()
    }

    #[tokio::test]
    async fn admit_allows_non_billable_destination() {
        let (tenant, _, hook, engine) = setup(0.0, true, None);
        let d = engine.admit(&tenant, "sip:alice@example.com").await;
        assert!(d.is_allowed());
        assert!(hook.events().is_empty());
    }

    #[tokio::test]
    async fn admit_denies_and_emits_rejection() {
        let (tenant, _, hook, engine) = setup(0.0, true, None);
        let d = engine.admit(&tenant, "sip:+15550100@example.com").await;
        assert!(!d.is_allowed());
        assert_eq!(names(&hook), vec!["billing.call_rejected"]);
    }

    #[tokio::test]
    async fn admit_caps_duration_when_enforced() {
        let (tenant, _, _, engine) = setup(0.06, true, None);
        match engine.admit(&tenant, "sip:+15550100@example.com").await {
            AdmissionDecision::Allow { max_duration } => {
                assert_eq!(max_duration, Some(Duration::from_secs(180)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn admit_ignores_unenforced_balance() {
        let (tenant, _, _, engine) = setup(-5.0, false, None);
        assert_eq!(
            engine.admit(&tenant, "sip:+15550100@example.com").await,
            AdmissionDecision::Allow { max_duration: None }
        );
    }

    #[tokio::test]
    async fn admit_allows_without_plan() {
        let (tenant, store, _, engine) = setup(0.0, true, None);
        store.plans.lock().unwrap().clear();
        assert!(engine.admit(&tenant, "sip:+15550100@x").await.is_allowed());
    }

    #[tokio::test]
    async fn settle_rates_debits_and_emits() {
        let (tenant, store, hook, engine) = setup(1.0, true, Some(0.95));
        let call_id = CallId::new();
        let s = engine
            .settle(&tenant, call_id, "sip:+442079460000@x", 90_000)
            .await
            .expect("settled");
        // 90s → 120s billed at 0.03/min = 0.06
        assert!((s.rated.cost - 0.06).abs() < 1e-9);
        assert_eq!(s.balance_after, Some(0.94));
        assert_eq!(s.destination, "442079460000");
        let bal = store.balances.lock().unwrap()[&tenant].balance;
        assert!((bal - 0.94).abs() < 1e-9);
        assert_eq!(
            names(&hook),
            vec![
                "billing.balance_changed",
                "billing.balance_low",
                "billing.call_rated"
            ]
        );
    }

    #[tokio::test]
    async fn settle_emits_depleted_when_enforced_balance_hits_zero() {
        let (tenant, _, hook, engine) = setup(0.02, true, None);
        engine
            .settle(&tenant, CallId::new(), "sip:+15550100@x", 60_000)
            .await
            .expect("settled");
        assert!(names(&hook).contains(&"billing.balance_depleted"));
    }

    #[tokio::test]
    async fn settle_does_not_emit_depleted_when_not_enforced() {
        let (tenant, _, hook, engine) = setup(0.02, false, None);
        engine
            .settle(&tenant, CallId::new(), "sip:+15550100@x", 60_000)
            .await
            .expect("settled");
        assert!(!names(&hook).contains(&"billing.balance_depleted"));
    }

    #[tokio::test]
    async fn settle_is_idempotent_per_call() {
        let (tenant, store, _, engine) = setup(1.0, true, None);
        let call_id = CallId::new();
        let first = engine
            .settle(&tenant, call_id, "sip:+15550100@x", 60_000)
            .await
            .unwrap();
        let second = engine
            .settle(&tenant, call_id, "sip:+15550100@x", 60_000)
            .await
            .unwrap();
        assert_eq!(first.balance_after, Some(0.98));
        assert_eq!(second.balance_after, Some(0.98));
        assert_eq!(store.ledger.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn settle_skips_debit_for_unanswered_call() {
        let (tenant, store, hook, engine) = setup(1.0, true, None);
        let s = engine
            .settle(&tenant, CallId::new(), "sip:+15550100@x", 0)
            .await
            .unwrap();
        assert_eq!(s.rated.cost, 0.0);
        assert_eq!(s.balance_after, None);
        assert!(store.ledger.lock().unwrap().is_empty());
        assert_eq!(names(&hook), vec!["billing.call_rated"]);
    }

    #[tokio::test]
    async fn settle_without_balance_row_only_rates() {
        let (tenant, store, hook, engine) = setup(1.0, true, None);
        store.balances.lock().unwrap().clear();
        let s = engine
            .settle(&tenant, CallId::new(), "sip:+15550100@x", 60_000)
            .await
            .unwrap();
        assert!((s.rated.cost - 0.02).abs() < 1e-9);
        assert_eq!(s.balance_after, None);
        assert_eq!(names(&hook), vec!["billing.call_rated"]);
    }

    #[tokio::test]
    async fn top_up_emits_change_only() {
        let (tenant, _, hook, engine) = setup(0.0, true, Some(1.0));
        let txn = engine
            .apply_transaction(BalanceTransactionRequest {
                tenant_id: tenant,
                kind: BalanceTransactionKind::TopUp,
                amount: 10.0,
                currency: "USD".into(),
                call_id: None,
                description: Some("card payment".into()),
                external_ref: Some("inv-1".into()),
            })
            .await
            .unwrap();
        assert_eq!(txn.balance_after, 10.0);
        assert_eq!(names(&hook), vec!["billing.balance_changed"]);
    }

    #[tokio::test]
    async fn negative_adjustment_crossing_threshold_emits_low() {
        let (tenant, _, hook, engine) = setup(2.0, false, Some(1.0));
        engine
            .apply_transaction(BalanceTransactionRequest {
                tenant_id: tenant,
                kind: BalanceTransactionKind::Adjustment,
                amount: -1.5,
                currency: "USD".into(),
                call_id: None,
                description: None,
                external_ref: None,
            })
            .await
            .unwrap();
        assert_eq!(
            names(&hook),
            vec!["billing.balance_changed", "billing.balance_low"]
        );
    }
}
