//! Redis-based cluster membership via heartbeat keys.
//!
//! Each node writes a hash to `cluster:node:{node_id}` with a 15s TTL,
//! refreshed every 5s. Nodes discover each other via SCAN.

use dashmap::DashMap;
use fcp_events::EventPublisher;
use fcp_storage_redis::RedisStorage;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::versions::{group_versions, ClusterVersions, VersionGroups, VersionTracker};
use crate::view::{ClusterView, Departure, ViewState, ViewTracker};

/// Information about a cluster member node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterMember {
    /// Unique node identifier.
    pub node_id: String,
    /// Node type (e.g. "call_manager", "events_server").
    pub node_type: String,
    /// HTTP API address for inter-node communication.
    pub http_address: SocketAddr,
    /// SIP signaling address.
    pub sip_address: SocketAddr,
    /// SIP-over-TLS address, when the node runs a TLS listener. Peers
    /// dial inter-node conference trunks here (mutual TLS); absent on
    /// nodes without one and on records written by older versions.
    #[serde(default)]
    pub sip_tls_address: Option<SocketAddr>,
    /// Node capabilities (e.g. ["registrar", "b2bua", "queue"]).
    pub capabilities: Vec<String>,
    /// Number of active calls on this node.
    pub active_calls: usize,
    /// Node start time (Unix epoch seconds).
    pub started_at: u64,
    /// Software version.
    pub version: String,
    /// Shutting down: still serving what it has, taking nothing new.
    /// Conference placement skips a draining node. Absent on records
    /// written by older versions.
    #[serde(default)]
    pub draining: bool,
    /// Deployment region this node runs in (`[cluster] region`, or
    /// `FCP_REGION`), e.g. `us-east`. A free-form label that only has to
    /// agree across nodes; it is not the geographic routing `Region`.
    /// Placement prefers nodes in the caller's region. Absent on nodes
    /// without one and on records written by older versions. See
    /// `docs/MULTI_REGION.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// The registering trunks this node has loaded
    /// (`docs/TENANT_TRUNKS.md` §8), by trunk id. A trunk one node reads
    /// from its own `[[trunks]]` may be on no other node, so a node cannot
    /// tell from the store alone who else could register it; each node
    /// says what it has and the owner is picked from that. Empty on nodes
    /// with no registering trunks and on records written by older
    /// versions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registering_trunks: Vec<String>,
    /// Where a browser reaches this node, when it is browser-facing
    /// (`[http] public_url` on the WebRTC gateway), e.g.
    /// `https://gw2.example.com`. The bind address in `http_address` is
    /// what peers dial inside the cluster; it is often `0.0.0.0` and
    /// never something to hand a browser. A node that leaves it unset
    /// takes no handovers from a draining peer
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). Absent on nodes that
    /// are not browser-facing and on records written by older versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    /// What this node will carry and what it is carrying
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §7), so headroom is
    /// computable by anything that reads membership: placement prefers
    /// the node with the most room rather than the fewest calls, and the
    /// admin UI shows both. Absent on a node that keeps no limits and on
    /// records written before this existed — such a node is placed on its
    /// call count alone, as every node was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<NodeCapacity>,
    /// Video capacity left per device, advertised by conference nodes
    /// (`docs/VIDEO_CONFERENCING.md` §9.3). Placement prefers, for a video
    /// room, the node with the most left on a device that has the room's
    /// codecs. Empty on nodes without video and on records written by
    /// older versions: such a node is neither preferred nor excluded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub video_budget: Vec<VideoDeviceBudget>,
    /// This node's own view of the cluster, when it does not look whole
    /// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3): a node that lost every
    /// other region at once says so here, where the admin API reads it.
    /// Absent while the view is whole and on records written by older
    /// versions. A node that cannot reach the store cannot write this at
    /// all; its own `/status` is where that is said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<crate::view::ClusterView>,
}

/// What a node will carry, from its `[admission]` section, and what it
/// is carrying now (`docs/CAPACITY_AND_SURVIVABILITY.md` §7). Zero means
/// no limit, as it does in the configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCapacity {
    /// `[admission] max_concurrent_calls`.
    #[serde(default)]
    pub max_concurrent_calls: u32,
    /// `[admission] max_cps`.
    #[serde(default)]
    pub max_cps: u32,
    /// `[admission] max_concurrent_registrations`.
    #[serde(default)]
    pub max_concurrent_registrations: u32,
    /// Registrations held now.
    #[serde(default)]
    pub registrations: u32,
    /// Calls started in the last second.
    #[serde(default)]
    pub cps: u32,
}

impl NodeCapacity {
    /// How many more calls this node will take, given what it is already
    /// carrying. `None` when it keeps no call limit — room unknown, not
    /// room infinite, which is why placement treats it apart.
    pub fn calls_left(&self, active_calls: usize) -> Option<u32> {
        (self.max_concurrent_calls != 0).then(|| {
            self.max_concurrent_calls
                .saturating_sub(active_calls as u32)
        })
    }
}

/// One device's video budget on a node, in work units: one unit is a
/// thousandth of one execution unit's time per second (on the host, a
/// millicore of one codec-pool thread). `left` is what a new room or
/// participant may still cost here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoDeviceBudget {
    /// `host`, or a GPU as `backend:address`.
    pub device: String,
    /// SDP names of the codecs the device *encodes* (`H264`, `VP8`, …).
    pub codecs: Vec<String>,
    /// What a new room or participant may still cost here: on a device
    /// with engines, the tightest engine's.
    pub left: u32,
    pub total: u32,
    /// A GPU's engines, each a budget of its own (FCP §15.8): NVENC,
    /// NVDEC and the compute cores saturate separately, and placement
    /// wants the encoder's. Absent on the host, and on a node from
    /// before engines were advertised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engines: Option<VideoEngines>,
}

impl VideoDeviceBudget {
    /// What placement weighs: the encode engine's `left` when the
    /// device has engines, the device's otherwise.
    pub fn placement_left(&self) -> u32 {
        self.engines
            .as_ref()
            .map(|e| e.encode.left)
            .unwrap_or(self.left)
    }
}

/// The three budgets of a GPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoEngines {
    pub encode: EngineBudget,
    pub decode: EngineBudget,
    pub compose: EngineBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineBudget {
    pub left: u32,
    pub total: u32,
}

/// Writes the video budget a [`ClusterManager`] advertises.
#[derive(Clone)]
pub struct VideoBudgetHandle(Arc<std::sync::RwLock<Vec<VideoDeviceBudget>>>);

impl VideoBudgetHandle {
    pub fn set(&self, budget: Vec<VideoDeviceBudget>) {
        if let Ok(mut b) = self.0.write() {
            *b = budget;
        }
    }

