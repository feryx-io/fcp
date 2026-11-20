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
use crate::roster::{Audio, Member, Roster};
use fcp_cluster::ClusterMember;
use fcp_core::webinar::WebinarPhase;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// Someone on a webinar's stage, or an attendee with their hand up, as one
/// node of the tree tells another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeMember {
    pub participant_id: String,
    pub name: String,
    #[serde(default)]
    pub host: bool,
    #[serde(default)]
    pub panelist: bool,
    /// When their hand went up, in milliseconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_at_ms: Option<u64>,
    /// `sending`, `muted` or `held`.
    #[serde(default)]
    pub audio: String,
    #[serde(default)]
    pub joined_at_ms: u64,
}

/// What a webinar's stage tells each of its edges, once a second: the
/// sitting, its phase, who is on the stage (theirs and every edge's
/// promoted attendees) and how many are in the audience on every node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageNews {
    pub node: String,
    pub sitting_id: String,
    pub phase: WebinarPhase,
    pub audience: usize,
    #[serde(default)]
    pub members: Vec<TreeMember>,
}

/// What an edge tells its stage, once a second: how many are in its
/// audience and its lobby, whose hands are up, and its attendees brought to
/// the stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeNews {
    pub node: String,
    pub audience: usize,
    #[serde(default)]
    pub lobby: usize,
    #[serde(default)]
    pub hands: Vec<TreeMember>,
    #[serde(default)]
    pub stage: Vec<TreeMember>,
}

/// Where a tree's news goes: from a stage to every edge, or from an edge to
/// its stage.
#[derive(Debug, Clone)]
pub enum TreeNews {
    Stage(StageNews),
    Edge { stage: String, news: EdgeNews },
}

fn millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn tree_member(id: &str, m: &Member) -> TreeMember {
    TreeMember {
        participant_id: id.to_string(),
        name: m.name.clone(),
        host: m.host,
        panelist: m.panelist,
        hand_at_ms: m.hand.map(millis),
        audio: match m.audio {
            Audio::Muted => "muted",
            Audio::Held => "held",
            Audio::Sending => "sending",
        }
        .to_string(),
        joined_at_ms: millis(m.joined_at),
    }
}

