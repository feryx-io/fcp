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

    /// Get aggregate statistics across all cluster nodes.
    pub async fn get_stats(&self) -> anyhow::Result<ClusterStats> {
        let nodes = self.list_nodes().await?;

        let mut healthy = 0usize;
        let mut degraded = 0usize;
        let mut offline = 0usize;
        let mut total_calls = 0usize;
        let mut by_type: HashMap<String, usize> = HashMap::new();

        for node in &nodes {
            match node.status {
                ClusterNodeStatus::Healthy => healthy += 1,
                ClusterNodeStatus::Degraded => degraded += 1,
                ClusterNodeStatus::Offline => offline += 1,
            }
            total_calls += node.active_calls;
            *by_type.entry(node.node_type.clone()).or_default() += 1;
        }

        Ok(ClusterStats {
            total_nodes: nodes.len(),
            healthy_nodes: healthy,
            degraded_nodes: degraded,
            offline_nodes: offline,
            total_active_calls: total_calls,
            nodes_by_type: by_type,
        })
    }
}
