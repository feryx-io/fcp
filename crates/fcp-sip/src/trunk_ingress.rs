//! Which trunk a call arrived on, and so whose call it is
//! (`docs/TENANT_TRUNKS.md` §7).
//!
//! A call from a carrier carries the carrier's addressing, not the
//! tenant's; the trunk it arrived on is what says whose it is. The node's
//! trunk set knows which trunks take calls from an address
//! ([`InboundTrunks`]); [`choose_inbound`] picks among them.

use fcp_core::{TenantId, Trunk};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;

/// The trunks a node takes inbound calls on.
pub trait InboundTrunks: Send + Sync {
    /// The trunks a call from `source` may have arrived on: enabled, not
    /// outbound-only, and taking calls from that address. Most specific
    /// first.
    fn inbound_from(&self, source: IpAddr) -> Vec<Arc<Trunk>>;
}

/// Which trunk a call arrived on, and so whose call it is (§7).
#[derive(Debug, Clone)]
pub enum InboundChoice {
    /// No trunk takes calls from this address.
    NoTrunk,
    /// The call arrived on `trunk` and is `tenant`'s — the trunk's own
    /// tenant, or, on a platform trunk, whoever owns the dialled number.
    Tenant { trunk: Arc<Trunk>, tenant: TenantId },
    /// A platform trunk carrying a number nobody here owns: the tenant is
    /// whatever the request's addressing says, as before.
    Platform { trunk: Arc<Trunk> },
    /// Several tenants' trunks take calls from this address and the
    /// dialled number belongs to none of them.
    Ambiguous,
}

impl InboundChoice {
    /// The trunk the call arrived on, if one was chosen.
    pub fn trunk(&self) -> Option<&Arc<Trunk>> {
        match self {
            Self::Tenant { trunk, .. } | Self::Platform { trunk } => Some(trunk),
            Self::NoTrunk | Self::Ambiguous => None,
        }
    }
}

