//! What reaches this node's SIP port, and what a flood of it costs
//! (`docs/EDGE_SECURITY.md` §2.4).
//!
//! The call manager is the edge in every single-binary deployment, in
//! `deploy/docker-compose` and in the quickstart, and it had none of the
//! defences the SBC has: no address list, no per-source call rate, no
//! policy for a message that is not SIP. `[admission] max_cps` is a
//! node-wide budget, so one source could spend all of it, and the shared
//! handler's own limiters are per-dialog, which bounds one caller's
//! mischief and nothing about the node.
//!
//! This guard runs in the shared handler, so every binary built on it can
//! have the same defences. It sits in two places, because the two kinds of
//! check can do different things:
//!
//! - **Before parsing**, on the bytes: the address list, the message size
//!   and whether this is SIP at all. Each of these **drops**. A peer the
//!   node will not talk to is told nothing — answering an address that is
//!   denied confirms the port is live and makes the node a reflector —
//!   and a message that is not SIP has nothing to answer to.
//! - **On a parsed request**, where there is a transaction to answer on:
//!   the per-source call rate, which is refused `503` with a
//!   `Retry-After`, the same way every other refusal on the platform is
//!   (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
//!
//! The SBC keeps its own richer pipeline, which runs before its handler
//! and answers rather than drops; the ACL types below are its types,
//! moved here so there is one implementation of them rather than two.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use tracing::{debug, warn};

/// ACL rule action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AclAction {
    /// Allow the traffic.
    #[default]
    Allow,

    /// Deny the traffic.
    Deny,
}

/// A single IP ACL rule entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IpAclEntry {
    /// CIDR notation network (e.g. "10.0.0.0/8", "192.168.1.1/32").
    pub cidr: String,

    /// Action to take when this rule matches.
    pub action: AclAction,

    /// Optional description for this rule.
    pub description: Option<String>,
}

/// IP access control list.
///
/// Rules are evaluated in order; the first matching rule wins.
/// If no rule matches, `default_action` is returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct IpAcl {
    /// Ordered list of ACL rules.
    pub rules: Vec<IpAclEntry>,

    /// Action when no rule matches.
    pub default_action: AclAction,
}

impl Default for IpAcl {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            default_action: AclAction::Allow,
        }
    }
}

impl IpAcl {
    /// Evaluate the ACL for a given IP address.
    ///
    /// Returns the action from the first matching rule, or `default_action`
    /// if no rule matches. Unparseable CIDR entries are skipped.
    pub fn evaluate(&self, ip: IpAddr) -> AclAction {
        // An IPv4 peer on a dual-stack socket arrives mapped into IPv6, and
        // a rule written as IPv4 has to match it.
        let ip = unmap(ip);
        for entry in &self.rules {
            if let Ok(network) = entry.cidr.parse::<ipnetwork::IpNetwork>() {
                if network.contains(ip) {
                    return entry.action;
                }
            }
        }
        self.default_action
    }

