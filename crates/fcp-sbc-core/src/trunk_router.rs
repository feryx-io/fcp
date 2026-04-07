//! Trunk routing for the SBC.
//!
//! Selects an outbound trunk based on direction, enabled state, health,
//! capacity, priority, and weighted random selection among equal-priority trunks.
//!
//! When LCR is enabled, adds destination-aware prefix matching, cost-based
//! ordering, quality filtering (ASR), and multiple load balancing strategies.
//! Returns ordered trunk lists for failover.
//!
//! Integrated into `SipMessageHandler::forward_invite_to_b_leg()` via the
//! `OutboundRouteProvider` trait as a fallback when registrar lookup fails.

use crate::cps::CpsChecker;
use crate::lcr::{LcrConfig, LcrRouteTable, LoadBalanceStrategy};
use crate::trunk::{TrunkConfig, TrunkDirection};
use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk_quality::TrunkQualityTracker;
use fcp_sip::{OutboundRoute, OutboundRouteProvider};
use rand::Rng;
use std::sync::Arc;

/// Minimum number of calls in the quality window before ASR filtering kicks in.
const MIN_SAMPLES_FOR_QUALITY_FILTER: u64 = 10;

/// Trunk router selects outbound trunks based on policy.
pub struct TrunkRouter {
    trunks: Vec<TrunkConfig>,
    health: Arc<TrunkHealthMonitor>,
    cps: Arc<dyn CpsChecker>,
    lcr: Option<LcrRouteTable>,
    lcr_config: LcrConfig,
    quality: Option<Arc<TrunkQualityTracker>>,
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
            lcr: None,
            lcr_config: LcrConfig::default(),
            quality: None,
        }
    }

    /// Create a router with LCR support.
    pub fn with_lcr(
        trunks: Vec<TrunkConfig>,
        health: Arc<TrunkHealthMonitor>,
        cps: Arc<dyn CpsChecker>,
        lcr_config: LcrConfig,
        quality: Arc<TrunkQualityTracker>,
    ) -> Self {
        let lcr = if lcr_config.enabled {
            Some(LcrRouteTable::new(&lcr_config.routes))
        } else {
            None
        };
        Self {
            trunks,
            health,
            cps,
            lcr,
            lcr_config,
            quality: Some(quality),
        }
    }

    /// Select the best trunk for the given direction (legacy selection).
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
        let candidates: Vec<&TrunkConfig> = self.filter_candidates(direction);

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

    /// Select ordered list of trunks for the given destination, applying LCR
    /// when enabled, falling back to legacy priority/weight selection.
    ///
    /// Returns trunks in preference order for failover. The caller should
    /// try each trunk in order until one succeeds or all are exhausted.
    pub fn select_trunks_ordered(
        &self,
        direction: TrunkDirection,
        destination: &str,
    ) -> Vec<&TrunkConfig> {
        // If LCR disabled or no route table: fall back to legacy single trunk
        let lcr_table = match &self.lcr {
            Some(table) => table,
            None => {
                return self.select_trunk(direction).into_iter().collect();
            }
        };

        // Find matching routes from LCR table (ordered by prefix len desc, cost asc, priority asc)
        let matched_routes = lcr_table.find_routes_ordered(destination);

        if matched_routes.is_empty() {
            // No LCR routes match — fall back to legacy selection
            return self.select_trunk(direction).into_iter().collect();
        }

        // For each matched route, look up the trunk and apply filters
        let mut candidates: Vec<(&TrunkConfig, usize, f64, u32)> = Vec::new();

        for route in &matched_routes {
            let trunk = match self.trunks.iter().find(|t| t.name == route.trunk_name) {
                Some(t) => t,
                None => continue,
            };

            // Apply standard filters
            if !self.is_trunk_eligible(trunk, direction) {
                continue;
            }

            // Apply quality filter: skip trunks with ASR < min_asr
            if self.lcr_config.min_asr > 0.0 {
                if let Some(ref quality) = self.quality {
                    let sample_count = quality.total_calls_in_window(&trunk.name);
                    if sample_count >= MIN_SAMPLES_FOR_QUALITY_FILTER {
                        let asr = quality.asr(&trunk.name);
                        if asr < self.lcr_config.min_asr {
                            continue;
                        }
                    }
                }
            }

            candidates.push((
                trunk,
                route.prefix_len,
                route.cost_per_minute,
                route.priority,
            ));
        }

        if candidates.is_empty() {
            return Vec::new();
        }

        // Apply load balancing among equal-cost/equal-priority/equal-prefix candidates
        self.apply_load_balancing(&mut candidates);

        // Limit to max_failover_attempts + 1
        let max_results = (self.lcr_config.max_failover_attempts + 1) as usize;
        candidates.truncate(max_results);

        candidates
            .into_iter()
            .map(|(trunk, _, _, _)| trunk)
            .collect()
    }

    /// Check if a trunk passes standard eligibility filters.
    fn is_trunk_eligible(&self, trunk: &TrunkConfig, direction: TrunkDirection) -> bool {
        // Direction match
        let dir_match = trunk.direction == direction
            || trunk.direction == TrunkDirection::Bidirectional
            || direction == TrunkDirection::Bidirectional;

        let enabled = trunk.enabled;
        let healthy = self.health.is_healthy(&trunk.name);
        let has_capacity = trunk.max_concurrent_calls == 0
            || self.health.concurrent_calls(&trunk.name) < trunk.max_concurrent_calls as u64;
        let within_cps = self.cps.is_trunk_within_cps(&trunk.name);

        dir_match && enabled && healthy && has_capacity && within_cps
    }

    /// Filter candidates by direction, enabled, health, capacity, CPS.
    fn filter_candidates(&self, direction: TrunkDirection) -> Vec<&TrunkConfig> {
        self.trunks
            .iter()
            .filter(|t| self.is_trunk_eligible(t, direction))
            .collect()
    }

    /// Apply load balancing strategy to reorder equal-tier candidates.
    fn apply_load_balancing(&self, candidates: &mut Vec<(&TrunkConfig, usize, f64, u32)>) {
        match self.lcr_config.load_balance_strategy {
            LoadBalanceStrategy::LeastCost => {
                // Already sorted by prefix_len desc, cost asc, priority asc — no change needed
            }
            LoadBalanceStrategy::RoundRobin => {
                self.apply_round_robin(candidates);
            }
            LoadBalanceStrategy::WeightedRandom => {
                self.apply_weighted_random(candidates);
            }
            LoadBalanceStrategy::LeastConnections => {
                self.apply_least_connections(candidates);
            }
        }
    }

    /// Rotate among candidates that share the same tier (prefix_len + cost + priority).
    fn apply_round_robin(&self, candidates: &mut Vec<(&TrunkConfig, usize, f64, u32)>) {
        if candidates.len() <= 1 {
            return;
        }

        // Group equal-tier candidates and rotate within groups
        let mut i = 0;
        while i < candidates.len() {
            let tier = (
                candidates[i].1,
                format!("{:.10}", candidates[i].2),
                candidates[i].3,
            );
            let start = i;
            while i < candidates.len() {
                let curr = (
                    candidates[i].1,
                    format!("{:.10}", candidates[i].2),
                    candidates[i].3,
                );
                if curr != tier {
                    break;
                }
                i += 1;
            }
            let group_len = i - start;
            if group_len > 1 {
                if let Some(ref quality) = self.quality {
                    // Use the first trunk's counter to determine rotation offset
                    let counter = quality.next_round_robin(&candidates[start].0.name);
                    let offset = (counter as usize) % group_len;
                    // Rotate the group
                    candidates[start..i].rotate_left(offset);
                }
            }
        }
    }

    /// Weighted random selection among equal-tier candidates.
    fn apply_weighted_random(&self, candidates: &mut Vec<(&TrunkConfig, usize, f64, u32)>) {
        if candidates.len() <= 1 {
            return;
        }

        let mut rng = rand::thread_rng();
        let mut i = 0;
        while i < candidates.len() {
            let tier = (
                candidates[i].1,
                format!("{:.10}", candidates[i].2),
                candidates[i].3,
            );
            let start = i;
            while i < candidates.len() {
                let curr = (
                    candidates[i].1,
                    format!("{:.10}", candidates[i].2),
                    candidates[i].3,
                );
                if curr != tier {
                    break;
                }
                i += 1;
            }
            let group_len = i - start;
            if group_len > 1 {
                // Shuffle within group using weighted random
                let total_weight: u32 = candidates[start..i]
                    .iter()
                    .map(|(t, _, _, _)| t.weight)
                    .sum();
                if total_weight > 0 {
                    let pick = rng.gen_range(0..total_weight);
                    let mut acc = 0u32;
                    for j in start..i {
                        acc += candidates[j].0.weight;
                        if acc > pick {
                            // Move the picked candidate to the front of the group
                            candidates[start..=j].rotate_right(1);
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Sort equal-tier candidates by concurrent calls (ascending).
    fn apply_least_connections(&self, candidates: &mut Vec<(&TrunkConfig, usize, f64, u32)>) {
        if candidates.len() <= 1 {
            return;
        }

        // Stable sort preserves existing ordering within ties
        candidates.sort_by(|a, b| {
            // First by tier (prefix_len desc, cost asc, priority asc)
            let tier_cmp =
                b.1.cmp(&a.1)
                    .then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
                    .then(a.3.cmp(&b.3));

            if tier_cmp != std::cmp::Ordering::Equal {
                return tier_cmp;
            }

            // Within same tier, sort by concurrent calls ascending
            let a_calls = self.health.concurrent_calls(&a.0.name);
            let b_calls = self.health.concurrent_calls(&b.0.name);
            a_calls.cmp(&b_calls)
        });
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

    /// Build an OutboundRoute from a trunk and destination URI.
    fn build_outbound_route(
        &self,
        trunk: &TrunkConfig,
        destination_uri: &str,
    ) -> Option<OutboundRoute> {
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

        let allowed_codecs = if trunk.codec.allowed_codecs.is_empty() {
            None
        } else {
            Some(trunk.codec.allowed_codecs.clone())
        };

        Some(OutboundRoute {
            target_uri: format!("sip:{}@{}:{}{}", user_part, host, port, transport_param),
            trunk_name: Some(trunk.name.clone()),
            allowed_codecs,
        })
    }
}

#[async_trait::async_trait]
impl OutboundRouteProvider for TrunkRouter {
    async fn resolve_outbound(&self, destination_uri: &str) -> Option<OutboundRoute> {
        // Use first result from ordered selection if LCR is enabled
        let trunks = self.select_trunks_ordered(TrunkDirection::Upstream, destination_uri);
        if let Some(trunk) = trunks.first() {
            self.health.call_started(&trunk.name);
            self.cps.record_trunk_call(&trunk.name);
            return self.build_outbound_route(trunk, destination_uri);
        }

        // Fall through to legacy (should not normally reach here, but safety net)
        let trunk = self.select_trunk(TrunkDirection::Upstream)?;
        let trunk_name = trunk.name.clone();
        self.health.call_started(&trunk_name);
        self.build_outbound_route(trunk, destination_uri)
    }

    async fn resolve_outbound_ordered(&self, destination_uri: &str) -> Vec<OutboundRoute> {
        let trunks = self.select_trunks_ordered(TrunkDirection::Upstream, destination_uri);
        trunks
            .iter()
            .filter_map(|trunk| {
                self.health.call_started(&trunk.name);
                self.cps.record_trunk_call(&trunk.name);
                self.build_outbound_route(trunk, destination_uri)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cps::NoCpsLimit;
    use crate::lcr::{LcrConfig, LcrRouteEntry, LoadBalanceStrategy};
    use crate::trunk::TrunkFailover;
    use crate::trunk_quality::TrunkQualityTracker;

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

    fn make_quality(names: &[&str]) -> Arc<TrunkQualityTracker> {
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        Arc::new(TrunkQualityTracker::new(&names, 3600))
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

    // ── LCR selection tests ──────────────────────────────────────────────

    fn lcr_setup() -> (
        Vec<TrunkConfig>,
        Arc<TrunkHealthMonitor>,
        Arc<TrunkQualityTracker>,
    ) {
        let trunks = vec![
            make_trunk("us-cheap", 100, 50, TrunkDirection::Upstream),
            make_trunk("us-premium", 100, 50, TrunkDirection::Upstream),
            make_trunk("nyc-local", 100, 50, TrunkDirection::Upstream),
            make_trunk("uk-carrier", 100, 50, TrunkDirection::Upstream),
        ];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        for t in &trunks {
            health.record_success(&t.name);
        }
        let quality = make_quality(&["us-cheap", "us-premium", "nyc-local", "uk-carrier"]);
        (trunks, health, quality)
    }

    fn lcr_config() -> LcrConfig {
        LcrConfig {
            enabled: true,
            routes: vec![
                LcrRouteEntry {
                    prefix: "+1".into(),
                    trunk_name: "us-cheap".into(),
                    cost_per_minute: 0.01,
                    priority: 100,
                    enabled: true,
                },
                LcrRouteEntry {
                    prefix: "+1".into(),
                    trunk_name: "us-premium".into(),
                    cost_per_minute: 0.05,
                    priority: 100,
                    enabled: true,
                },
                LcrRouteEntry {
                    prefix: "+1212".into(),
                    trunk_name: "nyc-local".into(),
                    cost_per_minute: 0.005,
                    priority: 100,
                    enabled: true,
                },
                LcrRouteEntry {
                    prefix: "+44".into(),
                    trunk_name: "uk-carrier".into(),
                    cost_per_minute: 0.08,
                    priority: 100,
                    enabled: true,
                },
            ],
            load_balance_strategy: LoadBalanceStrategy::LeastCost,
            min_asr: 0.0,
            max_failover_attempts: 2,
            quality_window_secs: 3600,
        }
    }

    #[test]
    fn test_select_trunks_ordered_lcr() {
        let (trunks, health, quality) = lcr_setup();
        let router = TrunkRouter::with_lcr(trunks, health, no_cps(), lcr_config(), quality);

        let selected =
            router.select_trunks_ordered(TrunkDirection::Upstream, "sip:+12125551234@example.com");
        // nyc-local (+1212, $0.005) first, then us-cheap (+1, $0.01), then us-premium (+1, $0.05)
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].name, "nyc-local");
        assert_eq!(selected[1].name, "us-cheap");
        assert_eq!(selected[2].name, "us-premium");
    }

    #[test]
    fn test_select_trunks_ordered_fallback() {
        // LCR disabled — should fall back to legacy single trunk
        let trunks = vec![make_trunk("a", 100, 100, TrunkDirection::Upstream)];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("a");
        let router = TrunkRouter::new(trunks, health, no_cps());

        let selected = router.select_trunks_ordered(TrunkDirection::Upstream, "+15551234567");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "a");
    }

    #[test]
    fn test_select_trunks_ordered_no_match_falls_back() {
        let (trunks, health, quality) = lcr_setup();
        let router = TrunkRouter::with_lcr(trunks, health, no_cps(), lcr_config(), quality);

        // +86 has no LCR route — falls back to legacy selection
        let selected = router.select_trunks_ordered(TrunkDirection::Upstream, "+8612345678");
        assert_eq!(selected.len(), 1); // gets one from legacy selection
    }

    #[test]
    fn test_select_trunks_ordered_quality_filter() {
        let (trunks, health, quality) = lcr_setup();

        // Record 20 calls on us-cheap: 2 answered, 18 failed = 10% ASR
        for _ in 0..2 {
            quality.record_call_attempt("us-cheap");
            quality.record_call_answered("us-cheap", 100);
            quality.record_call_ended("us-cheap", 30.0, 200);
        }
        for _ in 0..18 {
            quality.record_call_attempt("us-cheap");
            quality.record_call_failed("us-cheap", 503);
        }

        let mut config = lcr_config();
        config.min_asr = 0.5; // 50% minimum
        let router = TrunkRouter::with_lcr(trunks, health, no_cps(), config, quality);

        let selected =
            router.select_trunks_ordered(TrunkDirection::Upstream, "sip:+15551234567@host");
        // us-cheap should be excluded (ASR 10% < 50% min)
        // Only us-premium should remain
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "us-premium");
    }

    #[test]
    fn test_select_trunks_ordered_quality_filter_insufficient_samples() {
        let (trunks, health, quality) = lcr_setup();

        // Only 5 calls — below MIN_SAMPLES_FOR_QUALITY_FILTER (10)
        for _ in 0..1 {
            quality.record_call_attempt("us-cheap");
            quality.record_call_answered("us-cheap", 100);
            quality.record_call_ended("us-cheap", 30.0, 200);
        }
        for _ in 0..4 {
            quality.record_call_attempt("us-cheap");
            quality.record_call_failed("us-cheap", 503);
        }

        let mut config = lcr_config();
        config.min_asr = 0.5;
        let router = TrunkRouter::with_lcr(trunks, health, no_cps(), config, quality);

        let selected =
            router.select_trunks_ordered(TrunkDirection::Upstream, "sip:+15551234567@host");
        // us-cheap should NOT be excluded (insufficient samples)
        let names: Vec<&str> = selected.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"us-cheap"));
    }

    #[test]
    fn test_select_trunks_ordered_max_failover() {
        let (trunks, health, quality) = lcr_setup();
        let mut config = lcr_config();
        config.max_failover_attempts = 1; // limit to 2 results

        let router = TrunkRouter::with_lcr(trunks, health, no_cps(), config, quality);

        let selected =
            router.select_trunks_ordered(TrunkDirection::Upstream, "sip:+12125551234@host");
        // max_failover_attempts=1 → 2 results max
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_load_balance_least_connections() {
        let (trunks, health, quality) = lcr_setup();

        // Give us-cheap some concurrent calls
        health.call_started("us-cheap");
        health.call_started("us-cheap");
        health.call_started("us-cheap");

        // Use equal costs for us-cheap and us-premium so they're in the same tier
        let config = LcrConfig {
            enabled: true,
            routes: vec![
                LcrRouteEntry {
                    prefix: "+1".into(),
                    trunk_name: "us-cheap".into(),
                    cost_per_minute: 0.01,
                    priority: 100,
                    enabled: true,
                },
                LcrRouteEntry {
                    prefix: "+1".into(),
                    trunk_name: "us-premium".into(),
                    cost_per_minute: 0.01,
                    priority: 100,
                    enabled: true,
                },
            ],
            load_balance_strategy: LoadBalanceStrategy::LeastConnections,
            min_asr: 0.0,
            max_failover_attempts: 2,
            quality_window_secs: 3600,
        };

        let router = TrunkRouter::with_lcr(trunks, health, no_cps(), config, quality);

        let selected =
            router.select_trunks_ordered(TrunkDirection::Upstream, "sip:+15551234567@host");
        // Both us-cheap and us-premium match +1 at same cost and priority
        // With LeastConnections, us-premium (0 calls) should come before us-cheap (3 calls)
        assert!(selected.len() >= 2);
        assert_eq!(selected[0].name, "us-premium");
        assert_eq!(selected[1].name, "us-cheap");
    }

    #[tokio::test]
    async fn test_resolve_outbound_ordered() {
        let (trunks, health, quality) = lcr_setup();
        let router = TrunkRouter::with_lcr(trunks, health, no_cps(), lcr_config(), quality);

        let routes = router
            .resolve_outbound_ordered("sip:+12125551234@example.com")
            .await;
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].trunk_name, Some("nyc-local".to_string()));
        assert_eq!(routes[1].trunk_name, Some("us-cheap".to_string()));
        assert_eq!(routes[2].trunk_name, Some("us-premium".to_string()));
        // All target URIs should contain the user part
        for route in &routes {
            assert!(route.target_uri.contains("+12125551234"));
        }
    }

    #[tokio::test]
    async fn test_resolve_outbound_with_codec_config() {
        use crate::trunk::TrunkCodecConfig;

        let mut trunks = vec![make_trunk(
            "codec-trunk",
            100,
            100,
            TrunkDirection::Upstream,
        )];
        trunks[0].codec = TrunkCodecConfig {
            allowed_codecs: vec!["PCMU".into(), "PCMA".into()],
            allow_transcoding: true,
        };
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("codec-trunk");
        let router = TrunkRouter::new(trunks, health, no_cps());

        let route = router
            .resolve_outbound("sip:+15551234567@example.com")
            .await;
        assert!(route.is_some());
        let route = route.unwrap();
        assert_eq!(route.trunk_name, Some("codec-trunk".to_string()));
        assert_eq!(
            route.allowed_codecs,
            Some(vec!["PCMU".to_string(), "PCMA".to_string()])
        );
    }

    #[tokio::test]
    async fn test_resolve_outbound_no_codec_restriction() {
        let trunks = vec![make_trunk(
            "plain-trunk",
            100,
            100,
            TrunkDirection::Upstream,
        )];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("plain-trunk");
        let router = TrunkRouter::new(trunks, health, no_cps());

        let route = router
            .resolve_outbound("sip:+15551234567@example.com")
            .await;
        assert!(route.is_some());
        let route = route.unwrap();
        assert_eq!(route.trunk_name, Some("plain-trunk".to_string()));
        assert_eq!(route.allowed_codecs, None);
    }
}
