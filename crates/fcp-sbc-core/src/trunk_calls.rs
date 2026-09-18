//! The calls each trunk carries on this node (`docs/TENANT_TRUNKS.md` §7).
//!
//! A trunk's `max_concurrent_calls` and `cps_limit` are its carrier's, and
//! a carrier sells channels, not directions: a call it brings in and a call
//! it takes out count against the same limits. The counts are this node's
//! (§13.3).
//!
//! A call is counted against a trunk when it arrives on it, or when the
//! trunk answers it outbound — the handler never learns when a failover
//! attempt that did not answer has gone, so counting at the INVITE would
//! hold that attempt's channel for the life of the call. Everything a call
//! was counted against is released when it ends, from a per-call ledger.

use crate::cps::CpsChecker;
use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk_set::TrunkSet;
use dashmap::DashMap;
use fcp_core::Trunk;
use fcp_sip::trunk_ingress::{TrunkCallCounter, TrunkLimit};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Calls per second per trunk, from each trunk's own `cps_limit`: the
/// calls started in the current second, in either direction.
pub struct TrunkCps {
    trunks: Arc<TrunkSet>,
    /// Trunk key → (second, calls started in it).
    windows: DashMap<String, (u64, u32)>,
}

impl TrunkCps {
    pub fn new(trunks: Arc<TrunkSet>) -> Self {
        Self {
            trunks,
            windows: DashMap::new(),
        }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn limit(&self, key: &str) -> u32 {
        self.trunks.find(key).map(|t| t.cps_limit).unwrap_or(0)
    }

    /// Record a call on `key` unless that would take it past `limit` this
    /// second (0: no limit). Check and record are one step.
    fn try_record(&self, key: &str, limit: u32, at: u64) -> bool {
        let mut window = self.windows.entry(key.to_string()).or_insert((at, 0));
        if window.0 != at {
            *window = (at, 0);
        }
        if limit != 0 && window.1 >= limit {
            return false;
        }
        window.1 += 1;
        true
    }
}

impl CpsChecker for TrunkCps {
    fn is_trunk_within_cps(&self, key: &str) -> bool {
        let limit = self.limit(key);
        if limit == 0 {
            return true;
        }
        let now = Self::now();
        self.windows
            .get(key)
            .is_none_or(|w| w.0 != now || w.1 < limit)
    }

    fn record_trunk_call(&self, key: &str) {
        self.try_record(key, 0, Self::now());
    }
}

/// What each trunk carries on this node, and what each call was counted
/// against.
pub struct TrunkCalls {
    health: Arc<TrunkHealthMonitor>,
    cps: Arc<TrunkCps>,
    /// Call → the trunks it was counted against.
    ledger: DashMap<String, Vec<String>>,
}

impl TrunkCalls {
    pub fn new(health: Arc<TrunkHealthMonitor>, cps: Arc<TrunkCps>) -> Self {
        Self {
            health,
            cps,
            ledger: DashMap::new(),
        }
    }

    fn count(&self, call: &str, key: String) {
        self.ledger.entry(call.to_string()).or_default().push(key);
    }

    /// How many calls this node has counted and not yet released.
    pub fn calls_in_flight(&self) -> usize {
        self.ledger.len()
    }
}

impl TrunkCallCounter for TrunkCalls {
    fn admit(&self, call: &str, trunk: &Trunk) -> Result<(), TrunkLimit> {
        // A trunk the health monitor has not met yet (a tenant's, created
        // since the last probe) is met now, so its calls are counted.
        self.health.track(&[Arc::new(trunk.clone())]);
        let key = trunk.key();
        if !self
            .health
            .try_call_started(&key, trunk.max_concurrent_calls)
        {
            return Err(TrunkLimit::Concurrency);
        }
        if !self.cps.try_record(&key, trunk.cps_limit, TrunkCps::now()) {
            self.health.call_ended(&key);
            return Err(TrunkLimit::CallsPerSecond);
        }
        self.count(call, key);
        Ok(())
    }