    /// Validate all ACL entries.
    ///
    /// Returns `Err` if:
    /// - Any `cidr` value cannot be parsed as an IP network
    /// - Any `description` contains CR/LF characters
    pub fn validate(&self, context: &str) -> Result<(), String> {
        for (i, entry) in self.rules.iter().enumerate() {
            if entry.cidr.parse::<ipnetwork::IpNetwork>().is_err() {
                return Err(format!(
                    "{context}: rule {i} has invalid CIDR: {:?}",
                    entry.cidr
                ));
            }
            if let Some(description) = &entry.description {
                if description.contains(['\r', '\n']) {
                    return Err(format!(
                        "{context}: rule {i} description must not contain CR/LF characters"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Policy for handling messages that are not SIP.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MalformedPolicy {
    /// Drop it and say nothing, not even in the log. What a node under a
    /// flood of rubbish wants, since the log is a cost too.
    Drop,

    /// Log it and drop it. The default, and what the handler did before
    /// there was a policy.
    #[default]
    LogAndDrop,
}

/// An IPv4 peer on a dual-stack socket arrives mapped into IPv6.
fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

/// What this node accepts at its SIP port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct EdgeConfig {
    /// Addresses this node answers at all. Empty rules with an `Allow`
    /// default — which is the default — refuse nobody.
    pub acl: IpAcl,

    /// The largest SIP message this node will look at. A datagram larger
    /// than this is dropped before it is parsed.
    pub max_message_bytes: usize,

    /// What to do with a message that is not SIP.
    pub malformed_policy: MalformedPolicy,

    /// Calls per second one source address may start. Zero is no limit.
    ///
    /// The default bounds a flood without refusing anything a working
    /// deployment does: a busy office behind one NAT peaks at a few calls
    /// a second, and this is several times that. A carrier trunk that
    /// legitimately exceeds it is either listed in `[e911] trusted_edges`,
    /// which is exempt, or given a higher limit here.
    ///
    /// Every INVITE counts, before anything is checked of it: a call
    /// challenged under `[registrar] auth_invites` costs two, the challenged
    /// attempt and the retry with credentials. Not counting an INVITE that
    /// carries credentials would mean checking them first, a database read
    /// for every packet of a flood.
    pub max_invite_cps_per_source: u32,
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            acl: IpAcl::default(),
            max_message_bytes: 65_536,
            malformed_policy: MalformedPolicy::default(),
            max_invite_cps_per_source: 20,
        }
    }
}

impl EdgeConfig {
    /// Validate the configuration, so a typo is a startup error rather
    /// than a node that quietly refuses everyone.
    pub fn validate(&self) -> Result<(), String> {
        self.acl.validate("security.acl")?;
        if self.max_message_bytes == 0 || self.max_message_bytes > 1_048_576 {
            return Err("security.max_message_bytes must be between 1 and 1,048,576".into());
        }
        if self.max_invite_cps_per_source > 1_000_000 {
            return Err("security.max_invite_cps_per_source must not exceed 1,000,000".into());
        }
        Ok(())
    }
}

/// What the guard says about one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeVerdict {
    /// Carry on.
    Allow,
    /// Say nothing. The reason is for the log and the counter, not the
    /// peer.
    Drop(&'static str),
}

/// The defences at this node's SIP port.
pub struct EdgeGuard {
    config: EdgeConfig,
    /// Peers exempt from the per-source call rate: an edge or an inter-node
    /// peer carries other people's calls by design, so its rate is not one
    /// device's.
    trusted_edges: crate::emergency::TrustedEdges,
    /// Calls started this second, per source address.
    invites: PerSecond,
}

impl EdgeGuard {
    pub fn new(config: EdgeConfig, trusted_edges: crate::emergency::TrustedEdges) -> Self {
        Self {
            config,
            trusted_edges,
            invites: PerSecond::default(),
        }
    }

    /// What this node accepts, for a caller that wants to report it.
    pub fn config(&self) -> &EdgeConfig {
        &self.config
    }

    /// Whether these bytes get as far as the parser.
    ///
    /// Everything here **drops**: a denied address is told nothing, and a
    /// message that is not SIP has nothing to answer to.
    pub fn admit_bytes(&self, peer: IpAddr, payload: &[u8]) -> EdgeVerdict {
        if self.config.acl.evaluate(peer) == AclAction::Deny {
            debug!(peer = %peer, "Dropping a message from an address this node does not answer");
            return EdgeVerdict::Drop("acl");
        }
        if payload.len() > self.config.max_message_bytes {
            warn!(
                peer = %peer,
                bytes = payload.len(),
                max = self.config.max_message_bytes,
                "Dropping an oversized SIP message"
            );
            return EdgeVerdict::Drop("oversize");
        }
        if !looks_like_sip(payload) {
            if self.config.malformed_policy == MalformedPolicy::LogAndDrop {
                warn!(peer = %peer, bytes = payload.len(), "Dropping a message that is not SIP");
            }
            return EdgeVerdict::Drop("malformed");
        }
        EdgeVerdict::Allow
    }

    /// Whether this source may start another call this second.
    ///
    /// Counted only for the request that starts one: a re-INVITE inside a
    /// dialog is the shared handler's `reinvite_rate_limit` to bound, and
    /// counting it here would refuse a long call for holding twice.
    pub fn admit_invite(&self, peer: IpAddr) -> EdgeVerdict {
        let limit = self.config.max_invite_cps_per_source;
        if limit == 0 || self.trusted_edges.contains(peer) {
            return EdgeVerdict::Allow;
        }
        if self.invites.try_record(&unmap(peer).to_string(), limit) {
            self.invites.prune();
            return EdgeVerdict::Allow;
        }
        warn!(
            peer = %peer,
            limit,
            "Refusing an INVITE: this source is starting calls faster than the node allows"
        );
        EdgeVerdict::Drop("source_rate")
    }
}

/// Whether these bytes are worth handing to the parser.
///
/// A first line that ends in `SIP/2.0` (a request) or begins with it (a
/// response). Deliberately shallow: this is not a second parser, it is
/// the cheapest way to tell SIP from a port scan, a TLS handshake sent to
/// the wrong port, or a UDP amplification probe.
fn looks_like_sip(payload: &[u8]) -> bool {
    let Some(end) = payload.iter().position(|&b| b == b'\n') else {
        return false;
    };
    let Ok(first) = std::str::from_utf8(&payload[..end]) else {
        return false;
    };
    let first = first.trim_end_matches('\r').trim_end();
    if first.starts_with("SIP/2.0 ") {
        return true;
    }
    // METHOD SP URI SP SIP/2.0
    first.ends_with("SIP/2.0") && first.split(' ').count() >= 3
}

/// Events in the current second, per key. The same shape the admission
/// controller uses for REGISTER floods, kept here so the edge guard does
/// not need one.
#[derive(Default)]
struct PerSecond {
    windows: dashmap::DashMap<String, (u64, u32)>,
}

impl PerSecond {
    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Record one event on `key` unless that would take it past `limit`
    /// this second. Check and record are one step.
    fn try_record(&self, key: &str, limit: u32) -> bool {
        let at = Self::now();
        let mut window = self.windows.entry(key.to_string()).or_insert((at, 0));
        if window.0 != at {
            *window = (at, 0);
        }
        if limit != 0 && window.1 >= limit {
            return false;
        }
        window.1 += 1;
        true
    }

    /// Forget keys not seen this second or last, so a node that has met a
    /// million source addresses does not remember them.
    fn prune(&self) {
        let now = Self::now();
        self.windows
            .retain(|_, (at, _)| now.saturating_sub(*at) < 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("address")
    }

    fn guard(config: EdgeConfig) -> EdgeGuard {
        EdgeGuard::new(config, Default::default())
    }

    const INVITE: &[u8] = b"INVITE sip:bob@fcp.local SIP/2.0\r\nVia: SIP/2.0/UDP 10.0.0.9\r\n\r\n";

    #[test]
    fn nothing_is_refused_by_default() {
        let guard = guard(EdgeConfig::default());
        assert_eq!(
            guard.admit_bytes(ip("203.0.113.9"), INVITE),
            EdgeVerdict::Allow
        );
        assert_eq!(guard.admit_invite(ip("203.0.113.9")), EdgeVerdict::Allow);
    }

    #[test]
    fn an_address_the_node_denies_is_told_nothing() {
        // Answering a denied address confirms the port is live and makes
        // the node a reflector (`docs/EDGE_SECURITY.md` §2.4).
        let config = EdgeConfig {
            acl: IpAcl {
                rules: vec![IpAclEntry {
                    cidr: "203.0.113.0/24".into(),
                    action: AclAction::Deny,
                    description: None,
                }],
                default_action: AclAction::Allow,
            },
            ..Default::default()
        };
        let guard = guard(config);
        assert_eq!(
            guard.admit_bytes(ip("203.0.113.9"), INVITE),
            EdgeVerdict::Drop("acl")
        );
        assert_eq!(
            guard.admit_bytes(ip("198.51.100.9"), INVITE),
            EdgeVerdict::Allow
        );
    }

    #[test]
    fn a_deny_by_default_acl_admits_only_what_it_lists() {
        let config = EdgeConfig {
            acl: IpAcl {
                rules: vec![IpAclEntry {
                    cidr: "10.0.0.0/8".into(),
                    action: AclAction::Allow,
                    description: Some("the office".into()),
                }],
                default_action: AclAction::Deny,
            },
            ..Default::default()
        };
        let guard = guard(config);
        assert_eq!(
            guard.admit_bytes(ip("10.1.2.3"), INVITE),
            EdgeVerdict::Allow
        );
        assert_eq!(
            guard.admit_bytes(ip("203.0.113.9"), INVITE),
            EdgeVerdict::Drop("acl")
        );
    }

    #[test]
    fn an_ipv4_rule_matches_a_mapped_peer() {
        // A dual-stack socket hands IPv4 peers over as ::ffff:a.b.c.d.
        let config = EdgeConfig {
            acl: IpAcl {
                rules: vec![IpAclEntry {
                    cidr: "203.0.113.0/24".into(),
                    action: AclAction::Deny,
                    description: None,
                }],
                default_action: AclAction::Allow,
            },
            ..Default::default()
        };
        assert_eq!(
            guard(config).admit_bytes(ip("::ffff:203.0.113.9"), INVITE),
            EdgeVerdict::Drop("acl")
        );
    }

    #[test]
    fn an_oversized_message_is_dropped_before_it_is_parsed() {
        let config = EdgeConfig {
            max_message_bytes: 64,
            ..Default::default()
        };
        let big = vec![b'x'; 128];
        assert_eq!(
            guard(config).admit_bytes(ip("203.0.113.9"), &big),
            EdgeVerdict::Drop("oversize")
        );
    }

    #[test]
    fn what_is_not_sip_is_dropped() {
        let guard = guard(EdgeConfig::default());
        for rubbish in [
            &b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"[..],
            &b"\x16\x03\x01\x00\xa5\x01"[..], // a TLS hello at the wrong port
            &b"hello?"[..],
            &b""[..],
        ] {
            assert_eq!(
                guard.admit_bytes(ip("203.0.113.9"), rubbish),
                EdgeVerdict::Drop("malformed"),
                "{:?} is not SIP",
                String::from_utf8_lossy(rubbish)
            );
        }
    }

    #[test]
    fn a_response_is_sip_too() {
        let guard = guard(EdgeConfig::default());
        let response = b"SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP 10.0.0.9\r\n\r\n";
        assert_eq!(
            guard.admit_bytes(ip("203.0.113.9"), response),
            EdgeVerdict::Allow
        );
    }

    #[test]
    fn one_source_cannot_spend_the_whole_node() {
        // `[admission] max_cps` is node-wide, so without this one source
        // spends all of it (`docs/EDGE_SECURITY.md` §2.4).
        let config = EdgeConfig {
            max_invite_cps_per_source: 3,
            ..Default::default()
        };
        let guard = guard(config);
        let flooder = ip("203.0.113.9");
        for _ in 0..3 {
            assert_eq!(guard.admit_invite(flooder), EdgeVerdict::Allow);
        }
        assert_eq!(
            guard.admit_invite(flooder),
            EdgeVerdict::Drop("source_rate")
        );
        // Its neighbour is unaffected: the limit is per source, not a
        // node-wide budget one source can exhaust.
        assert_eq!(guard.admit_invite(ip("203.0.113.10")), EdgeVerdict::Allow);
    }

    #[test]
    fn a_trusted_edge_carries_other_peoples_calls() {
        let config = EdgeConfig {
            max_invite_cps_per_source: 1,
            ..Default::default()
        };
        let edges = crate::emergency::TrustedEdges::parse(&["203.0.113.9".to_string()])
            .expect("edges parse");
        let guard = EdgeGuard::new(config, edges);
        for _ in 0..10 {
            assert_eq!(guard.admit_invite(ip("203.0.113.9")), EdgeVerdict::Allow);
        }
        assert_eq!(
            guard.admit_invite(ip("203.0.113.10")),
            EdgeVerdict::Allow,
            "the first one is within the limit"
        );
        assert_eq!(
            guard.admit_invite(ip("203.0.113.10")),
            EdgeVerdict::Drop("source_rate")
        );
    }

    #[test]
    fn zero_is_no_limit() {
        let config = EdgeConfig {
            max_invite_cps_per_source: 0,
            ..Default::default()
        };
        let guard = guard(config);
        for _ in 0..1000 {
            assert_eq!(guard.admit_invite(ip("203.0.113.9")), EdgeVerdict::Allow);
        }
    }

    #[test]
    fn a_typo_is_a_startup_error() {
        let bad = EdgeConfig {
            acl: IpAcl {
                rules: vec![IpAclEntry {
                    cidr: "10.0.0.0/33".into(),
                    action: AclAction::Deny,
                    description: None,
                }],
                default_action: AclAction::Allow,
            },
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        assert!(EdgeConfig::default().validate().is_ok());
    }
}
