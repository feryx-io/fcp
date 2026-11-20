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

/// Hash field naming a webinar's stage node (`docs/WEBINARS.md` §2.7): the
/// node holding its hosts and panelists, to which every other node hosting
/// the room (an edge) is trunked. A meeting never has one.
const STAGE_FIELD: &str = "__stage";

/// Take the stage when nobody holds it, or when the holder is the one the
/// caller judged gone (`ARGV[2]`, empty for none). Answers the holder.
const CLAIM_STAGE_SCRIPT: &str = r#"
local cur = redis.call('HGET', KEYS[1], ARGV[3])
if cur == false or cur == ARGV[2] then
  redis.call('HSET', KEYS[1], ARGV[3], ARGV[1])
  redis.call('EXPIRE', KEYS[1], ARGV[4])
  return ARGV[1]
end
return cur
"#;

/// Give the stage up, only if this node holds it.
const RELEASE_STAGE_SCRIPT: &str = r#"
if redis.call('HGET', KEYS[1], ARGV[2]) == ARGV[1] then
  return redis.call('HDEL', KEYS[1], ARGV[2])
end
return 0
"#;

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
    /// The node claiming a webinar's stage, as written: see
    /// [`RoomRecord::live_stage`] for whether it still holds the room.
    pub stage: Option<String>,
}

impl RoomRecord {
    /// The stage, while it is among the room's fresh hosts. A stage whose
    /// node stopped publishing the room has gone, and may be taken.
    pub fn live_stage(&self) -> Option<&ConferenceHost> {
        let stage = self.stage.as_deref()?;
        self.hosts.iter().find(|h| h.node_id == stage)
    }
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
        } else if field == STAGE_FIELD {
            record.stage = (!value.is_empty()).then_some(value);
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

    /// Take a webinar's stage for this node (`docs/WEBINARS.md` §2.7) when
    /// nobody holds it, or when `gone` holds it: a stage the caller found
    /// no longer hosting the room or alive. Answers the node holding the
    /// stage afterwards, this one when the claim won.
    pub async fn claim_stage(
        &self,
        tenant_id: &str,
        room_number: &str,
        gone: Option<&str>,
    ) -> anyhow::Result<String> {
        let key = self.room_key(tenant_id, room_number);
        let mut conn = self.storage.connection();
        let holder: String = redis::Script::new(CLAIM_STAGE_SCRIPT)
            .key(&key)
            .arg(&self.node_id)
            .arg(gone.unwrap_or(""))
            .arg(STAGE_FIELD)
            .arg(self.ttl_secs as i64)
            .invoke_async(&mut conn)
            .await?;
        debug!(tenant_id, room_number, holder = %holder, node = %self.node_id, "Webinar stage claimed");
        Ok(holder)
    }

    /// Give up a webinar's stage, if this node holds it.
    pub async fn release_stage(&self, tenant_id: &str, room_number: &str) -> anyhow::Result<()> {
        let key = self.room_key(tenant_id, room_number);
        let mut conn = self.storage.connection();
        let _: i64 = redis::Script::new(RELEASE_STAGE_SCRIPT)
            .key(&key)
            .arg(&self.node_id)
            .arg(STAGE_FIELD)
            .invoke_async(&mut conn)
            .await?;
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
/// codecs the meeting allows, and the one it prefers. `None` for an
/// audio-only meeting.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VideoNeed {
    pub codecs: Vec<String>,
    /// The meeting's first choice of codec, when it has one. A device is
    /// preferred only when it encodes this one (design §15.7, decision
    /// 5): a VP8-first meeting is not sent to a GPU whose encoder has no
    /// VP8, even though the GPU node could run it with the encode on its
    /// host.
    pub first: Option<String>,
    /// Kilobits a second a join may send out of the node
    /// (`docs/FORWARDING_ROOMS.md` §3). A node with less egress left
    /// counts as having no video budget.
    pub egress_kbps: u32,
}

/// What [`VideoNeed::of`] expects a join to send: a forwarding viewer's
/// default view of a full room, the speaker at 720p and eight others at
/// 180p (1,200 + 8 × 200 kb/s). A composite join sends less.
pub const DEFAULT_JOIN_EGRESS_KBPS: u32 = 2_800;

impl VideoNeed {
    /// A need for `codecs` in order of preference.
    pub fn of(codecs: Vec<String>) -> Self {
        let first = codecs.first().cloned();
        Self {
            codecs,
            first,
            egress_kbps: DEFAULT_JOIN_EGRESS_KBPS,
        }
    }
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
///   left on a device that runs one of them wins, then the one with the
///   most egress left; a node with a known budget of zero, or with less
///   egress left than the join needs, comes last, and a node advertising
///   no budget sits in between — unknown, neither preferred nor excluded. Hosting nodes
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
                let (exhausted, left, egress) = match video {
                    Some(need) => {
                        let egress = m.video_egress_left_kbps();
                        let out_of_egress = egress.is_some_and(|e| e < need.egress_kbps);
                        match m.video_budget_left_for(need) {
                            Some(0) => (true, 0, 0),
                            Some(left) => (out_of_egress, left, egress.unwrap_or(0)),
                            None => (out_of_egress, 0, egress.unwrap_or(0)),
                        }
                    }
                    None => (false, 0, 0),
                };
                // Room before load: a node carrying ten calls of a
                // hundred has more to give than one carrying two of four
                // (`docs/CAPACITY_AND_SURVIVABILITY.md` §7). A node that
                // advertises no limit has unknown room rather than
                // infinite room, so it is neither preferred nor
                // excluded — it is chosen on its calls, as every node
                // was before capacity was published.
                let headroom = m.capacity.and_then(|c| c.calls_left(m.active_calls));
                (
                    exhausted,
                    headroom == Some(0),
                    std::cmp::Reverse(left),
                    std::cmp::Reverse(egress),
                    std::cmp::Reverse(headroom.unwrap_or(0)),
                    m.active_calls,
                    m.node_id.clone(),
                )
            })
            .map(|m| (*m).clone())
    };

    // A node already at capacity for this room is full for it: the
    // least-loaded step must not hand it the caller the hosting step
    // just refused it (membership's load is a heartbeat old, and a tie
    // there falls to node id — the same node, every time). Only when
    // every live node is full does the room overfill one.
    let full: Vec<&str> = if capacity == 0 {
        Vec::new()
    } else {
        hosts
            .iter()
            .filter(|h| h.participants >= capacity)
            .map(|h| h.node_id.as_str())
            .collect()
    };
    let least_loaded_open = |candidates: &[&ClusterMember]| -> Option<ClusterMember> {
        let open: Vec<&ClusterMember> = candidates
            .iter()
            .copied()
            .filter(|m| !full.contains(&m.node_id.as_str()))
            .collect();
        least_loaded(&open).or_else(|| least_loaded(candidates))
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
        if let Some(pick) = least_loaded_open(&local) {
            return Some(pick);
        }
    }

    if let Some((_, member)) = hosting.first() {
        return Some((*member).clone());
    }
    least_loaded_open(&live)
}

