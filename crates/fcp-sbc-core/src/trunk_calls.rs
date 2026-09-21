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
use fcp_core::trunk_report::TrunkOutcome;
use fcp_core::Trunk;
use fcp_sip::trunk_ingress::{TrunkCallCounter, TrunkLimit};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, warn};

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

/// How many ended calls' trunk leases may wait for the settling tick
/// before the oldest are dropped and left to lapse.
const MAX_PENDING_TRUNK_RELEASES: usize = 10_000;

/// What each trunk carries on this node, and what each call was counted
/// against.
pub struct TrunkCalls {
    health: Arc<TrunkHealthMonitor>,
    cps: Arc<TrunkCps>,
    /// Call → the trunks it was counted against.
    ledger: DashMap<String, Vec<String>>,
    /// The cluster's count of each trunk's channels, when this node is in
    /// a cluster (`docs/CAPACITY_AND_SURVIVABILITY.md` §7). A carrier
    /// sells channels to the platform, not to a node of it, so without
    /// this a two-node cluster can offer a trunk twice its channels —
    /// which tenant trunks 4b counted per node and said so.
    cluster: std::sync::RwLock<Option<Arc<dyn ClusterTrunkChannels>>>,
    /// Whether the cluster's count has been unreadable since it last
    /// worked, so an outage is said once rather than per call.
    cluster_failing: std::sync::atomic::AtomicBool,
    /// Leases to give back, for the settling tick: a call ends in a
    /// callback that cannot await.
    released: std::sync::Mutex<Vec<(String, String)>>,
}

/// The cluster's count of one trunk's calls (§7). The same shape the
/// tenant's contract uses, over `fcp_cluster::CallLeases`.
#[async_trait::async_trait]
pub trait ClusterTrunkChannels: Send + Sync {
    /// Take a lease for this call on this trunk, and say how many the
    /// trunk carries across the cluster, this one included.
    async fn take(&self, trunk_key: &str, call: &str) -> anyhow::Result<u64>;

    /// Give a lease back.
    async fn release(&self, trunk_key: &str, call: &str) -> anyhow::Result<()>;

    /// Put back what this node still carries, with a fresh expiry.
    async fn refresh(&self, held: &[(String, String)]) -> anyhow::Result<()>;
}

/// The cluster's count of a trunk's channels, in Redis.
pub struct RedisTrunkChannels {
    leases: fcp_cluster::CallLeases,
}

impl RedisTrunkChannels {
    pub fn new(leases: fcp_cluster::CallLeases) -> Self {
        Self { leases }
    }
}

#[async_trait::async_trait]
impl ClusterTrunkChannels for RedisTrunkChannels {
    async fn take(&self, trunk_key: &str, call: &str) -> anyhow::Result<u64> {
        self.leases
            .take(fcp_cluster::LeaseSubject::Trunk(trunk_key), call)
            .await
    }

    async fn release(&self, trunk_key: &str, call: &str) -> anyhow::Result<()> {
        self.leases
            .release(fcp_cluster::LeaseSubject::Trunk(trunk_key), call)
            .await
    }

    async fn refresh(&self, held: &[(String, String)]) -> anyhow::Result<()> {
        let mut by_trunk: HashMap<&str, Vec<String>> = HashMap::new();
        for (trunk_key, call) in held {
            by_trunk
                .entry(trunk_key.as_str())
                .or_default()
                .push(call.clone());
        }
        for (trunk_key, calls) in by_trunk {
            self.leases
                .refresh(fcp_cluster::LeaseSubject::Trunk(trunk_key), &calls)
                .await?;
        }
        Ok(())
    }
}

