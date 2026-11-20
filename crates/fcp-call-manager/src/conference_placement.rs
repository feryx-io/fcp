//! Cluster-aware conference placement for the B2BUA.
//!
//! Implements [`fcp_sip::ConferenceNodeSelector`] on top of the conference
//! registry (which nodes host a room, with how many participants) and live
//! cluster membership: a new participant stays on a node already hosting
//! the room while that node is under `conference_node_capacity`, otherwise
//! goes to the least-loaded conference node, preferring this node's region
//! at every step and, for a video meeting, the node with the most video
//! budget left for its codecs. See `docs/DISTRIBUTED_CONFERENCING.md`,
//! `docs/MULTI_REGION.md` and `docs/VIDEO_CONFERENCING.md` §9.3.

use dashmap::DashMap;
use fcp_cluster::{
    choose_conference_node, choose_webinar_node, ClusterMember, ConferenceRegistry, RoomRecord,
    VideoNeed,
};
use fcp_core::TenantId;
use fcp_sip::ConferenceVideoNeed;
use std::sync::Arc;
use tracing::{debug, warn};

pub struct ClusterConferenceSelector {
    registry: ConferenceRegistry,
    members: Arc<DashMap<String, ClusterMember>>,
    capacity: usize,
    /// Percent of `capacity` a webinar's stage takes of its attendees
    /// before they go to edges (`docs/WEBINARS.md` §2.7).
    stage_share_percent: u8,
    /// This node's region: participants it places are joining from here.
    local_region: Option<String>,
    /// Every address a participant was placed on: a conference node's,
    /// so one no live member has is a node that has gone.
    placed_on: DashMap<String, ()>,
}

impl ClusterConferenceSelector {
    pub fn new(
        registry: ConferenceRegistry,
        members: Arc<DashMap<String, ClusterMember>>,
        capacity: usize,
        local_region: Option<String>,
    ) -> Self {
        Self {
            registry,
            members,
            capacity,
            stage_share_percent: DEFAULT_STAGE_SHARE_PERCENT,
            local_region,
            placed_on: DashMap::new(),
        }
    }
}

/// What a webinar's stage takes of `conference_node_capacity` unless told.
pub const DEFAULT_STAGE_SHARE_PERCENT: u8 = 50;

impl ClusterConferenceSelector {
    /// Set the stage's share (`[b2bua] conference_webinar_stage_share`).
    pub fn with_stage_share_percent(mut self, percent: u8) -> Self {
        self.stage_share_percent = percent.min(100);
        self
    }
}

#[async_trait::async_trait]
impl fcp_sip::ConferenceNodeSelector for ClusterConferenceSelector {
    async fn select_conference_node(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
        video: Option<&ConferenceVideoNeed>,
        webinar: bool,
    ) -> Option<String> {
        let record = match self
            .registry
            .room(&tenant_id.to_string(), room_number)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, room = %room_number, "Conference registry lookup failed; placing by load only");
                RoomRecord::default()
            }
        };
        let hosts = &record.hosts;
        let members: Vec<ClusterMember> = self.members.iter().map(|e| e.value().clone()).collect();
        let need = video.map(|v| VideoNeed::of(v.codecs.clone()));
        let pick = if webinar {
            choose_webinar_node(
                &record,
                &members,
                self.capacity,
                self.stage_share_percent,
                self.local_region.as_deref(),
                need.as_ref(),
            )
        } else {
            choose_conference_node(
                hosts,
                &members,
                self.capacity,
                self.local_region.as_deref(),
                need.as_ref(),
            )
        }?;
        debug!(
            room = %room_number,
            node = %pick.node_id,
            video = ?need.as_ref().map(|n| &n.codecs),
            video_budget_left = ?need.as_ref().and_then(|n| pick.video_budget_left_for(n)),
            node_region = pick.region.as_deref().unwrap_or("-"),
            local_region = self.local_region.as_deref().unwrap_or("-"),
            hosting = hosts.len(),
            webinar,
            stage = record.stage.as_deref().unwrap_or("-"),
            capacity = self.capacity,
            "Conference participant placed"
        );
        let host = pick.sip_address.to_string();
        self.placed_on.insert(host.clone(), ());
        Some(host)
    }

    async fn conference_node_gone(&self, host: &str) -> bool {
        self.placed_on.contains_key(host)
            && !self
                .members
                .iter()
                .any(|m| m.value().sip_address.to_string() == host)
    }
}
