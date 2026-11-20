//! A webinar's stage across nodes (`docs/WEBINARS.md` §2.7, W5).
//!
//! A webinar spread over several nodes is a tree, not a mesh. The stage
//! node holds its hosts and panelists; every other node hosting it is an
//! edge holding attendees, trunked to the stage alone. The stage is the hub:
//! its trunk to an edge carries the whole room but that edge, so a promoted
//! attendee on one edge is heard on every other.
//!
//! The stage is the `__stage` field of the room's registry entry. The first
//! host or panelist to join claims the node they are on. One who joins
//! anywhere else is moved to the stage by the drain's migration: a REFER
//! with a signed ticket, carried out by whoever holds their leg. Placement
//! cannot do this itself, because it places a caller before the room knows
//! their role.

use super::{ConferenceCallHandler, ConferenceRole, RoomKey};
use fcp_cluster::ClusterMember;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// What a claim on a webinar's stage found.
#[derive(Debug, Clone)]
pub enum StageClaim {
    /// No cluster: this node is the whole webinar.
    Alone,
    /// This node is the stage.
    Mine,
    /// Another live node is.
    Elsewhere(Box<ClusterMember>),
    /// The registry could not be read, or the holder is not a member this
    /// node knows: the caller stays where they are.
    Unknown,
}

/// How many times a host on an edge is referred to the stage before they
/// are left where they are.
const MOVE_ATTEMPTS: usize = 10;
/// The pause between attempts: the leg's dialog may still be confirming.
const MOVE_RETRY: Duration = Duration::from_millis(500);

impl ConferenceCallHandler {
    /// The flag a trunk's audio bridge reads to know this node is the hub
    /// of `key`'s trunks.
    pub(super) fn stage_hub(&self, key: &RoomKey) -> Arc<AtomicBool> {
        self.stage_hubs
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    /// Whether this node is the stage of `key`'s webinar.
    pub fn is_stage(&self, key: &RoomKey) -> bool {
        self.stage_hubs
            .get(key)
            .is_some_and(|h| h.load(Ordering::Relaxed))
    }

    /// This node's part in `key`'s webinar across nodes: `stage`, `edge`,
    /// or `None` on one node or in a meeting.
    pub fn webinar_node(&self, key: &RoomKey) -> Option<&'static str> {
        if self.is_stage(key) {
            Some("stage")
        } else if self.is_edge(key) {
            Some("edge")
        } else {
            None
        }
    }

    /// Whether `key`'s webinar has its stage on another node: this node is
    /// an edge, whose own stage is always empty.
    pub fn is_edge(&self, key: &RoomKey) -> bool {
        self.stage_edges.contains_key(key)
    }

    /// Record what the registry says of `key`'s stage: `Some(true)` this
    /// node, `Some(false)` another, `None` none. The trunk manager does
    /// this as it reads the registry, so a node that lost the stage stops
    /// relaying within one reconcile.
    pub fn set_stage_node(&self, key: &RoomKey, stage: Option<bool>) {
        let on = stage == Some(true);
        let was = self.stage_hub(key).swap(on, Ordering::Relaxed);
        let edge = stage == Some(false);
        let was_edge = if edge {
            self.stage_edges.insert(key.clone(), ()).is_some()
        } else {
            self.stage_edges.remove(key).is_some()
        };
        if was != on || was_edge != edge {
            info!(room = %key.room, stage = on, edge, "This node's part in a webinar changed");
        }
    }

    /// A host or panelist of a webinar has joined `key` as `call_id`: claim
    /// the stage for this node, or move them to the node that has it.
    pub(super) fn take_the_stage(&self, key: &RoomKey, call_id: &str, role: ConferenceRole) {
        if !matches!(role, ConferenceRole::Host | ConferenceRole::Panelist) {
            return;
        }
        let Some(registry) = self.room_registry.read().unwrap().clone() else {
            return;
        };
        let Some(me) = self.me() else {
            return;
        };
        let key = key.clone();
        let call_id = call_id.to_string();
        tokio::spawn(async move {
            match registry
                .claim_stage(&key.tenant.to_string(), &key.room)
                .await
            {
                StageClaim::Alone | StageClaim::Unknown => {}
                StageClaim::Mine => me.set_stage_node(&key, Some(true)),
                StageClaim::Elsewhere(stage) => {
                    me.set_stage_node(&key, Some(false));
                    me.move_to_stage(&key, &call_id, &stage).await;
                }
            }
        });
    }

    /// Refer `call_id` to the stage. The REFER may come before the leg's
    /// dialog is confirmed, so it is tried again for a few seconds.
    async fn move_to_stage(&self, key: &RoomKey, call_id: &str, stage: &ClusterMember) {
        for attempt in 1..=MOVE_ATTEMPTS {
            if !self.sessions.contains_key(call_id) {
                tokio::time::sleep(MOVE_RETRY).await;
                continue;
            }
            match self.refer_to_node(call_id, stage).await {
                Ok(()) => {
                    info!(room = %key.room, call_id, stage = %stage.node_id, "A host on an edge was moved to the webinar's stage");
                    return;
                }
                Err(e) if attempt < MOVE_ATTEMPTS => {
                    tracing::debug!(room = %key.room, call_id, attempt, error = %e, "Not yet referred to the stage");
                    tokio::time::sleep(MOVE_RETRY).await;
                }
                Err(e) => {
                    warn!(room = %key.room, call_id, stage = %stage.node_id, error = %e, "A host on an edge could not be moved to the stage; they stay");
                }
            }
        }
    }

    /// `key`'s room has ended on this node: give the stage up.
    pub(super) fn leave_the_stage(&self, key: &RoomKey) {
        self.stage_edges.remove(key);
        let held = self
            .stage_hubs
            .remove(key)
            .is_some_and(|(_, h)| h.load(Ordering::Relaxed));
        if !held {
            return;
        }
        let Some(registry) = self.room_registry.read().unwrap().clone() else {
            return;
        };
        let key = key.clone();
        tokio::spawn(async move {
            registry
                .release_stage(&key.tenant.to_string(), &key.room)
                .await;
        });
    }
}
