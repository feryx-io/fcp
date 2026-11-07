//! Call admission control: the limits a node keeps, and the controller
//! that keeps them.
//!
//! Maximum calls per second, concurrent calls and registrations, per-tenant
//! concurrency, per-trunk CPS and the REGISTER rate, with the overload
//! action taken when a limit is reached. One controller serves every
//! routing binary (`docs/CAPACITY_AND_SURVIVABILITY.md` §4): the SBC, which
//! has had it since it was written, and the call manager, which reaches it
//! through `fcp_sip::admission::NodeAdmission`.
//!
//! **Zero means unlimited** for every limit here, so a node that configures
//! nothing admits everything, as it did before there was a controller.

use dashmap::DashMap;
use fcp_core::TenantId;
use fcp_sip::admission::{NodeAdmission, Refusal};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tokio::sync::oneshot;
use tracing::{debug, warn};

/// How many ended calls' leases may wait for the settling tick before
/// the oldest are dropped and left to lapse on their own.
const MAX_PENDING_RELEASES: usize = 10_000;

/// Action to take when the SBC is overloaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdmissionAction {
    /// Reject new calls with 503 Service Unavailable.
    #[default]
    Reject,

    /// Queue new calls for later processing (future use).
    Queue,
}

/// Per-trunk CPS limit entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkCpsLimit {
    /// Name of the trunk this limit applies to.
    pub trunk_name: String,

    /// Maximum calls per second for this trunk.
    pub max_cps: u32,
}

/// Call admission control configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdmissionConfig {
    /// Maximum global calls per second.
    pub max_cps: u32,

    /// Maximum concurrent active calls.
    pub max_concurrent_calls: u32,

    /// Maximum concurrent registrations.
    pub max_concurrent_registrations: u32,

    /// Maximum REGISTERs per second from one source address (0: no
    /// limit). Every REGISTER counts, and a challenged registration is
    /// two of them, so `n` admits `n / 2` registrations a second from one
    /// address once `auth_enabled` is on.
    pub max_register_cps_per_source: u32,

    /// Maximum REGISTERs per second for one AOR (0: no limit).
    pub max_register_cps_per_aor: u32,

    /// The longest registration a draining node grants, in seconds
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6): a device comes back
    /// soon and is placed on a node that is serving. Zero keeps the
    /// expiry the device asked for.
    pub drain_register_expiry_secs: u32,

    /// Per-trunk CPS overrides.
    pub per_trunk_cps: Vec<TrunkCpsLimit>,

    /// Action to take when overloaded.
    pub overload_action: AdmissionAction,

    /// Include Retry-After header in 503 responses.
    pub include_retry_after: bool,

    /// Retry-After value in seconds.
    pub retry_after_secs: u64,

    /// Maximum number of calls to queue when `overload_action` is `Queue`.
    pub queue_max_size: u32,

    /// Timeout in seconds for queued calls before rejecting with 503.
    pub queue_timeout_secs: u64,
}

impl Default for AdmissionConfig {
    fn default() -> Self {
        Self {
            max_cps: 100,
            max_concurrent_calls: 1000,
            max_concurrent_registrations: 5000,
            max_register_cps_per_source: 0,
            max_register_cps_per_aor: 0,
            drain_register_expiry_secs: 60,
            per_trunk_cps: Vec::new(),
            overload_action: AdmissionAction::Reject,
            include_retry_after: true,
            retry_after_secs: 5,
            queue_max_size: 100,
            queue_timeout_secs: 10,
        }
    }
}

impl AdmissionConfig {
    /// Limits that cap nothing: what a node runs with until a deployment
    /// writes one down (`docs/CAPACITY_AND_SURVIVABILITY.md` §10). The
    /// queue's own bounds keep their defaults, since they are only reached
    /// when `overload_action` is `Queue`.
    pub fn unlimited() -> Self {
        Self {
            max_cps: 0,
            max_concurrent_calls: 0,
            max_concurrent_registrations: 0,
            max_register_cps_per_source: 0,
            max_register_cps_per_aor: 0,
            ..Self::default()
        }
    }

    /// Whether this node limits what it carries — calls, calls a second,
    /// registrations — and so has a capacity to publish. REGISTER rates
    /// bound floods, not load.
    pub fn caps_capacity(&self) -> bool {
        self.max_cps != 0
            || self.max_concurrent_calls != 0
            || self.max_concurrent_registrations != 0
    }

    /// Whether any limit here would ever refuse anything.
    pub fn caps_anything(&self) -> bool {
        self.max_cps != 0
            || self.max_concurrent_calls != 0
            || self.max_concurrent_registrations != 0
            || self.max_register_cps_per_source != 0
            || self.max_register_cps_per_aor != 0
            || !self.per_trunk_cps.is_empty()
    }

    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - `max_cps` exceeds 100,000
    /// - `max_concurrent_calls` exceeds 1,000,000
    /// - `retry_after_secs` is not in 1..=3600
    /// - Any trunk name is empty or contains CR/LF
    pub fn validate(&self) -> Result<(), String> {
        if self.max_cps > 100_000 {
            return Err("max_cps must not exceed 100,000".into());
        }
        if self.max_concurrent_calls > 1_000_000 {
            return Err("max_concurrent_calls must not exceed 1,000,000".into());
        }
        if self.max_register_cps_per_source > 100_000 {
            return Err("max_register_cps_per_source must not exceed 100,000".into());
        }
        if self.max_register_cps_per_aor > 100_000 {
            return Err("max_register_cps_per_aor must not exceed 100,000".into());
        }
        if self.retry_after_secs < 1 || self.retry_after_secs > 3600 {
            return Err("retry_after_secs must be between 1 and 3600".into());
        }
        if self.queue_max_size < 1 || self.queue_max_size > 10_000 {
            return Err("queue_max_size must be between 1 and 10,000".into());
        }
        if self.queue_timeout_secs < 1 || self.queue_timeout_secs > 60 {
            return Err("queue_timeout_secs must be between 1 and 60".into());
        }
        for entry in &self.per_trunk_cps {
            if entry.trunk_name.is_empty() {
                return Err("per_trunk_cps trunk_name must not be empty".into());
            }
            if entry.trunk_name.contains('\r') || entry.trunk_name.contains('\n') {
                return Err("per_trunk_cps trunk_name must not contain CR/LF characters".into());
            }
        }
        Ok(())
    }
}

/// The cluster's count of a tenant's calls
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §7).
///
/// A tenant's `max_concurrent_calls` is a contract with the platform, so
/// counting it on one node lets a two-node cluster carry twice it. A
/// node with this takes a lease as each call is admitted and gives it
/// back when the call ends; a node without one counts its own calls, as
/// phase 1 did.
#[async_trait::async_trait]
pub trait ClusterCallCount: Send + Sync {
    /// Take a lease for this call and say how many the tenant holds
    /// across the cluster, this one included.
    async fn take(&self, tenant: &TenantId, call_id: &str) -> anyhow::Result<u64>;

    /// Give a lease back.
    async fn release(&self, tenant: &TenantId, call_id: &str) -> anyhow::Result<()>;

    /// Put the calls this node still holds back, with a fresh expiry: a
    /// call longer than the lease keeps its place, and a lease lost to an
    /// outage comes back when Redis does.
    async fn refresh(&self, held: &[(TenantId, String)]) -> anyhow::Result<()>;
}

/// The cluster's count, in Redis (`fcp_cluster::CallLeases`).
pub struct RedisCallLeases {
    leases: fcp_cluster::CallLeases,
}

impl RedisCallLeases {
    pub fn new(leases: fcp_cluster::CallLeases) -> Self {
        Self { leases }
    }
}

#[async_trait::async_trait]
impl ClusterCallCount for RedisCallLeases {
    async fn take(&self, tenant: &TenantId, call_id: &str) -> anyhow::Result<u64> {
        let id = tenant.to_string();
        self.leases
            .take(fcp_cluster::LeaseSubject::Tenant(&id), call_id)
            .await
    }

