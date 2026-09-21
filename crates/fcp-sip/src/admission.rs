//! What this node accepts.
//!
//! The call path asks this before it does the work of a call, and before a
//! REGISTER reaches the credential store. The implementation lives in
//! `fcp_sbc_core::admission::AdmissionController`, one controller for every
//! routing binary (`docs/CAPACITY_AND_SURVIVABILITY.md` §5); the interface
//! is here so the SIP layer can call it without depending on the SBC.
//!
//! Emergency calls never reach these checks: the handler skips them, as it
//! does for a trunk's limits (`docs/TENANT_TRUNKS.md` §7).

use fcp_core::TenantId;
use std::net::IpAddr;

/// Why this node would not take the call or the registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The node is leaving service and takes no new calls
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    Draining,
    /// More calls started in the last second than the node allows.
    NodeCps,
    /// The node is already carrying as many calls as it allows.
    NodeCalls,
    /// The tenant is already at its `max_concurrent_calls`.
    TenantCalls,
    /// The node holds as many registrations as it allows.
    Registrations,
    /// This source address or this AOR is registering too often.
    RegisterRate,
}

impl Refusal {
    /// The label this refusal is counted under.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draining => "draining",
            Self::NodeCps => "node_cps",
            Self::NodeCalls => "node_calls",
            Self::TenantCalls => "tenant_calls",
            Self::Registrations => "registrations",
            Self::RegisterRate => "register_rate",
        }
    }

    /// What the far end is told, and what the record says.
    pub fn detail(&self) -> &'static str {
        match self {
            Self::Draining => "Node draining",
            Self::NodeCps => "Node over its calls per second",
            Self::NodeCalls => "Node at its concurrent call limit",
            Self::TenantCalls => "Tenant at its concurrent call limit",
            Self::Registrations => "Node at its registration limit",
            Self::RegisterRate => "Registering too often",
        }
    }
}

/// The limits this node keeps, and the counts behind them.
pub trait NodeAdmission: Send + Sync {
    /// Take a place for a call that is starting. Released by [`ended`] with
    /// the same `call_id`, however the call ends.
    ///
    /// [`ended`]: NodeAdmission::ended
    fn admit(&self, call_id: &str, tenant: &TenantId) -> Result<(), Refusal>;

    /// Give back the place a call held. Unknown ids are ignored, so this is
    /// safe to call for a call that was never admitted.
    fn ended(&self, call_id: &str);

    /// A REGISTER arriving from `source` for `aor`, before any credential
    /// work, so a flood cannot reach the store.
    fn admit_register(&self, source: IpAddr, aor: &str) -> Result<(), Refusal>;

    /// What to put in `Retry-After` when refusing.
    fn retry_after_secs(&self) -> u64 {
        5
    }

    /// Whether this node is leaving service. A draining node refuses new
    /// calls — emergency calls excepted, which never reach admission —
    /// and keeps the calls it has until its deadline.
    fn draining(&self) -> bool {
        false
    }

    /// The longest registration a draining node grants, in seconds, so a
    /// device comes back soon and is placed on a node that is serving.
    /// `None` while the node is serving, and for a deregistration.
    fn register_expiry_cap(&self) -> Option<u32> {
        None
    }
}
