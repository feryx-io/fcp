//! Inter-node conference trunks (distributed conferencing, phase 2).
//!
//! When two conference nodes host the same room, the node with the lower
//! id dials the other over SIP/TLS with a trunk header, and the callee
//! admits the call as a hidden `__trunk__<node>` participant that skips
//! every caller-facing admission check — PIN, lock, waiting room,
//! capacity. It can skip them because the connection itself is the
//! proof of who is calling: mutual TLS, with a client certificate that
//! names the node. Nothing in the SIP message is trusted for that.
//!
//! Each trunk carries only the sending node's local participants. The
//! room's frame-clock mixer produces that send-mix by excluding every
//! trunk (and the audio-feedback channel), so a full mesh never carries
//! audio back to the node it came from and no participant is heard
//! twice.
//!
//! Trunks follow the conference registry: a peer that starts hosting the
//! room is dialed within one reconcile interval, and a trunk is hung up
//! when its peer stops hosting the room, leaves cluster membership, or
//! this node's last local participant leaves. A trunk that has gone
//! silent for `cluster.trunk_rtp_timeout_secs` — a peer sends a frame
//! every 20 ms while its side is up — is torn down and dialed again, so
//! a peer that crashed and came back inside the membership TTL, or lost
//! the session without a BYE, does not leave a dead trunk behind.

use crate::conference_handler::ConferenceCallHandler;
use crate::rooms::RoomKey;
use dashmap::DashMap;
use fcp_cluster::{ClusterMember, ConferenceRegistry, CONFERENCE_NODE_TYPE};
use fcp_core::TenantId;
use fcp_sip::PeerIdentity;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Mixer id prefix of a trunk participant; the rest is the peer node id.
pub const TRUNK_PARTICIPANT_PREFIX: &str = "__trunk__";
/// Header a trunk INVITE carries with the dialing node's id. Never
/// trusted on its own: the peer certificate has to name the same node.
pub const TRUNK_NODE_HEADER: &str = "X-FCP-Trunk-Node";
/// Header a trunk INVITE carries with the room's tenant id.
pub const TRUNK_TENANT_HEADER: &str = "X-FCP-Trunk-Tenant";

/// How long a failed trunk dial is left alone before it is retried.
const DIAL_RETRY_AFTER: Duration = Duration::from_secs(10);
/// A dial still marked in flight after this long is presumed lost.
const DIAL_STALE_AFTER: Duration = Duration::from_secs(120);

/// Mixer participant id for the trunk to `node_id`.
pub fn trunk_participant_id(node_id: &str) -> String {
    format!("{TRUNK_PARTICIPANT_PREFIX}{node_id}")
}

/// Whether a mixer participant id is an inter-node trunk.
pub fn is_trunk_participant(id: &str) -> bool {
    id.starts_with(TRUNK_PARTICIPANT_PREFIX)
}

/// The peer node behind a trunk's participant id, if it is one.
pub fn trunk_peer_node(id: &str) -> Option<&str> {
    id.strip_prefix(TRUNK_PARTICIPANT_PREFIX)
}

/// Whether a verified peer certificate identifies `node_id`: as its
/// Common Name, as a DNS SAN, or as the user or host part of a `sip:` /
/// `sips:` URI SAN (RFC 5922 §7.1). Case-insensitive, no wildcards.
pub fn identity_names_node(identity: &PeerIdentity, node_id: &str) -> bool {
    if node_id.is_empty() {
        return false;
    }
    if identity
        .common_name()
        .map(|cn| cn.eq_ignore_ascii_case(node_id))
        .unwrap_or(false)
    {
        return true;
    }
    if identity.has_dns_name(node_id) {
        return true;
    }
    identity.uri_names().iter().any(|uri| {
        let Some(rest) = uri
            .strip_prefix("sip:")
            .or_else(|| uri.strip_prefix("sips:"))
        else {
            return false;
        };
        let rest = rest.split([';', '?']).next().unwrap_or(rest);
        match rest.split_once('@') {
            Some((user, host)) => {
                user.eq_ignore_ascii_case(node_id) || host.eq_ignore_ascii_case(node_id)
            }
            None => rest.eq_ignore_ascii_case(node_id),
        }
    })
}

/// The dialing side of a trunk between two nodes: the lower node id.
pub fn dials(my_node: &str, peer: &str) -> bool {
    my_node < peer
}

/// What a reconcile pass decided for one room.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TrunkPlan {
    /// Peers to dial (this node is the lower id and has no trunk yet).
    pub dial: Vec<String>,
    /// Peers whose trunk to hang up (either direction).
    pub hangup: Vec<String>,
}