    async fn release(&self, tenant: &TenantId, call_id: &str) -> anyhow::Result<()> {
        let id = tenant.to_string();
        self.leases
            .release(fcp_cluster::LeaseSubject::Tenant(&id), call_id)
            .await
    }

    async fn refresh(&self, held: &[(TenantId, String)]) -> anyhow::Result<()> {
        let mut by_tenant: HashMap<TenantId, Vec<String>> = HashMap::new();
        for (tenant, call_id) in held {
            by_tenant.entry(*tenant).or_default().push(call_id.clone());
        }
        for (tenant, call_ids) in by_tenant {
            let id = tenant.to_string();
            self.leases
                .refresh(fcp_cluster::LeaseSubject::Tenant(&id), &call_ids)
                .await?;
        }
        Ok(())
    }
}

/// What a tenant may carry at once, as of the last refresh of the
/// tenants (`fcp_sbc_core::TrunkSet` keeps them; see
/// `docs/CAPACITY_AND_SURVIVABILITY.md` §5).
pub trait TenantCallLimits: Send + Sync {
    /// The tenant's `max_concurrent_calls`, or `None` when it has none.
    fn max_concurrent_calls(&self, tenant: &TenantId) -> Option<u32>;
}

/// Calls admitted and refused on this node, by reason and tenant.
#[derive(Default)]
pub struct AdmissionCounters {
    admitted: Mutex<HashMap<String, u64>>,
    refused: Mutex<HashMap<(&'static str, String), u64>>,
}

impl AdmissionCounters {
    /// The counters of this process.
    pub fn global() -> &'static AdmissionCounters {
        static COUNTERS: std::sync::OnceLock<AdmissionCounters> = std::sync::OnceLock::new();
        COUNTERS.get_or_init(AdmissionCounters::default)
    }

    fn note_admitted(&self, tenant: &str) {
        if let Ok(mut map) = self.admitted.lock() {
            *map.entry(tenant.to_string()).or_insert(0) += 1;
        }
    }

    fn note_refused(&self, reason: Refusal, tenant: &str) {
        if let Ok(mut map) = self.refused.lock() {
            *map.entry((reason.as_str(), tenant.to_string()))
                .or_insert(0) += 1;
        }
    }

    /// How many calls were refused for `reason`, over every tenant.
    pub fn refused_for(&self, reason: Refusal) -> u64 {
        self.refused
            .lock()
            .map(|m| {
                m.iter()
                    .filter(|((r, _), _)| *r == reason.as_str())
                    .map(|(_, n)| *n)
                    .sum()
            })
            .unwrap_or(0)
    }

    /// The text exposition of both series, with `labels` on every sample.
    pub fn prometheus(&self, labels: &str) -> String {
        let sep = if labels.is_empty() { "" } else { "," };
        let mut out = String::new();
        if let Ok(map) = self.admitted.lock() {
            if !map.is_empty() {
                out.push_str(
                    "# HELP fcp_admission_calls_admitted_total Calls this node admitted.\n\
                     # TYPE fcp_admission_calls_admitted_total counter\n",
                );
                for (tenant, n) in map.iter() {
                    out.push_str(&format!(
                        "fcp_admission_calls_admitted_total{{{labels}{sep}tenant=\"{tenant}\"}} {n}\n"
                    ));
                }
            }
        }
        if let Ok(map) = self.refused.lock() {
            if !map.is_empty() {
                out.push_str(
                    "# HELP fcp_admission_refused_total Calls and registrations this node refused.\n\
                     # TYPE fcp_admission_refused_total counter\n",
                );
                for ((reason, tenant), n) in map.iter() {
                    out.push_str(&format!(
                        "fcp_admission_refused_total{{{labels}{sep}reason=\"{reason}\",\
                         tenant=\"{tenant}\"}} {n}\n"
                    ));
                }
            }
        }
        out
    }
}

/// A call waiting in the admission queue.
struct QueuedCall {
    sender: oneshot::Sender<bool>,
    queued_at: Instant,
}

/// Result of attempting to admit a call.
pub enum AdmitResult {
    /// Call admitted immediately.
    Admitted,
    /// Call rejected (overloaded and not queueable).
    Rejected,
    /// Call queued — await the receiver for admission result.
    /// Receives `true` if admitted when capacity frees up, `false` if expired.
    Queued(oneshot::Receiver<bool>),
}

/// Calls started in the current second, per key.
#[derive(Default)]
struct PerSecond {
    windows: DashMap<String, (u64, u32)>,
}

impl PerSecond {
    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Record one event on `key` unless that would take it past `limit`
    /// this second. Zero is no limit. Check and record are one step.
    fn try_record(&self, key: &str, limit: u32) -> bool {
        let at = Self::now();
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

    /// Forget keys that have not been seen this second or last, so a node
    /// that has met a million source addresses does not remember them.
    fn prune(&self) {
        let now = Self::now();
        self.windows
            .retain(|_, (at, _)| now.saturating_sub(*at) < 2);
    }
}

/// Runtime admission controller.
///
/// Thread-safe: atomics for counters, mutexes only for the sliding windows
/// and the queue. Config and trunk maps are behind `RwLock` for runtime
/// updates via the SBC's admin API.
pub struct AdmissionController {
    config: RwLock<AdmissionConfig>,
    concurrent_calls: AtomicU64,
    concurrent_registrations: AtomicU64,
    /// Sliding window of call-start timestamps for global CPS calculation.
    cps_window: Mutex<VecDeque<Instant>>,
    /// Per-trunk CPS limits (trunk_name -> max_cps).
    trunk_cps_limits: RwLock<HashMap<String, u32>>,
    /// Per-trunk sliding windows (trunk_name -> timestamps).
    trunk_cps_windows: RwLock<HashMap<String, Mutex<VecDeque<Instant>>>>,
    /// Queue of calls waiting for capacity when `overload_action` is `Queue`.
    call_queue: Mutex<VecDeque<QueuedCall>>,
    /// What each tenant may carry at once.
    tenant_limits: RwLock<Option<Arc<dyn TenantCallLimits>>>,
    /// Calls this node admitted and has not released, and whose they are.
    ledger: DashMap<String, TenantId>,
    /// Calls in flight per tenant.
    tenant_calls: DashMap<TenantId, u64>,
    /// REGISTERs per second, by source address and by AOR.
    register_rate: PerSecond,
    /// Whether this node is leaving service (§6). Set by a drain, cleared
    /// when one is called off.
    draining: AtomicBool,
    /// The cluster's count of each tenant's calls, when this node is in a
    /// cluster (§7). Without it the tenant limit is counted per node.
    cluster_calls: RwLock<Option<Arc<dyn ClusterCallCount>>>,
    /// Whether the cluster's count has been unreadable since the last
    /// time it worked, so a Redis outage is said once rather than per
    /// call.
    cluster_failing: AtomicBool,
    /// Leases to give back. A call ends in a callback that cannot await
    /// (the call manager's `set_on_ended`), so the release is handed to
    /// [`settle_leases`](AdmissionController::settle_leases) — and, when
    /// there is a runtime to spawn on, done at once as well. Releasing
    /// twice is harmless; never releasing is covered by the lease's own
    /// expiry.
    released: Mutex<Vec<(TenantId, String)>>,
}

impl AdmissionController {
    pub fn new(config: AdmissionConfig) -> Self {
        let mut trunk_cps_limits = HashMap::new();
        let mut trunk_cps_windows = HashMap::new();

        for entry in &config.per_trunk_cps {
            trunk_cps_limits.insert(entry.trunk_name.clone(), entry.max_cps);
            trunk_cps_windows.insert(entry.trunk_name.clone(), Mutex::new(VecDeque::new()));
        }

        Self {
            config: RwLock::new(config),
            concurrent_calls: AtomicU64::new(0),
            concurrent_registrations: AtomicU64::new(0),
            cps_window: Mutex::new(VecDeque::new()),
            trunk_cps_limits: RwLock::new(trunk_cps_limits),
            trunk_cps_windows: RwLock::new(trunk_cps_windows),
            call_queue: Mutex::new(VecDeque::new()),
            tenant_limits: RwLock::new(None),
            ledger: DashMap::new(),
            tenant_calls: DashMap::new(),
            register_rate: PerSecond::default(),
            draining: AtomicBool::new(false),
            cluster_calls: RwLock::new(None),
            cluster_failing: AtomicBool::new(false),
            released: Mutex::new(Vec::new()),
        }
    }

