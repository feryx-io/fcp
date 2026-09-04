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
}

/// The region a node should advertise: the configured value when it is
/// non-blank, else the `FCP_REGION` environment variable, else none.
/// Surrounding whitespace is dropped.
pub fn resolve_region(configured: Option<&str>) -> Option<String> {
    configured
        .map(str::to_owned)
        .or_else(|| std::env::var("FCP_REGION").ok())
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
}

impl ClusterMember {
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
}

impl ClusterManager {
    /// Create a new cluster manager.
    pub fn new(storage: RedisStorage, self_info: ClusterMember) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
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
        }
    }

    /// Set custom heartbeat interval and TTL (builder pattern).
    pub fn with_intervals(mut self, interval_secs: u64, ttl_secs: u64) -> Self {
        self.heartbeat_interval_secs = interval_secs;
        self.heartbeat_ttl_secs = ttl_secs;
        self
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
                        if let Err(e) = remove_heartbeat(&storage, &self_info.node_id).await {
                            error!(error = %e, "Failed to remove heartbeat on shutdown");
                        }
                        break;
                    }
                }

                // Read latest active call count from shared counter
                self_info.active_calls = active_calls_counter.load(Ordering::Relaxed);
                self_info.draining = draining.load(Ordering::Relaxed);

                // Publish heartbeat
                if let Err(e) = publish_heartbeat(&storage, &self_info, ttl).await {
                    error!(error = %e, "Failed to publish cluster heartbeat");
                }

                // Discover members
                if let Err(e) = discover_members(
                    &storage,
                    &self_info.node_id,
                    &members,
                    event_publisher.as_ref(),
                )
                .await
                {
                    error!(error = %e, "Failed to discover cluster members");
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
        publish_heartbeat(&self.storage, &info, self.heartbeat_ttl_secs).await?;
        info!(node_id = %info.node_id, draining, "Cluster draining flag published");
        Ok(())
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

    /// Get the least loaded member of a given type, anywhere in the cluster.
    pub fn least_loaded(&self, node_type: &str) -> Option<ClusterMember> {
        self.members
            .iter()
            .filter(|e| e.value().node_type == node_type)
            .min_by_key(|e| (e.value().active_calls, e.value().node_id.clone()))
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
                .min_by_key(|e| (e.value().active_calls, e.value().node_id.clone()))
                .map(|e| e.value().clone());
            if local.is_some() {
                return local;
            }
        }
        self.least_loaded(node_type)
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

/// Remove this node's heartbeat key from Redis (graceful shutdown).
async fn remove_heartbeat(storage: &RedisStorage, node_id: &str) -> anyhow::Result<()> {
    let key = storage.key(&format!("cluster:node:{}", node_id));
    let mut conn = storage.connection();

    let _: () = redis::cmd("DEL")
        .arg(&key)
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
/// Emits `NodeJoined` / `NodeLeft` events when the set of known members changes.
async fn discover_members(
    storage: &RedisStorage,
    self_node_id: &str,
    members: &DashMap<String, ClusterMember>,
    event_publisher: Option<&Arc<dyn EventPublisher>>,
) -> anyhow::Result<()> {
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

    // Detect departed members before retain
    let departed: Vec<String> = members
        .iter()
        .filter(|e| !seen_ids.contains(e.key()))
        .map(|e| e.key().clone())
        .collect();

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

        for node_id in &departed {
            let event = fcp_events::PlatformEvent::System(fcp_events::SystemEvent::NodeLeft {
                node_id: node_id.clone(),
                reason: "heartbeat_expired".to_string(),
                timestamp: now,
            });
            info!(node_id = %node_id, "Cluster member left (heartbeat expired)");
            if let Err(e) = publisher.publish(event).await {
                warn!(error = %e, "Failed to publish NodeLeft event");
            }
        }
    }

    debug!(member_count = seen_ids.len(), "Cluster discovery complete");
    Ok(())
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
            draining: false,
            region: region.map(str::to_owned),
        }
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

    #[test]
    fn configured_region_is_trimmed_and_blank_means_none() {
        assert_eq!(
            resolve_region(Some(" us-east ")).as_deref(),
            Some("us-east")
        );
        assert_eq!(resolve_region(Some("   ")), None);
        // No configured value and no FCP_REGION in the test environment.
        // (If the environment sets one, that is what a node would use.)
        match std::env::var("FCP_REGION") {
            Ok(env) if !env.trim().is_empty() => {
                assert_eq!(resolve_region(None).as_deref(), Some(env.trim()))
            }
            _ => assert_eq!(resolve_region(None), None),
        }
    }
}