    pub fn get(&self) -> Vec<VideoDeviceBudget> {
        self.0.read().map(|b| b.clone()).unwrap_or_default()
    }
}

pub use fcp_core::region::resolve_region;

impl ClusterMember {
    /// The most video budget left on a device of this node that runs one of
    /// `codecs` (any device when `codecs` is empty); `None` when the node
    /// advertises no video budget at all — unknown, not zero.
    pub fn video_budget_left(&self, codecs: &[String]) -> Option<u32> {
        if self.video_budget.is_empty() {
            return None;
        }
        self.video_budget
            .iter()
            .filter(|b| codecs.is_empty() || b.codecs.iter().any(|c| codecs.contains(c)))
            .map(|b| b.placement_left())
            .max()
            .or(Some(0))
    }

    /// [`video_budget_left`](Self::video_budget_left) for a
    /// [`VideoNeed`](crate::VideoNeed): when some device of this node
    /// encodes the meeting's first codec, the most left among those
    /// devices; otherwise any device with any of the codecs, as before.
    /// A GPU whose encoder lacks the first codec does not count, so the
    /// meeting is not sent to it for a budget it would spend on its host.
    pub fn video_budget_left_for(&self, need: &crate::VideoNeed) -> Option<u32> {
        if self.video_budget.is_empty() {
            return None;
        }
        if let Some(first) = &need.first {
            let with_first: Vec<u32> = self
                .video_budget
                .iter()
                .filter(|b| b.codecs.contains(first))
                .map(|b| b.placement_left())
                .collect();
            if !with_first.is_empty() {
                return with_first.into_iter().max();
            }
        }
        self.video_budget_left(&need.codecs)
    }

    /// Whether this node is in `region`. A node without a region is in
    /// no region.
    pub fn in_region(&self, region: &str) -> bool {
        self.region.as_deref() == Some(region)
    }
}

/// Manages cluster membership via Redis heartbeats.
pub struct ClusterManager {
    storage: RedisStorage,
    self_info: ClusterMember,
    members: Arc<DashMap<String, ClusterMember>>,
    heartbeat_interval_secs: u64,
    heartbeat_ttl_secs: u64,
    shutdown_tx: Arc<watch::Sender<bool>>,
    event_publisher: Option<Arc<dyn EventPublisher>>,
    /// Shared counter for active calls, updated externally and read by heartbeat loop.
    active_calls_counter: Arc<AtomicUsize>,
    /// Set by [`set_draining`](Self::set_draining); published with every heartbeat.
    draining: Arc<AtomicBool>,
    /// Set by [`set_capacity`](Self::set_capacity); published with every
    /// heartbeat so headroom is computable from membership (§7).
    capacity: Arc<std::sync::RwLock<Option<NodeCapacity>>>,
    /// Set by [`set_video_budget`](Self::set_video_budget); published with
    /// every heartbeat.
    video_budget: Arc<std::sync::RwLock<Vec<VideoDeviceBudget>>>,
    /// Set by [`set_registering_trunks`](Self::set_registering_trunks);
    /// published with every heartbeat.
    registering_trunks: Arc<std::sync::RwLock<Vec<String>>>,
    /// True once the discovery loop has looked for peers at least once.
    /// Before that this node knows of none, and must not conclude from
    /// that silence that it is alone.
    discovered: Arc<AtomicBool>,
    /// Whether this node's view of the cluster looks whole (§2.3).
    view: Arc<std::sync::Mutex<ViewTracker>>,
    /// Which release every node runs, as the last discovery found it,
    /// and the tracker that says when that set changes (`versions`).
    versions: Arc<std::sync::Mutex<(VersionTracker, VersionGroups)>>,
    /// Whether this node reaches its database, asked while the view is
    /// not whole.
    database_probe: Option<DatabaseProbe>,
}

/// Asks whether the node reaches its database: `true` when it does.
pub type DatabaseProbe = Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> + Send + Sync,
>;

/// A [`DatabaseProbe`] that reads one tenant: the cheapest question every
/// binary with a database can already ask. An answer — even "no such
/// tenant" — is a database reached.
pub fn tenant_probe(
    store: Arc<dyn fcp_core::storage::TenantStore>,
    tenant_id: fcp_core::TenantId,
) -> DatabaseProbe {
    Arc::new(move || {
        let store = Arc::clone(&store);
        Box::pin(async move { store.get_tenant(&tenant_id).await.is_ok() })
    })
}

