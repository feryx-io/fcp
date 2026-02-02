//! Trunk routing for the SBC.
//!
//! Selects an outbound trunk based on direction, enabled state, health,
//! capacity, priority, and weighted random selection among equal-priority trunks.
//!
//! Integrated into `SipMessageHandler::forward_invite_to_b_leg()` via the
//! `OutboundRouteProvider` trait as a fallback when registrar lookup fails.

use crate::cps::CpsChecker;
use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk::{TrunkConfig, TrunkDirection};
use fcp_sip::{OutboundRoute, OutboundRouteProvider};
use rand::Rng;
use std::sync::Arc;

/// Trunk router selects outbound trunks based on policy.
pub struct TrunkRouter {
    trunks: Vec<TrunkConfig>,
    health: Arc<TrunkHealthMonitor>,
    cps: Arc<dyn CpsChecker>,
}

impl TrunkRouter {
    pub fn new(
        trunks: Vec<TrunkConfig>,
        health: Arc<TrunkHealthMonitor>,
        cps: Arc<dyn CpsChecker>,
    ) -> Self {
        Self {
            trunks,
            health,
            cps,
        }
    }

    /// Select the best trunk for the given direction.
    ///
    /// Filters by:
    /// 1. Direction matches (or bidirectional)
    /// 2. Trunk is enabled
    /// 3. Trunk is healthy (not Down)
    /// 4. Trunk has capacity (concurrent calls < max, or max == 0 for unlimited)
    /// 5. Trunk is within its per-trunk CPS limit (if configured)
    ///
    /// After filtering, partitions candidates into preferred (Healthy/Unknown)
    /// and degraded pools. Preferred trunks are selected first; degraded trunks
    /// are used only as a fallback when no preferred candidates exist.
    ///
    /// Within a pool, selects by priority (lowest first), with weighted random
    /// selection among trunks sharing the best priority. Records the selected
    /// trunk's call in the CPS sliding window.
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
                // Per-trunk CPS limit
                let within_cps = self.cps.is_trunk_within_cps(&t.name);

                dir_match && enabled && healthy && has_capacity && within_cps
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Partition into preferred (not degraded) and degraded
        let (preferred, degraded): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|t| !self.health.is_degraded(&t.name));

        let pool = if !preferred.is_empty() {
            preferred
        } else {
            degraded
        };

        let selected = self.select_from_pool(&pool)?;

        // Record the call in the per-trunk CPS sliding window
        self.cps.record_trunk_call(&selected.name);
        Some(selected)
    }

    /// Select a trunk from a pool by priority (lowest first) with weighted
    /// random selection among equal-priority candidates.
    fn select_from_pool<'a>(&self, pool: &[&'a TrunkConfig]) -> Option<&'a TrunkConfig> {
        if pool.is_empty() {
            return None;
        }

        // Find the best (lowest) priority
        let best_priority = pool.iter().map(|t| t.priority).min().unwrap();

        // Filter to only best-priority trunks
        let best: Vec<&&TrunkConfig> = pool
            .iter()
            .filter(|t| t.priority == best_priority)
            .collect();

        if best.len() == 1 {
            return Some(best[0]);
        }

        // Weighted random selection among equal-priority trunks
        let total_weight: u32 = best.iter().map(|t| t.weight).sum();
        if total_weight == 0 {
            return Some(best[0]);
        }

        let mut rng = rand::thread_rng();
        let mut pick = rng.gen_range(0..total_weight);
        let mut chosen = best[0];
        for trunk in &best {
            if pick < trunk.weight {
                chosen = trunk;
                break;
            }
            pick -= trunk.weight;
        }
        Some(chosen)
    }
}