    /// Count each tenant's calls across the cluster rather than on this
    /// node alone (§7). Without it — no cluster, or no Redis — the count
    /// is this node's own, which is what phase 1 enforced.
    pub fn with_cluster_calls(self, cluster: Arc<dyn ClusterCallCount>) -> Self {
        *self.cluster_calls.write().unwrap() = Some(cluster);
        self
    }

    /// Give back the leases of calls that have ended, and push the
    /// expiry of the ones still up: the tick a node in a cluster runs
    /// every few seconds (§7).
    ///
    /// It is the whole of the lease's housekeeping: what a sync callback
    /// could not await, and what keeps a call longer than the lease from
    /// losing its place.
    pub async fn settle_leases(&self) {
        let Some(cluster) = self.cluster_calls.read().ok().and_then(|c| c.clone()) else {
            return;
        };
        let releasing = std::mem::take(&mut *self.released.lock().unwrap());
        for (tenant, call_id) in releasing {
            if let Err(e) = cluster.release(&tenant, &call_id).await {
                debug!(error = %e, "a lease could not be given back; it will lapse");
            }
        }
        let held = self.held_leases();
        if !held.is_empty() {
            if let Err(e) = cluster.refresh(&held).await {
                debug!(error = %e, "the leases this node holds were not refreshed");
            }
        }
    }

    /// The calls this node holds and whose they are, for the tick that
    /// puts their leases back (`ClusterCallCount::refresh`).
    pub fn held_leases(&self) -> Vec<(TenantId, String)> {
        self.ledger
            .iter()
            .map(|e| (*e.value(), e.key().clone()))
            .collect()
    }

    /// Where the tenants' own limits are read from — the trunk set, which
    /// already reads every tenant on its refresh tick.
    pub fn with_tenant_limits(self, limits: Arc<dyn TenantCallLimits>) -> Self {
        *self.tenant_limits.write().unwrap() = Some(limits);
        self
    }

    /// Set the tenants' limits after construction.
    pub fn set_tenant_limits(&self, limits: Arc<dyn TenantCallLimits>) {
        match self.tenant_limits.write() {
            Ok(mut slot) => *slot = Some(limits),
            Err(e) => warn!(error = %e, "Admission lock poisoned; tenant limits not set"),
        }
    }

    /// Begin or call off a drain: a draining node refuses new calls and
    /// shortens the registrations it grants
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    pub fn set_draining(&self, draining: bool) {
        self.draining.store(draining, Ordering::Relaxed);
    }

    /// Whether this node is leaving service.
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Relaxed)
    }

    /// Try to admit a new call. Returns `true` if allowed.
    ///
    /// Checks the concurrent call limit, then the CPS window. A limit of
    /// zero is no limit.
    pub fn try_admit_call(&self) -> bool {
        self.check_node().is_ok()
    }

    /// The node's own limits, in the order they are cheapest to check.
    /// A node on its way out of service takes nothing new at all.
    fn check_node(&self) -> Result<(), Refusal> {
        if self.is_draining() {
            return Err(Refusal::Draining);
        }
        let cfg = self.config.read().unwrap();

        let max_calls = cfg.max_concurrent_calls as u64;
        if max_calls != 0 && self.concurrent_calls.load(Ordering::Relaxed) >= max_calls {
            return Err(Refusal::NodeCalls);
        }

        if cfg.max_cps == 0 {
            return Ok(());
        }

        let now = Instant::now();
        let mut window = self.cps_window.lock().unwrap();

        // Prune entries older than 1 second
        while let Some(&front) = window.front() {
            if now.duration_since(front).as_secs() >= 1 {
                window.pop_front();
            } else {
                break;
            }
        }

        if window.len() as u32 >= cfg.max_cps {
            return Err(Refusal::NodeCps);
        }

        window.push_back(now);
        Ok(())
    }

    /// Undo what [`check_node`](Self::check_node) recorded, for a call that
    /// a later limit refused: its start must not count against the second.
    fn unrecord_cps(&self) {
        if let Ok(mut window) = self.cps_window.lock() {
            window.pop_back();
        }
    }

    /// Try to admit a call, or queue it if the overload action is `Queue`.
    ///
    /// Returns:
    /// - `Admitted` if the call can proceed immediately
    /// - `Rejected` if overloaded and action is Reject (or queue is full)
    /// - `Queued(rx)` if the call is enqueued — await `rx` for the result
    pub fn try_admit_or_queue(&self) -> AdmitResult {
        if self.try_admit_call() {
            return AdmitResult::Admitted;
        }

        let cfg = self.config.read().unwrap();
        if cfg.overload_action != AdmissionAction::Queue {
            return AdmitResult::Rejected;
        }

        let queue_max = cfg.queue_max_size as usize;
        drop(cfg);

        let (tx, rx) = oneshot::channel();
        let mut queue = self.call_queue.lock().unwrap();
        if queue.len() >= queue_max {
            return AdmitResult::Rejected;
        }
        queue.push_back(QueuedCall {
            sender: tx,
            queued_at: Instant::now(),
        });
        AdmitResult::Queued(rx)
    }

    /// Drain queued calls when capacity becomes available.
    ///
    /// Prunes expired entries and admits the next waiting call if possible.
    fn drain_queue(&self) {
        let cfg = self.config.read().unwrap();
        let timeout = std::time::Duration::from_secs(cfg.queue_timeout_secs);
        drop(cfg);

        let mut queue = self.call_queue.lock().unwrap();
        let now = Instant::now();

        // Prune expired entries
        while let Some(front) = queue.front() {
            if now.duration_since(front.queued_at) >= timeout {
                if let Some(expired) = queue.pop_front() {
                    let _ = expired.sender.send(false);
                }
            } else {
                break;
            }
        }

        // Admit the next waiting call if we have capacity
        if self.try_admit_call() {
            if let Some(next) = queue.pop_front() {
                let _ = next.sender.send(true);
            }
        }
    }

    /// Try to admit a new registration. Returns `true` if allowed.
    pub fn try_admit_registration(&self) -> bool {
        let cfg = self.config.read().unwrap();
        let max = cfg.max_concurrent_registrations as u64;
        max == 0 || self.concurrent_registrations.load(Ordering::Relaxed) < max
    }

    /// Record that a call has started (increment concurrent count).
    pub fn call_started(&self) {
        self.concurrent_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a call has ended (decrement concurrent count).
    ///
    /// Also drains the queue: if calls are waiting and capacity is now
    /// available, the next queued call is admitted.
    pub fn call_ended(&self) {
        decrement(&self.concurrent_calls);
        // Try to drain queued calls now that capacity may be free
        self.drain_queue();
    }

    /// Record that a registration was added.
    pub fn registration_added(&self) {
        self.concurrent_registrations
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a registration was removed.
    pub fn registration_removed(&self) {
        decrement(&self.concurrent_registrations);
    }

    /// Say how many registrations this node holds, from the store that
    /// keeps them — the count a registrar cannot keep in an atomic,
    /// because bindings also expire on their own.
    pub fn set_registration_count(&self, count: u64) {
        self.concurrent_registrations
            .store(count, Ordering::Relaxed);
    }

    /// Current concurrent calls count.
    pub fn concurrent_calls(&self) -> u64 {
        self.concurrent_calls.load(Ordering::Relaxed)
    }

    /// Current concurrent registrations count.
    pub fn concurrent_registrations(&self) -> u64 {
        self.concurrent_registrations.load(Ordering::Relaxed)
    }

    /// Take a place for this tenant from this node's own count. `true`
    /// when the tenant is already at its limit here.
    fn take_local(&self, tenant: &TenantId, limit: u32) -> bool {
        let mut count = self.tenant_calls.entry(*tenant).or_insert(0);
        if *count >= limit as u64 {
            return true;
        }
        *count += 1;
        false
    }

    /// Calls in flight for one tenant.
    pub fn tenant_concurrent_calls(&self, tenant: &TenantId) -> u64 {
        self.tenant_calls.get(tenant).map(|n| *n).unwrap_or(0)
    }

    /// Every tenant carrying a call here, and how many.
    pub fn tenant_calls_snapshot(&self) -> Vec<(TenantId, u64)> {
        self.tenant_calls
            .iter()
            .map(|e| (*e.key(), *e.value()))
            .collect()
    }

    /// Current CPS (calls in the last second).
    pub fn current_cps(&self) -> u64 {
        let now = Instant::now();
        let window = self.cps_window.lock().unwrap();
        window
            .iter()
            .filter(|&&ts| now.duration_since(ts).as_secs() < 1)
            .count() as u64
    }

    /// Check whether a trunk is within its per-trunk CPS limit.
    ///
    /// Returns `true` if the trunk has no CPS limit configured or if the
    /// current call rate is below the limit. Does NOT record a new call;
    /// use [`record_trunk_call`](Self::record_trunk_call) after the trunk is
    /// actually selected.
    pub fn is_trunk_within_cps(&self, trunk_name: &str) -> bool {
        let limits = self.trunk_cps_limits.read().unwrap();
        let limit = match limits.get(trunk_name) {
            Some(&limit) => limit,
            None => return true, // no limit configured
        };

        let windows = self.trunk_cps_windows.read().unwrap();
        let window = match windows.get(trunk_name) {
            Some(w) => w,
            None => return true,
        };

        let now = Instant::now();
        let mut guard = window.lock().unwrap();

        // Prune entries older than 1 second
        while let Some(&front) = guard.front() {
            if now.duration_since(front).as_secs() >= 1 {
                guard.pop_front();
            } else {
                break;
            }
        }

        (guard.len() as u32) < limit
    }

    /// Record a call attempt for a trunk's CPS sliding window.
    ///
    /// Call this after a trunk has been selected for an outbound call.
    pub fn record_trunk_call(&self, trunk_name: &str) {
        let windows = self.trunk_cps_windows.read().unwrap();
        if let Some(window) = windows.get(trunk_name) {
            let mut guard = window.lock().unwrap();
            guard.push_back(Instant::now());
        }
    }

    /// Current CPS for a specific trunk (calls in the last second).
    pub fn trunk_cps(&self, trunk_name: &str) -> u64 {
        let windows = self.trunk_cps_windows.read().unwrap();
        let window = match windows.get(trunk_name) {
            Some(w) => w,
            None => return 0,
        };
        let now = Instant::now();
        let guard = window.lock().unwrap();
        guard
            .iter()
            .filter(|&&ts| now.duration_since(ts).as_secs() < 1)
            .count() as u64
    }

    /// Return a cloned snapshot of the current admission config.
    pub fn config_snapshot(&self) -> AdmissionConfig {
        self.config.read().unwrap().clone()
    }

    /// Update the admission config at runtime.
    ///
    /// Validates the new config before applying. Rebuilds per-trunk CPS
    /// maps (existing sliding windows are reset).
    pub fn update_config(&self, new_config: AdmissionConfig) -> Result<(), String> {
        new_config.validate()?;

        let mut trunk_limits = HashMap::new();
        let mut trunk_windows = HashMap::new();
        for entry in &new_config.per_trunk_cps {
            trunk_limits.insert(entry.trunk_name.clone(), entry.max_cps);
            trunk_windows.insert(entry.trunk_name.clone(), Mutex::new(VecDeque::new()));
        }

        // Apply atomically: config first, then trunk maps
        *self.config.write().unwrap() = new_config;
        *self.trunk_cps_limits.write().unwrap() = trunk_limits;
        *self.trunk_cps_windows.write().unwrap() = trunk_windows;

        Ok(())
    }
}

/// Saturating decrement of a counter that several tasks touch.
fn decrement(counter: &AtomicU64) {
    loop {
        let current = counter.load(Ordering::Relaxed);
        if current == 0 {
            break;
        }
        if counter
            .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            break;
        }
    }
}