impl ClusterManager {
    /// Create a new cluster manager.
    pub fn new(storage: RedisStorage, self_info: ClusterMember) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        let self_info_capacity = self_info.capacity;
        let self_info_budget = self_info.video_budget.clone();
        let self_info_trunks = self_info.registering_trunks.clone();
        let self_info_region = self_info.region.clone();
        Self {
            storage,
            self_info,
            members: Arc::new(DashMap::new()),
            heartbeat_interval_secs: 5,
            heartbeat_ttl_secs: 15,
            shutdown_tx: Arc::new(shutdown_tx),
            event_publisher: None,
            active_calls_counter: Arc::new(AtomicUsize::new(0)),
            draining: Arc::new(AtomicBool::new(false)),
            capacity: Arc::new(std::sync::RwLock::new(self_info_capacity)),
            video_budget: Arc::new(std::sync::RwLock::new(self_info_budget)),
            registering_trunks: Arc::new(std::sync::RwLock::new(self_info_trunks)),
            discovered: Arc::new(AtomicBool::new(false)),
            view: Arc::new(std::sync::Mutex::new(ViewTracker::new(
                self_info_region,
                std::time::Duration::from_secs(15),
            ))),
            versions: Arc::new(std::sync::Mutex::new((
                VersionTracker::new(),
                VersionGroups::new(),
            ))),
            database_probe: None,
        }
    }

    /// Set custom heartbeat interval and TTL (builder pattern).
    pub fn with_intervals(mut self, interval_secs: u64, ttl_secs: u64) -> Self {
        self.heartbeat_interval_secs = interval_secs;
        self.heartbeat_ttl_secs = ttl_secs;
        self.view = Arc::new(std::sync::Mutex::new(ViewTracker::new(
            self.self_info.region.clone(),
            std::time::Duration::from_secs(ttl_secs),
        )));
        self
    }

    /// Ask `probe` whether this node reaches its database while its view of
    /// the cluster is not whole: one that reaches the database but not the
    /// cluster is almost certainly the node that is cut off
    /// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3).
    pub fn with_database_probe(mut self, probe: DatabaseProbe) -> Self {
        self.database_probe = Some(probe);
        self
    }

    /// Whether this node's view of the cluster looks whole, and if not,
    /// why (§2.3). Visibility only: nothing changes behaviour on it.
    pub fn view(&self) -> ClusterView {
        self.view
            .lock()
            .map(|v| v.view(std::time::Instant::now()))
            .unwrap_or_else(|p| p.into_inner().view(std::time::Instant::now()))
    }

    /// Which release every node runs, as the last discovery found it
    /// (`docs/BACKUP_AND_RESTORE.md` §5): the groups, whether there is
    /// more than one, and a sentence. Empty before the first discovery.
    /// Visibility only: a rolling upgrade is a skew on purpose.
    pub fn versions(&self) -> ClusterVersions {
        let groups = self
            .versions
            .lock()
            .map(|v| v.1.clone())
            .unwrap_or_else(|p| p.into_inner().1.clone());
        ClusterVersions::from_groups(groups)
    }

    /// Set an event publisher for emitting NodeJoined/NodeLeft events.
    pub fn with_event_publisher(mut self, publisher: Arc<dyn EventPublisher>) -> Self {
        self.event_publisher = Some(publisher);
        self
    }

    /// Start the heartbeat and discovery loops. Returns a JoinHandle.
    pub fn start(&self) -> JoinHandle<()> {
        let storage = self.storage.clone();
        let mut self_info = self.self_info.clone();
        let members = Arc::clone(&self.members);
        let interval = self.heartbeat_interval_secs;
        let ttl = self.heartbeat_ttl_secs;
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let event_publisher = self.event_publisher.clone();
        let active_calls_counter = Arc::clone(&self.active_calls_counter);
        let draining = Arc::clone(&self.draining);
        let capacity = Arc::clone(&self.capacity);
        let video_budget = Arc::clone(&self.video_budget);
        let registering_trunks = Arc::clone(&self.registering_trunks);
        let discovered = Arc::clone(&self.discovered);
        let view = Arc::clone(&self.view);
        let versions = Arc::clone(&self.versions);
        let database_probe = self.database_probe.clone();
        // A partition often drops packets rather than refusing them, and a
        // command on a dropped link waits forever. A tick that has not
        // finished within the heartbeat TTL has failed: by then this node's
        // own record has expired, whatever the tick was waiting on.
        let tick_budget = std::time::Duration::from_secs(ttl.max(2));

        info!(
            node_id = %self_info.node_id,
            node_type = %self_info.node_type,
            region = self_info.region.as_deref().unwrap_or("-"),
            "Cluster membership starting"
        );
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval));
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = shutdown_rx.changed() => {
                        info!(node_id = %self_info.node_id, "Cluster shutdown signal received");
                        if let Err(e) = remove_heartbeat(&storage, &self_info.node_id, ttl).await {
                            error!(error = %e, "Failed to remove heartbeat on shutdown");
                        }
                        break;
                    }
                }

                // Read latest active call count from shared counter
                self_info.active_calls = active_calls_counter.load(Ordering::Relaxed);
                self_info.draining = draining.load(Ordering::Relaxed);
                self_info.capacity = capacity.read().map(|c| *c).unwrap_or_default();
                self_info.video_budget = video_budget.read().map(|b| b.clone()).unwrap_or_default();
                self_info.registering_trunks = registering_trunks
                    .read()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                self_info.view = view
                    .lock()
                    .ok()
                    .map(|v| v.view(std::time::Instant::now()))
                    .filter(|v| !v.is_whole());

                let beat_and_look = async {
                    publish_heartbeat(&storage, &self_info, ttl).await?;
                    discover_members(
                        &storage,
                        &self_info.node_id,
                        &members,
                        event_publisher.as_ref(),
                    )
                    .await
                };
                let outcome = match tokio::time::timeout(tick_budget, beat_and_look).await {
                    Ok(outcome) => outcome,
                    Err(_) => Err(anyhow::anyhow!(
                        "the cluster store did not answer within {}s",
                        tick_budget.as_secs()
                    )),
                };
                // Which release every node runs, this one included; a
                // change in the set of releases is said once.
                if outcome.is_ok() {
                    let mut by_node: std::collections::BTreeMap<String, String> = members
                        .iter()
                        .map(|m| (m.node_id.clone(), m.version.clone()))
                        .collect();
                    by_node.insert(self_info.node_id.clone(), self_info.version.clone());
                    let groups =
                        group_versions(by_node.iter().map(|(n, v)| (n.as_str(), v.as_str())));
                    let change = match versions.lock() {
                        Ok(mut v) => {
                            let change = v.0.observe(&groups);
                            v.1 = groups;
                            change
                        }
                        Err(_) => None,
                    };
                    if let Some(change) = change {
                        announce_versions(&self_info, &change, event_publisher.as_ref()).await;
                    }
                }

                let now = std::time::Instant::now();
                let wall = std::time::SystemTime::now();
                let (transition, whole) = {
                    let Ok(mut tracker) = view.lock() else {
                        continue;
                    };
                    let transition = match &outcome {
                        Ok(departed) => {
                            let regions: Vec<Option<String>> =
                                members.iter().map(|m| m.region.clone()).collect();
                            tracker.on_discovery(now, wall, &regions, departed)
                        }
                        Err(e) => {
                            error!(error = %e, "Cluster heartbeat or discovery failed");
                            tracker.on_failure(now, wall)
                        }
                    };
                    (transition, tracker.state() == ViewState::Whole)
                };
                if outcome.is_ok() {
                    // Whatever it found, this node has now looked: an empty
                    // membership from here on means it really is alone.
                    discovered.store(true, Ordering::Relaxed);
                }

                // While the view is not whole, ask whether the database is
                // still there: it tells a node that is cut off from one whose
                // own network is down.
                if !whole {
                    if let Some(probe) = &database_probe {
                        let reachable = tokio::time::timeout(tick_budget, probe())
                            .await
                            .unwrap_or(false);
                        if let Ok(mut tracker) = view.lock() {
                            tracker.set_database_reachable(Some(reachable));
                        }
                    }
                }

                if let Some(transition) = transition {
                    let current = view
                        .lock()
                        .map(|v| v.view(now))
                        .unwrap_or(transition.view.clone());
                    announce_view(
                        &self_info,
                        transition.from,
                        &current,
                        transition.lasted_secs,
                        event_publisher.as_ref(),
                    )
                    .await;
                }
            }
        })
    }

    /// Mark this node as draining (or not) and publish the change at once
    /// rather than on the next heartbeat, so placement stops sending new
    /// work here before the node starts letting go of what it has. The
    /// flag is repeated with every later heartbeat.
    pub async fn set_draining(&self, draining: bool) -> anyhow::Result<()> {
        self.draining.store(draining, Ordering::Relaxed);
        let mut info = self.self_info.clone();
        info.active_calls = self.active_calls_counter.load(Ordering::Relaxed);
        info.draining = draining;
        info.video_budget = self
            .video_budget
            .read()
            .map(|b| b.clone())
            .unwrap_or_default();
        info.registering_trunks = self
            .registering_trunks
            .read()
            .map(|t| t.clone())
            .unwrap_or_default();
        info.view = Some(self.view()).filter(|v| !v.is_whole());
        publish_heartbeat(&self.storage, &info, self.heartbeat_ttl_secs).await?;
        info!(node_id = %info.node_id, draining, "Cluster draining flag published");
        Ok(())
    }

    /// Replace the video budget this node advertises; the heartbeat loop
    /// publishes it with the next beat.
    /// What this node will carry and what it is carrying, for the next
    /// heartbeat (`docs/CAPACITY_AND_SURVIVABILITY.md` §7). A node that
    /// keeps no limits publishes nothing, and placement then has only its
    /// call count to go on — which is what every node had before.
    pub fn set_capacity(&self, capacity: Option<NodeCapacity>) {
        if let Ok(mut held) = self.capacity.write() {
            *held = capacity;
        }
    }

    pub fn set_video_budget(&self, budget: Vec<VideoDeviceBudget>) {
        self.video_budget_handle().set(budget);
    }

    /// A handle a background task can write the budget through.
    pub fn video_budget_handle(&self) -> VideoBudgetHandle {
        VideoBudgetHandle(Arc::clone(&self.video_budget))
    }

    /// Say which registering trunks this node has loaded; the heartbeat
    /// loop publishes them with the next beat and
    /// [`registers_trunk`](Self::registers_trunk) answers from what every
    /// node has published.
    /// Returns whether this changed what the node had said before, which
    /// is when [`publish_now`](Self::publish_now) is worth the write.
    pub fn set_registering_trunks(&self, trunk_ids: Vec<String>) -> bool {
        match self.registering_trunks.write() {
            Ok(mut held) if *held != trunk_ids => {
                *held = trunk_ids;
                true
            }
            _ => false,
        }
    }

    /// Publish this node's heartbeat at once rather than on the next beat.
    ///
    /// A node that has just loaded a registering trunk has a claim to it
    /// that its peers cannot see until it beats, and a peer that does not
    /// see the claim takes the trunk for its own: the sooner the claim is
    /// out, the narrower that window.
    pub async fn publish_now(&self) {
        let mut info = self.self_info.clone();
        info.active_calls = self.active_calls_counter.load(Ordering::Relaxed);
        info.draining = self.draining.load(Ordering::Relaxed);
        info.video_budget = self
            .video_budget
            .read()
            .map(|b| b.clone())
            .unwrap_or_default();
        info.registering_trunks = self
            .registering_trunks
            .read()
            .map(|t| t.clone())
            .unwrap_or_default();
        info.view = Some(self.view()).filter(|v| !v.is_whole());
        if let Err(e) = publish_heartbeat(&self.storage, &info, self.heartbeat_ttl_secs).await {
            warn!(error = %e, "Failed to publish cluster heartbeat");
        }
    }

    /// Which node registers `trunk_id`: of the nodes that have loaded it,
    /// the lowest node id (`docs/TENANT_TRUNKS.md` §8), preferring one
    /// that is not draining — a node on its way out gives the binding up
    /// while it still has peers, but keeps it rather than let it lapse
    /// when it is the last one that can hold it.
    ///
    /// `None` when no live node has loaded the trunk.
    pub fn trunk_owner(&self, trunk_id: &str) -> Option<String> {
        if !self.discovered.load(Ordering::Relaxed) {
            // A node that has not yet looked for its peers would take
            // every trunk it loaded for its own and register a second
            // binding at the carrier for one another node already holds.
            return None;
        }
        let mine = self
            .registering_trunks
            .read()
            .map(|t| t.iter().any(|id| id == trunk_id))
            .unwrap_or(false);
        pick_trunk_owner(
            trunk_id,
            mine.then(|| {
                (
                    self.self_info.node_id.as_str(),
                    self.draining.load(Ordering::Relaxed),
                )
            }),
            self.members.iter().map(|m| m.value().clone()),
        )
    }

    /// Whether this node is the one that registers `trunk_id`. False for a
    /// trunk it has not loaded.
    pub fn registers_trunk(&self, trunk_id: &str) -> bool {
        self.trunk_owner(trunk_id)
            .is_some_and(|owner| owner == self.self_info.node_id)
    }

    /// How often this node beats, in seconds; what a caller waits on to
    /// know its advertisement has reached its peers and theirs it.
    pub fn heartbeat_interval_secs(&self) -> u64 {
        self.heartbeat_interval_secs
    }

    /// Whether this node is draining.
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Relaxed)
    }

    /// Signal the heartbeat loop to stop and remove this node's heartbeat key.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        // Give the background task a moment to clean up
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    /// Get all known cluster members (excluding self).
    pub fn members(&self) -> Vec<ClusterMember> {
        self.members.iter().map(|e| e.value().clone()).collect()
    }

    /// Get a specific cluster member by node_id.
    pub fn get_member(&self, node_id: &str) -> Option<ClusterMember> {
        self.members.get(node_id).map(|e| e.value().clone())
    }

    /// The member of a given type with the most room, anywhere in the
    /// cluster (`docs/CAPACITY_AND_SURVIVABILITY.md` §7): room where a
    /// node advertises what it will carry, fewest calls where it does
    /// not, which is what this was before.
    pub fn least_loaded(&self, node_type: &str) -> Option<ClusterMember> {
        self.members
            .iter()
            .filter(|e| e.value().node_type == node_type)
            .min_by_key(|e| headroom_key(e.value()))
            .map(|e| e.value().clone())
    }

    // ── Regions ────────────────────────────────────────────────────────

    /// This node's region, if it advertises one.
    pub fn region(&self) -> Option<&str> {
        self.self_info.region.as_deref()
    }

    /// Known members (excluding self) in `region`.
    pub fn members_in_region(&self, region: &str) -> Vec<ClusterMember> {
        self.members
            .iter()
            .filter(|e| e.value().in_region(region))
            .map(|e| e.value().clone())
            .collect()
    }

    /// Known members (excluding self) in this node's region. Empty when
    /// this node has no region: without one, nothing is local.
    pub fn local_members(&self) -> Vec<ClusterMember> {
        match self.region() {
            Some(region) => self.members_in_region(region),
            None => Vec::new(),
        }
    }

    /// The least loaded member of a given type in this node's region, or,
    /// when the region has none, anywhere in the cluster. Callers that
    /// must stay in region use [`members_in_region`](Self::members_in_region).
    pub fn least_loaded_near(&self, node_type: &str) -> Option<ClusterMember> {
        if let Some(region) = self.region() {
            let local = self
                .members
                .iter()
                .filter(|e| e.value().node_type == node_type && e.value().in_region(region))
                .min_by_key(|e| headroom_key(e.value()))
                .map(|e| e.value().clone());
            if local.is_some() {
                return local;
            }
        }
        self.least_loaded(node_type)
    }

    /// The node a browser on this one should move to, when this node is
    /// leaving service (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    ///
    /// A peer of the same kind that is serving, is reachable by a browser
    /// (it advertises a `public_url`) and is in this node's region when
    /// any peer there is; the least loaded of those. `None` when no peer
    /// qualifies, and then the browser is told to come back where it is —
    /// which is what a load balancer in front of the pool answers.
    pub fn handover_peer(&self) -> Option<ClusterMember> {
        choose_handover_node(
            &self.members(),
            &self.self_info.node_type,
            self.region(),
            &self.self_info.node_id,
        )
    }

    /// Get the local node info.
    pub fn self_info(&self) -> &ClusterMember {
        &self.self_info
    }

    /// Update the active call count for this node.
    /// This is thread-safe and the heartbeat loop will pick up the new value.
    pub fn update_active_calls(&self, count: usize) {
        self.active_calls_counter.store(count, Ordering::Relaxed);
    }

    /// Get the shared active calls counter (for external periodic updates).
    pub fn active_calls_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.active_calls_counter)
    }

    /// Get the local DashMap of members (for direct iteration).
    pub fn members_map(&self) -> &Arc<DashMap<String, ClusterMember>> {
        &self.members
    }
}

