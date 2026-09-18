//! Participant migration off a draining node
//! (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5).
//!
//! When this node drains it asks whoever holds each caller's leg to
//! re-place the call on another node: an in-dialog REFER whose `Refer-To`
//! names the room on the target with a [`MigrationTicket`] as a URI
//! header. The leg's holder dials the target with the ticket, reports
//! with a `message/sipfrag` NOTIFY, and BYEs the old leg. This module is
//! the parts of that with no SIP in them: which caller goes where
//! ([`plan`]), the ledger of REFERs in flight ([`Migrations`]), the
//! tickets a node has already admitted ([`UsedTickets`]), and the
//! `Refer-To` and sipfrag forms.

use dashmap::DashMap;
use fcp_cluster::{choose_conference_node, ClusterMember, ConferenceHost, VideoNeed};
use fcp_core::{MigrationTicket, TenantId, MIGRATION_TICKET_HEADER};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How many REFERs go out at once when a node drains.
pub const REFER_CONCURRENCY: usize = 32;

/// How long one REFER may take to be answered.
pub const REFER_TIMEOUT: Duration = Duration::from_secs(5);

/// A caller this node may move: not a trunk, not in the waiting room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub call_id: String,
    pub tenant_id: TenantId,
    pub room_number: String,
    /// The meeting's video, when it has any: placement then prefers a
    /// node with budget for it.
    pub video: Option<VideoNeed>,
}

/// Where a candidate goes.
#[derive(Debug, Clone)]
pub struct Assignment {
    pub call_id: String,
    pub target: ClusterMember,
}

/// What [`plan`] decided.
#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub assignments: Vec<Assignment>,
    /// Callers with nowhere to go: no other live conference node.
    pub unplaced: usize,
}

/// Choose a target for every candidate with the rule the call manager
/// applies to a new caller (`choose_conference_node`): a node already
/// hosting the room first (capacity 0: always, so the room stays on as
/// few nodes as possible), else the least-loaded live conference node,
/// in `local_region` first. The draining node never appears: membership
/// shows it draining and the rule skips those. `hosts` is each room's
/// registry entry, keyed by `(tenant, room)`.
pub fn plan(
    candidates: Vec<Candidate>,
    hosts: &HashMap<(String, String), Vec<ConferenceHost>>,
    members: &[ClusterMember],
    local_region: Option<&str>,
) -> Plan {
    let mut out = Plan::default();
    for c in candidates {
        let key = (c.tenant_id.to_string(), c.room_number.clone());
        let room_hosts = hosts.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        match choose_conference_node(room_hosts, members, 0, local_region, c.video.as_ref()) {
            Some(target) => out.assignments.push(Assignment {
                call_id: c.call_id,
                target,
            }),
            None => out.unplaced += 1,
        }
    }
    out
}

/// The `Refer-To` for a migration: the room on the target's SIP address,
/// the ticket as the one URI header the leg's holder copies into its
/// INVITE.
pub fn refer_to(room_number: &str, target: &ClusterMember, ticket: &str) -> String {
    format!(
        "sip:{}@{}?{}={}",
        room_number, target.sip_address, MIGRATION_TICKET_HEADER, ticket
    )
}

/// The status a `message/sipfrag` body reports: `SIP/2.0 200 OK` → 200.
pub fn sipfrag_status(body: &str) -> Option<u16> {
    let line = body.lines().next()?.trim();
    let mut parts = line.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("SIP/2.0") {
        return None;
    }
    parts.next()?.parse().ok()
}

/// How a REFER in flight stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Accepted (202); the leg's holder is dialing the target.
    Referred,
    /// The holder reported 2xx: the caller is on the target, and this
    /// node is dropping its leg.
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrating {
    pub target_node: String,
    pub stage: Stage,
    pub deadline: Instant,
}

/// What a NOTIFY meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// A provisional (1xx): still dialing.
    Pending,
    /// The caller is on the target.
    Succeeded,
    /// The target refused, or the holder gave up; the caller stays here.
    Failed(u16),
}

/// The REFERs this node has sent and not yet seen through, by Call-ID.
#[derive(Default)]
pub struct Migrations {
    entries: DashMap<String, Migrating>,
}

