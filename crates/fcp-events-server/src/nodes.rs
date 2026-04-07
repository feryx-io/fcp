//! Node registry for tracking call-manager nodes.

use dashmap::DashMap;
use serde::Serialize;
use std::time::{Duration, SystemTime};

/// Status of a registered node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    /// Node is healthy (recent heartbeat)
    Healthy,
    /// Node may be degraded (heartbeat delayed)
    Degraded,
    /// Node is offline (no heartbeat for too long)
    Offline,
}

/// Type of node (for future expansion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    /// SIP call manager node
    CallManager,
    /// Session manager (trunk termination)
    #[allow(dead_code)]
    SessionManager,
    /// Unknown/generic node
    Unknown,
}

impl Default for NodeType {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Information about a registered node.
#[derive(Debug, Clone, Serialize)]
pub struct NodeInfo {
    /// Unique node identifier
    pub node_id: String,
    /// Type of node
    pub node_type: NodeType,
    /// When the node first connected
    #[serde(with = "system_time_serde")]
    pub first_seen: SystemTime,
    /// When we last heard from this node
    #[serde(with = "system_time_serde")]
    pub last_seen: SystemTime,
    /// Number of active calls on this node
    pub active_calls: usize,
    /// Current health status
    pub status: NodeStatus,
}

/// Registry for tracking call-manager nodes.
pub struct NodeRegistry {
    nodes: DashMap<String, NodeInfo>,
    /// Duration after which a node is considered degraded
    degraded_threshold: Duration,
    /// Duration after which a node is considered offline
    offline_threshold: Duration,
}

impl NodeRegistry {
    /// Create a new node registry with default thresholds.
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
            degraded_threshold: Duration::from_secs(30),
            offline_threshold: Duration::from_secs(60),
        }
    }

    /// Create a new node registry with custom thresholds.
    #[allow(dead_code)]
    pub fn with_thresholds(degraded_threshold: Duration, offline_threshold: Duration) -> Self {
        Self {
            nodes: DashMap::new(),
            degraded_threshold,
            offline_threshold,
        }
    }

    /// Register a node or update its last_seen timestamp.
    pub fn register_or_update(&self, node_id: &str, node_type: Option<NodeType>) {
        let now = SystemTime::now();

        self.nodes
            .entry(node_id.to_string())
            .and_modify(|info| {
                info.last_seen = now;
                info.status = NodeStatus::Healthy;
                if let Some(ref nt) = node_type {
                    if *nt != NodeType::Unknown {
                        info.node_type = nt.clone();
                    }
                }
            })
            .or_insert_with(|| NodeInfo {
                node_id: node_id.to_string(),
                node_type: node_type.unwrap_or_default(),
                first_seen: now,
                last_seen: now,
                active_calls: 0,
                status: NodeStatus::Healthy,
            });
    }

    /// Update the active call count for a node.
    #[allow(dead_code)]
    pub fn update_call_count(&self, node_id: &str, count: usize) {
        if let Some(mut info) = self.nodes.get_mut(node_id) {
            info.active_calls = count;
            info.last_seen = SystemTime::now();
        }
    }

    /// Increment the active call count for a node.
    pub fn increment_calls(&self, node_id: &str) {
        if let Some(mut info) = self.nodes.get_mut(node_id) {
            info.active_calls = info.active_calls.saturating_add(1);
            info.last_seen = SystemTime::now();
        } else {
            // Auto-register node with one call
            self.register_or_update(node_id, None);
            if let Some(mut info) = self.nodes.get_mut(node_id) {
                info.active_calls = 1;
            }
        }
    }

    /// Decrement the active call count for a node.
    pub fn decrement_calls(&self, node_id: &str) {
        if let Some(mut info) = self.nodes.get_mut(node_id) {
            info.active_calls = info.active_calls.saturating_sub(1);
            info.last_seen = SystemTime::now();
        }
    }

    /// Get all registered nodes.
    pub fn get_all_nodes(&self) -> Vec<NodeInfo> {
        self.nodes
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get a specific node by ID.
    #[allow(dead_code)]
    pub fn get_node(&self, node_id: &str) -> Option<NodeInfo> {
        self.nodes.get(node_id).map(|entry| entry.value().clone())
    }

    /// Check for stale nodes and update their status.
    /// Returns the number of nodes marked offline.
    pub fn check_stale_nodes(&self) -> usize {
        let now = SystemTime::now();
        let mut offline_count = 0;

        for mut entry in self.nodes.iter_mut() {
            if let Ok(elapsed) = now.duration_since(entry.last_seen) {
                let new_status = if elapsed > self.offline_threshold {
                    offline_count += 1;
                    NodeStatus::Offline
                } else if elapsed > self.degraded_threshold {
                    NodeStatus::Degraded
                } else {
                    NodeStatus::Healthy
                };

                if entry.status != new_status {
                    entry.status = new_status;
                }
            }
        }

        offline_count
    }

    /// Remove nodes that have been offline for longer than the specified duration.
    /// Returns the number of nodes removed.
    pub fn remove_stale_nodes(&self, max_offline_duration: Duration) -> usize {
        let now = SystemTime::now();
        let mut removed = 0;

        self.nodes.retain(|_, info| {
            if info.status != NodeStatus::Offline {
                return true;
            }

            if let Ok(elapsed) = now.duration_since(info.last_seen) {
                if elapsed > max_offline_duration {
                    removed += 1;
                    return false;
                }
            }
            true
        });

        removed
    }

    /// Get the total number of registered nodes.
    #[allow(dead_code)]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the number of healthy nodes.
    #[allow(dead_code)]
    pub fn healthy_node_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|entry| entry.status == NodeStatus::Healthy)
            .count()
    }

    /// Get aggregated statistics about nodes.
    pub fn get_stats(&self) -> NodeStats {
        let mut stats = NodeStats::default();

        for entry in self.nodes.iter() {
            stats.total += 1;
            stats.total_calls += entry.active_calls;

            match entry.status {
                NodeStatus::Healthy => stats.healthy += 1,
                NodeStatus::Degraded => stats.degraded += 1,
                NodeStatus::Offline => stats.offline += 1,
            }
        }

        stats
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about registered nodes.
#[derive(Debug, Clone, Default, Serialize)]
pub struct NodeStats {
    /// Total number of nodes
    pub total: usize,
    /// Number of healthy nodes
    pub healthy: usize,
    /// Number of degraded nodes
    pub degraded: usize,
    /// Number of offline nodes
    pub offline: usize,
    /// Total active calls across all nodes
    pub total_calls: usize,
}

/// Serde module for SystemTime serialization.
mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time
            .duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?;
        duration.as_secs().serialize(serializer)
    }

    #[allow(dead_code)]
    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_node() {
        let registry = NodeRegistry::new();
        registry.register_or_update("node-1", Some(NodeType::CallManager));

        assert_eq!(registry.node_count(), 1);
        let node = registry.get_node("node-1").unwrap();
        assert_eq!(node.node_type, NodeType::CallManager);
        assert_eq!(node.status, NodeStatus::Healthy);
    }

    #[test]
    fn test_update_call_count() {
        let registry = NodeRegistry::new();
        registry.register_or_update("node-1", None);

        registry.increment_calls("node-1");
        assert_eq!(registry.get_node("node-1").unwrap().active_calls, 1);

        registry.increment_calls("node-1");
        assert_eq!(registry.get_node("node-1").unwrap().active_calls, 2);

        registry.decrement_calls("node-1");
        assert_eq!(registry.get_node("node-1").unwrap().active_calls, 1);
    }

    #[test]
    fn test_auto_register_on_increment() {
        let registry = NodeRegistry::new();

        // Node doesn't exist yet
        assert!(registry.get_node("node-1").is_none());

        // Incrementing auto-registers
        registry.increment_calls("node-1");

        let node = registry.get_node("node-1").unwrap();
        assert_eq!(node.active_calls, 1);
    }

    #[test]
    fn test_stale_nodes() {
        let registry =
            NodeRegistry::with_thresholds(Duration::from_millis(50), Duration::from_millis(100));

        registry.register_or_update("node-1", None);

        // Initially healthy
        assert_eq!(
            registry.get_node("node-1").unwrap().status,
            NodeStatus::Healthy
        );

        // Wait and check
        std::thread::sleep(Duration::from_millis(60));
        registry.check_stale_nodes();
        assert_eq!(
            registry.get_node("node-1").unwrap().status,
            NodeStatus::Degraded
        );

        // Wait more
        std::thread::sleep(Duration::from_millis(50));
        registry.check_stale_nodes();
        assert_eq!(
            registry.get_node("node-1").unwrap().status,
            NodeStatus::Offline
        );
    }

    #[test]
    fn test_stats() {
        let registry = NodeRegistry::new();

        registry.register_or_update("node-1", None);
        registry.register_or_update("node-2", None);

        registry.update_call_count("node-1", 5);
        registry.update_call_count("node-2", 3);

        let stats = registry.get_stats();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.healthy, 2);
        assert_eq!(stats.total_calls, 8);
    }
}
