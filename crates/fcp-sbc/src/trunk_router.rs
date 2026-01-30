//! Trunk routing for the SBC.
//!
//! Selects an outbound trunk based on direction, enabled state, health,
//! capacity, priority, and weighted random selection among equal-priority trunks.
//!
//! **v1 scope**: Built but NOT integrated into the handler's INVITE routing flow.
//! This module is a standalone trunk selector; integration into
//! `SipMessageHandler::forward_invite_to_b_leg()` is deferred.

use crate::trunk_health::TrunkHealthMonitor;
use fcp_sbc_core::trunk::{TrunkConfig, TrunkDirection};
use rand::Rng;
use std::sync::Arc;

/// Trunk router selects outbound trunks based on policy.
pub struct TrunkRouter {
    trunks: Vec<TrunkConfig>,
    health: Arc<TrunkHealthMonitor>,
}

impl TrunkRouter {
    pub fn new(trunks: Vec<TrunkConfig>, health: Arc<TrunkHealthMonitor>) -> Self {
        Self { trunks, health }
    }

    /// Select the best trunk for the given direction.
    ///
    /// Filters by:
    /// 1. Direction matches (or bidirectional)
    /// 2. Trunk is enabled
    /// 3. Trunk is healthy (not Down)
    /// 4. Trunk has capacity (concurrent calls < max, or max == 0 for unlimited)
    ///
    /// Then selects by priority (lowest first), with weighted random selection
    /// among trunks sharing the best priority.
    pub fn select_trunk(&self, direction: TrunkDirection) -> Option<&TrunkConfig> {
        let candidates: Vec<&TrunkConfig> = self
            .trunks
            .iter()
            .filter(|t| {
                // Direction match
                let dir_match = t.direction == direction
                    || t.direction == TrunkDirection::Bidirectional
                    || direction == TrunkDirection::Bidirectional;
                // Enabled
                let enabled = t.enabled;
                // Healthy
                let healthy = self.health.is_healthy(&t.name);
                // Capacity (0 = unlimited)
                let has_capacity = t.max_concurrent_calls == 0
                    || self.health.concurrent_calls(&t.name) < t.max_concurrent_calls as u64;

                dir_match && enabled && healthy && has_capacity
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Find the best (lowest) priority
        let best_priority = candidates.iter().map(|t| t.priority).min().unwrap();

        // Filter to only best-priority trunks
        let best: Vec<&TrunkConfig> = candidates
            .into_iter()
            .filter(|t| t.priority == best_priority)
            .collect();

        if best.len() == 1 {
            return Some(best[0]);
        }

        // Weighted random selection among equal-priority trunks
        let total_weight: u32 = best.iter().map(|t| t.weight).sum();
        if total_weight == 0 {
            // All zero weight — pick first
            return best.first().copied();
        }

        let mut rng = rand::thread_rng();
        let mut pick = rng.gen_range(0..total_weight);
        for trunk in &best {
            if pick < trunk.weight {
                return Some(trunk);
            }
            pick -= trunk.weight;
        }

        // Fallback (shouldn't reach here)
        best.first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_sbc_core::trunk::TrunkFailover;

    fn make_trunk(name: &str, priority: u32, weight: u32, direction: TrunkDirection) -> TrunkConfig {
        TrunkConfig {
            name: name.into(),
            enabled: true,
            direction,
            priority,
            weight,
            hosts: vec!["sip.example.com".into()],
            failover: TrunkFailover::default(),
            ..Default::default()
        }
    }

    #[test]
    fn test_select_single_trunk() {
        let trunks = vec![make_trunk("a", 100, 100, TrunkDirection::Upstream)];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("a");
        let router = TrunkRouter::new(trunks, health);

        let selected = router.select_trunk(TrunkDirection::Upstream);
        assert_eq!(selected.unwrap().name, "a");
    }

    #[test]
    fn test_select_by_priority() {
        let trunks = vec![
            make_trunk("low", 200, 100, TrunkDirection::Upstream),
            make_trunk("high", 100, 100, TrunkDirection::Upstream),
        ];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("low");
        health.record_success("high");
        let router = TrunkRouter::new(trunks, health);

        let selected = router.select_trunk(TrunkDirection::Upstream);
        assert_eq!(selected.unwrap().name, "high"); // priority 100 < 200
    }

    #[test]
    fn test_direction_filter() {
        let trunks = vec![
            make_trunk("upstream", 100, 100, TrunkDirection::Upstream),
            make_trunk("downstream", 100, 100, TrunkDirection::Downstream),
        ];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("upstream");
        health.record_success("downstream");
        let router = TrunkRouter::new(trunks, health);

        let selected = router.select_trunk(TrunkDirection::Downstream);
        assert_eq!(selected.unwrap().name, "downstream");
    }

    #[test]
    fn test_bidirectional_matches_all() {
        let trunks = vec![make_trunk("bidi", 100, 100, TrunkDirection::Bidirectional)];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("bidi");
        let router = TrunkRouter::new(trunks, health);

        assert!(router.select_trunk(TrunkDirection::Upstream).is_some());
        assert!(router.select_trunk(TrunkDirection::Downstream).is_some());
    }

    #[test]
    fn test_disabled_trunk_excluded() {
        let mut trunk = make_trunk("a", 100, 100, TrunkDirection::Upstream);
        trunk.enabled = false;
        let trunks = vec![trunk];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("a");
        let router = TrunkRouter::new(trunks, health);

        assert!(router.select_trunk(TrunkDirection::Upstream).is_none());
    }

    #[test]
    fn test_unhealthy_trunk_excluded() {
        let trunks = vec![make_trunk("a", 100, 100, TrunkDirection::Upstream)];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        // Mark as down
        for _ in 0..5 {
            health.record_failure("a");
        }
        let router = TrunkRouter::new(trunks, health);

        assert!(router.select_trunk(TrunkDirection::Upstream).is_none());
    }

    #[test]
    fn test_no_trunks_returns_none() {
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        let router = TrunkRouter::new(Vec::new(), health);
        assert!(router.select_trunk(TrunkDirection::Upstream).is_none());
    }

    #[test]
    fn test_capacity_limit() {
        let mut trunk = make_trunk("a", 100, 100, TrunkDirection::Upstream);
        trunk.max_concurrent_calls = 1;
        let trunks = vec![trunk];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("a");
        health.call_started("a"); // now at 1 concurrent

        let router = TrunkRouter::new(trunks, health);
        assert!(router.select_trunk(TrunkDirection::Upstream).is_none());
    }
}