impl crate::cps::CpsChecker for AdmissionController {
    fn is_trunk_within_cps(&self, trunk_name: &str) -> bool {
        self.is_trunk_within_cps(trunk_name)
    }

    fn record_trunk_call(&self, trunk_name: &str) {
        self.record_trunk_call(trunk_name)
    }
}

/// The call path's view: a place taken when a call starts and given back
/// when it ends, with the tenant's own limit checked alongside the node's
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
#[async_trait::async_trait]
impl NodeAdmission for AdmissionController {
    async fn admit(&self, call_id: &str, tenant: &TenantId) -> Result<(), Refusal> {
        let label = tenant.to_string();

        // The node's limits first: they are this node's survival, and the
        // check is a load and a window.
        if let Err(refusal) = self.check_node() {
            AdmissionCounters::global().note_refused(refusal, &label);
            return Err(refusal);
        }

        // Then the tenant's own, which is a contract with the platform
        // rather than a capacity of this node — so where there is a
        // cluster, the count is the cluster's (§7).
        let limit = self
            .tenant_limits
            .read()
            .ok()
            .and_then(|l| l.as_ref().and_then(|l| l.max_concurrent_calls(tenant)));
        let cluster = self.cluster_calls.read().ok().and_then(|c| c.clone());
        let over = match (limit.filter(|l| *l != 0), cluster) {
            // A limit, and a cluster to count it across: the lease is
            // taken on the way in and counted with this call included.
            (Some(limit), Some(cluster)) => {
                match cluster.take(tenant, call_id).await {
                    Ok(held) => {
                        self.cluster_failing.store(false, Ordering::Relaxed);
                        let over = held > limit as u64;
                        if over {
                            // Give back what this call took, or the
                            // refusal would count against the tenant
                            // until the lease lapsed.
                            if let Err(e) = cluster.release(tenant, call_id).await {
                                warn!(error = %e, "could not give back a refused call's lease");
                            }
                        } else {
                            *self.tenant_calls.entry(*tenant).or_insert(0) += 1;
                        }
                        over
                    }
                    // Redis is unreachable. A call must not fail because
                    // the count is unreadable: fall back to this node's
                    // own, which is what phase 1 enforced, and say so
                    // once rather than per call.
                    Err(e) => {
                        if !self.cluster_failing.swap(true, Ordering::Relaxed) {
                            warn!(error = %e, "the cluster's call count is unreadable; \
                                  counting this tenant's calls on this node alone");
                        }
                        self.take_local(tenant, limit)
                    }
                }
            }
            // A limit and no cluster: this node's count, as before.
            (Some(limit), None) => self.take_local(tenant, limit),
            // No limit: count the call and admit it.
            (None, _) => {
                *self.tenant_calls.entry(*tenant).or_insert(0) += 1;
                false
            }
        };
        if over {
            self.unrecord_cps();
            AdmissionCounters::global().note_refused(Refusal::TenantCalls, &label);
            return Err(Refusal::TenantCalls);
        }

        self.ledger.insert(call_id.to_string(), *tenant);
        self.call_started();
        AdmissionCounters::global().note_admitted(&label);
        Ok(())
    }