/// Remove this node's heartbeat key from Redis (graceful shutdown), and
/// say it left on purpose: a peer that sees it go reads the marker and does
/// not take a shutdown for a region going quiet (§2.3).
async fn remove_heartbeat(
    storage: &RedisStorage,
    node_id: &str,
    ttl_secs: u64,
) -> anyhow::Result<()> {
    let key = storage.key(&format!("cluster:node:{}", node_id));
    let left = storage.key(&format!("cluster:left:{}", node_id));
    let mut conn = storage.connection();

    let (): () = redis::pipe()
        .atomic()
        .set_ex(&left, "shutdown", (ttl_secs * 4).max(60))
        .del(&key)
        .query_async(&mut conn)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to remove heartbeat: {}", e))?;

    info!(node_id = %node_id, "Removed cluster heartbeat on shutdown");
    Ok(())
}

/// Publish this node's heartbeat to Redis.
async fn publish_heartbeat(
    storage: &RedisStorage,
    info: &ClusterMember,
    ttl_secs: u64,
) -> anyhow::Result<()> {
    let key = storage.key(&format!("cluster:node:{}", info.node_id));
    let mut conn = storage.connection();

    let json = serde_json::to_string(info)?;

    let (): () = redis::pipe()
        .atomic()
        .set(&key, &json)
        .expire(&key, ttl_secs as i64)
        .query_async(&mut conn)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to publish heartbeat: {}", e))?;

    debug!(node_id = %info.node_id, "Published cluster heartbeat");
    Ok(())
}

