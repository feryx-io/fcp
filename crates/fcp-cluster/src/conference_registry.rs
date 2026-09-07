//! Conference registry: which nodes host which conference room, and how
//! many participants each has there.
//!
//! Redis layout: one hash per room, `cluster:conference:<tenant>:<room>`,
//! field = node id, value = local participant count. Every write refreshes
//! the key's TTL, so a node that stops heartbeating its rooms disappears
//! from the registry on its own.
//!
//! Each node's field is `<participants>@<unix seconds>` (phase 4): the key's
//! TTL only expires the whole room, and a live host refreshing it would
//! keep a dead node's field alive forever. Readers drop any field older
//! than the TTL, so a node that stops publishing disappears from a room
//! within one TTL even while other nodes keep hosting it. A bare integer
//! (written by older versions) is taken as fresh.
//!
//! The same hash carries the room-wide lock flag in the `__locked` field
//! (phase 3, cross-node control): a node that starts hosting a room other
//! nodes already host inherits the lock from here, so a locked room stays
//! locked when placement spreads it to a fresh node. Fields starting with
//! `__` are never node ids.
//!
//! [`choose_conference_node`] is the placement rule the call manager uses
//! when a new participant dials a room: stay on a node already hosting the
//! room while it is under the configured capacity, otherwise the
//! least-loaded conference node in the cluster — preferring, at each step,
//! nodes in the caller's own region (`docs/MULTI_REGION.md`).

use crate::membership::ClusterMember;
use fcp_storage_redis::RedisStorage;
use redis::AsyncCommands;
use tracing::debug;

/// Node type advertised by conference servers in cluster membership.
pub const CONFERENCE_NODE_TYPE: &str = "conference";

/// Hash field holding the room-wide lock flag. Not a node id.
const LOCKED_FIELD: &str = "__locked";

/// One node hosting a room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConferenceHost {
    pub node_id: String,
    pub participants: usize,
}

/// Everything the registry knows about one room.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomRecord {
    /// Nodes hosting the room, sorted by node id.
    pub hosts: Vec<ConferenceHost>,
    /// Room-wide lock flag (see [`ConferenceRegistry::set_locked`]).
    pub locked: bool,
}

/// Seconds since the Unix epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Decode one node's field: `<participants>@<unix seconds>`, or a bare
/// count from an older writer (taken as fresh). `None` when the entry is
/// older than `stale_after_secs`.
fn parse_host_value(value: &str, now: u64, stale_after_secs: u64) -> Option<usize> {
    let (count, written_at) = match value.split_once('@') {
        Some((count, ts)) => (count, ts.parse::<u64>().ok()),
        None => (value, None),
    };
    if let Some(written_at) = written_at {
        if now.saturating_sub(written_at) > stale_after_secs {
            return None;
        }
    }
    Some(count.parse::<i64>().unwrap_or(0).max(0) as usize)
}

/// Interpret a room hash: node fields become hosts (unless stale),
/// `__locked` becomes the flag, anything else starting with `__` is
/// ignored.
fn parse_room_hash(raw: Vec<(String, String)>, now: u64, stale_after_secs: u64) -> RoomRecord {
    let mut record = RoomRecord::default();
    for (field, value) in raw {
        if field == LOCKED_FIELD {
            record.locked = value == "1";
        } else if !field.starts_with("__") {
            if let Some(participants) = parse_host_value(&value, now, stale_after_secs) {
                record.hosts.push(ConferenceHost {
                    node_id: field,
                    participants,
                });
            }
        }
    }
    record.hosts.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    record
}

#[derive(Clone)]
pub struct ConferenceRegistry {
    storage: RedisStorage,
    node_id: String,
    ttl_secs: u64,
}

impl ConferenceRegistry {
    /// Registry for `node_id`; room entries expire 60 s after their last
    /// refresh.
    pub fn new(storage: RedisStorage, node_id: String) -> Self {
        Self::with_ttl(storage, node_id, 60)
    }

