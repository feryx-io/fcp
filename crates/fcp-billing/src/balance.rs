//! Balance admission math: whether a tenant can afford a destination and
//! for how long. Pure functions; the async lookups live in [`crate::engine`].

use crate::rating::{round_money, select_rate};
use fcp_core::billing::{Rate, RatePlan, TenantBalance};
use std::time::Duration;

/// Tolerance for floating-point comparisons on money (well below the
/// ledger's 4-decimal precision).
const EPSILON: f64 = 1e-9;

/// Result of a balance admission check.
#[derive(Debug, Clone, PartialEq)]
pub enum AdmissionDecision {
    /// The call may proceed. `max_duration` caps the answered duration when
    /// the tenant enforces its balance and the destination has a non-zero
    /// per-minute rate; `None` means no cut-off applies.
    Allow { max_duration: Option<Duration> },
    /// The tenant cannot cover the first billing increment.
    Deny {
        /// Funds available (balance + credit limit).
        available: f64,
        /// Cost of the first billing increment (connect fee included).
        required: f64,
        currency: String,
    },
}

impl AdmissionDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, AdmissionDecision::Allow { .. })
    }
}

/// Decide whether `balance` can pay for a call to `dialed_digits` under
/// `plan`, and how long such a call may last.
///
/// Rules:
/// - A tenant that does not enforce its balance is always admitted.
/// - The call must be able to cover its first billing increment
///   (connect fee + one increment at the selected rate) from
///   `balance + credit_limit`. Zero-rated destinations (e.g. internal
///   extensions under a plan whose default rate is 0) therefore always pass.
/// - When the per-minute rate is positive, the affordable duration is the
///   number of whole billing increments the available funds cover after the
///   connect fee.
pub fn check_admission(
    plan: &RatePlan,
    rates: &[Rate],
    balance: &TenantBalance,
    dialed_digits: &str,
) -> AdmissionDecision {
    if !balance.enforce {
        return AdmissionDecision::Allow { max_duration: None };
    }

    let selected = select_rate(plan, rates, dialed_digits);
    let increment_secs = u64::from(plan.billing_increment_secs.max(1));
    let per_increment = selected.rate_per_minute * increment_secs as f64 / 60.0;
    let required = round_money(selected.connect_fee + per_increment);
    let available = balance.available();

    if required > available + EPSILON {
        return AdmissionDecision::Deny {
            available: round_money(available),
            required,
            currency: plan.currency.clone(),
        };
    }

    if selected.rate_per_minute <= 0.0 {
        return AdmissionDecision::Allow { max_duration: None };
    }

    let spendable = available - selected.connect_fee;
    let affordable_secs = (spendable / selected.rate_per_minute * 60.0 + EPSILON).floor();
    let whole_increments = (affordable_secs / increment_secs as f64).floor().max(1.0);
    let max_secs = (whole_increments as u64).saturating_mul(increment_secs);
    AdmissionDecision::Allow {
        max_duration: Some(Duration::from_secs(max_secs)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rating::test_support::{plan, rate};
    use std::time::SystemTime;

    fn balance(amount: f64, enforce: bool) -> TenantBalance {
        TenantBalance {
            tenant_id: fcp_core::TenantId::new(),
            balance: amount,
            currency: "USD".into(),
            credit_limit: 0.0,
            enforce,
            low_balance_threshold: None,
            updated_at: SystemTime::now(),
        }
    }

    #[test]
    fn unenforced_balance_always_allows() {
        let p = plan();
        let d = check_admission(&p, &[], &balance(-100.0, false), "15550100");
        assert_eq!(d, AdmissionDecision::Allow { max_duration: None });
    }

    #[test]
    fn denies_when_first_increment_unaffordable() {
        let p = plan(); // 0.02/min, 60s increments → 0.02 per increment
        let d = check_admission(&p, &[], &balance(0.01, true), "15550100");
        match d {
            AdmissionDecision::Deny {
                available,
                required,
                currency,
            } => {
                assert!((available - 0.01).abs() < 1e-9);
                assert!((required - 0.02).abs() < 1e-9);
                assert_eq!(currency, "USD");
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn credit_limit_extends_available_funds() {
        let p = plan();
        let mut b = balance(-0.01, true);
        b.credit_limit = 0.05; // available 0.04 → two increments
        let d = check_admission(&p, &[], &b, "15550100");
        assert_eq!(
            d,
            AdmissionDecision::Allow {
                max_duration: Some(Duration::from_secs(120))
            }
        );
    }

    #[test]
    fn max_duration_is_whole_increments_after_connect_fee() {
        let mut p = plan();
        p.connect_fee = 0.01;
        let rates = vec![rate(p.id, "44", 0.03)];
        // available 0.10 - fee 0.01 = 0.09 → 3 min at 0.03/min → 180s
        let d = check_admission(&p, &rates, &balance(0.10, true), "442079460000");
        assert_eq!(
            d,
            AdmissionDecision::Allow {
                max_duration: Some(Duration::from_secs(180))
            }
        );
        // 0.095 → 2.83 min → floor to 2 whole increments = 120s
        let d = check_admission(&p, &rates, &balance(0.095, true), "442079460000");
        assert_eq!(
            d,
            AdmissionDecision::Allow {
                max_duration: Some(Duration::from_secs(120))
            }
        );
    }

    #[test]
    fn zero_rated_destination_allows_without_cap() {
        let mut p = plan();
        p.default_rate_per_minute = 0.0;
        let d = check_admission(&p, &[], &balance(0.0, true), "2001");
        assert_eq!(d, AdmissionDecision::Allow { max_duration: None });
    }

    #[test]
    fn exactly_one_increment_is_allowed() {
        let p = plan();
        let d = check_admission(&p, &[], &balance(0.02, true), "15550100");
        assert_eq!(
            d,
            AdmissionDecision::Allow {
                max_duration: Some(Duration::from_secs(60))
            }
        );
    }

    #[test]
    fn per_second_billing_caps_to_the_second() {
        let mut p = plan();
        p.billing_increment_secs = 1; // 0.02/min → 0.000333/s
        let d = check_admission(&p, &[], &balance(0.01, true), "15550100");
        // 0.01 / 0.02 * 60 = 30s
        assert_eq!(
            d,
            AdmissionDecision::Allow {
                max_duration: Some(Duration::from_secs(30))
            }
        );
    }
}