    fn ended(&self, call_id: &str) {
        let Some((_, tenant)) = self.ledger.remove(call_id) else {
            return;
        };
        // Give the cluster its place back: now if there is a runtime to
        // do it on, and on the settling tick either way (§7).
        if let Some(cluster) = self.cluster_calls.read().ok().and_then(|c| c.clone()) {
            {
                let mut released = self.released.lock().unwrap();
                released.push((tenant, call_id.to_string()));
                // A node whose settling tick never runs must not grow a
                // queue for ever; what is dropped lapses on its own.
                if released.len() > MAX_PENDING_RELEASES {
                    let overflow = released.len() - MAX_PENDING_RELEASES;
                    released.drain(..overflow);
                }
            }
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let call_id = call_id.to_string();
                handle.spawn(async move {
                    if let Err(e) = cluster.release(&tenant, &call_id).await {
                        debug!(error = %e, "a lease will be given back on the next tick");
                    }
                });
            }
        }
        if let Some(mut count) = self.tenant_calls.get_mut(&tenant) {
            *count = count.saturating_sub(1);
        }
        self.tenant_calls.retain(|_, n| *n > 0);
        self.call_ended();
    }

    fn admit_register(&self, source: Option<IpAddr>, aor: &str) -> Result<(), Refusal> {
        let cfg = self.config.read().unwrap();
        let (per_source, per_aor) = (
            cfg.max_register_cps_per_source,
            cfg.max_register_cps_per_aor,
        );
        let max_registrations = cfg.max_concurrent_registrations as u64;
        drop(cfg);

        // An exempt peer carries many devices: no per-source limit is its.
        let over_source = source.is_some_and(|source| {
            per_source != 0
                && !self
                    .register_rate
                    .try_record(&format!("src:{source}"), per_source)
        });
        if over_source {
            AdmissionCounters::global().note_refused(Refusal::RegisterRate, "");
            return Err(Refusal::RegisterRate);
        }
        if per_aor != 0
            && !self
                .register_rate
                .try_record(&format!("aor:{}", aor.to_ascii_lowercase()), per_aor)
        {
            AdmissionCounters::global().note_refused(Refusal::RegisterRate, "");
            return Err(Refusal::RegisterRate);
        }
        self.register_rate.prune();

        if max_registrations != 0
            && self.concurrent_registrations.load(Ordering::Relaxed) >= max_registrations
        {
            AdmissionCounters::global().note_refused(Refusal::Registrations, "");
            return Err(Refusal::Registrations);
        }
        Ok(())
    }

    fn retry_after_secs(&self) -> u64 {
        self.config
            .read()
            .map(|c| c.retry_after_secs)
            .unwrap_or(5)
            .max(1)
    }

    fn draining(&self) -> bool {
        self.is_draining()
    }

    fn set_draining(&self, draining: bool) {
        AdmissionController::set_draining(self, draining);
    }

    fn drain_expiry_cap(&self) -> Option<u32> {
        if !self.is_draining() {
            return None;
        }
        self.config
            .read()
            .ok()
            .map(|c| c.drain_register_expiry_secs)
            .filter(|secs| *secs != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = AdmissionConfig::default();
        assert_eq!(cfg.max_cps, 100);
        assert_eq!(cfg.max_concurrent_calls, 1000);
        assert_eq!(cfg.max_concurrent_registrations, 5000);
        assert!(cfg.per_trunk_cps.is_empty());
        assert_eq!(cfg.overload_action, AdmissionAction::Reject);
        assert!(cfg.include_retry_after);
        assert_eq!(cfg.retry_after_secs, 5);
    }

    #[test]
    fn test_validate_valid() {
        assert!(AdmissionConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_with_trunk_limits() {
        let cfg = AdmissionConfig {
            per_trunk_cps: vec![
                TrunkCpsLimit {
                    trunk_name: "carrier-a".into(),
                    max_cps: 50,
                },
                TrunkCpsLimit {
                    trunk_name: "carrier-b".into(),
                    max_cps: 30,
                },
            ],
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_excessive_cps() {
        let cfg = AdmissionConfig {
            max_cps: 100_001,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_cps"));
    }

    #[test]
    fn test_validate_excessive_concurrent_calls() {
        let cfg = AdmissionConfig {
            max_concurrent_calls: 1_000_001,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_concurrent_calls"));
    }

    #[test]
    fn test_validate_retry_after_too_low() {
        let cfg = AdmissionConfig {
            retry_after_secs: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("retry_after_secs"));
    }

    #[test]
    fn test_validate_retry_after_too_high() {
        let cfg = AdmissionConfig {
            retry_after_secs: 3601,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("retry_after_secs"));
    }

    #[test]
    fn test_validate_empty_trunk_name() {
        let cfg = AdmissionConfig {
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: String::new(),
                max_cps: 10,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("trunk_name"));
    }

    #[test]
    fn test_validate_crlf_trunk_name() {
        let cfg = AdmissionConfig {
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: "bad\r\nname".into(),
                max_cps: 10,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_boundary_values() {
        let mut cfg = AdmissionConfig {
            max_cps: 100_000,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());

        // Exactly 1,000,000 concurrent calls is valid
        cfg.max_concurrent_calls = 1_000_000;
        assert!(cfg.validate().is_ok());

        // Retry-After boundaries
        cfg.retry_after_secs = 1;
        assert!(cfg.validate().is_ok());
        cfg.retry_after_secs = 3600;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = AdmissionConfig {
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: "carrier-a".into(),
                max_cps: 50,
            }],
            overload_action: AdmissionAction::Queue,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: AdmissionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}

#[cfg(test)]
mod controller_tests {
    use super::*;

    fn default_controller() -> AdmissionController {
        AdmissionController::new(AdmissionConfig::default())
    }

    fn controller_with_trunk_cps(trunk_name: &str, max_cps: u32) -> AdmissionController {
        let config = AdmissionConfig {
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: trunk_name.into(),
                max_cps,
            }],
            ..Default::default()
        };
        AdmissionController::new(config)
    }

    #[test]
    fn test_admit_call_under_limits() {
        let ctrl = default_controller();
        assert!(ctrl.try_admit_call());
    }

    #[test]
    fn test_concurrent_call_tracking() {
        let ctrl = default_controller();
        ctrl.call_started();
        assert_eq!(ctrl.concurrent_calls(), 1);
        ctrl.call_started();
        assert_eq!(ctrl.concurrent_calls(), 2);
        ctrl.call_ended();
        assert_eq!(ctrl.concurrent_calls(), 1);
        ctrl.call_ended();
        assert_eq!(ctrl.concurrent_calls(), 0);
        // Underflow protection
        ctrl.call_ended();
        assert_eq!(ctrl.concurrent_calls(), 0);
    }

    #[test]
    fn test_concurrent_registration_tracking() {
        let ctrl = default_controller();
        ctrl.registration_added();
        assert_eq!(ctrl.concurrent_registrations(), 1);
        ctrl.registration_removed();
        assert_eq!(ctrl.concurrent_registrations(), 0);
        ctrl.registration_removed();
        assert_eq!(ctrl.concurrent_registrations(), 0);
    }

    #[test]
    fn test_concurrent_call_limit() {
        let config = AdmissionConfig {
            max_concurrent_calls: 2,
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);
        ctrl.call_started();
        ctrl.call_started();
        // At limit: new admission should fail
        assert!(!ctrl.try_admit_call());
        ctrl.call_ended();
        assert!(ctrl.try_admit_call());
    }

    #[test]
    fn test_registration_limit() {
        let config = AdmissionConfig {
            max_concurrent_registrations: 1,
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);
        assert!(ctrl.try_admit_registration());
        ctrl.registration_added();
        assert!(!ctrl.try_admit_registration());
        ctrl.registration_removed();
        assert!(ctrl.try_admit_registration());
    }

    #[test]
    fn test_trunk_cps_no_limit_always_allowed() {
        let ctrl = default_controller(); // no per_trunk_cps entries
        assert!(ctrl.is_trunk_within_cps("any-trunk"));
    }

    #[test]
    fn test_trunk_cps_within_limit() {
        let ctrl = controller_with_trunk_cps("carrier-a", 5);
        ctrl.record_trunk_call("carrier-a");
        ctrl.record_trunk_call("carrier-a");
        assert!(ctrl.is_trunk_within_cps("carrier-a"));
        assert_eq!(ctrl.trunk_cps("carrier-a"), 2);
    }

    #[test]
    fn test_trunk_cps_at_limit() {
        let ctrl = controller_with_trunk_cps("carrier-a", 2);
        ctrl.record_trunk_call("carrier-a");
        ctrl.record_trunk_call("carrier-a");
        assert!(!ctrl.is_trunk_within_cps("carrier-a"));
    }

    #[test]
    fn test_trunk_cps_unknown_trunk_allowed() {
        let ctrl = controller_with_trunk_cps("carrier-a", 1);
        // "carrier-b" has no limit configured
        assert!(ctrl.is_trunk_within_cps("carrier-b"));
    }

    #[test]
    fn test_trunk_cps_independent_per_trunk() {
        let config = AdmissionConfig {
            per_trunk_cps: vec![
                TrunkCpsLimit {
                    trunk_name: "trunk-a".into(),
                    max_cps: 1,
                },
                TrunkCpsLimit {
                    trunk_name: "trunk-b".into(),
                    max_cps: 1,
                },
            ],
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);

        ctrl.record_trunk_call("trunk-a");
        // trunk-a is at limit, trunk-b is still open
        assert!(!ctrl.is_trunk_within_cps("trunk-a"));
        assert!(ctrl.is_trunk_within_cps("trunk-b"));
    }

    #[test]
    fn test_update_config_valid() {
        let ctrl = default_controller();
        assert_eq!(ctrl.config_snapshot().max_cps, 100);

        let new_config = AdmissionConfig {
            max_cps: 200,
            max_concurrent_calls: 2000,
            per_trunk_cps: vec![TrunkCpsLimit {
                trunk_name: "new-trunk".into(),
                max_cps: 50,
            }],
            ..Default::default()
        };
        assert!(ctrl.update_config(new_config).is_ok());
        assert_eq!(ctrl.config_snapshot().max_cps, 200);
        assert_eq!(ctrl.config_snapshot().max_concurrent_calls, 2000);
        // New trunk CPS limit should be active
        assert!(ctrl.is_trunk_within_cps("new-trunk"));
    }

    #[test]
    fn test_update_config_invalid() {
        let ctrl = default_controller();
        let bad_config = AdmissionConfig {
            max_cps: 200_000, // exceeds 100,000 limit
            ..Default::default()
        };
        let result = ctrl.update_config(bad_config);
        assert!(result.is_err());
        // Original config should be unchanged
        assert_eq!(ctrl.config_snapshot().max_cps, 100);
    }

    #[test]
    fn test_queue_admit_when_capacity_available() {
        let ctrl = default_controller();
        // Should be admitted directly (capacity available)
        let result = ctrl.try_admit_or_queue();
        assert!(matches!(result, AdmitResult::Admitted));
    }

    #[tokio::test]
    async fn test_queue_then_drain() {
        let config = AdmissionConfig {
            max_concurrent_calls: 1,
            overload_action: AdmissionAction::Queue,
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);
        ctrl.call_started(); // fill capacity

        // Should be queued
        let result = ctrl.try_admit_or_queue();
        let rx = match result {
            AdmitResult::Queued(rx) => rx,
            other => panic!(
                "expected Queued, got {:?}",
                matches!(other, AdmitResult::Admitted)
            ),
        };

        // End a call — drain_queue should admit the queued call
        ctrl.call_ended();

        let admitted = rx.await.unwrap();
        assert!(admitted);
    }

    #[test]
    fn test_queue_full_rejects() {
        let config = AdmissionConfig {
            max_concurrent_calls: 1,
            overload_action: AdmissionAction::Queue,
            queue_max_size: 1,
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);
        ctrl.call_started(); // fill capacity

        // First queue should succeed
        let r1 = ctrl.try_admit_or_queue();
        assert!(matches!(r1, AdmitResult::Queued(_)));

        // Second queue should be rejected (queue full)
        let r2 = ctrl.try_admit_or_queue();
        assert!(matches!(r2, AdmitResult::Rejected));
    }

    #[test]
    fn test_queue_disabled_rejects() {
        let config = AdmissionConfig {
            max_concurrent_calls: 1,
            overload_action: AdmissionAction::Reject, // not Queue
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);
        ctrl.call_started(); // fill capacity

        let result = ctrl.try_admit_or_queue();
        assert!(matches!(result, AdmitResult::Rejected));
    }

    #[test]
    fn test_drain_prunes_expired() {
        let config = AdmissionConfig {
            max_concurrent_calls: 1,
            overload_action: AdmissionAction::Queue,
            queue_timeout_secs: 1, // very short
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config);
        ctrl.call_started(); // fill capacity

        // Enqueue a call
        let result = ctrl.try_admit_or_queue();
        let mut rx = match result {
            AdmitResult::Queued(rx) => rx,
            _ => panic!("expected Queued"),
        };

        // Manually expire by sleeping > 1s
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // End a call → drain_queue will prune the expired entry
        ctrl.call_ended();

        // The receiver should get false (expired)
        let result = rx.try_recv();
        assert!(matches!(result, Ok(false)));
    }

    #[test]
    fn test_config_snapshot() {
        let config = AdmissionConfig {
            max_cps: 42,
            max_concurrent_calls: 99,
            ..Default::default()
        };
        let ctrl = AdmissionController::new(config.clone());
        let snapshot = ctrl.config_snapshot();
        assert_eq!(snapshot.max_cps, 42);
        assert_eq!(snapshot.max_concurrent_calls, 99);
    }
}

/// What the call path asks of the controller
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
#[cfg(test)]
mod node_admission_tests {
    use super::*;

    struct Limits(HashMap<TenantId, u32>);

    impl TenantCallLimits for Limits {
        fn max_concurrent_calls(&self, tenant: &TenantId) -> Option<u32> {
            self.0.get(tenant).copied()
        }
    }

    fn controller(max_calls: u32, max_cps: u32) -> AdmissionController {
        AdmissionController::new(AdmissionConfig {
            max_concurrent_calls: max_calls,
            max_cps,
            ..AdmissionConfig::unlimited()
        })
    }

    /// A node that writes no limit down carries whatever arrives — the
    /// behaviour of every call manager before there was a controller (§10).
    #[tokio::test]
    async fn zero_is_no_limit() {
        let ctrl = controller(0, 0);
        let tenant = TenantId::new();
        for i in 0..500 {
            assert!(ctrl.admit(&format!("call-{i}"), &tenant).await.is_ok());
        }
        assert_eq!(ctrl.concurrent_calls(), 500);
    }

    /// The node's own limit refuses the call past it, and the place is
    /// given back when a call ends.
    #[tokio::test]
    async fn the_node_limit_holds_and_is_released() {
        let ctrl = controller(2, 0);
        let tenant = TenantId::new();
        assert!(ctrl.admit("a", &tenant).await.is_ok());
        assert!(ctrl.admit("b", &tenant).await.is_ok());
        assert_eq!(ctrl.admit("c", &tenant).await, Err(Refusal::NodeCalls));

        ctrl.ended("a");
        assert!(ctrl.admit("c", &tenant).await.is_ok());
        // A call that was never admitted releases nothing.
        ctrl.ended("never-admitted");
        assert_eq!(ctrl.concurrent_calls(), 2);
    }

    /// A tenant at its limit is refused; another tenant on the same node is
    /// not.
    /// The tenant's limit across the cluster
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §7): the count that decides
    /// is the cluster's, and a cluster that cannot be read falls back to
    /// this node's rather than failing the call.
    mod cluster {
        use super::*;
        use std::sync::atomic::AtomicU64;

        /// A cluster count in memory: what Redis would say, and whether
        /// it can be reached at all.
        #[derive(Default)]
        struct Cluster {
            held: Mutex<HashMap<String, Vec<String>>>,
            broken: AtomicBool,
            takes: AtomicU64,
        }

        #[async_trait::async_trait]
        impl ClusterCallCount for Cluster {
            async fn take(&self, tenant: &TenantId, call_id: &str) -> anyhow::Result<u64> {
                self.takes.fetch_add(1, Ordering::SeqCst);
                if self.broken.load(Ordering::SeqCst) {
                    anyhow::bail!("redis is away");
                }
                let mut held = self.held.lock().unwrap();
                let calls = held.entry(tenant.to_string()).or_default();
                if !calls.iter().any(|c| c == call_id) {
                    calls.push(call_id.to_string());
                }
                Ok(calls.len() as u64)
            }

            async fn release(&self, tenant: &TenantId, call_id: &str) -> anyhow::Result<()> {
                if self.broken.load(Ordering::SeqCst) {
                    anyhow::bail!("redis is away");
                }
                if let Some(calls) = self.held.lock().unwrap().get_mut(&tenant.to_string()) {
                    calls.retain(|c| c != call_id);
                }
                Ok(())
            }

            async fn refresh(&self, _held: &[(TenantId, String)]) -> anyhow::Result<()> {
                Ok(())
            }
        }

        fn controller(cluster: &Arc<Cluster>, limit: u32) -> AdmissionController {
            let tenant = TenantId::new();
            let _ = tenant;
            AdmissionController::new(AdmissionConfig::unlimited())
                .with_cluster_calls(Arc::clone(cluster) as Arc<dyn ClusterCallCount>)
                .with_tenant_limits(Arc::new(Limits(limit)))
        }

        /// Every tenant has the same limit here.
        struct Limits(u32);

        impl TenantCallLimits for Limits {
            fn max_concurrent_calls(&self, _tenant: &TenantId) -> Option<u32> {
                Some(self.0)
            }
        }

        /// Two nodes, one tenant, one contract: the second node sees the
        /// first node's calls and refuses past the limit.
        #[tokio::test]
        async fn the_limit_is_the_clusters_not_the_nodes() {
            let cluster = Arc::new(Cluster::default());
            let one = controller(&cluster, 2);
            let two = controller(&cluster, 2);
            let tenant = TenantId::new();

            assert!(one.admit("a", &tenant).await.is_ok());
            assert!(two.admit("b", &tenant).await.is_ok(), "the other node");
            assert_eq!(
                two.admit("c", &tenant).await,
                Err(Refusal::TenantCalls),
                "two calls is the contract, wherever they are"
            );
            // Each node counted one of its own, and the refusal left
            // nothing behind in the cluster.
            assert_eq!(one.tenant_concurrent_calls(&tenant), 1);
            assert_eq!(two.tenant_concurrent_calls(&tenant), 1);
            assert_eq!(
                cluster.held.lock().unwrap()[&tenant.to_string()].len(),
                2,
                "the refused call gave its lease back"
            );

            // A call ending on one node frees the contract for the
            // other, once its node has settled the lease.
            one.ended("a");
            one.settle_leases().await;
            assert!(two.admit("c", &tenant).await.is_ok());
        }

        /// Redis being away must not fail a call: the tenant is then
        /// capped per node, which is what phase 1 enforced.
        #[tokio::test]
        async fn an_unreadable_cluster_falls_back_to_this_node() {
            let cluster = Arc::new(Cluster::default());
            let node = controller(&cluster, 2);
            let tenant = TenantId::new();
            cluster.broken.store(true, Ordering::SeqCst);

            assert!(node.admit("a", &tenant).await.is_ok());
            assert!(node.admit("b", &tenant).await.is_ok());
            assert_eq!(
                node.admit("c", &tenant).await,
                Err(Refusal::TenantCalls),
                "the node's own count still holds the line"
            );
            assert_eq!(node.tenant_concurrent_calls(&tenant), 2);
        }

        /// A lease is per call, so the same call admitted twice — a
        /// retried INVITE — does not spend the contract twice.
        #[tokio::test]
        async fn one_call_takes_one_lease() {
            let cluster = Arc::new(Cluster::default());
            let node = controller(&cluster, 1);
            let tenant = TenantId::new();

            assert!(node.admit("a", &tenant).await.is_ok());
            assert!(node.admit("a", &tenant).await.is_ok(), "the same call");
            assert_eq!(cluster.held.lock().unwrap()[&tenant.to_string()].len(), 1);
        }

        /// What the node holds, for the tick that puts the leases back.
        #[tokio::test]
        async fn the_node_can_say_what_it_holds() {
            let cluster = Arc::new(Cluster::default());
            let node = controller(&cluster, 10);
            let tenant = TenantId::new();
            node.admit("a", &tenant).await.ok();
            node.admit("b", &tenant).await.ok();
            let mut held: Vec<String> = node.held_leases().into_iter().map(|(_, c)| c).collect();
            held.sort();
            assert_eq!(held, vec!["a".to_string(), "b".to_string()]);
            node.ended("a");
            assert_eq!(node.held_leases().len(), 1);
        }
    }

    #[tokio::test]
    async fn a_tenants_limit_is_its_own() {
        let ctrl = controller(0, 0);
        let (small, large) = (TenantId::new(), TenantId::new());
        ctrl.set_tenant_limits(Arc::new(Limits(HashMap::from([(small, 1)]))));

        assert!(ctrl.admit("a", &small).await.is_ok());
        assert_eq!(ctrl.admit("b", &small).await, Err(Refusal::TenantCalls));
        assert!(ctrl.admit("c", &large).await.is_ok());
        assert_eq!(ctrl.tenant_concurrent_calls(&small), 1);

        ctrl.ended("a");
        assert_eq!(ctrl.tenant_concurrent_calls(&small), 0);
        assert!(ctrl.admit("b", &small).await.is_ok());
    }

    /// A call the tenant's limit refuses must not spend the node's second:
    /// the CPS window is put back as it was.
    #[tokio::test]
    async fn a_refused_call_does_not_spend_the_nodes_second() {
        let ctrl = controller(0, 2);
        let (small, other) = (TenantId::new(), TenantId::new());
        ctrl.set_tenant_limits(Arc::new(Limits(HashMap::from([(small, 1)]))));

        assert!(ctrl.admit("a", &small).await.is_ok());
        assert_eq!(ctrl.admit("b", &small).await, Err(Refusal::TenantCalls));
        // One call has started this second, not two.
        assert_eq!(ctrl.current_cps(), 1);
        assert!(ctrl.admit("c", &other).await.is_ok());
        assert_eq!(ctrl.admit("d", &other).await, Err(Refusal::NodeCps));
    }

    /// A registration flood is refused by source address and by AOR, and
    /// each key is counted on its own.
    #[test]
    fn registers_are_limited_by_source_and_by_aor() {
        let ctrl = AdmissionController::new(AdmissionConfig {
            max_register_cps_per_source: 2,
            max_register_cps_per_aor: 3,
            ..AdmissionConfig::unlimited()
        });
        let one: IpAddr = "198.51.100.7".parse().unwrap();
        let two: IpAddr = "198.51.100.8".parse().unwrap();

        assert!(ctrl
            .admit_register(Some(one), "sip:alice@example.com")
            .is_ok());
        assert!(ctrl
            .admit_register(Some(one), "sip:bob@example.com")
            .is_ok());
        assert_eq!(
            ctrl.admit_register(Some(one), "sip:carol@example.com"),
            Err(Refusal::RegisterRate)
        );
        // Another address has its own bucket.
        assert!(ctrl
            .admit_register(Some(two), "sip:dave@example.com")
            .is_ok());

        // And one AOR across addresses: alice has one so far, two more fit.
        let ctrl = AdmissionController::new(AdmissionConfig {
            max_register_cps_per_aor: 2,
            ..AdmissionConfig::unlimited()
        });
        assert!(ctrl
            .admit_register(Some(one), "sip:alice@example.com")
            .is_ok());
        assert!(ctrl
            .admit_register(Some(two), "SIP:Alice@example.com")
            .is_ok());
        assert_eq!(
            ctrl.admit_register(Some(two), "sip:alice@example.com"),
            Err(Refusal::RegisterRate)
        );
    }

    /// A peer that carries many devices has no per-source limit, but each
    /// AOR it carries keeps its own (#314).
    #[test]
    fn an_exempt_peer_is_limited_only_by_aor() {
        let ctrl = AdmissionController::new(AdmissionConfig {
            max_register_cps_per_source: 1,
            max_register_cps_per_aor: 1,
            ..AdmissionConfig::unlimited()
        });
        for user in ["alice", "bob", "carol"] {
            assert!(ctrl
                .admit_register(None, &format!("sip:{user}@example.com"))
                .is_ok());
        }
        assert_eq!(
            ctrl.admit_register(None, "sip:alice@example.com"),
            Err(Refusal::RegisterRate)
        );
    }

    /// REGISTERs per source are the one limit on when `[admission]` says
    /// nothing; `0` turns it off (#314).
    #[test]
    fn registers_per_source_are_limited_by_default() {
        let unset = AdmissionSection::default().resolve();
        assert_eq!(
            unset.max_register_cps_per_source,
            DEFAULT_REGISTER_CPS_PER_SOURCE
        );
        assert!(!unset.caps_capacity(), "a flood bound is not a capacity");
        let off = AdmissionSection {
            max_register_cps_per_source: Some(0),
            ..Default::default()
        }
        .resolve();
        assert_eq!(off.max_register_cps_per_source, 0);
        assert!(!off.caps_anything());
    }

    /// The registration limit is read from the count the store keeps,
    /// because bindings also expire on their own.
    #[test]
    fn the_registration_limit_follows_the_store() {
        let ctrl = AdmissionController::new(AdmissionConfig {
            max_concurrent_registrations: 10,
            ..AdmissionConfig::unlimited()
        });
        let source: IpAddr = "198.51.100.9".parse().unwrap();
        ctrl.set_registration_count(9);
        assert!(ctrl
            .admit_register(Some(source), "sip:alice@example.com")
            .is_ok());
        ctrl.set_registration_count(10);
        assert_eq!(
            ctrl.admit_register(Some(source), "sip:alice@example.com"),
            Err(Refusal::Registrations)
        );
        ctrl.set_registration_count(4);
        assert!(ctrl
            .admit_register(Some(source), "sip:alice@example.com")
            .is_ok());
    }

    /// Every refusal is counted under its own reason, and the admitted
    /// calls under their tenant.
    #[tokio::test]
    async fn refusals_are_counted_by_reason() {
        let ctrl = controller(1, 0);
        let tenant = TenantId::new();
        let before = AdmissionCounters::global().refused_for(Refusal::NodeCalls);
        assert!(ctrl.admit("a", &tenant).await.is_ok());
        assert!(ctrl.admit("b", &tenant).await.is_err());
        // The counters are the process's, and other tests refuse too.
        assert!(AdmissionCounters::global().refused_for(Refusal::NodeCalls) > before);
        let text = AdmissionCounters::global().prometheus("node=\"n1\"");
        assert!(text.contains("fcp_admission_refused_total{node=\"n1\",reason=\"node_calls\""));
        assert!(text.contains("fcp_admission_calls_admitted_total{node=\"n1\""));
    }
}

/// A node on its way out of service
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
#[cfg(test)]
mod drain_tests {
    use super::*;

    /// A draining node refuses new calls whatever its limits allow, and
    /// takes them again the moment the drain is called off.
    #[tokio::test]
    async fn a_draining_node_takes_no_new_call() {
        let ctrl = AdmissionController::new(AdmissionConfig::unlimited());
        let tenant = TenantId::new();
        assert!(ctrl.admit("before", &tenant).await.is_ok());

        ctrl.set_draining(true);
        assert_eq!(ctrl.admit("during", &tenant).await, Err(Refusal::Draining));
        // The call it was already carrying is untouched.
        assert_eq!(ctrl.concurrent_calls(), 1);

        ctrl.set_draining(false);
        assert!(ctrl.admit("after", &tenant).await.is_ok());
    }

    /// A draining node keeps its devices, on a short registration; a
    /// serving node caps nothing.
    #[test]
    fn a_draining_node_shortens_the_registration_it_grants() {
        let ctrl = AdmissionController::new(AdmissionConfig {
            drain_register_expiry_secs: 60,
            ..AdmissionConfig::unlimited()
        });
        let source: IpAddr = "198.51.100.11".parse().unwrap();
        assert_eq!(ctrl.drain_expiry_cap(), None);

        ctrl.set_draining(true);
        assert_eq!(ctrl.drain_expiry_cap(), Some(60));
        // And the REGISTER itself is still answered: a device that is
        // moving needs somewhere to move from.
        assert!(ctrl
            .admit_register(Some(source), "sip:alice@example.com")
            .is_ok());

        // A deployment that would rather not shorten anything says zero.
        let ctrl = AdmissionController::new(AdmissionConfig {
            drain_register_expiry_secs: 0,
            ..AdmissionConfig::unlimited()
        });
        ctrl.set_draining(true);
        assert_eq!(ctrl.drain_expiry_cap(), None);
    }
}

/// REGISTERs a second one source address may send when `[admission]` says
/// nothing (#314): a 50-phone office behind one NAT, all booting at once,
/// registers within a few seconds with its challenges, and a device
/// refused is told to come back in `retry_after_secs`. A flood from one
/// address stops here, before any credential work.
pub const DEFAULT_REGISTER_CPS_PER_SOURCE: u32 = 50;

/// The limits a node keeps (`docs/CAPACITY_AND_SURVIVABILITY.md` §4).
///
/// Every field is optional and unset means unlimited, so a node that
/// writes `[admission]` for one limit does not silently acquire the
/// others — which is what would happen if the SBC's own `AdmissionConfig`
/// defaults were deserialized here.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdmissionSection {
    /// Calls this node may start in one second.
    pub max_cps: Option<u32>,
    /// Calls this node may carry at once.
    pub max_concurrent_calls: Option<u32>,
    /// Registrations this node may hold.
    pub max_concurrent_registrations: Option<u32>,
    /// REGISTERs a second from one source address. A challenged
    /// registration is two REGISTERs, so a limit of `n` admits `n / 2`
    /// registrations a second from one address. Unset is
    /// [`DEFAULT_REGISTER_CPS_PER_SOURCE`], the one limit on by default
    /// (#314); `0` is none. Peers that carry many devices (an SBC in
    /// front, a site's link, another node) are exempt.
    pub max_register_cps_per_source: Option<u32>,
    /// REGISTERs a second for one AOR, across addresses.
    pub max_register_cps_per_aor: Option<u32>,
    /// The longest registration a draining node grants, so a device comes
    /// back soon and is placed on a node that is serving. Zero keeps what
    /// the device asked for; unset is 60 seconds.
    pub drain_register_expiry_secs: Option<u32>,
    /// Seconds to put in `Retry-After` when refusing.
    pub retry_after_secs: Option<u64>,
}

impl AdmissionSection {
    /// The controller's configuration: what this section says, and no
    /// limit anywhere it says nothing, but for REGISTERs per source.
    pub fn resolve(&self) -> AdmissionConfig {
        let mut cfg = AdmissionConfig::unlimited();
        cfg.max_cps = self.max_cps.unwrap_or(0);
        cfg.max_concurrent_calls = self.max_concurrent_calls.unwrap_or(0);
        cfg.max_concurrent_registrations = self.max_concurrent_registrations.unwrap_or(0);
        cfg.max_register_cps_per_source = self
            .max_register_cps_per_source
            .unwrap_or(DEFAULT_REGISTER_CPS_PER_SOURCE);
        cfg.max_register_cps_per_aor = self.max_register_cps_per_aor.unwrap_or(0);
        if let Some(secs) = self.retry_after_secs {
            cfg.retry_after_secs = secs.clamp(1, 3600);
        }
        if let Some(secs) = self.drain_register_expiry_secs {
            cfg.drain_register_expiry_secs = secs;
        }
        cfg
    }
}