/// Discover cluster members by scanning Redis keys.
///
/// Emits `NodeJoined` / `NodeLeft` events when the set of known members
/// changes, and returns who left and whether they said so.
async fn discover_members(
    storage: &RedisStorage,
    self_node_id: &str,
    members: &DashMap<String, ClusterMember>,
    event_publisher: Option<&Arc<dyn EventPublisher>>,
) -> anyhow::Result<Vec<Departure>> {
    let mut conn = storage.connection();
    let pattern = storage.key("cluster:node:*");

    let mut seen_ids = std::collections::HashSet::new();
    // Collect newly seen members for event emission
    let mut new_members: Vec<ClusterMember> = Vec::new();
    let mut cursor = 0u64;

    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(&pattern)
            .arg("COUNT")
            .arg(100)
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("Redis SCAN failed: {}", e))?;

        for key in &keys {
            let json: Option<String> = conn
                .get(key)
                .await
                .map_err(|e| anyhow::anyhow!("Redis GET failed for {}: {}", key, e))?;

            if let Some(json) = json {
                match serde_json::from_str::<ClusterMember>(&json) {
                    Ok(member) => {
                        if member.node_id != self_node_id {
                            seen_ids.insert(member.node_id.clone());
                            if !members.contains_key(&member.node_id) {
                                new_members.push(member.clone());
                            }
                            members.insert(member.node_id.clone(), member);
                        }
                    }
                    Err(e) => {
                        warn!(key = %key, error = %e, "Failed to deserialize cluster member");
                    }
                }
            }
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }

    // Detect departed members before retain, and whether each left on
    // purpose: a shutdown leaves a marker, a node that went quiet does not.
    let gone: Vec<(String, Option<String>)> = members
        .iter()
        .filter(|e| !seen_ids.contains(e.key()))
        .map(|e| (e.key().clone(), e.value().region.clone()))
        .collect();
    let mut departed: Vec<(String, Departure)> = Vec::with_capacity(gone.len());
    for (node_id, region) in gone {
        let marker = storage.key(&format!("cluster:left:{}", node_id));
        let graceful: bool = conn
            .exists(&marker)
            .await
            .map_err(|e| anyhow::anyhow!("Redis EXISTS failed for {}: {}", marker, e))?;
        departed.push((node_id, Departure { region, graceful }));
    }

    // Remove members whose keys have expired (no longer seen)
    members.retain(|node_id, _| seen_ids.contains(node_id));

    // Emit events
    if let Some(publisher) = event_publisher {
        let now = std::time::SystemTime::now();

        for member in &new_members {
            let event = fcp_events::PlatformEvent::System(fcp_events::SystemEvent::NodeJoined {
                node_id: member.node_id.clone(),
                node_type: member.node_type.clone(),
                address: member.http_address.to_string(),
                region: member.region.clone(),
                timestamp: now,
            });
            info!(
                node_id = %member.node_id,
                node_type = %member.node_type,
                region = member.region.as_deref().unwrap_or("-"),
                "Cluster member joined"
            );
            if let Err(e) = publisher.publish(event).await {
                warn!(error = %e, "Failed to publish NodeJoined event");
            }
        }

        for (node_id, departure) in &departed {
            let reason = if departure.graceful {
                "shutdown"
            } else {
                "heartbeat_expired"
            };
            let event = fcp_events::PlatformEvent::System(fcp_events::SystemEvent::NodeLeft {
                node_id: node_id.clone(),
                reason: reason.to_string(),
                timestamp: now,
            });
            info!(node_id = %node_id, reason, "Cluster member left");
            if let Err(e) = publisher.publish(event).await {
                warn!(error = %e, "Failed to publish NodeLeft event");
            }
        }
    }

    debug!(member_count = seen_ids.len(), "Cluster discovery complete");
    Ok(departed.into_iter().map(|(_, d)| d).collect())
}

