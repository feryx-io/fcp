//! FCP Cluster — membership, call ownership, and inter-node communication.
//!
//! Uses Redis for lightweight cluster membership via heartbeat keys:
//! - Each node writes `cluster:node:{node_id}` with a 15s TTL, refreshed every 5s
//! - If a node crashes, its key expires and other nodes detect it
//! - Call ownership is tracked via `cluster:call:{call_id}` keys

pub mod call_registry;
pub mod membership;
pub mod node_client;

pub use call_registry::CallRegistry;
pub use membership::{ClusterManager, ClusterMember};
pub use node_client::NodeClient;
