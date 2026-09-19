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
use fcp_sip::{OutboundRoute, OutboundRouteProvider, RouteResolution};
use rand::Rng;
use std::sync::Arc;

/// Minimum number of calls in the quality window before ASR filtering kicks in.
const MIN_SAMPLES_FOR_QUALITY_FILTER: u64 = 10;

/// Trunk router selects outbound trunks based on policy.
pub struct TrunkRouter {
    trunks: Arc<crate::trunk_set::TrunkSet>,
    health: Arc<TrunkHealthMonitor>,
    cps: Arc<dyn CpsChecker>,
    lcr: Option<LcrRouteTable>,
    lcr_config: LcrConfig,
    quality: Option<Arc<TrunkQualityTracker>>,
    /// Whether choosing a route counts a call on its trunk. The SBC's does,
    /// and releases it when the call ends; a node whose handler counts the
    /// calls its trunks carry ([`crate::TrunkCalls`]) turns this off.
    counts_calls: bool,
}

impl TrunkRouter {
    pub fn new(
        trunks: Vec<TrunkConfig>,
        health: Arc<TrunkHealthMonitor>,
        cps: Arc<dyn CpsChecker>,
    ) -> Self {
        Self {
            trunks: Arc::new(crate::trunk_set::TrunkSet::platform_only(trunks)),
            health,
            cps,
            lcr: None,
            lcr_config: LcrConfig::default(),
            quality: None,
            counts_calls: true,
        }
    }