fn roster_member(t: &TreeMember, attendee: bool) -> Member {
    Member {
        name: t.name.clone(),
        host: t.host,
        panelist: t.panelist,
        attendee,
        hand: t
            .hand_at_ms
            .map(|ms| UNIX_EPOCH + Duration::from_millis(ms)),
        audio: match t.audio.as_str() {
            "muted" => Audio::Muted,
            "held" => Audio::Held,
            _ => Audio::Sending,
        },
        video: None,
        speaking: false,
        presenting: false,
        spotlight: false,
        stream: None,
        speaker_stream: None,
        joined_at: UNIX_EPOCH + Duration::from_millis(t.joined_at_ms),
    }
}

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

    /// The node holding the stage of `key`'s webinar, when this node is
    /// one of its edges.
    pub fn stage_of(&self, key: &RoomKey) -> Option<String> {
        self.stage_edges.get(key).map(|s| s.clone())
    }

    /// Record what the registry says of `key`'s stage: the node holding it
    /// (this one or another), or `None`. The trunk manager does this as it
    /// reads the registry, so a node that lost the stage stops relaying
    /// within one reconcile.
    pub fn set_stage_node(&self, key: &RoomKey, stage: Option<&str>) {
        let on = stage == Some(self.node_id.as_str());
        let was = self.stage_hub(key).swap(on, Ordering::Relaxed);
        let edge = stage.is_some() && !on;
        let was_edge = match stage.filter(|_| edge) {
            Some(stage) => self
                .stage_edges
                .insert(key.clone(), stage.to_string())
                .is_some(),
            None => self.stage_edges.remove(key).is_some(),
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
                StageClaim::Mine => {
                    let node = me.node_id.clone();
                    me.set_stage_node(&key, Some(&node));
                }
                StageClaim::Elsewhere(stage) => {
                    me.set_stage_node(&key, Some(&stage.node_id));
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
        self.stage_news.remove(key);
        self.edge_news.retain(|(k, _), _| k != key);
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

impl ConferenceCallHandler {
    /// The rooms this node is a stage or an edge of.
    pub fn tree_rooms(&self) -> Vec<RoomKey> {
        let mut keys: Vec<RoomKey> = self
            .stage_hubs
            .iter()
            .filter(|h| h.load(Ordering::Relaxed))
            .map(|h| h.key().clone())
            .collect();
        keys.extend(self.stage_edges.iter().map(|e| e.key().clone()));
        keys
    }

    /// What this node tells the rest of `key`'s tree now.
    pub fn tree_news(&self, key: &RoomKey) -> Option<TreeNews> {
        if self.is_stage(key) {
            let (sitting_id, phase) = self.sitting_news(key)?;
            let roster = self.roster_of(key)?;
            Some(TreeNews::Stage(StageNews {
                node: self.node_id.clone(),
                sitting_id,
                phase,
                audience: roster.audience,
                members: roster
                    .members
                    .iter()
                    .map(|(id, m)| tree_member(id, m))
                    .collect(),
            }))
        } else if let Some(stage) = self.stage_of(key) {
            let roster = self.local_roster(key)?;
            Some(TreeNews::Edge {
                stage,
                news: EdgeNews {
                    node: self.node_id.clone(),
                    audience: roster.audience,
                    lobby: self.lobby_count(key),
                    hands: roster
                        .hands
                        .iter()
                        .map(|(id, m)| tree_member(id, m))
                        .collect(),
                    stage: roster
                        .members
                        .iter()
                        .map(|(id, m)| tree_member(id, m))
                        .collect(),
                },
            })
        } else {
            None
        }
    }

    /// The stage of `key` sent its news. Kept for this edge's rosters, and
    /// its sitting and phase followed. Refused (`false`) where this node is
    /// not that stage's edge: a stale stage still announcing.
    pub fn apply_stage_news(&self, key: &RoomKey, news: StageNews) -> bool {
        if self.stage_of(key).as_deref() != Some(news.node.as_str()) {
            return false;
        }
        self.follow_stage(key, &news.sitting_id, news.phase);
        self.stage_news.insert(key.clone(), (news, Instant::now()));
        true
    }

    /// An edge of `key` sent its news. Kept for the stage's rosters.
    /// Refused (`false`) where this node is not the stage.
    pub fn apply_edge_news(&self, key: &RoomKey, news: EdgeNews) -> bool {
        if !self.is_stage(key) {
            return false;
        }
        self.edge_news
            .insert((key.clone(), news.node.clone()), (news, Instant::now()));
        true
    }

    /// `key`'s webinar ended here: when this node is its stage, its last
    /// news (the phase `ended`) is queued for the edges.
    pub(super) fn queue_tree_ending(&self, key: &RoomKey) {
        if let Some(TreeNews::Stage(news)) = self.tree_news(key) {
            self.tree_endings.insert(key.clone(), news);
        }
    }

    /// The endings waiting to be told to their edges.
    pub fn take_tree_endings(&self) -> Vec<(RoomKey, StageNews)> {
        let keys: Vec<RoomKey> = self.tree_endings.iter().map(|e| e.key().clone()).collect();
        keys.into_iter()
            .filter_map(|k| self.tree_endings.remove(&k))
            .collect()
    }

    /// `key`'s tree for the webinar's JSON: on the stage, each edge as it
    /// last said; on an edge, its stage. `None` off a tree.
    pub(super) fn tree_json(&self, key: &RoomKey) -> Option<serde_json::Value> {
        if self.is_stage(key) {
            let mut edges: Vec<serde_json::Value> = self
                .edge_news
                .iter()
                .filter(|e| &e.key().0 == key)
                .map(|e| {
                    let (news, _) = e.value();
                    serde_json::json!({
                        "node": news.node,
                        "audience": news.audience,
                        "lobby": news.lobby,
                        "hands": news.hands.iter().map(|h| &h.participant_id).collect::<Vec<_>>(),
                        "stage": news.stage.iter().map(|m| &m.participant_id).collect::<Vec<_>>(),
                    })
                })
                .collect();
            edges.sort_by(|a, b| a["node"].as_str().cmp(&b["node"].as_str()));
            Some(serde_json::json!({ "role": "stage", "edges": edges }))
        } else {
            let stage = self.stage_of(key)?;
            let heard = self.stage_news.contains_key(key);
            Some(serde_json::json!({ "role": "edge", "stage": stage, "heard": heard }))
        }
    }

    /// Drop news not refreshed within `max_age`: its node has gone quiet.
    pub fn expire_tree_news(&self, max_age: Duration) {
        self.stage_news.retain(|_, (_, at)| at.elapsed() <= max_age);
        self.edge_news.retain(|_, (_, at)| at.elapsed() <= max_age);
    }

    /// `roster` with the rest of the tree laid on it: on an edge, the
    /// stage's members and the whole audience; on the stage, each edge's
    /// audience, hands and promoted attendees.
    pub(super) fn tree_roster(&self, key: &RoomKey, mut roster: Roster) -> Roster {
        if let Some(news) = self.stage_news.get(key) {
            let (news, _) = news.value();
            for m in &news.members {
                roster
                    .members
                    .entry(m.participant_id.clone())
                    .or_insert_with(|| roster_member(m, false));
            }
            // The stage counts this edge's audience in its own.
            roster.audience = news.audience;
        }
        if self.is_stage(key) {
            for e in self.edge_news.iter().filter(|e| &e.key().0 == key) {
                let (news, _) = e.value();
                roster.audience += news.audience;
                for m in &news.stage {
                    roster
                        .members
                        .entry(m.participant_id.clone())
                        .or_insert_with(|| roster_member(m, false));
                }
                for m in &news.hands {
                    roster
                        .hands
                        .entry(m.participant_id.clone())
                        .or_insert_with(|| roster_member(m, true));
                }
            }
        }
        roster
    }
}