/// Say that the set of releases the cluster runs changed: in the log,
/// and as `system.version_skew` — when a second release appears (the
/// first node of an upgrade), when the set changes again, and when one
/// release is left (the upgrade done, or rolled back).
async fn announce_versions(
    self_info: &ClusterMember,
    versions: &ClusterVersions,
    event_publisher: Option<&Arc<dyn EventPublisher>>,
) {
    if versions.skew {
        warn!(
            node_id = %self_info.node_id,
            versions = ?versions.versions,
            "Cluster runs more than one release: {}",
            versions.summary
        );
    } else {
        info!(
            node_id = %self_info.node_id,
            "Cluster runs one release again: {}",
            versions.summary
        );
    }
    if let Some(publisher) = event_publisher {
        let event = fcp_events::PlatformEvent::System(fcp_events::SystemEvent::VersionSkew {
            node_id: self_info.node_id.clone(),
            versions: versions.versions.clone(),
            skew: versions.skew,
            summary: versions.summary.clone(),
            timestamp: std::time::SystemTime::now(),
        });
        if let Err(e) = publisher.publish(event).await {
            debug!(error = %e, "Failed to publish VersionSkew event");
        }
    }
}

/// Say that this node's view of the cluster changed: in the log, and as
/// `system.cluster_view_changed`. When the store is what cannot be reached
/// and the events travel through it, the event of going does not land;
/// the one of coming back does, and says how long it lasted.
async fn announce_view(
    self_info: &ClusterMember,
    from: ViewState,
    view: &ClusterView,
    lasted_secs: Option<u64>,
    event_publisher: Option<&Arc<dyn EventPublisher>>,
) {
    if view.is_whole() {
        info!(
            node_id = %self_info.node_id,
            from = from.as_str(),
            lasted_secs = lasted_secs.unwrap_or_default(),
            "Cluster view whole again"
        );
    } else {
        warn!(
            node_id = %self_info.node_id,
            state = view.state.as_str(),
            lost_regions = ?view.lost_regions,
            "Cluster view not whole: {}",
            view.summary
        );
    }
    if let Some(publisher) = event_publisher {
        let event =
            fcp_events::PlatformEvent::System(fcp_events::SystemEvent::ClusterViewChanged {
                node_id: self_info.node_id.clone(),
                region: self_info.region.clone(),
                state: view.state.as_str().to_string(),
                previous: from.as_str().to_string(),
                lost_regions: view.lost_regions.clone(),
                database_reachable: view.database_reachable,
                lasted_secs,
                summary: view.summary.clone(),
                timestamp: std::time::SystemTime::now(),
            });
        if let Err(e) = publisher.publish(event).await {
            debug!(error = %e, "Failed to publish ClusterViewChanged event");
        }
    }
}

/// Who registers `trunk_id`: the lowest node id of those that loaded it,
/// preferring a node that is not draining. `me` is this node when it
/// loaded the trunk itself, and `peers` every other node that is live.
fn pick_trunk_owner(
    trunk_id: &str,
    me: Option<(&str, bool)>,
    peers: impl Iterator<Item = ClusterMember>,
) -> Option<String> {
    // Ordered (draining, node_id): a node still taking work outranks one
    // on its way out, and the lowest id settles it either way.
    let mut best: Option<(bool, String)> = me.map(|(id, draining)| (draining, id.to_string()));
    for peer in peers {
        if !peer.registering_trunks.iter().any(|id| id == trunk_id) {
            continue;
        }
        let candidate = (peer.draining, peer.node_id);
        if best.as_ref().is_none_or(|held| candidate < *held) {
            best = Some(candidate);
        }
    }
    best.map(|(_, node_id)| node_id)
}

/// How a node is ranked for placement: full nodes last, then most room
/// first, then fewest calls, then by id so two choosers agree
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §7). A node that advertises no
/// capacity has unknown room, not infinite: it is ranked on its calls,
/// as every node was before capacity was published.
fn headroom_key(m: &ClusterMember) -> (bool, std::cmp::Reverse<u32>, usize, String) {
    let headroom = m.capacity.and_then(|c| c.calls_left(m.active_calls));
    (
        headroom == Some(0),
        std::cmp::Reverse(headroom.unwrap_or(0)),
        m.active_calls,
        m.node_id.clone(),
    )
}

