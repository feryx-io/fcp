//! A tenant's least-cost routes (`docs/TENANT_TRUNKS.md` §12, phase 6c).
//!
//! A route says what a destination prefix costs on one of the tenant's own
//! trunks. When a tenant's call is dialled to a number one of its routes
//! matches, the trunks those routes name are tried longest prefix first,
//! then cheapest, then by the route's priority — ahead of the tenant's
//! ordinary choice by trunk priority and weight. The platform's trunks keep
//! the node's `[lcr]` table.
//!
//! Cost orders routes and nothing else: it is what the carrier charges the
//! tenant, not what the tenant is charged (`docs/BILLING.md`).

use crate::identifiers::TenantId;
use crate::trunk::TrunkId;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::SystemTime;
use uuid::Uuid;

/// Unique identifier for a least-cost route.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LcrRouteId(String);

impl Default for LcrRouteId {
    fn default() -> Self {
        Self::new()
    }
}

impl LcrRouteId {
    pub fn new() -> Self {
        Self(format!("lcr-{}", Uuid::new_v4()))
    }

    pub fn from_string(s: String) -> Self {
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LcrRouteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The priority a route has when none is given: lower is preferred.
pub const DEFAULT_LCR_PRIORITY: u32 = 100;

/// One least-cost route of a tenant's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcrRoute {
    pub id: LcrRouteId,
    pub tenant_id: TenantId,
    /// The dialled-number prefix it prices: digits, optionally a leading
    /// `+` (`+44`, `+1212`, `011`). The longest that matches wins.
    pub prefix: String,
    /// The tenant's own trunk that carries it at this cost.
    pub trunk_id: TrunkId,
    /// What a minute costs on this trunk, for ordering only.
    pub cost_per_minute: Decimal,
    /// Breaks a tie in cost at the same prefix; lower is preferred.
    pub priority: u32,
    pub enabled: bool,
    pub description: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl LcrRoute {
    pub fn new(
        tenant_id: TenantId,
        prefix: impl Into<String>,
        trunk_id: TrunkId,
        cost_per_minute: Decimal,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            id: LcrRouteId::new(),
            tenant_id,
            prefix: prefix.into(),
            trunk_id,
            cost_per_minute,
            priority: DEFAULT_LCR_PRIORITY,
            enabled: true,
            description: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// What is wrong with this route, if anything: a prefix of digits with
    /// an optional leading `+`, a cost that is not negative, a priority of
    /// at most 1000.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let digits = self.prefix.strip_prefix('+').unwrap_or(&self.prefix);
        if digits.is_empty() {
            return Err("prefix must have at least one digit".to_string());
        }
        if !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!(
                "prefix '{}' may hold only digits, with an optional leading '+'",
                self.prefix
            ));
        }
        if self.cost_per_minute.is_sign_negative() && !self.cost_per_minute.is_zero() {
            return Err(format!(
                "cost_per_minute must not be negative, got {}",
                self.cost_per_minute
            ));
        }
        if self.priority > 1000 {
            return Err(format!(
                "priority must be at most 1000, got {}",
                self.priority
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn route(prefix: &str, cost: &str) -> LcrRoute {
        LcrRoute::new(
            TenantId::new(),
            prefix,
            TrunkId::from_string("trunk-1".into()),
            Decimal::from_str(cost).unwrap(),
        )
    }

    #[test]
    fn what_a_route_may_say() {
        assert_eq!(route("+44", "0.012").validate(), Ok(()));
        assert_eq!(route("011", "0").validate(), Ok(()));
        assert!(route("+", "0.01").validate().is_err());
        assert!(route("", "0.01").validate().is_err());
        assert!(route("+44*", "0.01").validate().is_err());
        assert!(route("4 4", "0.01").validate().is_err());
        assert!(route("+44", "-0.01").validate().is_err());
        let mut too_low = route("+44", "0.01");
        too_low.priority = 1001;
        assert!(too_low.validate().is_err());
    }
}
