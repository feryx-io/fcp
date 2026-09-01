//! Billing: per-tenant rate plans and call rating.
//!
//! MVP scope (Phase 4): a tenant has at most one *default* rate plan; a plan
//! carries a currency, a default per-minute rate, a connect fee, a billing
//! increment, and a set of longest-prefix-match rates. Calls are rated once,
//! at CDR creation time, and the cost/currency are stamped onto the CDR.
//! Balance enforcement and external billing-system hooks are follow-up work.

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

/// Outcome of rating one call.
#[derive(Debug, Clone, PartialEq)]
pub struct RatedCall {
    /// Total cost in the plan's currency, rounded to 4 decimal places.
    pub cost: f64,
    pub currency: String,
    /// The rate prefix that matched, or `None` when the default rate applied.
    pub matched_prefix: Option<String>,
    /// Seconds actually billed after applying the billing increment.
    pub billable_secs: u64,
}

/// Extract the dialed digits from a SIP or tel URI for prefix matching.
///
/// `sip:+14155550100@example.com;user=phone` → `14155550100`,
/// `tel:+44-20-7946-0000` → `442079460000`, `sip:2001@pbx` → `2001`.
/// Returns `None` when the user part contains no digits (e.g. `sip:alice@…`).
pub fn dialed_digits_from_uri(uri: &str) -> Option<String> {
    let without_scheme = uri
        .strip_prefix("sips:")
        .or_else(|| uri.strip_prefix("sip:"))
        .or_else(|| uri.strip_prefix("tel:"))
        .unwrap_or(uri);
    let user_part = without_scheme
        .split('@')
        .next()
        .unwrap_or(without_scheme)
        .split(';')
        .next()
        .unwrap_or_default();
    let digits: String = user_part.chars().filter(|c| c.is_ascii_digit()).collect();
    // Reject user parts that are names with incidental digits (e.g. "bob2"):
    // billable destinations are all-digit (after stripping +, separators).
    let significant: String = user_part
        .chars()
        .filter(|c| !matches!(c, '+' | '-' | '.' | '(' | ')' | ' '))
        .collect();
    if digits.is_empty() || !significant.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(digits)
}