impl TrunkCalls {
    pub fn new(health: Arc<TrunkHealthMonitor>, cps: Arc<TrunkCps>) -> Self {
        Self {
            health,
            cps,
            ledger: DashMap::new(),
            cluster: std::sync::RwLock::new(None),
            cluster_failing: std::sync::atomic::AtomicBool::new(false),
            released: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Count each trunk's channels across the cluster rather than on this
    /// node alone (§7).
    pub fn with_cluster(self, cluster: Arc<dyn ClusterTrunkChannels>) -> Self {
        *self.cluster.write().unwrap() = Some(cluster);
        self
    }

    /// Give back the leases of calls that have ended and push out the
    /// expiry of the ones still up: the tick a node in a cluster runs.
    pub async fn settle_leases(&self) {
        let Some(cluster) = self.cluster.read().ok().and_then(|c| c.clone()) else {
            return;
        };
        let releasing = std::mem::take(&mut *self.released.lock().unwrap());
        for (trunk_key, call) in releasing {
            if let Err(e) = cluster.release(&trunk_key, &call).await {
                debug!(error = %e, "a trunk lease could not be given back; it will lapse");
            }
        }
        let held: Vec<(String, String)> = self
            .ledger
            .iter()
            .flat_map(|e| {
                let call = e.key().clone();
                e.value()
                    .iter()
                    .map(move |key| (key.clone(), call.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        if !held.is_empty() {
            if let Err(e) = cluster.refresh(&held).await {
                debug!(error = %e, "this node's trunk leases were not refreshed");
            }
        }
    }

    /// Take a place for this call on `key` across the cluster. `true`
    /// when the trunk is already at its channels there. A cluster that
    /// cannot be read leaves the decision to this node's own count.
    async fn cluster_full(&self, key: &str, call: &str, channels: u32) -> bool {
        if channels == 0 {
            return false;
        }
        let Some(cluster) = self.cluster.read().ok().and_then(|c| c.clone()) else {
            return false;
        };
        match cluster.take(key, call).await {
            Ok(held) => {
                self.cluster_failing
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                if held > channels as u64 {
                    if let Err(e) = cluster.release(key, call).await {
                        warn!(error = %e, "could not give back a refused call's trunk lease");
                    }
                    return true;
                }
                false
            }
            Err(e) => {
                if !self
                    .cluster_failing
                    .swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    warn!(error = %e, "the cluster's trunk count is unreadable; \
                          counting this trunk's calls on this node alone");
                }
                false
            }
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

#[async_trait::async_trait]
impl TrunkCallCounter for TrunkCalls {
    async fn admit(&self, call: &str, trunk: &Trunk) -> Result<(), TrunkLimit> {
        // A trunk the health monitor has not met yet (a tenant's, created
        // since the last probe) is met now, so its calls are counted.
        self.health.track(&[Arc::new(trunk.clone())]);
        let key = trunk.key();
        note(trunk, &key, TrunkOutcome::Attempted);
        if !self
            .health
            .try_call_started(&key, trunk.max_concurrent_calls)
        {
            note(trunk, &key, TrunkOutcome::RejectedCapacity);
            return Err(TrunkLimit::Concurrency);
        }
        if !self.cps.try_record(&key, trunk.cps_limit, TrunkCps::now()) {
            self.health.call_ended(&key);
            note(trunk, &key, TrunkOutcome::RejectedCps);
            return Err(TrunkLimit::CallsPerSecond);
        }
        // The carrier's channels are the platform's to fill, not this
        // node's (§7): where there is a cluster, the count is its.
        if self
            .cluster_full(&key, call, trunk.max_concurrent_calls)
            .await
        {
            self.health.call_ended(&key);
            note(trunk, &key, TrunkOutcome::RejectedCapacity);
            return Err(TrunkLimit::Concurrency);
        }
        self.count(call, key);
        Ok(())
    }

    async fn carried(&self, call: &str, trunk: &Trunk) {
        self.health.track(&[Arc::new(trunk.clone())]);
        let key = trunk.key();
        // A call the trunk answered was a call it was offered: both, so
        // the two series can be read against each other (§9).
        note(trunk, &key, TrunkOutcome::Attempted);
        note(trunk, &key, TrunkOutcome::Answered);
        self.health.call_started(&key);
        // A leg the trunk answered fills a channel like any other, so it
        // takes a lease too — its selection already weighed the limits,
        // so the count is taken rather than tested.
        if let Some(cluster) = self.cluster.read().ok().and_then(|c| c.clone()) {
            if let Err(e) = cluster.take(&key, call).await {
                debug!(error = %e, "an answered leg's trunk lease was not taken");
            }
        }
        self.count(call, key);
    }

    fn ended(&self, call: &str) {
        // Give the cluster its channels back: now if there is a runtime
        // to do it on, and on the settling tick either way (§7).
        if let Some(cluster) = self.cluster.read().ok().and_then(|c| c.clone()) {
            if let Some(keys) = self.ledger.get(call).map(|e| e.value().clone()) {
                {
                    let mut released = self.released.lock().unwrap();
                    for key in &keys {
                        released.push((key.clone(), call.to_string()));
                    }
                    if released.len() > MAX_PENDING_TRUNK_RELEASES {
                        let overflow = released.len() - MAX_PENDING_TRUNK_RELEASES;
                        released.drain(..overflow);
                    }
                }
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let call = call.to_string();
                    handle.spawn(async move {
                        for key in keys {
                            if let Err(e) = cluster.release(&key, &call).await {
                                debug!(error = %e, "a trunk lease waits for the next tick");
                            }
                        }
                    });
                }
            }
        }
        if let Some((_, keys)) = self.ledger.remove(call) {
            for key in keys {
                self.health.call_ended(&key);
            }
        }
    }
}

/// One call's outcome on a trunk, for `/metrics` (§9).
fn note(trunk: &Trunk, key: &str, outcome: TrunkOutcome) {
    fcp_core::trunk_report::TrunkCounters::global().note(
        key,
        trunk.tenant_id.map(|t| t.to_string()).as_deref(),
        outcome,
    );
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

    #[tokio::test]
    async fn a_trunk_at_its_channels_refuses_the_next_call_until_one_ends() {
        let t = trunk(2, 0);
        let (calls, health) = calls_for(&t);
        assert_eq!(calls.admit("a", &t).await, Ok(()));
        // An outbound call it answered counts against the same channels.
        calls.carried("b", &t).await;
        assert_eq!(calls.admit("c", &t).await, Err(TrunkLimit::Concurrency));
        assert_eq!(health.concurrent_calls(&t.key()), 2);

        calls.ended("a");
        assert_eq!(health.concurrent_calls(&t.key()), 1);
        assert_eq!(calls.admit("c", &t).await, Ok(()));
        calls.ended("b");
        calls.ended("c");
        calls.ended("c"); // ending twice releases once
        assert_eq!(health.concurrent_calls(&t.key()), 0);
        assert_eq!(calls.calls_in_flight(), 0);
    }

    /// A carrier's channels across the cluster
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §7): two nodes offering the
    /// same trunk share its channels rather than each filling them.
    mod cluster {
        use super::*;
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Default)]
        struct Channels {
            held: std::sync::Mutex<HashMap<String, Vec<String>>>,
            broken: AtomicBool,
        }

        #[async_trait::async_trait]
        impl ClusterTrunkChannels for Channels {
            async fn take(&self, trunk_key: &str, call: &str) -> anyhow::Result<u64> {
                if self.broken.load(Ordering::SeqCst) {
                    anyhow::bail!("redis is away");
                }
                let mut held = self.held.lock().unwrap();
                let calls = held.entry(trunk_key.to_string()).or_default();
                if !calls.iter().any(|c| c == call) {
                    calls.push(call.to_string());
                }
                Ok(calls.len() as u64)
            }

            async fn release(&self, trunk_key: &str, call: &str) -> anyhow::Result<()> {
                if self.broken.load(Ordering::SeqCst) {
                    anyhow::bail!("redis is away");
                }
                if let Some(calls) = self.held.lock().unwrap().get_mut(trunk_key) {
                    calls.retain(|c| c != call);
                }
                Ok(())
            }

            async fn refresh(&self, _held: &[(String, String)]) -> anyhow::Result<()> {
                Ok(())
            }
        }

        /// A node offering `t`, counting its channels across `channels`.
        fn node(channels: &Arc<Channels>, t: &Trunk) -> TrunkCalls {
            let (calls, _health) = calls_for(t);
            calls.with_cluster(Arc::clone(channels) as Arc<dyn ClusterTrunkChannels>)
        }

        /// One trunk of two channels, two nodes: the second node cannot
        /// sell the carrier's channels twice.
        #[tokio::test]
        async fn channels_are_the_carriers_not_a_nodes() {
            let channels = Arc::new(Channels::default());
            let t = trunk(2, 0);
            let one = node(&channels, &t);
            let two = node(&channels, &t);

            assert_eq!(one.admit("a", &t).await, Ok(()));
            assert_eq!(two.admit("b", &t).await, Ok(()), "the other node");
            assert_eq!(
                two.admit("c", &t).await,
                Err(TrunkLimit::Concurrency),
                "the carrier sold two channels, not four"
            );
            assert_eq!(
                channels.held.lock().unwrap()[&t.key()].len(),
                2,
                "the refused call gave its channel back"
            );

            // The refused call left no channel held on its own node
            // either, so the node's count and the cluster's agree.
            one.ended("a");
            one.settle_leases().await;
            assert_eq!(two.admit("c", &t).await, Ok(()));
        }

        /// A leg the trunk answered fills a channel too.
        #[tokio::test]
        async fn an_answered_leg_takes_a_channel() {
            let channels = Arc::new(Channels::default());
            let t = trunk(2, 0);
            let one = node(&channels, &t);
            one.carried("outbound", &t).await;
            assert_eq!(channels.held.lock().unwrap()[&t.key()].len(), 1);
        }

        /// Redis being away must not refuse a call: the trunk is then
        /// counted per node, as tenant trunks 4b did.
        #[tokio::test]
        async fn an_unreadable_cluster_falls_back_to_this_node() {
            let channels = Arc::new(Channels::default());
            let t = trunk(2, 0);
            let one = node(&channels, &t);
            channels.broken.store(true, Ordering::SeqCst);

            assert_eq!(one.admit("a", &t).await, Ok(()));
            assert_eq!(one.admit("b", &t).await, Ok(()));
            assert_eq!(
                one.admit("c", &t).await,
                Err(TrunkLimit::Concurrency),
                "this node's own count still holds the line"
            );
        }
    }

    #[tokio::test]
    async fn a_trunk_past_its_calls_per_second_refuses_and_holds_no_channel() {
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
        let mut refused = 0;
        for i in 0..3 {
            if calls.admit(&format!("call-{i}"), &one_a_second).await
                == Err(TrunkLimit::CallsPerSecond)
            {
                refused += 1;
            }
        }
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