#[async_trait::async_trait]
impl OutboundRouteProvider for TrunkRouter {
    async fn resolve_outbound(&self, destination_uri: &str) -> Option<OutboundRoute> {
        // Select an upstream trunk for outbound calls
        let trunk = self.select_trunk(TrunkDirection::Upstream)?;
        let trunk_name = trunk.name.clone();

        // Increment per-trunk concurrent call counter
        self.health.call_started(&trunk_name);

        // Extract user part from destination URI
        let user_part = destination_uri
            .strip_prefix("sip:")
            .or_else(|| destination_uri.strip_prefix("sips:"))
            .unwrap_or(destination_uri)
            .split('@')
            .next()
            .unwrap_or(destination_uri);

        let host = trunk.hosts.first()?;
        let port = trunk
            .transport
            .port
            .unwrap_or_else(|| trunk.transport.protocol.default_port());
        let transport_param = match trunk.transport.protocol {
            fcp_core::transport::TransportType::Udp => String::new(),
            other => format!(";transport={}", other.as_str()),
        };

        Some(OutboundRoute {
            target_uri: format!("sip:{}@{}:{}{}", user_part, host, port, transport_param),
            trunk_name: Some(trunk_name),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cps::NoCpsLimit;
    use crate::trunk::TrunkFailover;

    fn make_trunk(
        name: &str,
        priority: u32,
        weight: u32,
        direction: TrunkDirection,
    ) -> TrunkConfig {
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

    fn no_cps() -> Arc<dyn CpsChecker> {
        Arc::new(NoCpsLimit)
    }

    #[test]
    fn test_select_single_trunk() {
        let trunks = vec![make_trunk("a", 100, 100, TrunkDirection::Upstream)];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("a");
        let router = TrunkRouter::new(trunks, health, no_cps());

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
        let router = TrunkRouter::new(trunks, health, no_cps());

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
        let router = TrunkRouter::new(trunks, health, no_cps());

        let selected = router.select_trunk(TrunkDirection::Downstream);
        assert_eq!(selected.unwrap().name, "downstream");
    }

    #[test]
    fn test_bidirectional_matches_all() {
        let trunks = vec![make_trunk("bidi", 100, 100, TrunkDirection::Bidirectional)];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("bidi");
        let router = TrunkRouter::new(trunks, health, no_cps());

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
        let router = TrunkRouter::new(trunks, health, no_cps());

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
        let router = TrunkRouter::new(trunks, health, no_cps());

        assert!(router.select_trunk(TrunkDirection::Upstream).is_none());
    }

    #[test]
    fn test_no_trunks_returns_none() {
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        let router = TrunkRouter::new(Vec::new(), health, no_cps());
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

        let router = TrunkRouter::new(trunks, health, no_cps());
        assert!(router.select_trunk(TrunkDirection::Upstream).is_none());
    }

    #[test]
    fn test_degraded_trunk_deprioritized() {
        // Two trunks at same priority: one Healthy, one Degraded
        let trunks = vec![
            make_trunk("healthy", 100, 100, TrunkDirection::Upstream),
            make_trunk("degraded", 100, 100, TrunkDirection::Upstream),
        ];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("healthy");
        // Push "degraded" to Degraded state (max_failures defaults to 5, so 3 >= 5/2)
        health.record_failure("degraded");
        health.record_failure("degraded");
        health.record_failure("degraded");
        assert!(health.is_degraded("degraded"));

        let router = TrunkRouter::new(trunks, health, no_cps());
        // Healthy trunk should always be selected over Degraded at same priority
        for _ in 0..20 {
            let selected = router.select_trunk(TrunkDirection::Upstream);
            assert_eq!(selected.unwrap().name, "healthy");
        }
    }

    #[test]
    fn test_degraded_trunk_used_as_fallback() {
        // Only trunk is Degraded — should still be selected
        let trunks = vec![make_trunk("degraded", 100, 100, TrunkDirection::Upstream)];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_failure("degraded");
        health.record_failure("degraded");
        health.record_failure("degraded");
        assert!(health.is_degraded("degraded"));

        let router = TrunkRouter::new(trunks, health, no_cps());
        let selected = router.select_trunk(TrunkDirection::Upstream);
        assert_eq!(selected.unwrap().name, "degraded");
    }

    #[test]
    fn test_all_degraded_selects_by_priority() {
        // Two degraded trunks with different priorities
        let trunks = vec![
            make_trunk("low-pri", 200, 100, TrunkDirection::Upstream),
            make_trunk("high-pri", 100, 100, TrunkDirection::Upstream),
        ];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        // Push both to Degraded
        for name in &["low-pri", "high-pri"] {
            health.record_failure(name);
            health.record_failure(name);
            health.record_failure(name);
            assert!(health.is_degraded(name));
        }

        let router = TrunkRouter::new(trunks, health, no_cps());
        let selected = router.select_trunk(TrunkDirection::Upstream);
        assert_eq!(selected.unwrap().name, "high-pri"); // priority 100 < 200
    }
}