    /// A router over the trunks a node routes by — its own and every
    /// tenant's (`docs/TENANT_TRUNKS.md` §5).
    pub fn with_trunk_set(
        trunks: Arc<crate::trunk_set::TrunkSet>,
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
            counts_calls: true,
        }
    }

    /// As [`TrunkRouter::with_trunk_set`], with LCR.
    pub fn with_trunk_set_and_lcr(
        trunks: Arc<crate::trunk_set::TrunkSet>,
        health: Arc<TrunkHealthMonitor>,
        cps: Arc<dyn CpsChecker>,
        lcr_config: LcrConfig,
        quality: Arc<TrunkQualityTracker>,
    ) -> Self {
        let lcr = lcr_config
            .enabled
            .then(|| LcrRouteTable::new(&lcr_config.routes));
        Self {
            trunks,
            health,
            cps,
            lcr,
            lcr_config,
            quality: Some(quality),
            counts_calls: true,
        }
    }

    /// Leave counting the calls a trunk carries to the node's handler,
    /// which releases each when its call ends. Selection still honours the
    /// counts, and still records calls against each trunk's CPS.
    pub fn without_call_counting(mut self) -> Self {
        self.counts_calls = false;
        self
    }

    /// The trunks this router routes by.
    pub fn trunk_set(&self) -> &Arc<crate::trunk_set::TrunkSet> {
        &self.trunks
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
            trunks: Arc::new(crate::trunk_set::TrunkSet::platform_only(trunks)),
            health,
            cps,
            lcr,
            lcr_config,
            quality: Some(quality),
            counts_calls: true,
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
    pub fn select_trunk(
        &self,
        tenant: Option<fcp_core::TenantId>,
        direction: TrunkDirection,
    ) -> Option<Arc<TrunkConfig>> {
        // The tenant's own first, then the platform's (`docs/TENANT_TRUNKS.md`
        // §5): a platform trunk's better priority never outranks the carrier
        // the tenant bought.
        let (own, platform) = self.split_for(tenant);
        self.pick(&own, direction)
            .or_else(|| self.pick(&platform, direction))
    }

    /// The trunks `tenant`'s calls may leave on, split into its own and the
    /// platform's (empty when it refused them).
    fn split_for(
        &self,
        tenant: Option<fcp_core::TenantId>,
    ) -> (Vec<Arc<TrunkConfig>>, Vec<Arc<TrunkConfig>>) {
        self.trunks
            .for_tenant(tenant)
            .into_iter()
            .partition(|t| t.tenant_id.is_some())
    }

    /// The ordinary choice among `pool`: eligible trunks, the healthy ahead
    /// of the degraded, the best priority, then weighted at random.
    fn pick(
        &self,
        pool: &[Arc<TrunkConfig>],
        direction: TrunkDirection,
    ) -> Option<Arc<TrunkConfig>> {
        let candidates: Vec<&Arc<TrunkConfig>> = self.filter_candidates(pool, direction);
        if candidates.is_empty() {
            return None;
        }

        // Partition into preferred (not degraded) and degraded
        let (preferred, degraded): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|t| !self.health.is_degraded(&t.key()));

        let pool = if !preferred.is_empty() {
            preferred
        } else {
            degraded
        };

        // Selection is pure: call accounting (health concurrency + CPS
        // window) is recorded once at the OutboundRouteProvider layer for
        // the route actually returned, not here.
        self.select_from_pool(&pool).cloned()
    }

    /// `pool` ordered by what `table` says `destination` costs on each, or
    /// `None` when no route in the table matches it. `key` is how a route
    /// names a trunk: the node's `[lcr]` by name, a tenant's table by id.
    /// Trunks passed over for eligibility or quality are left out, and the
    /// list is cut at `max_failover_attempts + 1`.
    fn lcr_ordered(
        &self,
        pool: &[Arc<TrunkConfig>],
        table: &LcrRouteTable,
        direction: TrunkDirection,
        destination: &str,
        key: impl Fn(&TrunkConfig) -> String,
    ) -> Option<Vec<Arc<TrunkConfig>>> {
        // Ordered by prefix length desc, cost asc, priority asc.
        let matched_routes = table.find_routes_ordered(destination);
        if matched_routes.is_empty() {
            return None;
        }

        let mut candidates: Vec<(&Arc<TrunkConfig>, usize, f64, u32)> = Vec::new();
        for route in &matched_routes {
            let Some(trunk) = pool.iter().find(|t| key(t) == route.trunk_name) else {
                continue;
            };
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

            // A trunk two routes name is tried once, at its best.
            if candidates.iter().any(|(t, ..)| t.key() == trunk.key()) {
                continue;
            }
            candidates.push((
                trunk,
                route.prefix_len,
                route.cost_per_minute,
                route.priority,
            ));
        }

        // Apply load balancing among equal-cost/equal-priority/equal-prefix candidates
        self.apply_load_balancing(&mut candidates);

        // Limit to max_failover_attempts + 1
        let max_results = (self.lcr_config.max_failover_attempts + 1) as usize;
        candidates.truncate(max_results);

        Some(
            candidates
                .into_iter()
                .map(|(trunk, _, _, _)| Arc::clone(trunk))
                .collect(),
        )
    }

    /// The tenant's own trunks for `destination`, best first: by its
    /// least-cost routes when one matches (`docs/TENANT_TRUNKS.md` §12,
    /// phase 6c), else its ordinary choice.
    fn own_ordered(
        &self,
        tenant: Option<fcp_core::TenantId>,
        own: &[Arc<TrunkConfig>],
        direction: TrunkDirection,
        destination: &str,
    ) -> Vec<Arc<TrunkConfig>> {
        let table = tenant.and_then(|t| self.trunks.lcr_for(t));
        match table.and_then(|table| {
            self.lcr_ordered(own, &table, direction, destination, |t| {
                t.trunk_id().to_string()
            })
        }) {
            Some(ordered) => ordered,
            None => self.pick(own, direction).into_iter().collect(),
        }
    }

    /// The platform's trunks for `destination`, best first: by the node's
    /// `[lcr]` when a route matches, else the ordinary choice. The node's
    /// table names the platform's trunks, by name, and only those.
    fn platform_ordered(
        &self,
        platform: &[Arc<TrunkConfig>],
        direction: TrunkDirection,
        destination: &str,
    ) -> Vec<Arc<TrunkConfig>> {
        match self.lcr.as_ref().and_then(|table| {
            self.lcr_ordered(platform, table, direction, destination, |t| t.name.clone())
        }) {
            Some(ordered) => ordered,
            None => self.pick(platform, direction).into_iter().collect(),
        }
    }

    /// Trunks for an emergency call (`docs/TENANT_TRUNKS.md` §5,
    /// `docs/E911_BLOCK_2.md` §12.1): the tenant's own `emergency_capable`
    /// trunks first, then the platform's, then the ordinary ordered choice
    /// so a 911 call still goes out when nothing is flagged at all.
    ///
    /// The tenant's come first because they are the carrier that tenant
    /// bought, and their order among themselves is priority then weight.
    /// The platform's follow **whatever the tenant said about falling back
    /// to them**: `platform_trunk_fallback = false` is a billing and
    /// routing preference for ordinary calls, and nobody's 911 call should
    /// fail because their own carrier list is short or their own carrier is
    /// down.
    pub fn select_emergency_trunks_ordered(
        &self,
        tenant: Option<fcp_core::TenantId>,
        direction: TrunkDirection,
        destination: &str,
    ) -> Vec<Arc<TrunkConfig>> {
        let eligible =
            |t: &Arc<TrunkConfig>| t.emergency_capable && self.is_trunk_eligible(t, direction);
        let by_preference = |trunks: &mut Vec<Arc<TrunkConfig>>| {
            trunks.sort_by_key(|t| (t.priority, std::cmp::Reverse(t.weight)))
        };

        // The tenant's own, in their own order.
        let mut ordered: Vec<Arc<TrunkConfig>> = match tenant {
            Some(_) => {
                let mut theirs: Vec<Arc<TrunkConfig>> = self
                    .trunks
                    .for_tenant(tenant)
                    .into_iter()
                    .filter(|t| t.tenant_id.is_some() && eligible(t))
                    .collect();
                by_preference(&mut theirs);
                theirs
            }
            None => Vec::new(),
        };

        // Then the platform's, asked for directly rather than through the
        // tenant's fallback preference.
        let mut platform: Vec<Arc<TrunkConfig>> = self
            .trunks
            .platform()
            .iter()
            .filter(|t| eligible(t))
            .cloned()
            .collect();
        by_preference(&mut platform);
        for trunk in platform {
            if !ordered.iter().any(|t| t.key() == trunk.key()) {
                ordered.push(trunk);
            }
        }

        // And last, whatever would have carried an ordinary call: a
        // deployment that flagged nothing still places the call.
        for trunk in self.select_trunks_ordered(tenant, direction, destination) {
            if !ordered.iter().any(|t| t.key() == trunk.key()) {
                ordered.push(trunk);
            }
        }
        ordered
    }

    /// Trunks for a call a tenant's `route_to_trunk` rule has confined to
    /// `target` (`docs/TENANT_TRUNKS.md` §12, phase 6b), in the order to try
    /// them, and whether the target names any trunk this node knows.
    ///
    /// A trunk named by id is taken only if it is one this tenant's calls
    /// may leave on. A group is its members among those: the tenant's own
    /// first, then the platform's unless the tenant has refused them, each
    /// by priority then weight. Nothing outside the target is added — a
    /// rule that routes a call somewhere means only there, and a group is
    /// how a rule gets failover.
    pub fn select_targeted_trunks(
        &self,
        tenant: Option<fcp_core::TenantId>,
        direction: TrunkDirection,
        destination: &str,
        target: &fcp_core::TrunkTarget,
    ) -> (Vec<Arc<TrunkConfig>>, bool) {
        let (own, platform) = self.split_for(tenant);
        let named = |pool: Vec<Arc<TrunkConfig>>| -> Vec<Arc<TrunkConfig>> {
            pool.into_iter()
                .filter(|t| fcp_sip::handler::trunk_is_target(t, target))
                .collect()
        };
        let (own, platform) = (named(own), named(platform));
        let known = !own.is_empty() || !platform.is_empty();

        // Within each part, what the least-cost routes say about this
        // destination first (phase 6c); then the rest by priority and
        // weight, so a group keeps its failover past the priced members.
        let ordered_part = |pool: &[Arc<TrunkConfig>], by_lcr: Vec<Arc<TrunkConfig>>| {
            let mut rest: Vec<Arc<TrunkConfig>> = pool
                .iter()
                .filter(|t| !by_lcr.iter().any(|p| p.key() == t.key()))
                .filter(|t| self.is_trunk_eligible(t, direction))
                .cloned()
                .collect();
            rest.sort_by_key(|t| (t.priority, std::cmp::Reverse(t.weight)));
            let mut part = by_lcr;
            part.extend(rest);
            part
        };
        let own_lcr = tenant
            .and_then(|t| self.trunks.lcr_for(t))
            .and_then(|table| {
                self.lcr_ordered(&own, &table, direction, destination, |t| {
                    t.trunk_id().to_string()
                })
            })
            .unwrap_or_default();
        let platform_lcr = self
            .lcr
            .as_ref()
            .and_then(|table| {
                self.lcr_ordered(&platform, table, direction, destination, |t| t.name.clone())
            })
            .unwrap_or_default();

        let mut ordered = ordered_part(&own, own_lcr);
        ordered.extend(ordered_part(&platform, platform_lcr));
        (ordered, known)
    }

    /// Trunks for an ordinary call, in the order to try them: the tenant's
    /// own — by its least-cost routes when one matches the destination,
    /// else its ordinary choice — then the platform's — by the node's
    /// `[lcr]` when a route matches, else the ordinary choice — unless the
    /// tenant refused them (`docs/TENANT_TRUNKS.md` §5, §12 phase 6c).
    pub fn select_trunks_ordered(
        &self,
        tenant: Option<fcp_core::TenantId>,
        direction: TrunkDirection,
        destination: &str,
    ) -> Vec<Arc<TrunkConfig>> {
        let (own, platform) = self.split_for(tenant);
        let mut ordered = self.own_ordered(tenant, &own, direction, destination);
        for trunk in self.platform_ordered(&platform, direction, destination) {
            if !ordered.iter().any(|t| t.key() == trunk.key()) {
                ordered.push(trunk);
            }
        }
        ordered
    }

    /// Check if a trunk passes standard eligibility filters.
    fn is_trunk_eligible(&self, trunk: &TrunkConfig, direction: TrunkDirection) -> bool {
        // Direction match
        let dir_match = trunk.direction == direction
            || trunk.direction == TrunkDirection::Bidirectional
            || direction == TrunkDirection::Bidirectional;

        let enabled = trunk.enabled;
        // Health, capacity and CPS are kept per trunk key, not per name:
        // two tenants may both have a `carrier-a` (`docs/TENANT_TRUNKS.md`
        // §4).
        let key = trunk.key();
        let healthy = self.health.is_healthy(&key);
        let has_capacity = trunk.max_concurrent_calls == 0
            || self.health.concurrent_calls(&key) < trunk.max_concurrent_calls as u64;
        let within_cps = self.cps.is_trunk_within_cps(&key);

        // A trunk passed over for its limits is the one thing here worth
        // counting: it is why a call this node could otherwise place went
        // somewhere else, or nowhere (§9). Being disabled or unreachable
        // is already visible in the trunk's own state.
        if dir_match && enabled && healthy {
            let outcome = if !has_capacity {
                Some(fcp_core::trunk_report::TrunkOutcome::RejectedCapacity)
            } else if !within_cps {
                Some(fcp_core::trunk_report::TrunkOutcome::RejectedCps)
            } else {
                None
            };
            if let Some(outcome) = outcome {
                fcp_core::trunk_report::TrunkCounters::global().note(
                    &key,
                    trunk.tenant_id.map(|t| t.to_string()).as_deref(),
                    outcome,
                );
            }
        }

        dir_match && enabled && healthy && has_capacity && within_cps
    }

    /// Filter candidates by direction, enabled, health, capacity, CPS.
    fn filter_candidates<'a>(
        &self,
        trunks: &'a [Arc<TrunkConfig>],
        direction: TrunkDirection,
    ) -> Vec<&'a Arc<TrunkConfig>> {
        trunks
            .iter()
            .filter(|t| self.is_trunk_eligible(t, direction))
            .collect()
    }

    /// Apply load balancing strategy to reorder equal-tier candidates.
    fn apply_load_balancing(&self, candidates: &mut Vec<(&Arc<TrunkConfig>, usize, f64, u32)>) {
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
    fn apply_round_robin(&self, candidates: &mut [(&Arc<TrunkConfig>, usize, f64, u32)]) {
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
    fn apply_weighted_random(&self, candidates: &mut [(&Arc<TrunkConfig>, usize, f64, u32)]) {
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
    fn apply_least_connections(&self, candidates: &mut Vec<(&Arc<TrunkConfig>, usize, f64, u32)>) {
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
    fn select_from_pool<'a>(&self, pool: &[&'a Arc<TrunkConfig>]) -> Option<&'a Arc<TrunkConfig>> {
        if pool.is_empty() {
            return None;
        }

        // Find the best (lowest) priority
        let best_priority = pool.iter().map(|t| t.priority).min().unwrap();

        // Filter to only best-priority trunks
        let best: Vec<&&Arc<TrunkConfig>> = pool
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
        trunk: &Arc<TrunkConfig>,
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

        // Transform stage: apply the trunk's egress number transform so the
        // carrier receives the dialed number in the format it expects.
        let user_part = match trunk.egress_number_transform {
            Some(ref transform) if !transform.is_noop() => transform.apply(user_part),
            _ => user_part.to_string(),
        };
        let user_part = user_part.as_str();

        // Where the INVITE goes: the trunk's outbound proxy when it has
        // one, else its own hosts (`docs/TENANT_TRUNKS.md` §6). A proxy is
        // given as a host, `host:port`, or a whole SIP URI; the dialled
        // number stays the user part, as the proxy routes by it.
        let (host, port) = match trunk.outbound_proxy.as_deref().map(str::trim) {
            Some(proxy) if !proxy.is_empty() => proxy_host_port(proxy, trunk),
            _ => (
                trunk.hosts.first()?.clone(),
                trunk
                    .transport
                    .port
                    .unwrap_or_else(|| trunk.transport.protocol.default_port()),
            ),
        };
        let transport_param = match trunk.transport.protocol {
            fcp_core::transport::TransportType::Udp => String::new(),
            other => format!(";transport={}", other.as_str()),
        };

        Some(OutboundRoute {
            target_uri: format!("sip:{}@{}:{}{}", user_part, host, port, transport_param),
            trunk: Some(Arc::clone(trunk)),
        })
    }
}

