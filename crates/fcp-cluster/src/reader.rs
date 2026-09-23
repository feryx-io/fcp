//! Read-only cluster state reader.
//!
//! Scans Redis `cluster:node:*` heartbeat keys and deserializes
//! [`ClusterMember`] JSON to provide a snapshot of all running nodes.
//! Unlike [`ClusterManager`], this does not write heartbeats or manage
//! membership — it is purely a reader for admin/monitoring use.

use crate::membership::ClusterMember;
use fcp_storage_redis::RedisStorage;
use redis::AsyncCommands;
use serde::Serialize;
use std::collections::HashMap;
use tracing::{debug, warn};

/// Read-only view of the cluster state from Redis.
#[derive(Clone)]
pub struct ClusterReader {
    storage: RedisStorage,
    /// Heartbeat TTL in seconds — keys older than this are considered degraded/offline.
    heartbeat_ttl_secs: u64,
}

/// Status of a cluster node, derived from its Redis key TTL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClusterNodeStatus {
    Healthy,
    Degraded,
    Offline,
}

/// Information about a single cluster node, enriched with derived fields.
#[derive(Debug, Clone, Serialize)]
pub struct ClusterNodeInfo {
    pub node_id: String,
    pub node_type: String,
    pub http_address: String,
    pub sip_address: String,
    pub capabilities: Vec<String>,
    pub active_calls: usize,
    pub started_at: u64,
    pub version: String,
    pub status: ClusterNodeStatus,
    pub uptime_seconds: u64,
    /// Shutting down; taking nothing new.
    pub draining: bool,
    /// Deployment region the node advertises, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Video budget per device, when the node advertises one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub video_budget: Vec<crate::membership::VideoDeviceBudget>,
    /// What the node will carry and what it is carrying, when it keeps
    /// limits (`docs/CAPACITY_AND_SURVIVABILITY.md` §7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<crate::membership::NodeCapacity>,
    /// Calls this node will still take, when it says what it allows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calls_left: Option<u32>,
    /// The node's own view of the cluster, when it does not look whole
    /// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<crate::view::ClusterView>,
}

/// Aggregate statistics across all cluster nodes.
#[derive(Debug, Clone, Serialize)]
pub struct ClusterStats {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub degraded_nodes: usize,
    pub offline_nodes: usize,
    pub total_active_calls: usize,
    pub nodes_by_type: HashMap<String, usize>,
    /// Nodes per advertised region; nodes without one are not counted here.
    pub nodes_by_region: HashMap<String, usize>,
    /// Nodes that advertise no region.
    pub nodes_without_region: usize,
    /// Nodes whose own view of the cluster does not look whole — most
    /// often every node of a region that has lost the others (§2.3).
    pub nodes_with_partial_view: usize,
}

impl ClusterReader {
    /// Create a new cluster reader.
    ///
    /// `heartbeat_ttl_secs` should match the TTL used by `ClusterManager` (default 15s).
    pub fn new(storage: RedisStorage, heartbeat_ttl_secs: u64) -> Self {
        Self {
            storage,
            heartbeat_ttl_secs,
        }
    }

    /// List all cluster nodes by scanning Redis heartbeat keys.
    ///
    /// Node status is derived from Redis TTL:
    /// - TTL > 2/3 of heartbeat_ttl → Healthy
    /// - TTL > 0 → Degraded (heartbeat overdue)
    /// - TTL expired / key gone → not returned (already cleaned up by Redis)
    pub async fn list_nodes(&self) -> anyhow::Result<Vec<ClusterNodeInfo>> {
        let mut conn = self.storage.connection();
        let pattern = self.storage.key("cluster:node:*");

        let mut nodes = Vec::new();
        let mut cursor = 0u64;

        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

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
                // GET the JSON value
                let json: Option<String> = conn
                    .get(key)
                    .await
                    .map_err(|e| anyhow::anyhow!("Redis GET failed for {}: {}", key, e))?;

                let Some(json) = json else { continue };

                let member = match serde_json::from_str::<ClusterMember>(&json) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(key = %key, error = %e, "Failed to deserialize cluster member");
                        continue;
                    }
                };

                // Check TTL to derive status
                let ttl: i64 = redis::cmd("TTL")
                    .arg(key)
                    .query_async(&mut conn)
                    .await
                    .unwrap_or(-1);

                let healthy_threshold = (self.heartbeat_ttl_secs as i64 * 2) / 3;
                let status = if ttl > healthy_threshold {
                    ClusterNodeStatus::Healthy
                } else if ttl > 0 {
                    ClusterNodeStatus::Degraded
                } else {
                    // TTL <= 0 means key is about to expire or has no TTL
                    ClusterNodeStatus::Offline
                };

                let uptime_seconds = now_epoch.saturating_sub(member.started_at);

                nodes.push(ClusterNodeInfo {
                    node_id: member.node_id,
                    node_type: member.node_type,
                    http_address: member.http_address.to_string(),
                    sip_address: member.sip_address.to_string(),
                    capabilities: member.capabilities,
                    active_calls: member.active_calls,
                    started_at: member.started_at,
                    version: member.version,
                    status,
                    uptime_seconds,
                    draining: member.draining,
                    region: member.region,
                    video_budget: member.video_budget,
                    calls_left: member
                        .capacity
                        .and_then(|c| c.calls_left(member.active_calls)),
                    capacity: member.capacity,
                    view: member.view,
                });
            }

            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }

        debug!(node_count = nodes.len(), "Cluster reader scan complete");
        Ok(nodes)
    }

    /// The cluster nodes in `region` (by advertised region label).
    pub async fn list_nodes_in_region(&self, region: &str) -> anyhow::Result<Vec<ClusterNodeInfo>> {
        Ok(self
            .list_nodes()
            .await?
            .into_iter()
            .filter(|n| n.region.as_deref() == Some(region))
            .collect())
    }

    /// Get aggregate statistics across all cluster nodes.
    pub async fn get_stats(&self) -> anyhow::Result<ClusterStats> {
        let nodes = self.list_nodes().await?;
        Ok(summarize(&nodes))
    }
}