/// Rate a call against a plan. `duration_ms` is the *answered* duration
/// (`call_duration_ms`); unanswered calls (zero duration) cost nothing.
pub fn rate_call(
    plan: &RatePlan,
    rates: &[Rate],
    dialed_digits: &str,
    duration_ms: u64,
) -> RatedCall {
    if duration_ms == 0 {
        return RatedCall {
            cost: 0.0,
            currency: plan.currency.clone(),
            matched_prefix: None,
            billable_secs: 0,
        };
    }

    // Longest matching prefix wins; ties cannot occur (prefixes are unique
    // per plan, enforced by the storage layer).
    let best = rates
        .iter()
        .filter(|r| dialed_digits.starts_with(r.prefix.trim_start_matches('+')))
        .max_by_key(|r| r.prefix.trim_start_matches('+').len());

    let (rate_per_minute, connect_fee, matched_prefix) = match best {
        Some(rate) => (
            rate.rate_per_minute,
            rate.connect_fee.unwrap_or(plan.connect_fee),
            Some(rate.prefix.clone()),
        ),
        None => (plan.default_rate_per_minute, plan.connect_fee, None),
    };

    let increment = u64::from(plan.billing_increment_secs.max(1));
    let duration_secs = duration_ms.div_ceil(1000);
    let billable_secs = duration_secs.div_ceil(increment) * increment;

    let cost = connect_fee + rate_per_minute * (billable_secs as f64) / 60.0;
    RatedCall {
        cost: (cost * 10_000.0).round() / 10_000.0,
        currency: plan.currency.clone(),
        matched_prefix,
        billable_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> RatePlan {
        RatePlan {
            id: Uuid::new_v4(),
            tenant_id: crate::TenantId::new(),
            name: "standard".into(),
            currency: "USD".into(),
            default_rate_per_minute: 0.02,
            connect_fee: 0.0,
            billing_increment_secs: 60,
            is_default: true,
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }

    fn rate(plan_id: Uuid, prefix: &str, per_minute: f64) -> Rate {
        Rate {
            id: Uuid::new_v4(),
            rate_plan_id: plan_id,
            prefix: prefix.into(),
            rate_per_minute: per_minute,
            connect_fee: None,
            description: None,
        }
    }

    #[test]
    fn test_dialed_digits_from_uri() {
        assert_eq!(
            dialed_digits_from_uri("sip:+14155550100@example.com;user=phone").as_deref(),
            Some("14155550100")
        );
        assert_eq!(
            dialed_digits_from_uri("tel:+44-20-7946-0000").as_deref(),
            Some("442079460000")
        );
        assert_eq!(
            dialed_digits_from_uri("sip:2001@pbx.local").as_deref(),
            Some("2001")
        );
        assert_eq!(dialed_digits_from_uri("sip:alice@example.com"), None);
        assert_eq!(dialed_digits_from_uri("sip:bob2@example.com"), None);
        assert_eq!(dialed_digits_from_uri("queue:support"), None);
    }

    #[test]
    fn test_longest_prefix_wins() {
        let p = plan();
        let rates = vec![
            rate(p.id, "1", 0.01),
            rate(p.id, "1415", 0.005),
            rate(p.id, "44", 0.03),
        ];
        let rated = rate_call(&p, &rates, "14155550100", 60_000);
        assert_eq!(rated.matched_prefix.as_deref(), Some("1415"));
        assert!((rated.cost - 0.005).abs() < 1e-9);
    }

    #[test]
    fn test_default_rate_when_no_prefix_matches() {
        let p = plan();
        let rates = vec![rate(p.id, "44", 0.03)];
        let rated = rate_call(&p, &rates, "3312345678", 120_000);
        assert_eq!(rated.matched_prefix, None);
        assert!((rated.cost - 0.04).abs() < 1e-9); // 2 min * 0.02
    }

    #[test]
    fn test_billing_increment_rounds_up() {
        let mut p = plan();
        p.billing_increment_secs = 60;
        let rated = rate_call(&p, &[], "15550100", 61_000); // 61s → 120s billed
        assert_eq!(rated.billable_secs, 120);
        assert!((rated.cost - 0.04).abs() < 1e-9);

        p.billing_increment_secs = 1; // per-second billing
        let rated = rate_call(&p, &[], "15550100", 61_000);
        assert_eq!(rated.billable_secs, 61);
        assert!((rated.cost - 0.0203).abs() < 1e-9); // rounded to 4 dp
    }

    #[test]
    fn test_connect_fee_and_override() {
        let mut p = plan();
        p.connect_fee = 0.01;
        let mut r = rate(p.id, "44", 0.03);
        let rated = rate_call(&p, std::slice::from_ref(&r), "442079460000", 60_000);
        assert!((rated.cost - 0.04).abs() < 1e-9); // 0.01 + 0.03

        r.connect_fee = Some(0.05);
        let rated = rate_call(&p, &[r], "442079460000", 60_000);
        assert!((rated.cost - 0.08).abs() < 1e-9); // 0.05 + 0.03
    }

    #[test]
    fn test_unanswered_call_costs_nothing() {
        let p = plan();
        let rated = rate_call(&p, &[], "15550100", 0);
        assert_eq!(rated.cost, 0.0);
        assert_eq!(rated.billable_secs, 0);
    }

    #[test]
    fn test_plus_prefix_in_rate_matches_bare_digits() {
        let p = plan();
        let rates = vec![rate(p.id, "+44", 0.03)];
        let rated = rate_call(&p, &rates, "442079460000", 60_000);
        assert_eq!(rated.matched_prefix.as_deref(), Some("+44"));
    }
}