/// The host and port an outbound proxy names. A proxy may be written as a
/// host, `host:port` or a whole SIP URI; what it does not say, the trunk's
/// own transport does.
fn proxy_host_port(proxy: &str, trunk: &TrunkConfig) -> (String, u16) {
    let proxy = proxy
        .strip_prefix("sip:")
        .or_else(|| proxy.strip_prefix("sips:"))
        .unwrap_or(proxy);
    let proxy = proxy.rsplit('@').next().unwrap_or(proxy);
    let proxy = proxy.split(';').next().unwrap_or(proxy).trim();
    let default_port = trunk
        .transport
        .port
        .unwrap_or_else(|| trunk.transport.protocol.default_port());
    // An IPv6 literal keeps its brackets; its port is what follows them.
    if let Some(rest) = proxy.strip_prefix('[') {
        return match rest.split_once("]:") {
            Some((host, port)) => (format!("[{host}]"), port.parse().unwrap_or(default_port)),
            None => (proxy.to_string(), default_port),
        };
    }
    match proxy.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            (host.to_string(), port.parse().unwrap_or(default_port))
        }
        _ => (proxy.to_string(), default_port),
    }
}

impl TrunkRouter {
    /// Record call accounting for the trunk a call is actually placed on.
    ///
    /// Paired with the Terminated-event decrement in the SBC hooks, which is
    /// keyed by the call's recorded trunk name.
    fn record_call_on(&self, trunk_key: &str) {
        if self.counts_calls {
            self.health.call_started(trunk_key);
        }
        self.cps.record_trunk_call(trunk_key);
    }

