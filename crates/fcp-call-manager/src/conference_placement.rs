//! Cluster-aware conference placement for the B2BUA.
//!
//! Implements [`fcp_sip::ConferenceNodeSelector`] on top of the conference
//! registry (which nodes host a room, with how many participants) and live
//! cluster membership: a new participant stays on a node already hosting
//! the room while that node is under `conference_node_capacity`, otherwise
//! goes to the least-loaded conference node. See
//! `docs/DISTRIBUTED_CONFERENCING.md`.

use dashmap::DashMap;
use fcp_cluster::{choose_conference_node, ClusterMember, ConferenceRegistry};
use fcp_core::TenantId;
use std::sync::Arc;
use tracing::{debug, warn};

pub struct ClusterConferenceSelector {
    registry: ConferenceRegistry,
    members: Arc<DashMap<String, ClusterMember>>,
    capacity: usize,
}

impl ClusterConferenceSelector {
    pub fn new(
        registry: ConferenceRegistry,
        members: Arc<DashMap<String, ClusterMember>>,
        capacity: usize,
    ) -> Self {
        Self {
            registry,
            members,
            capacity,
        }
    }
}

#[async_trait::async_trait]
impl fcp_sip::ConferenceNodeSelector for ClusterConferenceSelector {
    async fn select_conference_node(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
    ) -> Option<String> {
        let hosts = match self
            .registry
            .hosts(&tenant_id.to_string(), room_number)
            .await
        {
            Ok(h) => h,
            Err(e) => {
                warn!(error = %e, room = %room_number, "Conference registry lookup failed; placing by load only");
                Vec::new()
            }
        };
        let members: Vec<ClusterMember> = self.members.iter().map(|e| e.value().clone()).collect();
        let pick = choose_conference_node(&hosts, &members, self.capacity)?;
        debug!(
            room = %room_number,
            node = %pick.node_id,
            hosting = hosts.len(),
            capacity = self.capacity,
            "Conference participant placed"
        );
        Some(pick.sip_address.to_string())
    }
}