/// Pick the node a new caller of a webinar should be sent to
/// (`docs/WEBINARS.md` §2.7). The caller's role is not known yet: the PIN,
/// or the token, is read by the room.
///
/// - With a live stage (its node hosting the room, alive and not draining):
///   the stage while it hosts fewer than `stage_share_percent` of
///   `capacity`, else [`choose_conference_node`] over every other node, so
///   edges fill as rooms do, in the caller's region first. The stage again
///   when there is no other node. `capacity` 0 keeps the whole webinar on
///   the stage.
/// - Without one: [`choose_conference_node`], as for any room. The first
///   host or panelist to join claims the node they land on as the stage.
///
/// A host or panelist placed on an edge is moved to the stage by the room.
pub fn choose_webinar_node(
    record: &RoomRecord,
    members: &[ClusterMember],
    capacity: usize,
    stage_share_percent: u8,
    local_region: Option<&str>,
    video: Option<&VideoNeed>,
) -> Option<ClusterMember> {
    let stage = record.live_stage().and_then(|host| {
        members
            .iter()
            .find(|m| {
                m.node_id == host.node_id && m.node_type == CONFERENCE_NODE_TYPE && !m.draining
            })
            .map(|m| (host, m))
    });
    let Some((stage_host, stage_member)) = stage else {
        return choose_conference_node(&record.hosts, members, capacity, local_region, video);
    };
    let share = capacity * usize::from(stage_share_percent.min(100)) / 100;
    if capacity == 0 || stage_host.participants < share {
        return Some(stage_member.clone());
    }
    let edges: Vec<ConferenceHost> = record
        .hosts
        .iter()
        .filter(|h| h.node_id != stage_host.node_id)
        .cloned()
        .collect();
    let others: Vec<ClusterMember> = members
        .iter()
        .filter(|m| m.node_id != stage_host.node_id)
        .cloned()
        .collect();
    choose_conference_node(&edges, &others, capacity, local_region, video)
        .or_else(|| Some(stage_member.clone()))
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

    #[test]
    fn a_node_full_for_the_room_is_not_chosen_by_load_either() {
        // conf-a hosts the room at capacity; membership still shows both
        // nodes idle (the heartbeat is a second old) and conf-a sorts
        // first by id. The caller must go to conf-b.
        let members = vec![
            member("conf-a", "conference", 0),
            member("conf-b", "conference", 0),
        ];
        let pick = choose_conference_node(&[host("conf-a", 1)], &members, 1, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // The same in the caller's region.
        let mut regional = members.clone();
        regional[0].region = Some("east".into());
        regional[1].region = Some("east".into());
        let pick =
            choose_conference_node(&[host("conf-a", 1)], &regional, 1, Some("east"), None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // Every node full: the room overfills the least loaded one
        // rather than refusing the caller.
        let pick = choose_conference_node(
            &[host("conf-a", 1), host("conf-b", 1)],
            &members,
            1,
            None,
            None,
        )
        .unwrap();
        assert_eq!(pick.node_id, "conf-a");
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
            public_url: None,
            capacity: None,
            draining: false,
            region: None,
            video_budget: Vec::new(),
            registering_trunks: Vec::new(),
            view: None,
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

    fn webinar(hosts: Vec<ConferenceHost>, stage: Option<&str>) -> RoomRecord {
        RoomRecord {
            hosts,
            locked: false,
            stage: stage.map(str::to_string),
        }
    }

    #[test]
    fn the_stage_is_read_from_the_room_and_live_only_while_it_hosts() {
        let record = parse_room_hash(raw(&[("conf-a", "3@100"), ("__stage", "conf-a")]), 100, 60);
        assert_eq!(record.stage.as_deref(), Some("conf-a"));
        assert_eq!(record.live_stage().map(|h| h.participants), Some(3));
        assert_eq!(record.hosts.len(), 1, "the stage field is not a host");
        // The stage's own entry went stale: the stage has gone.
        let record = parse_room_hash(
            raw(&[
                ("conf-a", "3@10"),
                ("conf-b", "1@100"),
                ("__stage", "conf-a"),
            ]),
            100,
            60,
        );
        assert!(record.live_stage().is_none());
    }

    #[test]
    fn a_webinar_fills_its_stage_to_its_share_then_its_edges() {
        let members = vec![
            member("conf-a", "conference", 0),
            member("conf-b", "conference", 0),
            member("conf-c", "conference", 5),
        ];
        // Under half of 100 on the stage: the stage, though it is busiest.
        let record = webinar(vec![host("conf-a", 49)], Some("conf-a"));
        let pick = choose_webinar_node(&record, &members, 100, 50, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-a");
        // At its share: the least-loaded other node becomes an edge.
        let record = webinar(vec![host("conf-a", 50)], Some("conf-a"));
        let pick = choose_webinar_node(&record, &members, 100, 50, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // An edge already hosting fills before another is opened.
        let record = webinar(vec![host("conf-a", 50), host("conf-c", 20)], Some("conf-a"));
        let pick = choose_webinar_node(&record, &members, 100, 50, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-c");
        // Capacity 0 keeps the whole webinar on the stage.
        let record = webinar(vec![host("conf-a", 5_000)], Some("conf-a"));
        let pick = choose_webinar_node(&record, &members, 0, 50, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-a");
        // No other node: the stage overfills.
        let alone = vec![member("conf-a", "conference", 0)];
        let record = webinar(vec![host("conf-a", 80)], Some("conf-a"));
        let pick = choose_webinar_node(&record, &alone, 100, 50, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-a");
    }

    #[test]
    fn a_webinar_without_a_live_stage_is_placed_as_any_room() {
        let members = vec![
            member("conf-a", "conference", 0),
            member("conf-b", "conference", 0),
        ];
        // An attendee early in the lobby: today's rule keeps the room together.
        let record = webinar(vec![host("conf-b", 3)], None);
        let pick = choose_webinar_node(&record, &members, 100, 50, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // A stage that is draining is no stage.
        let mut draining = members.clone();
        draining[0].draining = true;
        let record = webinar(vec![host("conf-a", 3), host("conf-b", 1)], Some("conf-a"));
        let pick = choose_webinar_node(&record, &draining, 100, 50, None, None).unwrap();
        assert_eq!(pick.node_id, "conf-b");
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

    fn with_budget(m: ClusterMember, codecs: &[&str], left: u32) -> ClusterMember {
        with_device(m, "host", codecs, left)
    }

    fn with_device(
        mut m: ClusterMember,
        device: &str,
        codecs: &[&str],
        left: u32,
    ) -> ClusterMember {
        m.video_budget.push(crate::membership::VideoDeviceBudget {
            device: device.into(),
            codecs: codecs.iter().map(|c| c.to_string()).collect(),
            left,
            total: 8000,
            engines: None,
            egress: None,
        });
        m
    }

    fn with_egress(mut m: ClusterMember, left_kbps: u32) -> ClusterMember {
        if let Some(host) = m.video_budget.iter_mut().find(|b| b.device == "host") {
            host.egress = Some(crate::membership::VideoEgress {
                left_kbps,
                total_kbps: 1_000_000,
            });
        }
        m
    }

    #[test]
    fn a_node_out_of_egress_is_passed_over_for_one_with_its_link_free() {
        // The same codec budget; the busier link loses, and a link that
        // cannot carry a join's default view counts as out of budget —
        // behind even a node that says nothing of its link.
        let members = vec![
            with_egress(
                with_budget(member("conf-saturated", "conference", 0), &["VP8"], 4000),
                1_000,
            ),
            with_egress(
                with_budget(member("conf-busy-link", "conference", 0), &["VP8"], 4000),
                200_000,
            ),
            with_egress(
                with_budget(member("conf-free-link", "conference", 0), &["VP8"], 4000),
                900_000,
            ),
        ];
        let video = need(&["VP8"]);
        let pick = choose_conference_node(&[], &members, 0, None, Some(&video)).unwrap();
        assert_eq!(pick.node_id, "conf-free-link");
        let pick = choose_conference_node(&[], &members[..2], 0, None, Some(&video)).unwrap();
        assert_eq!(pick.node_id, "conf-busy-link");
        let unknown = with_budget(member("conf-unknown-link", "conference", 0), &["VP8"], 10);
        assert_eq!(unknown.video_egress_left_kbps(), None);
        let pick =
            choose_conference_node(&[], &[members[0].clone(), unknown], 0, None, Some(&video))
                .unwrap();
        assert_eq!(
            pick.node_id, "conf-unknown-link",
            "unknown beats too little"
        );
        assert_eq!(members[0].video_egress_left_kbps(), Some(1_000));
    }

    fn need(codecs: &[&str]) -> VideoNeed {
        VideoNeed::of(codecs.iter().map(|c| c.to_string()).collect())
    }

    #[test]
    fn a_gpu_node_is_preferred_only_for_meetings_whose_first_codec_it_encodes() {
        // The GPU node advertises its host beside its device; the device
        // encodes H.264 but not VP8 (§15.7, decision 5).
        let gpu = with_device(
            with_budget(member("conf-gpu", "conference", 0), &["H264", "VP8"], 2000),
            "cuda:0",
            &["H264", "H265", "AV1"],
            30000,
        );
        let cpu = with_budget(member("conf-cpu", "conference", 0), &["H264", "VP8"], 5000);
        let members = vec![gpu.clone(), cpu];
        // H.264 first: the GPU's device has by far the most left.
        let pick =
            choose_conference_node(&[], &members, 0, None, Some(&need(&["H264", "VP8"]))).unwrap();
        assert_eq!(pick.node_id, "conf-gpu");
        // VP8 first: the device does not count; the hosts compete, and
        // the CPU node's has more left.
        let pick =
            choose_conference_node(&[], &members, 0, None, Some(&need(&["VP8", "H264"]))).unwrap();
        assert_eq!(pick.node_id, "conf-cpu");
        assert_eq!(
            gpu.video_budget_left_for(&need(&["VP8", "H264"])),
            Some(2000)
        );
        assert_eq!(gpu.video_budget_left_for(&need(&["H264"])), Some(30000));
        // A first codec nobody's device encodes: any device with any of
        // the codecs, as before.
        assert_eq!(
            gpu.video_budget_left_for(&need(&["VP9", "H264"])),
            Some(30000)
        );
        // AV1 first: only the device has it.
        assert_eq!(gpu.video_budget_left_for(&need(&["AV1"])), Some(30000));
        let vp9 = VideoNeed {
            codecs: vec!["VP9".into()],
            first: Some("VP9".into()),
            egress_kbps: 0,
        };
        assert_eq!(gpu.video_budget_left_for(&vp9), Some(0), "nothing runs it");
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