    fn carried(&self, call: &str, trunk: &Trunk) {
        self.health.track(&[Arc::new(trunk.clone())]);
        let key = trunk.key();
        self.health.call_started(&key);
        self.count(call, key);
    }

    fn ended(&self, call: &str) {
        if let Some((_, keys)) = self.ledger.remove(call) {
            for key in keys {
                self.health.call_ended(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::TenantId;

    fn trunk(max: u32, cps: u32) -> Trunk {
        let mut t = Trunk::new("carrier".to_string(), TenantId::new());
        t.hosts = vec!["10.0.0.9".to_string()];
        t.max_concurrent_calls = max;
        t.cps_limit = cps;
        t
    }

    fn calls_for(t: &Trunk) -> (TrunkCalls, Arc<TrunkHealthMonitor>) {
        let set = Arc::new(TrunkSet::platform_only(Vec::new()));
        set.set_tenant_trunks(vec![t.clone()], &Default::default());
        let health = Arc::new(TrunkHealthMonitor::new(&[]));
        let calls = TrunkCalls::new(Arc::clone(&health), Arc::new(TrunkCps::new(set)));
        (calls, health)
    }

    #[test]
    fn a_trunk_at_its_channels_refuses_the_next_call_until_one_ends() {
        let t = trunk(2, 0);
        let (calls, health) = calls_for(&t);
        assert_eq!(calls.admit("a", &t), Ok(()));
        // An outbound call it answered counts against the same channels.
        calls.carried("b", &t);
        assert_eq!(calls.admit("c", &t), Err(TrunkLimit::Concurrency));
        assert_eq!(health.concurrent_calls(&t.key()), 2);

        calls.ended("a");
        assert_eq!(health.concurrent_calls(&t.key()), 1);
        assert_eq!(calls.admit("c", &t), Ok(()));
        calls.ended("b");
        calls.ended("c");
        calls.ended("c"); // ending twice releases once
        assert_eq!(health.concurrent_calls(&t.key()), 0);
        assert_eq!(calls.calls_in_flight(), 0);
    }

    #[test]
    fn a_trunk_past_its_calls_per_second_refuses_and_holds_no_channel() {
        let t = trunk(0, 2);
        let (calls, health) = calls_for(&t);
        // Pinned to one second, so the test cannot straddle two.
        let at = TrunkCps::now();
        assert!(calls.cps.try_record(&t.key(), 2, at));
        assert!(calls.cps.try_record(&t.key(), 2, at));
        assert!(!calls.cps.try_record(&t.key(), 2, at));
        assert!(
            calls.cps.try_record(&t.key(), 2, at + 1),
            "a new second, a new count"
        );

        // Through admission, three calls at once on a trunk that takes one a
        // second: the refused ones are not left holding a channel.
        let one_a_second = trunk(0, 1);
        let refused = (0..3)
            .filter(|i| {
                calls.admit(&format!("call-{i}"), &one_a_second) == Err(TrunkLimit::CallsPerSecond)
            })
            .count();
        assert!(refused >= 1, "one call per second, three at once");
        assert_eq!(
            health.concurrent_calls(&one_a_second.key()) as usize,
            3 - refused
        );
    }

    #[test]
    fn the_router_sees_the_cps_its_trunks_are_at() {
        let t = trunk(0, 1);
        let set = Arc::new(TrunkSet::platform_only(Vec::new()));
        set.set_tenant_trunks(vec![t.clone()], &Default::default());
        let cps = TrunkCps::new(set);
        assert!(cps.is_trunk_within_cps(&t.key()));
        cps.record_trunk_call(&t.key());
        // Unless the second turned over between the two calls.
        let now = TrunkCps::now();
        if cps.windows.get(&t.key()).is_some_and(|w| w.0 == now) {
            assert!(!cps.is_trunk_within_cps(&t.key()));
        }
    }
}