impl Migrations {
    /// A REFER was accepted; the holder has until `timeout` to report.
    pub fn begin(&self, call_id: &str, target_node: &str, timeout: Duration) {
        self.entries.insert(
            call_id.to_string(),
            Migrating {
                target_node: target_node.to_string(),
                stage: Stage::Referred,
                deadline: Instant::now() + timeout,
            },
        );
    }

    pub fn is_migrating(&self, call_id: &str) -> bool {
        self.entries.contains_key(call_id)
    }

    /// A NOTIFY with `code` arrived for `call_id`. `None` when the call
    /// is not migrating. A failure removes the entry: the caller stays.
    pub fn on_notify(&self, call_id: &str, code: u16) -> Option<Notice> {
        let mut entry = self.entries.get_mut(call_id)?;
        match code {
            100..=199 => Some(Notice::Pending),
            200..=299 => {
                entry.stage = Stage::Done;
                Some(Notice::Succeeded)
            }
            _ => {
                drop(entry);
                self.entries.remove(call_id);
                Some(Notice::Failed(code))
            }
        }
    }

    /// The entry for a leg that is going away, if it was migrating: the
    /// leave is then a migration, whether the holder reported first or
    /// simply hung up.
    pub fn take(&self, call_id: &str) -> Option<Migrating> {
        self.entries.remove(call_id).map(|(_, m)| m)
    }

    /// REFERs still waiting for a report.
    pub fn pending(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.stage == Stage::Referred)
            .count()
    }

    /// Drop the REFERs whose holder never reported; those callers stay.
    /// Returns their Call-IDs.
    pub fn expire(&self, now: Instant) -> Vec<String> {
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.stage == Stage::Referred && e.deadline <= now)
            .map(|e| e.key().clone())
            .collect();
        for id in &expired {
            self.entries.remove(id);
        }
        expired
    }
}

/// Tickets this node has admitted, by nonce, so one admits once.
#[derive(Default)]
pub struct UsedTickets {
    seen: DashMap<String, Instant>,
}

impl UsedTickets {
    /// Record `ticket` as used; `false` when it already was.
    pub fn admit(&self, ticket: &MigrationTicket) -> bool {
        self.sweep();
        self.seen
            .insert(ticket.nonce.clone(), Instant::now())
            .is_none()
    }

