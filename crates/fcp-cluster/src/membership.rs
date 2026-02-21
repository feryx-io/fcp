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
    /// Node capabilities (e.g. ["registrar", "b2bua", "queue"]).
    pub capabilities: Vec<String>,
    /// Number of active calls on this node.
    pub active_calls: usize,
    /// Node start time (Unix epoch seconds).
    pub started_at: u64,
    /// Software version.
    pub version: String,
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
        let self_info = self.self_info.clone();
        let members = Arc::clone(&self.members);
        let interval = self.heartbeat_interval_secs;
        let ttl = self.heartbeat_ttl_secs;
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let event_publisher = self.event_publisher.clone();

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

                // Publish heartbeat
                if let Err(e) = publish_heartbeat(&storage, &self_info, ttl).await {
                    error!(error = %e, "Failed to publish cluster heartbeat");
                }

                // Discover members
                if let Err(e) = discover_members(&storage, &self_info.node_id, &members, event_publisher.as_ref()).await {
                    error!(error = %e, "Failed to discover cluster members");
                }
            }
        })
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

    /// Get the least loaded member of a given type.
    pub fn least_loaded(&self, node_type: &str) -> Option<ClusterMember> {
        self.members
            .iter()
            .filter(|e| e.value().node_type == node_type)
            .min_by_key(|e| e.value().active_calls)
            .map(|e| e.value().clone())
    }

    /// Get the local node info.
    pub fn self_info(&self) -> &ClusterMember {
        &self.self_info
    }

    /// Update the active call count for this node.
    pub fn update_active_calls(&mut self, count: usize) {
        self.self_info.active_calls = count;
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
            let event = fcp_events::PlatformEvent::System(
                fcp_events::SystemEvent::NodeJoined {
                    node_id: member.node_id.clone(),
                    node_type: member.node_type.clone(),
                    address: member.http_address.to_string(),
                    timestamp: now,
                },
            );
            info!(node_id = %member.node_id, node_type = %member.node_type, "Cluster member joined");
            if let Err(e) = publisher.publish(event).await {
                warn!(error = %e, "Failed to publish NodeJoined event");
            }
        }

        for node_id in &departed {
            let event = fcp_events::PlatformEvent::System(
                fcp_events::SystemEvent::NodeLeft {
                    node_id: node_id.clone(),
                    reason: "heartbeat_expired".to_string(),
                    timestamp: now,
                },
            );
            info!(node_id = %node_id, "Cluster member left (heartbeat expired)");
            if let Err(e) = publisher.publish(event).await {
                warn!(error = %e, "Failed to publish NodeLeft event");
            }
        }
    }

    debug!(member_count = seen_ids.len(), "Cluster discovery complete");
    Ok(())
}