/// Decide the trunks for a room from what the registry says and what
/// this node has.
///
/// `hosting_peers` are the other nodes the registry reports hosting the
/// room (already filtered to live conference nodes); `trunked_peers` are
/// the nodes this node currently has a trunk with, whichever side dialed.
/// With no local participants every trunk goes: there is nothing to
/// send, and the peers' own reconcile takes care of theirs.
pub fn plan_trunks(
    my_node: &str,
    local_participants: usize,
    hosting_peers: &[String],
    trunked_peers: &[String],
) -> TrunkPlan {
    let mut plan = TrunkPlan::default();
    if local_participants == 0 {
        plan.hangup = trunked_peers.to_vec();
    } else {
        for peer in hosting_peers {
            if peer != my_node && dials(my_node, peer) && !trunked_peers.contains(peer) {
                plan.dial.push(peer.clone());
            }
        }
        for peer in trunked_peers {
            if !hosting_peers.contains(peer) {
                plan.hangup.push(peer.clone());
            }
        }
    }
    plan.dial.sort();
    plan.dial.dedup();
    plan.hangup.sort();
    plan.hangup.dedup();
    plan
}

type TrunkKey = (String, String, String);

/// Keeps this node's trunks in step with the conference registry.
pub struct TrunkManager {
    node_id: String,
    registry: ConferenceRegistry,
    members: Arc<DashMap<String, ClusterMember>>,
    handler: Arc<ConferenceCallHandler>,
    /// Dials in flight, so an overlapping reconcile does not start a second.
    dialing: DashMap<TrunkKey, Instant>,
    /// Last failed dial per trunk, for retry backoff.
    failed: DashMap<TrunkKey, Instant>,
    /// Peers already warned about for advertising no TLS address.
    warned_no_tls: DashMap<String, ()>,
    /// Tear down a trunk with no RTP for this long; `None` disables.
    rtp_timeout: Option<Duration>,
}

impl TrunkManager {
    pub fn new(
        node_id: String,
        registry: ConferenceRegistry,
        members: Arc<DashMap<String, ClusterMember>>,
        handler: Arc<ConferenceCallHandler>,
        rtp_timeout: Option<Duration>,
    ) -> Self {
        Self {
            node_id,
            registry,
            members,
            handler,
            dialing: DashMap::new(),
            failed: DashMap::new(),
            warned_no_tls: DashMap::new(),
            rtp_timeout,
        }
    }

    /// Reconcile one room: dial the peers this node should be trunked to
    /// and hang up the trunks that no longer belong.
    pub async fn reconcile_room(self: &Arc<Self>, tenant_id: &str, room_number: &str) {
        let Ok(tenant) = tenant_id.parse::<TenantId>() else {
            warn!(tenant = %tenant_id, room = %room_number, "Bad tenant id; not reconciling the room");
            return;
        };
        let room = RoomKey::new(tenant, room_number);
        // Dead trunks first, so the plan below sees them as missing and
        // dials again (if this is the dialing side; otherwise the peer's
        // next INVITE replaces what it thinks it still has).
        if let Some(timeout) = self.rtp_timeout {
            for peer in self.handler.silent_trunks(&room, timeout) {
                warn!(
                    room = %room_number,
                    peer = %peer,
                    timeout_secs = timeout.as_secs(),
                    "Conference trunk received no RTP; tearing it down"
                );
                self.handler.hangup_trunk(&room, &peer).await;
                // Retry at once rather than waiting out a failed-dial backoff.
                self.failed
                    .remove(&(tenant_id.to_string(), room_number.to_string(), peer));
            }
        }
        let local = self.handler.countable_participants(&room);
        let trunked = self.handler.room_trunk_peers(&room);
        let hosting: Vec<String> = if local == 0 {
            Vec::new()
        } else {
            match self.registry.peers(tenant_id, room_number).await {
                Ok(peers) => peers
                    .into_iter()
                    .filter(|host| host.participants > 0)
                    .map(|host| host.node_id)
                    .filter(|id| {
                        self.members
                            .get(id)
                            .map(|m| m.node_type == CONFERENCE_NODE_TYPE)
                            .unwrap_or(false)
                    })
                    .collect(),
                Err(e) => {
                    // Keep what we have rather than tear trunks down on a
                    // registry hiccup; the next pass sees the truth.
                    warn!(room = %room_number, error = %e, "Conference registry read failed; trunks left as they are");
                    return;
                }
            }
        };

        let plan = plan_trunks(&self.node_id, local, &hosting, &trunked);
        if plan.dial.is_empty() && plan.hangup.is_empty() {
            return;
        }
        debug!(room = %room_number, local, ?hosting, ?trunked, ?plan, "Trunk reconcile");
        for peer in plan.hangup {
            info!(room = %room_number, peer = %peer, "Hanging up conference trunk");
            self.handler.hangup_trunk(&room, &peer).await;
        }
        for peer in plan.dial {
            self.start_dial(tenant_id, room_number, peer);
        }
    }

