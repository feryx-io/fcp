//! Call rating: longest-prefix rate selection, billing increments, connect
//! fees. Pure functions over the `fcp_core::billing` model.

use fcp_core::billing::{Rate, RatePlan};

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

/// The rate that applies to one destination under a plan.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedRate {
    pub rate_per_minute: f64,
    pub connect_fee: f64,
    pub matched_prefix: Option<String>,
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

/// Pick the rate for a destination: longest matching prefix wins, else the
/// plan default. Ties cannot occur (prefixes are unique per plan, enforced
/// by the storage layer).
pub fn select_rate(plan: &RatePlan, rates: &[Rate], dialed_digits: &str) -> SelectedRate {
    let best = rates
        .iter()
        .filter(|r| dialed_digits.starts_with(r.prefix.trim_start_matches('+')))
        .max_by_key(|r| r.prefix.trim_start_matches('+').len());

    match best {
        Some(rate) => SelectedRate {
            rate_per_minute: rate.rate_per_minute,
            connect_fee: rate.connect_fee.unwrap_or(plan.connect_fee),
            matched_prefix: Some(rate.prefix.clone()),
        },
        None => SelectedRate {
            rate_per_minute: plan.default_rate_per_minute,
            connect_fee: plan.connect_fee,
            matched_prefix: None,
        },
    }
}

/// Round a duration up to the plan's billing increment.
pub fn billable_seconds(plan: &RatePlan, duration_ms: u64) -> u64 {
    let increment = u64::from(plan.billing_increment_secs.max(1));
    let duration_secs = duration_ms.div_ceil(1000);
    duration_secs.div_ceil(increment) * increment
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

    let selected = select_rate(plan, rates, dialed_digits);
    let billable_secs = billable_seconds(plan, duration_ms);
    let cost = selected.connect_fee + selected.rate_per_minute * (billable_secs as f64) / 60.0;
    RatedCall {
        cost: round_money(cost),
        currency: plan.currency.clone(),
        matched_prefix: selected.matched_prefix,
        billable_secs,
    }
}

/// Round to 4 decimal places (the ledger's precision).
pub fn round_money(amount: f64) -> f64 {
    (amount * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::time::SystemTime;
    use uuid::Uuid;

    pub fn plan() -> RatePlan {
        RatePlan {
            id: Uuid::new_v4(),
            tenant_id: fcp_core::TenantId::new(),
            name: "default".into(),
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

    pub fn rate(plan_id: Uuid, prefix: &str, per_minute: f64) -> Rate {
        Rate {
            id: Uuid::new_v4(),
            rate_plan_id: plan_id,
            prefix: prefix.into(),
            rate_per_minute: per_minute,
            connect_fee: None,
            description: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{plan, rate};
    use super::*;

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