    /// Whether any trunk is *configured* to carry this destination upstream,
    /// regardless of current health/capacity. Used to distinguish "no route
    /// exists" (404) from "routes exist but none is available right now"
    /// (503) — the reachability stage.
    pub fn has_configured_route(
        &self,
        tenant: Option<fcp_core::TenantId>,
        direction: TrunkDirection,
        destination: &str,
    ) -> bool {
        let dir_matches = |t: &Arc<TrunkConfig>| {
            t.direction == direction
                || t.direction == TrunkDirection::Bidirectional
                || direction == TrunkDirection::Bidirectional
        };
        // A part whose table has routes for this destination is configured
        // for it only through the trunks those routes name.
        let configured = |pool: &[Arc<TrunkConfig>],
                          table: Option<&LcrRouteTable>,
                          key: &dyn Fn(&TrunkConfig) -> String| {
            if let Some(table) = table {
                let matched = table.find_routes_ordered(destination);
                if !matched.is_empty() {
                    return matched.iter().any(|route| {
                        pool.iter()
                            .any(|t| key(t) == route.trunk_name && dir_matches(t))
                    });
                }
            }
            pool.iter().any(dir_matches)
        };
        let (own, platform) = self.split_for(tenant);
        let own_table = tenant.and_then(|t| self.trunks.lcr_for(t));
        configured(&own, own_table.as_deref(), &|t| t.trunk_id().to_string())
            || configured(&platform, self.lcr.as_ref(), &|t| t.name.clone())
    }
}

#[async_trait::async_trait]
impl OutboundRouteProvider for TrunkRouter {
    async fn resolve_outbound(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
    ) -> Option<OutboundRoute> {
        // Use first result from ordered selection if LCR is enabled
        let trunks = self.select_trunks_ordered(tenant, TrunkDirection::Upstream, destination_uri);
        if let Some(trunk) = trunks.first() {
            let route = self.build_outbound_route(trunk, destination_uri)?;
            self.record_call_on(&trunk.key());
            return Some(route);
        }

        // Fall through to legacy (should not normally reach here, but safety net)
        let trunk = self.select_trunk(tenant, TrunkDirection::Upstream)?;
        let route = self.build_outbound_route(&trunk, destination_uri)?;
        self.record_call_on(&trunk.key());
        Some(route)
    }

    async fn resolve_outbound_ordered(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
    ) -> Vec<OutboundRoute> {
        let trunks = self.select_trunks_ordered(tenant, TrunkDirection::Upstream, destination_uri);
        let routes: Vec<OutboundRoute> = trunks
            .iter()
            .filter_map(|trunk| self.build_outbound_route(trunk, destination_uri))
            .collect();

        // Account only the primary route. Previously every failover candidate
        // was recorded as a started call, but the Terminated event decrements
        // only the call's recorded trunk — unused candidates leaked
        // concurrency until they appeared permanently at max capacity. The
        // B2BUA records the call under the primary route's trunk
        // (call.trunk_name()), so start/end stay balanced on that trunk.
        if let Some(first) = routes.first() {
            if let Some(id) = &first.trunk_id() {
                self.record_call_on(id.as_str());
            }
        }
        routes
    }

    async fn resolve_emergency_ordered(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
    ) -> Vec<OutboundRoute> {
        self.select_emergency_trunks_ordered(tenant, TrunkDirection::Upstream, destination_uri)
            .into_iter()
            .filter_map(|trunk| self.build_outbound_route(&trunk, destination_uri))
            .collect()
    }

    async fn resolve_outbound_classified(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
    ) -> RouteResolution {
        let routes = self.resolve_outbound_ordered(tenant, destination_uri).await;
        if !routes.is_empty() {
            return RouteResolution::Routes(routes);
        }
        if self.has_configured_route(tenant, TrunkDirection::Upstream, destination_uri) {
            RouteResolution::Unavailable
        } else {
            RouteResolution::NoRoute
        }
    }

    async fn resolve_targeted_classified(
        &self,
        tenant: Option<fcp_core::TenantId>,
        destination_uri: &str,
        target: &fcp_core::TrunkTarget,
    ) -> RouteResolution {
        let (trunks, known) =
            self.select_targeted_trunks(tenant, TrunkDirection::Upstream, destination_uri, target);
        let routes: Vec<OutboundRoute> = trunks
            .iter()
            .filter_map(|trunk| self.build_outbound_route(trunk, destination_uri))
            .collect();
        if let Some(first) = routes.first() {
            // Only the primary is counted, as for an ordinary call.
            if let Some(id) = &first.trunk_id() {
                self.record_call_on(id.as_str());
            }
            return RouteResolution::Routes(routes);
        }
        if known {
            RouteResolution::Unavailable
        } else {
            RouteResolution::NoRoute
        }
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

    use crate::trunk_set::TrunkSet;
    use std::collections::HashMap;

    /// A trunk belonging to `tenant`, as the store gives one.
    fn tenant_trunk(name: &str, tenant: fcp_core::TenantId) -> TrunkConfig {
        let mut trunk = fcp_core::Trunk::new(name.to_string(), tenant);
        trunk.enabled = true;
        trunk.direction = TrunkDirection::Upstream;
        trunk.hosts = vec!["sip.example.com".into()];
        trunk
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

        let selected = router.select_trunk(None, TrunkDirection::Upstream);
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

        let selected = router.select_trunk(None, TrunkDirection::Upstream);
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

        let selected = router.select_trunk(None, TrunkDirection::Downstream);
        assert_eq!(selected.unwrap().name, "downstream");
    }

    #[test]
    fn test_bidirectional_matches_all() {
        let trunks = vec![make_trunk("bidi", 100, 100, TrunkDirection::Bidirectional)];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("bidi");
        let router = TrunkRouter::new(trunks, health, no_cps());

        assert!(router
            .select_trunk(None, TrunkDirection::Upstream)
            .is_some());
        assert!(router
            .select_trunk(None, TrunkDirection::Downstream)
            .is_some());
    }

    #[test]
    fn test_disabled_trunk_excluded() {
        let mut trunk = make_trunk("a", 100, 100, TrunkDirection::Upstream);
        trunk.enabled = false;
        let trunks = vec![trunk];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        health.record_success("a");
        let router = TrunkRouter::new(trunks, health, no_cps());

        assert!(router
            .select_trunk(None, TrunkDirection::Upstream)
            .is_none());
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

        assert!(router
            .select_trunk(None, TrunkDirection::Upstream)
            .is_none());
    }

    #[test]
    fn test_no_trunks_returns_none() {
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        let router = TrunkRouter::new(Vec::new(), health, no_cps());
        assert!(router
            .select_trunk(None, TrunkDirection::Upstream)
            .is_none());
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
        assert!(router
            .select_trunk(None, TrunkDirection::Upstream)
            .is_none());
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
            let selected = router.select_trunk(None, TrunkDirection::Upstream);
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
        let selected = router.select_trunk(None, TrunkDirection::Upstream);
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
        let selected = router.select_trunk(None, TrunkDirection::Upstream);
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

        let selected = router.select_trunks_ordered(
            None,
            TrunkDirection::Upstream,
            "sip:+12125551234@example.com",
        );
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

        let selected = router.select_trunks_ordered(None, TrunkDirection::Upstream, "+15551234567");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "a");
    }