/// Which peer a draining node hands its browsers to
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
///
/// The candidates are nodes of the same `node_type` that are serving (not
/// draining), are not this node, and advertise a `public_url` — without
/// one there is no address to send a browser to. Among them, a node in
/// `local_region` is preferred, as placement prefers one everywhere else;
/// among equals, the least loaded, then the lowest node id so two
/// draining nodes do not both crowd the same peer by accident.
pub fn choose_handover_node(
    members: &[ClusterMember],
    node_type: &str,
    local_region: Option<&str>,
    self_node_id: &str,
) -> Option<ClusterMember> {
    let candidates: Vec<&ClusterMember> = members
        .iter()
        .filter(|m| {
            m.node_type == node_type
                && !m.draining
                && m.node_id != self_node_id
                && m.public_url.as_ref().is_some_and(|u| !u.trim().is_empty())
        })
        .collect();
    let least_loaded = |from: &[&ClusterMember]| -> Option<ClusterMember> {
        from.iter()
            .min_by_key(|m| headroom_key(m))
            .map(|m| (*m).clone())
    };
    if let Some(region) = local_region {
        let local: Vec<&ClusterMember> = candidates
            .iter()
            .copied()
            .filter(|m| m.in_region(region))
            .collect();
        if let Some(pick) = least_loaded(&local) {
            return Some(pick);
        }
    }
    least_loaded(&candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, region: Option<&str>) -> ClusterMember {
        ClusterMember {
            node_id: id.to_string(),
            node_type: "call_manager".to_string(),
            http_address: "127.0.0.1:8080".parse().unwrap(),
            sip_address: "127.0.0.1:5060".parse().unwrap(),
            sip_tls_address: None,
            capabilities: Vec::new(),
            active_calls: 0,
            started_at: 0,
            version: "test".to_string(),
            public_url: None,
            capacity: None,
            draining: false,
            region: region.map(str::to_owned),
            video_budget: Vec::new(),
            registering_trunks: Vec::new(),
            view: None,
        }
    }

    /// Placement by room rather than by load
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §7).
    mod headroom {
        use super::*;

        fn node(id: &str, calls: usize, limit: Option<u32>) -> ClusterMember {
            let mut m = member(id, Some("us-east"));
            m.active_calls = calls;
            m.capacity = limit.map(|max_concurrent_calls| NodeCapacity {
                max_concurrent_calls,
                ..NodeCapacity::default()
            });
            m
        }

        /// The node with more calls can still have more room, and that
        /// is what placement is for.
        #[test]
        fn room_beats_load() {
            let small = node("small", 2, Some(4));
            let large = node("large", 10, Some(100));
            assert!(
                headroom_key(&large) < headroom_key(&small),
                "90 free beats 2 free, though it carries five times the calls"
            );
        }

        /// A node with no room left goes last, even against a node whose
        /// room is unknown.
        #[test]
        fn a_full_node_is_chosen_last() {
            let full = node("full", 4, Some(4));
            let unknown = node("unknown", 50, None);
            assert!(headroom_key(&unknown) < headroom_key(&full));
            assert_eq!(full.capacity.unwrap().calls_left(4), Some(0));
        }

        /// A node that advertises nothing is ranked on its calls, which
        /// is what every node was ranked on before.
        #[test]
        fn nodes_without_capacity_are_ranked_as_they_always_were() {
            let quiet = node("quiet", 1, None);
            let busy = node("busy", 9, None);
            assert!(headroom_key(&quiet) < headroom_key(&busy));
        }

        /// Room is what is left, not what was configured.
        #[test]
        fn room_is_what_is_left() {
            let capacity = NodeCapacity {
                max_concurrent_calls: 10,
                ..NodeCapacity::default()
            };
            assert_eq!(capacity.calls_left(3), Some(7));
            assert_eq!(capacity.calls_left(10), Some(0));
            assert_eq!(capacity.calls_left(99), Some(0), "never below none");
            assert_eq!(
                NodeCapacity::default().calls_left(3),
                None,
                "no limit is unknown room, not infinite room"
            );
        }
    }

    /// Where a draining browser-facing node sends its browsers
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    mod handover {
        use super::*;

        fn gateway(
            id: &str,
            region: Option<&str>,
            url: Option<&str>,
            calls: usize,
        ) -> ClusterMember {
            let mut m = member(id, region);
            m.node_type = "webrtc_gateway".to_string();
            m.public_url = url.map(str::to_owned);
            m.active_calls = calls;
            m
        }

        #[test]
        fn the_least_loaded_serving_peer_in_region_is_chosen() {
            let members = vec![
                gateway("gw-2", Some("us-east"), Some("https://gw2"), 9),
                gateway("gw-3", Some("us-east"), Some("https://gw3"), 2),
                gateway("gw-4", Some("us-west"), Some("https://gw4"), 0),
            ];
            let pick =
                choose_handover_node(&members, "webrtc_gateway", Some("us-east"), "gw-1").unwrap();
            assert_eq!(pick.node_id, "gw-3", "the emptier of the two in region");
        }

        /// A node with no peer in its own region still hands over rather
        /// than dropping the browser.
        #[test]
        fn another_region_is_better_than_nowhere() {
            let members = vec![gateway("gw-4", Some("us-west"), Some("https://gw4"), 0)];
            let pick =
                choose_handover_node(&members, "webrtc_gateway", Some("us-east"), "gw-1").unwrap();
            assert_eq!(pick.node_id, "gw-4");
        }

        /// A peer that is draining, has no browser-facing address, is of
        /// another kind, or is this node itself, is no place to send a
        /// browser.
        #[test]
        fn unusable_peers_are_skipped() {
            let mut draining = gateway("gw-2", Some("us-east"), Some("https://gw2"), 0);
            draining.draining = true;
            let mut wrong_kind = gateway("conf-1", Some("us-east"), Some("https://conf1"), 0);
            wrong_kind.node_type = "conference".to_string();
            let members = vec![
                draining,
                wrong_kind,
                gateway("gw-3", Some("us-east"), None, 0),
                gateway("gw-4", Some("us-east"), Some("   "), 0),
                gateway("gw-1", Some("us-east"), Some("https://gw1"), 0),
            ];
            assert!(
                choose_handover_node(&members, "webrtc_gateway", Some("us-east"), "gw-1").is_none(),
                "nothing here can take a browser"
            );
        }

        /// The field is optional on the wire: a node from before this
        /// existed simply takes no handovers.
        #[test]
        fn a_record_without_the_field_parses_and_takes_none() {
            let old = r#"{"node_id":"gw-9","node_type":"webrtc_gateway","http_address":"10.0.0.1:8443",
                "sip_address":"10.0.0.1:5060","capabilities":[],"active_calls":0,"started_at":1,
                "version":"0.25.0"}"#;
            let m: ClusterMember = serde_json::from_str(old).unwrap();
            assert_eq!(m.public_url, None);
            assert!(choose_handover_node(&[m], "webrtc_gateway", None, "gw-1").is_none());
        }
    }

    #[test]
    fn video_budget_is_optional_on_the_wire_and_read_per_codec() {
        // A record written before video budgets existed still parses, and
        // reads as "unknown".
        let old = r#"{"node_id":"conf-1","node_type":"conference","http_address":"10.0.0.1:8090",
            "sip_address":"10.0.0.1:5074","capabilities":[],"active_calls":0,"started_at":1,
            "version":"0.25.0","region":"us-east"}"#;
        let m: ClusterMember = serde_json::from_str(old).unwrap();
        assert!(m.video_budget.is_empty());
        assert_eq!(m.video_budget_left(&["H264".to_string()]), None);

        // Without a budget the field is omitted.
        let json = serde_json::to_string(&member("cm-1", None)).unwrap();
        assert!(!json.contains("video_budget"));

        let mut m = member("conf-2", None);
        m.video_budget = vec![
            VideoDeviceBudget {
                device: "host".into(),
                codecs: vec!["H264".into(), "VP8".into()],
                left: 700,
                total: 8000,
                engines: None,
            },
            VideoDeviceBudget {
                device: "vaapi:/dev/dri/renderD128".into(),
                codecs: vec!["H264".into()],
                left: 3000,
                total: 20000,
                engines: None,
            },
        ];
        let back: ClusterMember =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back.video_budget, m.video_budget);
        // The best device that has the codec; a codec no device runs is 0,
        // not unknown.
        assert_eq!(back.video_budget_left(&["H264".to_string()]), Some(3000));
        assert_eq!(back.video_budget_left(&["VP8".to_string()]), Some(700));
        assert_eq!(back.video_budget_left(&["AV1".to_string()]), Some(0));
        assert_eq!(back.video_budget_left(&[]), Some(3000));
    }

    #[test]
    fn a_device_s_engines_are_optional_on_the_wire_and_placement_reads_the_encoder_s() {
        // A budget written before engines existed: `engines` absent.
        let old = r#"{"device":"cuda:0","codecs":["H264"],"left":500,"total":800}"#;
        let b: VideoDeviceBudget = serde_json::from_str(old).unwrap();
        assert_eq!(b.engines, None);
        assert_eq!(b.placement_left(), 500);
        // Without engines the field is not written.
        assert!(!serde_json::to_string(&b).unwrap().contains("engines"));
        // With engines: placement weighs the encoder, `left` is the
        // tightest engine's for a reader that knows no better.
        let gpu = VideoDeviceBudget {
            device: "cuda:0".into(),
            codecs: vec!["H264".into()],
            left: 100,
            total: 800,
            engines: Some(VideoEngines {
                encode: EngineBudget {
                    left: 600,
                    total: 800,
                },
                decode: EngineBudget {
                    left: 790,
                    total: 800,
                },
                compose: EngineBudget {
                    left: 100,
                    total: 800,
                },
            }),
        };
        let back: VideoDeviceBudget =
            serde_json::from_str(&serde_json::to_string(&gpu).unwrap()).unwrap();
        assert_eq!(back, gpu);
        assert_eq!(back.placement_left(), 600);
        let mut m = member("conf-3", None);
        m.video_budget = vec![gpu];
        assert_eq!(m.video_budget_left(&["H264".to_string()]), Some(600));
        let need = crate::VideoNeed::of(vec!["H264".to_string()]);
        assert_eq!(m.video_budget_left_for(&need), Some(600));
    }

    #[test]
    fn region_is_optional_on_the_wire_and_absent_from_old_records() {
        // A record written before regions existed still parses.
        let old = r#"{"node_id":"cm-1","node_type":"call_manager","http_address":"10.0.0.1:8080",
            "sip_address":"10.0.0.1:5060","capabilities":[],"active_calls":0,"started_at":1,
            "version":"0.25.0"}"#;
        let m: ClusterMember = serde_json::from_str(old).unwrap();
        assert_eq!(m.region, None);
        assert!(!m.in_region("us-east"));

        // Without a region the field is omitted, so old readers see the
        // same shape they always did.
        let json = serde_json::to_string(&member("cm-1", None)).unwrap();
        assert!(!json.contains("region"));

        let json = serde_json::to_string(&member("cm-2", Some("eu-west"))).unwrap();
        let back: ClusterMember = serde_json::from_str(&json).unwrap();
        assert_eq!(back.region.as_deref(), Some("eu-west"));
        assert!(back.in_region("eu-west"));
        assert!(!back.in_region("us-east"));
    }
}