    fn start_dial(self: &Arc<Self>, tenant_id: &str, room_number: &str, peer: String) {
        let key = (tenant_id.to_string(), room_number.to_string(), peer.clone());
        if let Some(started) = self.dialing.get(&key) {
            if started.elapsed() < DIAL_STALE_AFTER {
                return;
            }
        }
        if let Some(failed_at) = self.failed.get(&key) {
            if failed_at.elapsed() < DIAL_RETRY_AFTER {
                return;
            }
        }
        let Some(target) = self.members.get(&peer).and_then(|m| m.sip_tls_address) else {
            if self.warned_no_tls.insert(peer.clone(), ()).is_none() {
                warn!(
                    peer = %peer,
                    "Peer conference node advertises no TLS address (sip.tls_bind); cannot trunk to it"
                );
            }
            return;
        };
        let tenant: TenantId = match tenant_id.parse() {
            Ok(t) => t,
            Err(e) => {
                warn!(tenant = %tenant_id, room = %room_number, error = %e, "Bad tenant id in registry key; not trunking");
                return;
            }
        };
        self.dialing.insert(key.clone(), Instant::now());
        info!(room = %room_number, peer = %peer, %target, "Dialing conference trunk");
        let this = Arc::clone(self);
        let room = room_number.to_string();
        tokio::spawn(async move {
            match this
                .handler
                .dial_trunk(&RoomKey::new(tenant, room.clone()), &peer, target)
                .await
            {
                Ok(()) => {
                    this.failed.remove(&key);
                }
                Err(e) => {
                    warn!(room = %room, peer = %peer, error = %e, "Conference trunk dial failed");
                    this.failed.insert(key.clone(), Instant::now());
                }
            }
            this.dialing.remove(&key);
        });
    }

    /// Reconcile every room this node hosts.
    pub async fn reconcile_all(self: &Arc<Self>) {
        for (key, _) in self.handler.active_rooms() {
            self.reconcile_room(&key.tenant.to_string(), &key.room)
                .await;
        }
    }

    /// Run [`reconcile_all`](Self::reconcile_all) every `interval`.
    pub fn spawn_reconcile_loop(self: Arc<Self>, interval: Duration) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                self.reconcile_all().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lower_node_id_dials() {
        assert!(dials("conf-a", "conf-b"));
        assert!(!dials("conf-b", "conf-a"));
        assert!(!dials("conf-a", "conf-a"));
    }

    #[test]
    fn plan_dials_higher_peers_it_lacks_and_keeps_the_rest() {
        let plan = plan_trunks(
            "conf-b",
            3,
            &ids(&["conf-a", "conf-c", "conf-d"]),
            &ids(&["conf-c"]),
        );
        // conf-a is lower: it dials us. conf-c is trunked. conf-d is ours to dial.
        assert_eq!(plan.dial, ids(&["conf-d"]));
        assert!(plan.hangup.is_empty());
    }

    #[test]
    fn plan_hangs_up_peers_that_stopped_hosting() {
        let plan = plan_trunks("conf-a", 1, &ids(&["conf-b"]), &ids(&["conf-b", "conf-c"]));
        assert!(plan.dial.is_empty());
        assert_eq!(plan.hangup, ids(&["conf-c"]));
    }

    #[test]
    fn plan_hangs_up_everything_without_local_participants() {
        let plan = plan_trunks(
            "conf-a",
            0,
            &ids(&["conf-b", "conf-c"]),
            &ids(&["conf-b", "conf-c"]),
        );
        assert!(plan.dial.is_empty());
        assert_eq!(plan.hangup, ids(&["conf-b", "conf-c"]));
    }

    #[test]
    fn plan_ignores_self_and_is_idempotent() {
        let plan = plan_trunks("conf-a", 2, &ids(&["conf-a", "conf-b"]), &ids(&["conf-b"]));
        assert_eq!(plan, TrunkPlan::default());
    }

    #[test]
    fn trunk_participant_ids_are_virtual() {
        let id = trunk_participant_id("conf-b");
        assert_eq!(id, "__trunk__conf-b");
        assert!(is_trunk_participant(&id));
        assert!(id.starts_with("__"), "excluded from placement counts");
        assert!(!is_trunk_participant("alice-1234"));
        assert!(!is_trunk_participant("__ai__"));
    }

    #[test]
    fn identity_matches_cn_dns_or_sip_uri_san() {
        let by_cn = PeerIdentity::builder("CN=conf-b")
            .common_name("conf-b")
            .build();
        assert!(identity_names_node(&by_cn, "conf-b"));
        assert!(identity_names_node(&by_cn, "CONF-B"));
        assert!(!identity_names_node(&by_cn, "conf-c"));

        let by_dns = PeerIdentity::builder("CN=node")
            .common_name("node")
            .dns_name("conf-b.cluster.internal")
            .build();
        assert!(identity_names_node(&by_dns, "conf-b.cluster.internal"));
        assert!(!identity_names_node(&by_dns, "conf-b"));

        let by_uri = PeerIdentity::builder("CN=node")
            .common_name("node")
            .uri_name("sip:conf-b@cluster.internal;transport=tls")
            .build();
        assert!(identity_names_node(&by_uri, "conf-b"));
        assert!(identity_names_node(&by_uri, "cluster.internal"));
        assert!(!identity_names_node(&by_uri, "conf-c"));

        let by_sips_host = PeerIdentity::builder("CN=node")
            .common_name("node")
            .uri_name("sips:conf-b")
            .build();
        assert!(identity_names_node(&by_sips_host, "conf-b"));

        let other_scheme = PeerIdentity::builder("CN=node")
            .common_name("node")
            .uri_name("https://conf-b")
            .build();
        assert!(!identity_names_node(&other_scheme, "conf-b"));
        assert!(!identity_names_node(&by_cn, ""));
    }
}