    pub fn with_ttl(storage: RedisStorage, node_id: String, ttl_secs: u64) -> Self {
        Self {
            storage,
            node_id,
            ttl_secs,
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    fn room_key(&self, tenant_id: &str, room_number: &str) -> String {
        self.storage
            .key(&format!("cluster:conference:{tenant_id}:{room_number}"))
    }

    /// Publish this node's local participant count for a room (0 removes
    /// the node from the room) and refresh the room's TTL. The entry is
    /// stamped with the current time; a node that stops refreshing it is
    /// dropped by readers after one TTL.
    pub async fn set_participants(
        &self,
        tenant_id: &str,
        room_number: &str,
        participants: usize,
    ) -> anyhow::Result<()> {
        let key = self.room_key(tenant_id, room_number);
        let mut conn = self.storage.connection();
        if participants == 0 {
            let _: () = conn.hdel(&key, &self.node_id).await?;
        } else {
            let value = format!("{}@{}", participants, unix_now());
            let _: () = redis::pipe()
                .atomic()
                .hset(&key, &self.node_id, value)
                .expire(&key, self.ttl_secs as i64)
                .query_async(&mut conn)
                .await?;
        }
        debug!(tenant_id, room_number, participants, node = %self.node_id, "Conference registry updated");
        Ok(())
    }

    /// Drop this node from a room regardless of count.
    pub async fn leave_room(&self, tenant_id: &str, room_number: &str) -> anyhow::Result<()> {
        self.set_participants(tenant_id, room_number, 0).await
    }

    /// Set or clear the room-wide lock flag. Setting it also refreshes the
    /// room's TTL, so the flag never outlives the room by more than one TTL
    /// once every host has left.
    pub async fn set_locked(
        &self,
        tenant_id: &str,
        room_number: &str,
        locked: bool,
    ) -> anyhow::Result<()> {
        let key = self.room_key(tenant_id, room_number);
        let mut conn = self.storage.connection();
        if locked {
            let _: () = redis::pipe()
                .atomic()
                .hset(&key, LOCKED_FIELD, "1")
                .expire(&key, self.ttl_secs as i64)
                .query_async(&mut conn)
                .await?;
        } else {
            let _: () = conn.hdel(&key, LOCKED_FIELD).await?;
        }
        debug!(
            tenant_id,
            room_number, locked, "Conference registry lock flag updated"
        );
        Ok(())
    }

    /// The room's hosts and lock flag in one read.
    pub async fn room(&self, tenant_id: &str, room_number: &str) -> anyhow::Result<RoomRecord> {
        let key = self.room_key(tenant_id, room_number);
        let mut conn = self.storage.connection();
        let raw: Vec<(String, String)> = conn.hgetall(&key).await?;
        Ok(parse_room_hash(raw, unix_now(), self.ttl_secs))
    }

    /// Every node hosting the room, with its participant count.
    pub async fn hosts(
        &self,
        tenant_id: &str,
        room_number: &str,
    ) -> anyhow::Result<Vec<ConferenceHost>> {
        Ok(self.room(tenant_id, room_number).await?.hosts)
    }

    /// Nodes hosting the room other than this one.
    pub async fn peers(
        &self,
        tenant_id: &str,
        room_number: &str,
    ) -> anyhow::Result<Vec<ConferenceHost>> {
        Ok(self
            .hosts(tenant_id, room_number)
            .await?
            .into_iter()
            .filter(|h| h.node_id != self.node_id)
            .collect())
    }
}

/// What a room needs of a node's video capacity: the SDP names of the
/// codecs the meeting allows. `None` for an audio-only meeting.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VideoNeed {
    pub codecs: Vec<String>,
}

/// Pick the node a new participant of `room` should be sent to.
///
/// - `hosts`: nodes currently hosting the room (from the registry).
/// - `members`: live cluster members; only `conference` nodes that are
///   not draining qualify.
/// - `capacity`: participants per node per room before the room is spread
///   to another node; `0` means never spread.
/// - `local_region`: the region the participant is joining from (the
///   placing node's own region); `None` places without regard to region.
/// - `video`: for a video meeting, the codecs it allows. Among otherwise
///   equal candidates (steps 2 and 4) the node with the most video budget
///   left on a device that runs one of them wins, a node with a known
///   budget of zero comes last, and a node advertising no budget sits in
///   between — unknown, neither preferred nor excluded. Hosting nodes
///   (steps 1 and 3) keep their priority regardless of budget: inter-node
///   trunks carry audio only, so spreading a video room would cost its
///   participants each other's video (`docs/VIDEO_CONFERENCING.md` §9.3).
///
/// In order:
///
/// 1. A node in the local region already hosting the room, live and under
///    capacity — the fullest, so a room stays on as few nodes as possible.
/// 2. The least-loaded live conference node in the local region, even
///    when another region hosts the room with room to spare: the
///    participant's media then stays in region, and the cascade trunk
///    between hosting nodes carries the cross-region audio once.
/// 3. A hosting node under capacity in any region, the fullest.
/// 4. The least-loaded live conference node anywhere.
///
/// `None` when no conference node is alive, so callers fall back to static
/// configuration. A draining node (shutting down, still hosting its
/// current callers) never takes new participants, even for a room it
/// hosts. Nodes without a region are never local; they take part in
/// steps 3 and 4 only. Without a `local_region`, only steps 3 and 4 apply.
pub fn choose_conference_node(
    hosts: &[ConferenceHost],
    members: &[ClusterMember],
    capacity: usize,
    local_region: Option<&str>,
    video: Option<&VideoNeed>,
) -> Option<ClusterMember> {
    let live: Vec<&ClusterMember> = members
        .iter()
        .filter(|m| m.node_type == CONFERENCE_NODE_TYPE && !m.draining)
        .collect();
    if live.is_empty() {
        return None;
    }

    // Hosting nodes that can take one more participant, fullest first.
    let mut hosting: Vec<(&ConferenceHost, &ClusterMember)> = hosts
        .iter()
        .filter_map(|h| {
            live.iter()
                .find(|m| m.node_id == h.node_id)
                .map(|m| (h, *m))
        })
        .filter(|(h, _)| capacity == 0 || h.participants < capacity)
        .collect();
    hosting.sort_by(|a, b| {
        b.0.participants
            .cmp(&a.0.participants)
            .then(a.0.node_id.cmp(&b.0.node_id))
    });

    // Least loaded; for a video room, most video budget first. A node that
    // advertises no budget ranks as 0 left but is not "exhausted", so it
    // sits between nodes with budget and nodes known to have none.
    let least_loaded = |candidates: &[&ClusterMember]| -> Option<ClusterMember> {
        candidates
            .iter()
            .min_by_key(|m| {
                let (exhausted, left) = match video {
                    Some(need) => match m.video_budget_left(&need.codecs) {
                        Some(0) => (true, 0),
                        Some(left) => (false, left),
                        None => (false, 0),
                    },
                    None => (false, 0),
                };
                (
                    exhausted,
                    std::cmp::Reverse(left),
                    m.active_calls,
                    m.node_id.clone(),
                )
            })
            .map(|m| (*m).clone())
    };

    if let Some(region) = local_region {
        if let Some((_, member)) = hosting.iter().find(|(_, m)| m.in_region(region)) {
            return Some((*member).clone());
        }
        let local: Vec<&ClusterMember> = live
            .iter()
            .copied()
            .filter(|m| m.in_region(region))
            .collect();
        if let Some(pick) = least_loaded(&local) {
            return Some(pick);
        }
    }

    if let Some((_, member)) = hosting.first() {
        return Some((*member).clone());
    }
    least_loaded(&live)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(f, v)| (f.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn room_hash_separates_hosts_from_flags() {
        let record = parse_room_hash(
            raw(&[
                ("conf-b", "12@1000"),
                ("__locked", "1"),
                ("conf-a", "3"),
                ("__future", "x"),
                ("conf-c", "garbage"),
            ]),
            1000,
            60,
        );
        assert!(record.locked);
        assert_eq!(
            record.hosts,
            vec![
                ConferenceHost {
                    node_id: "conf-a".into(),
                    participants: 3
                },
                ConferenceHost {
                    node_id: "conf-b".into(),
                    participants: 12
                },
                ConferenceHost {
                    node_id: "conf-c".into(),
                    participants: 0
                },
            ]
        );
        let unlocked = parse_room_hash(raw(&[("conf-a", "1"), ("__locked", "0")]), 0, 60);
        assert!(!unlocked.locked);
        assert!(!parse_room_hash(Vec::new(), 0, 60).locked);
    }

    #[test]
    fn stale_host_entries_are_dropped_and_bare_counts_are_fresh() {
        let now = 10_000;
        let record = parse_room_hash(
            raw(&[
                ("conf-dead", "40@9000"), // 1000 s old: gone
                ("conf-edge", "5@9940"),  // exactly 60 s old: kept
                ("conf-live", "7@9999"),
                ("conf-old-writer", "2"),   // no stamp: fresh
                ("conf-future", "1@10005"), // clock skew: kept
                ("conf-bad-stamp", "3@x"),  // unparsable stamp: treated as unstamped
            ]),
            now,
            60,
        );
        let ids: Vec<(&str, usize)> = record
            .hosts
            .iter()
            .map(|h| (h.node_id.as_str(), h.participants))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("conf-bad-stamp", 3),
                ("conf-edge", 5),
                ("conf-future", 1),
                ("conf-live", 7),
                ("conf-old-writer", 2),
            ]
        );
        assert_eq!(parse_host_value("9@100", 161, 60), None);
        assert_eq!(parse_host_value("-4@100", 100, 60), Some(0));
    }