    #[test]
    fn test_select_trunks_ordered_no_match_falls_back() {
        let (trunks, health, quality) = lcr_setup();
        let router = TrunkRouter::with_lcr(trunks, health, no_cps(), lcr_config(), quality);

        // +86 has no LCR route — falls back to legacy selection
        let selected = router.select_trunks_ordered(None, TrunkDirection::Upstream, "+8612345678");
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
            router.select_trunks_ordered(None, TrunkDirection::Upstream, "sip:+15551234567@host");
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
            router.select_trunks_ordered(None, TrunkDirection::Upstream, "sip:+15551234567@host");
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
            router.select_trunks_ordered(None, TrunkDirection::Upstream, "sip:+12125551234@host");
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
            router.select_trunks_ordered(None, TrunkDirection::Upstream, "sip:+15551234567@host");
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
            .resolve_outbound_ordered(None, "sip:+12125551234@example.com")
            .await;
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].trunk_name(), Some("nyc-local".to_string()));
        assert_eq!(routes[1].trunk_name(), Some("us-cheap".to_string()));
        assert_eq!(routes[2].trunk_name(), Some("us-premium".to_string()));
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
            .resolve_outbound(None, "sip:+15551234567@example.com")
            .await;
        assert!(route.is_some());
        let route = route.unwrap();
        assert_eq!(route.trunk_name(), Some("codec-trunk".to_string()));
        assert_eq!(
            route.allowed_codecs(),
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
            .resolve_outbound(None, "sip:+15551234567@example.com")
            .await;
        assert!(route.is_some());
        let route = route.unwrap();
        assert_eq!(route.trunk_name(), Some("plain-trunk".to_string()));
        assert_eq!(route.allowed_codecs(), None);
    }

    fn lcr_router_two_trunks() -> (TrunkRouter, Arc<TrunkHealthMonitor>) {
        let trunks = vec![
            make_trunk("primary", 100, 100, TrunkDirection::Upstream),
            make_trunk("backup", 200, 100, TrunkDirection::Upstream),
        ];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        let lcr = LcrConfig {
            enabled: true,
            routes: vec![
                LcrRouteEntry {
                    prefix: "1".into(),
                    trunk_name: "primary".into(),
                    cost_per_minute: 0.01,
                    priority: 100,
                    enabled: true,
                },
                LcrRouteEntry {
                    prefix: "1".into(),
                    trunk_name: "backup".into(),
                    cost_per_minute: 0.02,
                    priority: 100,
                    enabled: true,
                },
            ],
            load_balance_strategy: LoadBalanceStrategy::LeastCost,
            min_asr: 0.0,
            max_failover_attempts: 2,
            ..Default::default()
        };
        let names = vec!["primary".to_string(), "backup".to_string()];
        let quality = Arc::new(TrunkQualityTracker::new(&names, 300));
        let router = TrunkRouter::with_lcr(trunks, Arc::clone(&health), no_cps(), lcr, quality);
        (router, health)
    }

    #[tokio::test]
    async fn test_ordered_resolve_accounts_only_primary_trunk() {
        let (router, health) = lcr_router_two_trunks();

        let routes = router
            .resolve_outbound_ordered(None, "sip:14155550100@sbc")
            .await;
        assert_eq!(routes.len(), 2, "both candidates returned for failover");

        // Regression: previously every candidate was recorded as a started
        // call, leaking concurrency on unused failover trunks.
        assert_eq!(health.concurrent_calls("primary"), 1);
        assert_eq!(health.concurrent_calls("backup"), 0);
    }

    #[tokio::test]
    async fn test_classified_resolution_reachability() {
        // No trunks at all → NoRoute (404).
        let router = TrunkRouter::new(vec![], Arc::new(TrunkHealthMonitor::new(&[])), no_cps());
        assert!(matches!(
            router
                .resolve_outbound_classified(None, "sip:14155550100@sbc")
                .await,
            RouteResolution::NoRoute
        ));

        // Routes configured but the only trunks are at capacity → Unavailable (503).
        let (router, health) = lcr_router_two_trunks();
        // make_trunk uses max_concurrent_calls = 0 (unlimited); saturate via
        // health "down" state instead: repeated failures mark trunks unhealthy.
        for _ in 0..10 {
            health.record_failure("primary");
            health.record_failure("backup");
        }
        assert!(matches!(
            router
                .resolve_outbound_classified(None, "sip:14155550100@sbc")
                .await,
            RouteResolution::Unavailable
        ));

        // Healthy again → Routes.
        health.record_success("primary");
        health.record_success("backup");
        assert!(matches!(
            router
                .resolve_outbound_classified(None, "sip:14155550100@sbc")
                .await,
            RouteResolution::Routes(_)
        ));
    }

    #[tokio::test]
    async fn an_outbound_proxy_is_where_the_invite_goes() {
        let mut trunk = make_trunk("proxied", 100, 100, TrunkDirection::Upstream);
        trunk.hosts = vec!["carrier.example".to_string()];
        trunk.outbound_proxy = Some("sbc.example:5080".to_string());
        let trunks = vec![trunk];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        let router = TrunkRouter::new(trunks, health, no_cps());

        let route = router
            .resolve_outbound(None, "sip:+15125550100@pbx")
            .await
            .expect("route");
        assert_eq!(
            route.target_uri, "sip:+15125550100@sbc.example:5080",
            "the dialled number stays the user part; the proxy routes by it"
        );
        assert_eq!(route.trunk_name().as_deref(), Some("proxied"));
    }

    #[tokio::test]
    async fn a_proxy_that_names_no_port_takes_the_trunks_own() {
        let mut trunk = make_trunk("proxied", 100, 100, TrunkDirection::Upstream);
        trunk.hosts = vec!["carrier.example".to_string()];
        trunk.transport.port = Some(5062);
        trunk.outbound_proxy = Some("sip:sbc.example".to_string());
        let trunks = vec![trunk];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        let router = TrunkRouter::new(trunks, health, no_cps());

        let route = router
            .resolve_outbound(None, "sip:+15125550100@pbx")
            .await
            .expect("route");
        assert_eq!(route.target_uri, "sip:+15125550100@sbc.example:5062");
    }

    #[tokio::test]
    async fn test_egress_number_transform_applied_to_route() {
        let mut trunk = make_trunk("carrier", 100, 100, TrunkDirection::Upstream);
        trunk.egress_number_transform = Some(crate::number_transform::NumberTransform {
            strip_prefixes: vec!["+".into()],
            prepend: String::new(),
            strip_leading: 0,
        });
        let trunks = vec![trunk];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        let router = TrunkRouter::new(trunks, health, no_cps());

        let route = router
            .resolve_outbound(None, "sip:+14155550100@sbc")
            .await
            .expect("route");
        assert!(
            route.target_uri.starts_with("sip:14155550100@"),
            "'+' must be stripped for this carrier, got {}",
            route.target_uri
        );

        // Named users pass through untransformed.
        let route = router
            .resolve_outbound(None, "sip:alice@sbc")
            .await
            .expect("route");
        assert!(route.target_uri.starts_with("sip:alice@"));
    }

    #[tokio::test]
    async fn emergency_trunks_come_first_then_normal_choice() {
        let mut backup = make_trunk("backup-911", 10, 1, TrunkDirection::Upstream);
        backup.emergency_capable = true;
        let mut primary = make_trunk("primary-911", 1, 1, TrunkDirection::Upstream);
        primary.emergency_capable = true;
        let mut disabled = make_trunk("dead-911", 0, 1, TrunkDirection::Upstream);
        disabled.emergency_capable = true;
        disabled.enabled = false;
        let mut inbound_only = make_trunk("inbound-911", 0, 1, TrunkDirection::Downstream);
        inbound_only.emergency_capable = true;
        let regular = make_trunk("regular", 0, 1, TrunkDirection::Upstream);
        let trunks = vec![regular.clone(), backup, disabled, inbound_only, primary];
        let health = Arc::new(TrunkHealthMonitor::new(&trunks));
        let router = TrunkRouter::new(trunks, health, no_cps());

        let names: Vec<String> = router
            .select_emergency_trunks_ordered(None, TrunkDirection::Upstream, "sip:911@pbx")
            .into_iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(names[0], "primary-911", "{names:?}");
        assert_eq!(names[1], "backup-911", "{names:?}");
        assert!(
            !names.iter().any(|n| n == "dead-911"),
            "disabled trunk excluded: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "inbound-911"),
            "wrong direction excluded: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "regular"),
            "normal choice appended: {names:?}"
        );
        assert_eq!(names.len(), 3, "{names:?}");

        // Through the provider trait, the routes keep that order.
        let routes = router.resolve_emergency_ordered(None, "sip:911@pbx").await;
        assert_eq!(routes[0].trunk_name().as_deref(), Some("primary-911"));

        // With nothing flagged, emergency routing degrades to the normal choice.
        let plain = vec![regular];
        let health = Arc::new(TrunkHealthMonitor::new(&plain));
        let router = TrunkRouter::new(plain, health, no_cps());
        let routes = router.resolve_emergency_ordered(None, "sip:911@pbx").await;
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].trunk_name().as_deref(), Some("regular"));
    }

    /// A tenant's own emergency trunk carries its 911 call, ahead of the
    /// platform's (`docs/E911_BLOCK_2.md` §12.1): the carrier that tenant
    /// bought is the one its emergency calls leave on.
    #[test]
    fn a_tenants_own_emergency_trunk_comes_first() {
        let tenant = fcp_core::TenantId::new();
        let mut platform = make_trunk("platform-911", 1, 1, TrunkDirection::Upstream);
        platform.emergency_capable = true;
        let mut theirs = tenant_trunk("tenants-911", tenant);
        theirs.emergency_capable = true;
        // A worse priority than the platform's, to prove whose trunk it is
        // that decides and not the number beside it.
        theirs.priority = 50;

        let set = Arc::new(TrunkSet::platform_only(vec![platform]));
        set.set_tenant_trunks(vec![theirs], &HashMap::new());
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        health.track(&set.all());
        let router = TrunkRouter::with_trunk_set(Arc::clone(&set), health, no_cps());

        let names: Vec<String> = router
            .select_emergency_trunks_ordered(Some(tenant), TrunkDirection::Upstream, "sip:911@pbx")
            .into_iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(names[0], "tenants-911", "{names:?}");
        assert_eq!(names[1], "platform-911", "{names:?}");
    }

    /// A tenant with no emergency trunk of its own reaches the platform's,
    /// even having refused the platform's trunks for its ordinary calls: a
    /// billing preference is not a reason to fail a 911 call.
    #[test]
    fn the_platforms_emergency_trunks_are_the_last_resort_for_anyone() {
        let tenant = fcp_core::TenantId::new();
        let mut platform = make_trunk("platform-911", 1, 1, TrunkDirection::Upstream);
        platform.emergency_capable = true;
        // The tenant's own carrier, which is not emergency-capable.
        let theirs = tenant_trunk("tenants-ordinary", tenant);

        let set = Arc::new(TrunkSet::platform_only(vec![platform]));
        let mut fallback = HashMap::new();
        fallback.insert(tenant, false);
        set.set_tenant_trunks(vec![theirs], &fallback);
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        health.track(&set.all());
        let router = TrunkRouter::with_trunk_set(Arc::clone(&set), health, no_cps());

        let names: Vec<String> = router
            .select_emergency_trunks_ordered(Some(tenant), TrunkDirection::Upstream, "sip:911@pbx")
            .into_iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(
            names.first().map(String::as_str),
            Some("platform-911"),
            "the platform's trunk carries it: {names:?}"
        );

        // And an ordinary call still respects what the tenant asked for.
        let ordinary: Vec<String> = router
            .select_trunks_ordered(
                Some(tenant),
                TrunkDirection::Upstream,
                "sip:+15125550100@pbx",
            )
            .into_iter()
            .map(|t| t.name.clone())
            .collect();
        assert!(
            !ordinary.iter().any(|n| n == "platform-911"),
            "the tenant refused the platform's trunks for these: {ordinary:?}"
        );
    }

    /// Another tenant's emergency trunk is not a candidate, however well
    /// flagged: a trunk carries its own tenant's calls.
    #[test]
    fn another_tenants_emergency_trunk_is_not_a_candidate() {
        let mine = fcp_core::TenantId::new();
        let theirs = fcp_core::TenantId::new();
        let mut ours = tenant_trunk("ours-911", mine);
        ours.emergency_capable = true;
        let mut not_ours = tenant_trunk("theirs-911", theirs);
        not_ours.emergency_capable = true;

        let set = Arc::new(TrunkSet::platform_only(Vec::new()));
        set.set_tenant_trunks(vec![ours, not_ours], &HashMap::new());
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        health.track(&set.all());
        let router = TrunkRouter::with_trunk_set(Arc::clone(&set), health, no_cps());

        let names: Vec<String> = router
            .select_emergency_trunks_ordered(Some(mine), TrunkDirection::Upstream, "sip:911@pbx")
            .into_iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(names, vec!["ours-911".to_string()], "{names:?}");
    }

    /// A router over the platform's `platform` and `tenant`'s `theirs`.
    fn router_over(
        platform: Vec<TrunkConfig>,
        theirs: Vec<TrunkConfig>,
        fallback: &HashMap<fcp_core::TenantId, bool>,
    ) -> TrunkRouter {
        let set = Arc::new(TrunkSet::platform_only(platform));
        set.set_tenant_trunks(theirs, fallback);
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        health.track(&set.all());
        TrunkRouter::with_trunk_set(set, health, no_cps())
    }

    fn names(trunks: &[Arc<TrunkConfig>]) -> Vec<String> {
        trunks.iter().map(|t| t.name.clone()).collect()
    }

    /// A rule that names one trunk gets that trunk and no other, even one
    /// the ordinary choice would rank above it (`docs/TENANT_TRUNKS.md`
    /// §12, phase 6b).
    #[test]
    fn a_rule_naming_a_trunk_gets_only_that_trunk() {
        let tenant = fcp_core::TenantId::new();
        let mut preferred = tenant_trunk("preferred", tenant);
        preferred.priority = 1;
        let mut named = tenant_trunk("named", tenant);
        named.priority = 90;
        let target = fcp_core::TrunkTarget::Trunk(named.trunk_id());
        let router = router_over(
            vec![make_trunk("platform", 1, 1, TrunkDirection::Upstream)],
            vec![preferred, named],
            &HashMap::new(),
        );

        let (trunks, known) = router.select_targeted_trunks(
            Some(tenant),
            TrunkDirection::Upstream,
            "sip:+442079460000@pbx",
            &target,
        );
        assert!(known);
        assert_eq!(names(&trunks), vec!["named"]);
    }

    /// A group is its members: the tenant's own by priority, then the
    /// platform's in the same group unless the tenant refused them.
    #[test]
    fn a_rule_naming_a_group_gets_its_members_the_tenants_first() {
        let tenant = fcp_core::TenantId::new();
        let in_group = |mut t: TrunkConfig, priority: u32| {
            t.trunk_group = Some("intl".into());
            t.priority = priority;
            t
        };
        let theirs = vec![
            in_group(tenant_trunk("intl-backup", tenant), 20),
            in_group(tenant_trunk("intl-primary", tenant), 10),
            tenant_trunk("domestic", tenant),
        ];
        let platform = vec![
            in_group(
                make_trunk("platform-intl", 1, 1, TrunkDirection::Upstream),
                1,
            ),
            make_trunk("platform-domestic", 1, 1, TrunkDirection::Upstream),
        ];
        let target = fcp_core::TrunkTarget::Group("intl".into());

        let router = router_over(platform.clone(), theirs.clone(), &HashMap::new());
        let (trunks, _) = router.select_targeted_trunks(
            Some(tenant),
            TrunkDirection::Upstream,
            "sip:+442079460000@pbx",
            &target,
        );
        assert_eq!(
            names(&trunks),
            vec!["intl-primary", "intl-backup", "platform-intl"]
        );

        let mut refused = HashMap::new();
        refused.insert(tenant, false);
        let router = router_over(platform, theirs, &refused);
        let (trunks, _) = router.select_targeted_trunks(
            Some(tenant),
            TrunkDirection::Upstream,
            "sip:+442079460000@pbx",
            &target,
        );
        assert_eq!(names(&trunks), vec!["intl-primary", "intl-backup"]);
    }

    /// When what the rule names cannot take the call, nothing else is
    /// offered in its place: 503 when the target exists, 404 when it names
    /// nothing this node knows.
    #[tokio::test]
    async fn a_rules_target_that_cannot_take_the_call_is_not_replaced() {
        let tenant = fcp_core::TenantId::new();
        let mut down = tenant_trunk("down", tenant);
        down.trunk_group = Some("intl".into());
        down.enabled = false;
        let router = router_over(
            vec![make_trunk("platform", 1, 1, TrunkDirection::Upstream)],
            vec![down, tenant_trunk("other", tenant)],
            &HashMap::new(),
        );

        let unavailable = router
            .resolve_targeted_classified(
                Some(tenant),
                "sip:+442079460000@pbx",
                &fcp_core::TrunkTarget::Group("intl".into()),
            )
            .await;
        assert!(
            matches!(unavailable, RouteResolution::Unavailable),
            "{unavailable:?}"
        );

        let nothing = router
            .resolve_targeted_classified(
                Some(tenant),
                "sip:+442079460000@pbx",
                &fcp_core::TrunkTarget::Group("nobody-has-this".into()),
            )
            .await;
        assert!(matches!(nothing, RouteResolution::NoRoute), "{nothing:?}");
    }

    /// A trunk id is taken only among the trunks this tenant's calls may
    /// leave on: never another tenant's.
    #[test]
    fn a_rule_cannot_reach_another_tenants_trunk() {
        let (mine, theirs) = (fcp_core::TenantId::new(), fcp_core::TenantId::new());
        let not_mine = tenant_trunk("theirs", theirs);
        let target = fcp_core::TrunkTarget::Trunk(not_mine.trunk_id());
        let router = router_over(
            Vec::new(),
            vec![tenant_trunk("mine", mine), not_mine],
            &HashMap::new(),
        );
        let (trunks, known) = router.select_targeted_trunks(
            Some(mine),
            TrunkDirection::Upstream,
            "sip:+442079460000@pbx",
            &target,
        );
        assert!(trunks.is_empty() && !known);
    }

    /// A tenant's least-cost route to `trunk`.
    fn lcr_route(
        tenant: fcp_core::TenantId,
        prefix: &str,
        trunk: &TrunkConfig,
        cost: &str,
    ) -> fcp_core::LcrRoute {
        use std::str::FromStr;
        fcp_core::LcrRoute::new(
            tenant,
            prefix,
            trunk.trunk_id(),
            rust_decimal::Decimal::from_str(cost).unwrap(),
        )
    }

    /// The tenant's own trunks come before the platform's, whatever the
    /// numbers beside them say: a platform trunk's better priority does not
    /// outrank the carrier the tenant bought (`docs/TENANT_TRUNKS.md` §5).
    /// The platform's follow as failover.
    #[test]
    fn a_tenants_own_trunk_comes_before_a_better_ranked_platform_trunk() {
        let tenant = fcp_core::TenantId::new();
        let mut theirs = tenant_trunk("theirs", tenant);
        theirs.priority = 50;
        let router = router_over(
            vec![make_trunk("platform", 1, 100, TrunkDirection::Upstream)],
            vec![theirs],
            &HashMap::new(),
        );
        for _ in 0..20 {
            let ordered = router.select_trunks_ordered(
                Some(tenant),
                TrunkDirection::Upstream,
                "sip:+15125550100@pbx",
            );
            assert_eq!(names(&ordered), vec!["theirs", "platform"]);
            assert_eq!(
                router
                    .select_trunk(Some(tenant), TrunkDirection::Upstream)
                    .map(|t| t.name.clone())
                    .as_deref(),
                Some("theirs")
            );
        }
    }

    /// A tenant's least-cost routes order its own trunks for the numbers
    /// they match — longest prefix, then cost — and a number they do not
    /// match takes the ordinary choice (`docs/TENANT_TRUNKS.md` §12, 6c).
    #[test]
    fn a_tenants_least_cost_routes_order_its_own_trunks() {
        let tenant = fcp_core::TenantId::new();
        let mut everyday = tenant_trunk("everyday", tenant);
        everyday.priority = 1;
        let mut cheap_uk = tenant_trunk("cheap-uk", tenant);
        cheap_uk.priority = 90;
        let mut london = tenant_trunk("london", tenant);
        london.priority = 90;
        let routes = vec![
            lcr_route(tenant, "+44", &everyday, "0.05"),
            lcr_route(tenant, "+44", &cheap_uk, "0.01"),
            lcr_route(tenant, "+4420", &london, "0.02"),
        ];
        let set = Arc::new(TrunkSet::platform_only(vec![make_trunk(
            "platform",
            1,
            1,
            TrunkDirection::Upstream,
        )]));
        set.set_tenant_trunks(vec![everyday, cheap_uk, london], &HashMap::new());
        assert_eq!(set.set_tenant_lcr(routes), 3);
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        health.track(&set.all());
        let router = TrunkRouter::with_trunk_set(set, health, no_cps());
        let ordered = |dest: &str| {
            names(&router.select_trunks_ordered(Some(tenant), TrunkDirection::Upstream, dest))
        };

        // London: the longest prefix first, then the rest of +44 by cost,
        // then the platform's.
        assert_eq!(
            ordered("sip:+442079460000@pbx"),
            vec!["london", "cheap-uk", "everyday", "platform"]
        );
        // Elsewhere in the UK: by cost.
        assert_eq!(
            ordered("sip:+441614960000@pbx"),
            vec!["cheap-uk", "everyday", "platform"]
        );
        // No route matches: the ordinary choice, then the platform's.
        assert_eq!(
            ordered("sip:+15125550100@pbx"),
            vec!["everyday", "platform"]
        );
    }

    /// A route in force names a trunk of its own tenant's: one naming
    /// another tenant's trunk, or switched off, is not.
    #[test]
    fn a_route_to_another_tenants_trunk_is_not_in_force() {
        let (mine, theirs) = (fcp_core::TenantId::new(), fcp_core::TenantId::new());
        let my_trunk = tenant_trunk("mine", mine);
        let their_trunk = tenant_trunk("theirs", theirs);
        let mut off = lcr_route(mine, "+1", &my_trunk, "0.01");
        off.enabled = false;
        let set = TrunkSet::platform_only(Vec::new());
        set.set_tenant_trunks(vec![my_trunk.clone(), their_trunk.clone()], &HashMap::new());
        let in_force = set.set_tenant_lcr(vec![
            lcr_route(mine, "+44", &their_trunk, "0.01"),
            off,
            lcr_route(mine, "+33", &my_trunk, "0.02"),
        ]);
        assert_eq!(in_force, 1);
        assert!(set.lcr_for(mine).is_some());
        assert!(set.lcr_for(theirs).is_none());
    }

    /// The node's `[lcr]` prices the platform's trunks, by name, and no
    /// tenant's trunk that happens to share one.
    #[test]
    fn the_nodes_table_does_not_price_a_tenant_trunk_of_the_same_name() {
        let tenant = fcp_core::TenantId::new();
        let mut namesake = tenant_trunk("uk-carrier", tenant);
        namesake.priority = 90;
        let mut ordinary = tenant_trunk("ordinary", tenant);
        ordinary.priority = 1;
        let set = Arc::new(TrunkSet::platform_only(vec![make_trunk(
            "uk-carrier",
            1,
            1,
            TrunkDirection::Upstream,
        )]));
        set.set_tenant_trunks(vec![namesake, ordinary], &HashMap::new());
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        health.track(&set.all());
        let router = TrunkRouter::with_trunk_set_and_lcr(
            set,
            health,
            no_cps(),
            lcr_config(),
            Arc::new(TrunkQualityTracker::new(&[], 3600)),
        );

        let ordered = router.select_trunks_ordered(
            Some(tenant),
            TrunkDirection::Upstream,
            "sip:+442079460000@pbx",
        );
        // The tenant's ordinary choice, then the platform's uk-carrier by
        // the node's table.
        assert_eq!(names(&ordered), vec!["ordinary", "uk-carrier"]);
        assert!(ordered[1].tenant_id.is_none(), "{ordered:?}");
    }

    /// Within what a rule names, the least-cost routes decide the order,
    /// and the group's other members stay behind them as failover.
    #[test]
    fn least_cost_routes_order_what_a_rule_left() {
        let tenant = fcp_core::TenantId::new();
        let in_group = |mut t: TrunkConfig, priority: u32| {
            t.trunk_group = Some("intl".into());
            t.priority = priority;
            t
        };
        let first_by_priority = in_group(tenant_trunk("first-by-priority", tenant), 1);
        let cheapest = in_group(tenant_trunk("cheapest", tenant), 50);
        let routes = vec![lcr_route(tenant, "+44", &cheapest, "0.001")];
        let set = Arc::new(TrunkSet::platform_only(Vec::new()));
        set.set_tenant_trunks(vec![first_by_priority, cheapest], &HashMap::new());
        set.set_tenant_lcr(routes);
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        health.track(&set.all());
        let router = TrunkRouter::with_trunk_set(set, health, no_cps());

        let (ordered, _) = router.select_targeted_trunks(
            Some(tenant),
            TrunkDirection::Upstream,
            "sip:+442079460000@pbx",
            &fcp_core::TrunkTarget::Group("intl".into()),
        );
        assert_eq!(names(&ordered), vec!["cheapest", "first-by-priority"]);
    }
}