/// Aggregate a node listing.
fn summarize(nodes: &[ClusterNodeInfo]) -> ClusterStats {
    let mut healthy = 0usize;
    let mut degraded = 0usize;
    let mut offline = 0usize;
    let mut total_calls = 0usize;
    let mut by_type: HashMap<String, usize> = HashMap::new();
    let mut by_region: HashMap<String, usize> = HashMap::new();
    let mut without_region = 0usize;
    let mut partial_view = 0usize;

    for node in nodes {
        if node.view.as_ref().is_some_and(|v| !v.is_whole()) {
            partial_view += 1;
        }
        match node.status {
            ClusterNodeStatus::Healthy => healthy += 1,
            ClusterNodeStatus::Degraded => degraded += 1,
            ClusterNodeStatus::Offline => offline += 1,
        }
        total_calls += node.active_calls;
        *by_type.entry(node.node_type.clone()).or_default() += 1;
        match &node.region {
            Some(region) => *by_region.entry(region.clone()).or_default() += 1,
            None => without_region += 1,
        }
    }

    ClusterStats {
        total_nodes: nodes.len(),
        healthy_nodes: healthy,
        degraded_nodes: degraded,
        offline_nodes: offline,
        total_active_calls: total_calls,
        nodes_by_type: by_type,
        nodes_by_region: by_region,
        nodes_without_region: without_region,
        nodes_with_partial_view: partial_view,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, node_type: &str, region: Option<&str>, calls: usize) -> ClusterNodeInfo {
        ClusterNodeInfo {
            node_id: id.to_string(),
            node_type: node_type.to_string(),
            http_address: "10.0.0.1:8080".to_string(),
            sip_address: "10.0.0.1:5060".to_string(),
            capabilities: Vec::new(),
            active_calls: calls,
            started_at: 0,
            version: "test".to_string(),
            status: ClusterNodeStatus::Healthy,
            capacity: None,
            calls_left: None,
            uptime_seconds: 0,
            draining: false,
            region: region.map(str::to_owned),
            video_budget: Vec::new(),
            view: None,
        }
    }

    #[test]
    fn stats_count_nodes_per_region_and_the_unlabeled() {
        let stats = summarize(&[
            node("cm-1", "call_manager", Some("us-east"), 3),
            node("conf-1", "conference", Some("us-east"), 10),
            node("cm-2", "call_manager", Some("eu-west"), 1),
            node("legacy", "call_manager", None, 0),
        ]);
        assert_eq!(stats.total_nodes, 4);
        assert_eq!(stats.total_active_calls, 14);
        assert_eq!(stats.nodes_by_region["us-east"], 2);
        assert_eq!(stats.nodes_by_region["eu-west"], 1);
        assert_eq!(stats.nodes_by_region.len(), 2);
        assert_eq!(stats.nodes_without_region, 1);
        assert_eq!(stats.nodes_by_type["call_manager"], 3);
        assert_eq!(stats.nodes_with_partial_view, 0);
    }

    #[test]
    fn stats_count_the_nodes_that_doubt_their_view() {
        let doubting = crate::view::ClusterView {
            state: crate::view::ViewState::RemoteRegionsLost,
            since: Some(0),
            lost_regions: vec!["eu-west".into()],
            view_age_secs: Some(1),
            database_reachable: Some(true),
            summary: "lost eu-west".into(),
        };
        let mut a = node("cm-1", "call_manager", Some("us-east"), 0);
        a.view = Some(doubting.clone());
        let mut b = node("conf-1", "conference", Some("us-east"), 0);
        b.view = Some(doubting);
        let stats = summarize(&[a, b, node("cm-2", "call_manager", Some("us-east"), 0)]);
        assert_eq!(stats.nodes_with_partial_view, 2);
    }
}