    #[test]
    fn draining_nodes_never_take_new_participants() {
        let mut draining = member("conf-a", "conference", 1);
        draining.draining = true;
        let members = vec![draining, member("conf-b", "conference", 50)];
        // conf-a hosts the room and is under capacity, but is draining.
        let pick = choose_conference_node(&[host("conf-a", 3)], &members, 100, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // A brand-new room avoids it too, whatever its load.
        let pick = choose_conference_node(&[], &members, 0, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // With every conference node draining there is nothing to pick.
        let mut all = members.clone();
        all[1].draining = true;
        assert!(choose_conference_node(&[], &all, 0, None, None).is_none());
    }

    fn member(id: &str, node_type: &str, calls: usize) -> ClusterMember {
        ClusterMember {
            node_id: id.to_string(),
            node_type: node_type.to_string(),
            http_address: "127.0.0.1:8090".parse().unwrap(),
            sip_address: "127.0.0.1:5074".parse().unwrap(),
            sip_tls_address: None,
            capabilities: vec![],
            active_calls: calls,
            started_at: 0,
            version: "test".to_string(),
            draining: false,
            region: None,
            video_budget: Vec::new(),
        }
    }

    fn in_region(mut m: ClusterMember, region: &str) -> ClusterMember {
        m.region = Some(region.to_string());
        m
    }

    fn host(id: &str, n: usize) -> ConferenceHost {
        ConferenceHost {
            node_id: id.to_string(),
            participants: n,
        }
    }

    #[test]
    fn new_room_goes_to_least_loaded_conference_node() {
        let members = vec![
            member("conf-a", "conference", 40),
            member("conf-b", "conference", 12),
            member("cm-1", "call_manager", 0),
        ];
        let pick = choose_conference_node(&[], &members, 0, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
    }

    #[test]
    fn running_room_stays_on_its_node_under_capacity() {
        let members = vec![
            member("conf-a", "conference", 40),
            member("conf-b", "conference", 1),
        ];
        // conf-a hosts the room with 30 participants; capacity 100 → stay.
        let pick =
            choose_conference_node(&[host("conf-a", 30)], &members, 100, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-a");
        // capacity 0 = never spread, even when a node is busy.
        let pick = choose_conference_node(&[host("conf-a", 30)], &members, 0, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-a");
    }

    #[test]
    fn full_host_spreads_to_least_loaded_other_node() {
        let members = vec![
            member("conf-a", "conference", 40),
            member("conf-b", "conference", 5),
            member("conf-c", "conference", 9),
        ];
        let pick =
            choose_conference_node(&[host("conf-a", 100)], &members, 100, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // Once conf-b hosts too, it is preferred until it is full as well.
        let pick = choose_conference_node(
            &[host("conf-a", 100), host("conf-b", 3)],
            &members,
            100,
            None,
            None,
        )
        .unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // The fullest under-capacity host wins, to keep the room compact.
        let pick = choose_conference_node(
            &[host("conf-a", 100), host("conf-b", 3), host("conf-c", 50)],
            &members,
            100,
            None,
            None,
        )
        .unwrap();
        assert_eq!(pick.node_id, "conf-c");
    }

    #[test]
    fn dead_hosts_are_ignored_and_no_conference_nodes_means_none() {
        let members = vec![member("conf-b", "conference", 5)];
        // conf-a is in the registry but not in membership: skip it.
        let pick = choose_conference_node(&[host("conf-a", 3)], &members, 100, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        assert!(choose_conference_node(
            &[host("conf-a", 3)],
            &[member("cm-1", "call_manager", 0)],
            100,
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn local_region_host_wins_over_a_fuller_remote_host() {
        let members = vec![
            in_region(member("conf-us", "conference", 40), "us-east"),
            in_region(member("conf-eu", "conference", 1), "eu-west"),
        ];
        let hosts = [host("conf-eu", 50), host("conf-us", 2)];
        // From the US the US host wins although the EU host is fuller.
        let pick = choose_conference_node(&hosts, &members, 100, Some("us-east"), None).unwrap();
        assert_eq!(pick.node_id, "conf-us");
        // From the EU, the EU host.
        let pick = choose_conference_node(&hosts, &members, 100, Some("eu-west"), None).unwrap();
        assert_eq!(pick.node_id, "conf-eu");
        // Region-blind placement keeps the old rule: the fullest host.
        let pick = choose_conference_node(&hosts, &members, 100, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-eu");
    }

    #[test]
    fn room_spreads_into_the_local_region_before_using_a_remote_host() {
        let members = vec![
            in_region(member("conf-us-a", "conference", 9), "us-east"),
            in_region(member("conf-us-b", "conference", 4), "us-east"),
            in_region(member("conf-eu", "conference", 0), "eu-west"),
        ];
        // The EU hosts the room with plenty of capacity, but a US caller
        // is placed on the least-loaded US node, starting a cascade.
        let pick =
            choose_conference_node(&[host("conf-eu", 3)], &members, 100, Some("us-east"), None)
                .unwrap();
        assert_eq!(pick.node_id, "conf-us-b");
        // A brand-new room from the US also starts in the US.
        let pick = choose_conference_node(&[], &members, 0, Some("us-east"), None).unwrap();
        assert_eq!(pick.node_id, "conf-us-b");
        // Once a US node hosts it, that node is preferred over the emptier US node.
        let pick = choose_conference_node(
            &[host("conf-eu", 3), host("conf-us-a", 1)],
            &members,
            100,
            Some("us-east"),
            None,
        )
        .unwrap();
        assert_eq!(pick.node_id, "conf-us-a");
    }

    #[test]
    fn full_or_draining_local_region_falls_back_to_other_regions() {
        let mut draining = in_region(member("conf-us", "conference", 1), "us-east");
        draining.draining = true;
        let members = vec![
            draining,
            in_region(member("conf-eu", "conference", 7), "eu-west"),
            member("conf-anywhere", "conference", 3),
        ];
        // Nothing usable in the US: a hosting node elsewhere wins first …
        let pick =
            choose_conference_node(&[host("conf-eu", 10)], &members, 100, Some("us-east"), None)
                .unwrap();
        assert_eq!(pick.node_id, "conf-eu");
        // … else the least-loaded node anywhere, unlabeled nodes included.
        let pick = choose_conference_node(&[], &members, 0, Some("us-east"), None).unwrap();
        assert_eq!(pick.node_id, "conf-anywhere");
        // A region nobody is in behaves the same.
        let pick = choose_conference_node(&[], &members, 0, Some("ap-south"), None).unwrap();
        assert_eq!(pick.node_id, "conf-anywhere");
    }

    #[test]
    fn unlabeled_nodes_are_never_local() {
        let members = vec![
            member("conf-unlabeled", "conference", 0),
            in_region(member("conf-us", "conference", 30), "us-east"),
        ];
        // A US caller stays in the US even though the unlabeled node is idle.
        let pick = choose_conference_node(&[], &members, 0, Some("us-east"), None).unwrap();
        assert_eq!(pick.node_id, "conf-us");
        // A caller with no region of their own takes the idle node.
        let pick = choose_conference_node(&[], &members, 0, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-unlabeled");
    }

    fn with_budget(mut m: ClusterMember, codecs: &[&str], left: u32) -> ClusterMember {
        m.video_budget = vec![crate::membership::VideoDeviceBudget {
            device: "host".into(),
            codecs: codecs.iter().map(|c| c.to_string()).collect(),
            left,
            total: 8000,
        }];
        m
    }

    fn need(codecs: &[&str]) -> VideoNeed {
        VideoNeed {
            codecs: codecs.iter().map(|c| c.to_string()).collect(),
        }
    }

    #[test]
    fn video_rooms_prefer_the_node_with_the_most_budget_for_their_codecs() {
        let members = vec![
            with_budget(
                member("conf-busy", "conference", 40),
                &["H264", "VP8"],
                5000,
            ),
            with_budget(member("conf-idle", "conference", 0), &["H264", "VP8"], 900),
            with_budget(member("conf-vp8", "conference", 0), &["VP8"], 7000),
        ];
        // Audio: least loaded, as before (both idle nodes tie on calls; id order).
        let pick = choose_conference_node(&[], &members, 0, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-idle");
        // H.264 video: the busy node has the most H.264 budget left.
        let pick = choose_conference_node(&[], &members, 0, None, Some(&need(&["H264"]))).unwrap();
        assert_eq!(pick.node_id, "conf-busy");
        // VP8 video: the VP8-only node has the most.
        let pick = choose_conference_node(&[], &members, 0, None, Some(&need(&["VP8"]))).unwrap();
        assert_eq!(pick.node_id, "conf-vp8");
        // A meeting allowing either codec takes the best device for any of them.
        let pick =
            choose_conference_node(&[], &members, 0, None, Some(&need(&["H264", "VP8"]))).unwrap();
        assert_eq!(pick.node_id, "conf-vp8");
    }

    #[test]
    fn exhausted_budget_ranks_last_and_unknown_budget_in_between() {
        let members = vec![
            with_budget(member("conf-empty", "conference", 0), &["H264"], 0),
            member("conf-unknown", "conference", 0),
            with_budget(member("conf-some", "conference", 50), &["H264"], 100),
        ];
        let h264 = need(&["H264"]);
        let pick = choose_conference_node(&[], &members, 0, None, Some(&h264)).unwrap();
        assert_eq!(pick.node_id, "conf-some", "budget beats load");
        let two = vec![members[0].clone(), members[1].clone()];
        let pick = choose_conference_node(&[], &two, 0, None, Some(&h264)).unwrap();
        assert_eq!(pick.node_id, "conf-unknown", "unknown beats known-empty");
        let one = vec![members[0].clone()];
        let pick = choose_conference_node(&[], &one, 0, None, Some(&h264)).unwrap();
        assert_eq!(
            pick.node_id, "conf-empty",
            "an exhausted node is still a node"
        );
        // A codec nobody runs natively: every budget reads 0, load decides.
        let pick = choose_conference_node(&[], &members, 0, None, Some(&need(&["AV1"]))).unwrap();
        assert_eq!(pick.node_id, "conf-unknown");
    }

    #[test]
    fn a_hosting_node_keeps_a_video_room_whatever_its_budget() {
        let members = vec![
            with_budget(member("conf-a", "conference", 40), &["H264"], 0),
            with_budget(member("conf-b", "conference", 0), &["H264"], 8000),
        ];
        let pick = choose_conference_node(
            &[host("conf-a", 3)],
            &members,
            100,
            None,
            Some(&need(&["H264"])),
        )
        .unwrap();
        assert_eq!(pick.node_id, "conf-a");
        // Once the host is at capacity the room spreads to the node with budget.
        let pick = choose_conference_node(
            &[host("conf-a", 100)],
            &members,
            100,
            None,
            Some(&need(&["H264"])),
        )
        .unwrap();
        assert_eq!(pick.node_id, "conf-b");
    }

    #[test]
    fn region_still_comes_before_budget() {
        let members = vec![
            in_region(
                with_budget(member("conf-us", "conference", 10), &["H264"], 100),
                "us-east",
            ),
            in_region(
                with_budget(member("conf-eu", "conference", 0), &["H264"], 8000),
                "eu-west",
            ),
        ];
        let pick =
            choose_conference_node(&[], &members, 0, Some("us-east"), Some(&need(&["H264"])))
                .unwrap();
        assert_eq!(pick.node_id, "conf-us");
    }
}