    /// Forget nonces older than a ticket's lifetime: their tickets have
    /// expired on their own.
    fn sweep(&self) {
        let ttl = fcp_core::MIGRATION_TICKET_TTL;
        self.seen.retain(|_, at| at.elapsed() < ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_cluster::CONFERENCE_NODE_TYPE;

    fn member(id: &str, region: Option<&str>, calls: u64, draining: bool) -> ClusterMember {
        ClusterMember {
            node_id: id.into(),
            node_type: CONFERENCE_NODE_TYPE.into(),
            http_address: "127.0.0.1:8080".parse().unwrap(),
            sip_address: format!("10.0.0.{}:5074", id.len()).parse().unwrap(),
            sip_tls_address: None,
            capabilities: vec![],
            active_calls: calls as usize,
            started_at: 0,
            version: String::new(),
            draining,
            region: region.map(String::from),
            video_budget: vec![],
            registering_trunks: Vec::new(),
        }
    }

    fn candidate(call: &str, tenant: TenantId, room: &str) -> Candidate {
        Candidate {
            call_id: call.into(),
            tenant_id: tenant,
            room_number: room.into(),
            video: None,
        }
    }

    #[test]
    fn a_hosting_node_is_preferred_and_the_draining_node_never_chosen() {
        let tenant = TenantId::new();
        let me = member("a", Some("east"), 3, true);
        let busy_host = member("bb", Some("east"), 40, false);
        let idle = member("ccc", Some("east"), 0, false);
        let members = vec![me, busy_host, idle];
        let mut hosts = HashMap::new();
        hosts.insert(
            (tenant.to_string(), "8000".to_string()),
            vec![
                ConferenceHost {
                    node_id: "a".into(),
                    participants: 3,
                },
                ConferenceHost {
                    node_id: "bb".into(),
                    participants: 2,
                },
            ],
        );
        let plan = plan(
            vec![
                candidate("c1", tenant, "8000"),
                candidate("c2", tenant, "8000"),
                candidate("c3", tenant, "9000"),
            ],
            &hosts,
            &members,
            Some("east"),
        );
        assert_eq!(plan.unplaced, 0);
        let by_call: HashMap<_, _> = plan
            .assignments
            .iter()
            .map(|a| (a.call_id.as_str(), a.target.node_id.as_str()))
            .collect();
        // The room's other host takes its callers, however busy; a room
        // nobody else hosts goes to the least loaded node.
        assert_eq!(by_call["c1"], "bb");
        assert_eq!(by_call["c2"], "bb");
        assert_eq!(by_call["c3"], "ccc");
    }

    #[test]
    fn nobody_to_go_to_is_counted_not_assigned() {
        let tenant = TenantId::new();
        let members = vec![member("a", None, 3, true)];
        let plan = plan(
            vec![candidate("c1", tenant, "8000")],
            &HashMap::new(),
            &members,
            None,
        );
        assert!(plan.assignments.is_empty());
        assert_eq!(plan.unplaced, 1);
    }

    #[test]
    fn the_refer_to_carries_the_ticket_as_a_uri_header() {
        let target = member("bb", None, 0, false);
        let uri = refer_to("8000", &target, "abc.def");
        assert_eq!(uri, "sip:8000@10.0.0.2:5074?X-FCP-Migrate=abc.def");
        let parsed = sip_core::SipUri::parse(&uri).unwrap();
        assert_eq!(parsed.user(), Some("8000"));
        assert_eq!(
            parsed.headers().get("x-fcp-migrate").map(|v| v.as_str()),
            Some("abc.def")
        );
        assert!(parsed.without_uri_headers().headers().is_empty());
    }

    #[test]
    fn sipfrag_status_reads_the_first_line() {
        assert_eq!(sipfrag_status("SIP/2.0 200 OK\r\n"), Some(200));
        assert_eq!(sipfrag_status("SIP/2.0 100 Trying"), Some(100));
        assert_eq!(
            sipfrag_status("SIP/2.0 503 Service Unavailable\r\nX: y"),
            Some(503)
        );
        assert_eq!(sipfrag_status(""), None);
        assert_eq!(sipfrag_status("INVITE sip:x SIP/2.0"), None);
    }

    #[test]
    fn the_ledger_follows_a_refer_to_its_end() {
        let m = Migrations::default();
        assert_eq!(m.on_notify("c1", 200), None, "not migrating");
        m.begin("c1", "bb", Duration::from_secs(10));
        m.begin("c2", "bb", Duration::from_secs(10));
        m.begin("c3", "bb", Duration::from_secs(0));
        assert_eq!(m.pending(), 3);
        assert_eq!(m.on_notify("c1", 100), Some(Notice::Pending));
        assert_eq!(m.on_notify("c1", 200), Some(Notice::Succeeded));
        assert_eq!(m.pending(), 2, "done is not pending");
        assert_eq!(m.on_notify("c2", 503), Some(Notice::Failed(503)));
        assert!(!m.is_migrating("c2"), "a failure leaves the caller here");
        // c3's holder never reported.
        assert_eq!(m.expire(Instant::now()), vec!["c3".to_string()]);
        assert_eq!(m.pending(), 0);
        // The leg that succeeded leaves as a migration.
        let done = m.take("c1").unwrap();
        assert_eq!((done.target_node.as_str(), done.stage), ("bb", Stage::Done));
        assert!(m.take("c1").is_none());
    }

    #[test]
    fn a_ticket_admits_once() {
        let used = UsedTickets::default();
        let t = MigrationTicket {
            version: 1,
            tenant_id: TenantId::new(),
            room_number: "8000".into(),
            participant_id: "p".into(),
            display_name: None,
            caller_id: None,
            is_host: false,
            muted: false,
            gain: 1.0,
            video: true,
            token_join: false,
            from_node: "a".into(),
            expires_at: MigrationTicket::expiry_from_now(),
            nonce: MigrationTicket::new_nonce(),
        };
        assert!(used.admit(&t));
        assert!(!used.admit(&t));
        let other = MigrationTicket {
            nonce: MigrationTicket::new_nonce(),
            ..t
        };
        assert!(used.admit(&other));
    }
}