#[cfg(test)]
mod trunk_owner_tests {
    use super::*;

    fn advertiser(node_id: &str, trunks: &[&str], draining: bool) -> ClusterMember {
        ClusterMember {
            node_id: node_id.to_string(),
            node_type: "call_manager".to_string(),
            http_address: "127.0.0.1:8080".parse().unwrap(),
            sip_address: "127.0.0.1:5060".parse().unwrap(),
            sip_tls_address: None,
            capabilities: Vec::new(),
            active_calls: 0,
            started_at: 0,
            version: "test".to_string(),
            draining,
            region: None,
            public_url: None,
            capacity: None,
            video_budget: Vec::new(),
            registering_trunks: trunks.iter().map(|t| t.to_string()).collect(),
            view: None,
        }
    }

    /// Of the nodes that loaded the trunk, the lowest id registers it.
    #[test]
    fn the_lowest_node_id_that_loaded_it_registers_it() {
        let peers = vec![
            advertiser("node-a", &["trunk-1"], false),
            advertiser("node-c", &["trunk-1"], false),
        ];
        let owner = pick_trunk_owner("trunk-1", Some(("node-b", false)), peers.into_iter());
        assert_eq!(owner.as_deref(), Some("node-a"));
    }

    /// A node that did not load it is not a candidate, however low its id.
    #[test]
    fn a_node_that_did_not_load_it_is_not_a_candidate() {
        let peers = vec![advertiser("node-b", &["trunk-1"], false)];
        // This node has not loaded it, so it offers itself as no candidate.
        let owner = pick_trunk_owner("trunk-1", None, peers.into_iter());
        assert_eq!(owner.as_deref(), Some("node-b"));

        // And a peer that loaded some other trunk is no candidate either.
        let peers = vec![advertiser("node-a", &["trunk-9"], false)];
        let owner = pick_trunk_owner("trunk-1", Some(("node-b", false)), peers.into_iter());
        assert_eq!(owner.as_deref(), Some("node-b"));
    }

    /// The owner going away leaves the trunk to the next-lowest node that
    /// has it: what failover comes to, driven by the heartbeat expiring.
    #[test]
    fn the_next_node_takes_it_when_the_owner_goes() {
        let peers = vec![advertiser("node-a", &["trunk-1"], false)];
        assert_eq!(
            pick_trunk_owner("trunk-1", Some(("node-b", false)), peers.into_iter()).as_deref(),
            Some("node-a")
        );
        assert_eq!(
            pick_trunk_owner("trunk-1", Some(("node-b", false)), std::iter::empty()).as_deref(),
            Some("node-b"),
            "the heartbeat expired and the trunk is node-b's now"
        );
    }

    /// A draining node hands the binding over while it has a peer that can
    /// hold it, and keeps it when it is the last one that can — a lapsed
    /// binding is worse than one held by a node on its way out.
    #[test]
    fn draining_gives_the_binding_up_but_not_the_last_one() {
        let peers = vec![advertiser("node-z", &["trunk-1"], false)];
        assert_eq!(
            pick_trunk_owner("trunk-1", Some(("node-a", true)), peers.into_iter()).as_deref(),
            Some("node-z")
        );
        assert_eq!(
            pick_trunk_owner("trunk-1", Some(("node-a", true)), std::iter::empty()).as_deref(),
            Some("node-a")
        );
    }

    /// Two draining nodes still settle it the same way.
    #[test]
    fn the_lowest_id_settles_it_between_draining_nodes_too() {
        let peers = vec![advertiser("node-a", &["trunk-1"], true)];
        assert_eq!(
            pick_trunk_owner("trunk-1", Some(("node-b", true)), peers.into_iter()).as_deref(),
            Some("node-a")
        );
    }

    /// Nobody has it: nobody registers it.
    #[test]
    fn a_trunk_no_live_node_loaded_has_no_owner() {
        assert_eq!(pick_trunk_owner("trunk-1", None, std::iter::empty()), None);
    }
}
