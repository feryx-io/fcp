//! What one node sees of the trunks it routes by (`docs/TENANT_TRUNKS.md` §9).
//!
//! Health is per node and always was: a carrier unreachable from one node
//! may be reachable from another, and a call should be placed by a node
//! that can reach it. These are the shapes a node publishes and an API
//! reads, so they live here rather than beside either one.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// What one node sees of one trunk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrunkNodeView {
    /// The node this is the view of. Absent inside a node's own report,
    /// which names its node once; set where views of one trunk from many
    /// nodes are gathered, as `/v1/trunks` does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub trunk_id: String,
    pub name: String,
    /// The platform's own trunks have no tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    pub enabled: bool,
    /// `healthy`, `degraded`, `down` or `unknown` — what this node's own
    /// probes have found, not what another node found.
    pub health: String,
    pub consecutive_failures: u32,
    /// Calls this node has on the trunk right now. Capacity is per node
    /// (§13.3), so this is a share of the carrier's channels, not all of
    /// them.
    pub calls_in_flight: u64,
    /// Whether this node would route a call to the trunk now: enabled,
    /// not administratively disabled and not down.
    pub routable: bool,
    /// For a registering trunk, what this node's binding is doing:
    /// `registered`, `pending`, `failed` or `unregistered`. Absent on a
    /// trunk that does not register, and `unregistered` on one another
    /// node holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration: Option<String>,
    /// The carrier account bound, when the trunk registers as itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// Every trunk one node routes by, as that node sees them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrunkNodeReport {
    pub node_id: String,
    #[serde(with = "epoch_secs")]
    pub reported_at: SystemTime,
    pub trunks: Vec<TrunkNodeView>,
}

mod epoch_secs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S: Serializer>(at: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        at.duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(UNIX_EPOCH + Duration::from_secs(secs))
    }
}

/// What happened on each trunk here (`docs/TENANT_TRUNKS.md` §9).
///
/// Per trunk and tenant, because a number for "the carrier" across every
/// tenant answers no question an operator has: whose calls a carrier
/// refused is the question, and a platform trunk carries everyone's.
#[derive(Debug, Default)]
pub struct TrunkCounters {
    outcomes: std::sync::Mutex<std::collections::BTreeMap<(String, String, &'static str), u64>>,
}

/// What a call did on a trunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrunkOutcome {
    /// A call was sent to the trunk, or admitted from it.
    Attempted,
    /// The trunk answered it.
    Answered,
    /// The trunk did not, and the call went to the next one.
    FailedOver,
    /// Refused here: the trunk's channels are full.
    RejectedCapacity,
    /// Refused here: the trunk's calls per second are spent.
    RejectedCps,
}

impl TrunkOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Attempted => "attempted",
            Self::Answered => "answered",
            Self::FailedOver => "failed_over",
            Self::RejectedCapacity => "rejected_capacity",
            Self::RejectedCps => "rejected_cps",
        }
    }
}

impl TrunkCounters {
    /// The process's counters.
    pub fn global() -> &'static TrunkCounters {
        static COUNTERS: std::sync::OnceLock<TrunkCounters> = std::sync::OnceLock::new();
        COUNTERS.get_or_init(TrunkCounters::default)
    }

    /// Note what a call did on a trunk. `tenant` is empty for the
    /// platform's own trunks, which carry any tenant's calls.
    pub fn note(&self, trunk_id: &str, tenant: Option<&str>, outcome: TrunkOutcome) {
        if let Ok(mut map) = self.outcomes.lock() {
            *map.entry((
                trunk_id.to_string(),
                tenant.unwrap_or_default().to_string(),
                outcome.as_str(),
            ))
            .or_insert(0) += 1;
        }
    }

    /// Prometheus text: calls by trunk, tenant and outcome. `labels` is
    /// prepended to each series' labels (`node="…"`), or empty.
    pub fn prometheus(&self, labels: &str) -> String {
        let Ok(map) = self.outcomes.lock() else {
            return String::new();
        };
        if map.is_empty() {
            return String::new();
        }
        let sep = if labels.is_empty() { "" } else { "," };
        let mut out = String::from(
            "# HELP fcp_trunk_calls_total Calls by trunk, tenant and outcome.\n\
             # TYPE fcp_trunk_calls_total counter\n",
        );
        for ((trunk_id, tenant, outcome), n) in map.iter() {
            out.push_str(&format!(
                "fcp_trunk_calls_total{{{labels}{sep}trunk_id=\"{trunk_id}\",\
                 tenant=\"{tenant}\",outcome=\"{outcome}\"}} {n}\n"
            ));
        }
        out
    }
}

#[cfg(test)]
mod counter_tests {
    use super::*;

    /// One trunk's calls are counted per tenant and outcome, and the
    /// platform's trunks — which carry anyone's calls — under no tenant.
    #[test]
    fn calls_are_counted_by_trunk_tenant_and_outcome() {
        let counters = TrunkCounters::default();
        counters.note("trunk-1", Some("tenant-a"), TrunkOutcome::Attempted);
        counters.note("trunk-1", Some("tenant-a"), TrunkOutcome::Answered);
        counters.note("trunk-1", Some("tenant-b"), TrunkOutcome::Attempted);
        counters.note("trunk-1", Some("tenant-b"), TrunkOutcome::RejectedCapacity);
        counters.note("trunk-2", None, TrunkOutcome::FailedOver);

        let text = counters.prometheus("node=\"a\"");
        assert!(text.contains(
            "fcp_trunk_calls_total{node=\"a\",trunk_id=\"trunk-1\",tenant=\"tenant-a\",outcome=\"answered\"} 1"
        ), "{text}");
        assert!(text.contains(
            "fcp_trunk_calls_total{node=\"a\",trunk_id=\"trunk-1\",tenant=\"tenant-b\",outcome=\"rejected_capacity\"} 1"
        ), "{text}");
        assert!(
            text.contains("trunk_id=\"trunk-2\",tenant=\"\",outcome=\"failed_over\"} 1"),
            "{text}"
        );
    }

    /// Nothing happened, nothing is said: an empty series is noise in a
    /// scrape.
    #[test]
    fn a_node_that_has_carried_nothing_says_nothing() {
        assert!(TrunkCounters::default().prometheus("").is_empty());
    }
}
