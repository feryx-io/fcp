//! Conference registry: which nodes host which conference room, and how
//! many participants each has there.
//!
//! Redis layout: one hash per room, `cluster:conference:<tenant>:<room>`,
//! field = node id, value = local participant count. Every write refreshes
//! the key's TTL, so a node that stops heartbeating its rooms disappears
//! from the registry on its own.
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
//! least-loaded conference node in the cluster.

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

/// Interpret a room hash: node fields become hosts, `__locked` becomes the
/// flag, anything else starting with `__` is ignored.
fn parse_room_hash(raw: Vec<(String, String)>) -> RoomRecord {
    let mut record = RoomRecord::default();
    for (field, value) in raw {
        if field == LOCKED_FIELD {
            record.locked = value == "1";
        } else if !field.starts_with("__") {
            record.hosts.push(ConferenceHost {
                node_id: field,
                participants: value.parse::<i64>().unwrap_or(0).max(0) as usize,
            });
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
    /// the node from the room) and refresh the room's TTL.
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
            let _: () = redis::pipe()
                .atomic()
                .hset(&key, &self.node_id, participants as i64)
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
        Ok(parse_room_hash(raw))
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

/// Pick the node a new participant of `room` should be sent to.
///
/// - `hosts`: nodes currently hosting the room (from the registry).
/// - `members`: live cluster members; only `conference` nodes qualify.
/// - `capacity`: participants per node per room before the room is spread
///   to another node; `0` means never spread.
///
/// A hosting node that is still live and under capacity wins (the fullest
/// one, so a room stays on as few nodes as possible). Otherwise the
/// least-loaded live conference node. `None` when no conference node is
/// alive, so callers fall back to static configuration.
pub fn choose_conference_node(
    hosts: &[ConferenceHost],
    members: &[ClusterMember],
    capacity: usize,
) -> Option<ClusterMember> {
    let live: Vec<&ClusterMember> = members
        .iter()
        .filter(|m| m.node_type == CONFERENCE_NODE_TYPE)
        .collect();
    if live.is_empty() {
        return None;
    }

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
    if let Some((_, member)) = hosting.first() {
        return Some((*member).clone());
    }

    live.iter()
        .min_by_key(|m| (m.active_calls, m.node_id.clone()))
        .map(|m| (*m).clone())
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
        let record = parse_room_hash(raw(&[
            ("conf-b", "12"),
            ("__locked", "1"),
            ("conf-a", "3"),
            ("__future", "x"),
            ("conf-c", "garbage"),
        ]));
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
        let unlocked = parse_room_hash(raw(&[("conf-a", "1"), ("__locked", "0")]));
        assert!(!unlocked.locked);
        assert!(!parse_room_hash(Vec::new()).locked);
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
        }
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
        let pick = choose_conference_node(&[], &members, 0).unwrap();
        assert_eq!(pick.node_id, "conf-b");
    }

    #[test]
    fn running_room_stays_on_its_node_under_capacity() {
        let members = vec![
            member("conf-a", "conference", 40),
            member("conf-b", "conference", 1),
        ];
        // conf-a hosts the room with 30 participants; capacity 100 → stay.
        let pick = choose_conference_node(&[host("conf-a", 30)], &members, 100).unwrap();
        assert_eq!(pick.node_id, "conf-a");
        // capacity 0 = never spread, even when a node is busy.
        let pick = choose_conference_node(&[host("conf-a", 30)], &members, 0).unwrap();
        assert_eq!(pick.node_id, "conf-a");
    }

    #[test]
    fn full_host_spreads_to_least_loaded_other_node() {
        let members = vec![
            member("conf-a", "conference", 40),
            member("conf-b", "conference", 5),
            member("conf-c", "conference", 9),
        ];
        let pick = choose_conference_node(&[host("conf-a", 100)], &members, 100).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // Once conf-b hosts too, it is preferred until it is full as well.
        let pick = choose_conference_node(&[host("conf-a", 100), host("conf-b", 3)], &members, 100)
            .unwrap();
        assert_eq!(pick.node_id, "conf-b");
        // The fullest under-capacity host wins, to keep the room compact.
        let pick = choose_conference_node(
            &[host("conf-a", 100), host("conf-b", 3), host("conf-c", 50)],
            &members,
            100,
        )
        .unwrap();
        assert_eq!(pick.node_id, "conf-c");
    }

    #[test]
    fn dead_hosts_are_ignored_and_no_conference_nodes_means_none() {
        let members = vec![member("conf-b", "conference", 5)];
        // conf-a is in the registry but not in membership: skip it.
        let pick = choose_conference_node(&[host("conf-a", 3)], &members, 100).unwrap();
        assert_eq!(pick.node_id, "conf-b");
        assert!(choose_conference_node(
            &[host("conf-a", 3)],
            &[member("cm-1", "call_manager", 0)],
            100
        )
        .is_none());
    }
}