/// Choose among the trunks a call's source matched (`candidates`, most
/// specific first), given the Request-URI's user and who owns the dialled
/// number (§7):
///
/// 1. A registering trunk whose account the call is addressed to is the
///    trunk — that is what separates two tenants on one carrier.
/// 2. One tenant's trunks and no other's: that tenant, whatever the number
///    — unless the number is another tenant's and a platform trunk, which
///    carries any tenant's numbers, matched too.
/// 3. Several tenants: the one that owns the number; else a platform
///    trunk for the owner; else ambiguous.
/// 4. Only platform trunks: the owner of the number, else the request's
///    own addressing.
pub fn choose_inbound(
    candidates: &[Arc<Trunk>],
    request_user: Option<&str>,
    number_owner: Option<TenantId>,
) -> InboundChoice {
    let decide = |trunk: &Arc<Trunk>| match (trunk.tenant_id, number_owner) {
        (Some(tenant), _) => InboundChoice::Tenant {
            trunk: Arc::clone(trunk),
            tenant,
        },
        (None, Some(owner)) => InboundChoice::Tenant {
            trunk: Arc::clone(trunk),
            tenant: owner,
        },
        (None, None) => InboundChoice::Platform {
            trunk: Arc::clone(trunk),
        },
    };
    if candidates.is_empty() {
        return InboundChoice::NoTrunk;
    }

    // 1. Addressed to a registering trunk's account.
    let addressed: Vec<&Arc<Trunk>> = match request_user {
        Some(user) if !user.is_empty() => candidates
            .iter()
            .filter(|t| {
                t.register
                    && t.credentials
                        .as_ref()
                        .is_some_and(|c| c.username.eq_ignore_ascii_case(user))
            })
            .collect(),
        _ => Vec::new(),
    };
    if let [only] = addressed.as_slice() {
        return decide(only);
    }

    let platform = candidates.iter().find(|t| t.tenant_id.is_none());
    let tenants: HashSet<TenantId> = candidates.iter().filter_map(|t| t.tenant_id).collect();
    let owners_trunk =
        number_owner.and_then(|owner| candidates.iter().find(|t| t.tenant_id == Some(owner)));

    if let Some(trunk) = owners_trunk {
        return decide(trunk);
    }
    match (tenants.len(), platform, number_owner) {
        // The number is someone else's and a shared carrier brought it.
        (_, Some(platform), Some(_)) => decide(platform),
        (0, Some(platform), None) => decide(platform),
        (1, _, _) => candidates
            .iter()
            .find(|t| t.tenant_id.is_some())
            .map(decide)
            .unwrap_or(InboundChoice::NoTrunk),
        _ => InboundChoice::Ambiguous,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound(name: &str, tenant: Option<TenantId>) -> Trunk {
        let mut t = match tenant {
            Some(tenant) => Trunk::new(name.to_string(), tenant),
            None => Trunk {
                name: name.to_string(),
                ..Default::default()
            },
        };
        t.direction = fcp_core::trunk::TrunkDirection::Bidirectional;
        t
    }

    fn arc(t: Trunk) -> Arc<Trunk> {
        Arc::new(t)
    }

    fn chosen(choice: &InboundChoice) -> (Option<String>, Option<TenantId>) {
        match choice {
            InboundChoice::Tenant { trunk, tenant } => (Some(trunk.name.clone()), Some(*tenant)),
            InboundChoice::Platform { trunk } => (Some(trunk.name.clone()), None),
            InboundChoice::NoTrunk | InboundChoice::Ambiguous => (None, None),
        }
    }

    #[test]
    fn one_tenants_trunk_is_that_tenants_call_whatever_the_number() {
        let a = TenantId::new();
        let c = [arc(inbound("acme", Some(a)))];
        assert_eq!(
            chosen(&choose_inbound(&c, None, None)),
            (Some("acme".into()), Some(a))
        );
        assert_eq!(
            chosen(&choose_inbound(&c, None, Some(TenantId::new()))),
            (Some("acme".into()), Some(a))
        );
        assert!(matches!(
            choose_inbound(&[], None, None),
            InboundChoice::NoTrunk
        ));
    }

    #[test]
    fn on_a_shared_carrier_the_number_decides() {
        let (a, b) = (TenantId::new(), TenantId::new());
        let c = [
            arc(inbound("acme", Some(a))),
            arc(inbound("globex", Some(b))),
        ];
        assert_eq!(
            chosen(&choose_inbound(&c, None, Some(b))),
            (Some("globex".into()), Some(b))
        );
        assert!(matches!(
            choose_inbound(&c, None, None),
            InboundChoice::Ambiguous
        ));
        assert!(matches!(
            choose_inbound(&c, None, Some(TenantId::new())),
            InboundChoice::Ambiguous
        ));
    }

    #[test]
    fn a_registering_trunk_is_known_by_the_account_a_call_is_addressed_to() {
        let (a, b) = (TenantId::new(), TenantId::new());
        let mut acme = inbound("acme", Some(a));
        acme.register = true;
        acme.credentials = Some(fcp_core::trunk::TrunkCredentials {
            username: "acme-account".to_string(),
            password: "x".to_string(),
            realm: None,
        });
        let c = [arc(acme), arc(inbound("globex", Some(b)))];
        assert_eq!(
            chosen(&choose_inbound(&c, Some("acme-account"), Some(b))),
            (Some("acme".into()), Some(a)),
            "the account outranks the number"
        );
    }

    #[test]
    fn a_platform_trunk_carries_whoever_owns_the_number() {
        let (a, owner) = (TenantId::new(), TenantId::new());
        let platform = [arc(inbound("shared", None))];
        assert_eq!(
            chosen(&choose_inbound(&platform, None, Some(owner))),
            (Some("shared".into()), Some(owner))
        );
        assert_eq!(
            chosen(&choose_inbound(&platform, None, None)),
            (Some("shared".into()), None)
        );

        // A tenant's trunk and the platform's at one address: the tenant's
        // own number stays its own, another tenant's rides the platform's.
        let both = [arc(inbound("acme", Some(a))), arc(inbound("shared", None))];
        assert_eq!(
            chosen(&choose_inbound(&both, None, Some(a))),
            (Some("acme".into()), Some(a))
        );
        assert_eq!(
            chosen(&choose_inbound(&both, None, Some(owner))),
            (Some("shared".into()), Some(owner))
        );
        assert_eq!(
            chosen(&choose_inbound(&both, None, None)),
            (Some("acme".into()), Some(a))
        );
    }
}
