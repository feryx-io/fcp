//! FCP Cluster — membership, call ownership, and inter-node communication.
//!
//! Uses Redis for lightweight cluster membership via heartbeat keys:
//! - Each node writes `cluster:node:{node_id}` with a 15s TTL, refreshed every 5s
//! - If a node crashes, its key expires and other nodes detect it
//! - Call ownership is tracked via `cluster:call:{call_id}` keys
//! - Every process, member or not, writes `service:{id}` for the admin's
//!   Services page (`services`, `docs/SERVICES.md`)
//! - Each node may advertise a deployment region; placement prefers the
//!   caller's region (`docs/MULTI_REGION.md`)

pub mod call_leases;
pub mod call_registry;
pub mod conference_registry;
pub mod line_appearances;
pub mod membership;
pub mod node_client;
pub mod reader;
pub mod services;
pub mod trunk_registry;
pub mod versions;
pub mod view;

pub use call_leases::{CallLeases, LeaseSubject, DEFAULT_LEASE_TTL_SECS};
pub use call_registry::CallRegistry;
pub use conference_registry::{
    choose_conference_node, ConferenceHost, ConferenceRegistry, RoomRecord, VideoNeed,
    CONFERENCE_NODE_TYPE,
};
pub use fcp_core::transport::advertised_address;
pub use line_appearances::LineAppearances;
pub use membership::{
    choose_handover_node, resolve_region, tenant_probe, ClusterManager, ClusterMember,
    DatabaseProbe, EngineBudget, NodeCapacity, VideoBudgetHandle, VideoDeviceBudget, VideoEngines,
};
pub use node_client::NodeClient;
pub use reader::{ClusterNodeInfo, ClusterNodeStatus, ClusterReader, ClusterStats};
pub use services::{
    BeaconHandle, Check, ServiceBeacon, ServiceIdentity, ServiceRecord, SERVICE_INTERVAL_SECS,
    SERVICE_TTL_SECS,
};
pub use trunk_registry::TrunkRegistry;
pub use versions::{ClusterVersions, VersionGroups, VersionTracker};
pub use view::{ClusterView, ViewState};
