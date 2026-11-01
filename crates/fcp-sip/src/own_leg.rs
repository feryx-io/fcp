//! A leg this node places to itself, and how the node knows it again.
//!
//! A queue rings its agents, a flow's `Dial` rings a user, by INVITing
//! this node at its own SIP address: the call is then routed like any
//! other. Those INVITEs carry no user's password, so under `[registrar]
//! auth_invites` (`docs/EDGE_SECURITY.md` §2.1) they would be challenged
//! and fail. The address they come from proves nothing — a UDP source can
//! be forged from outside — so each carries a token only this process
//! knows, chosen at random when it starts, sent only to this node's own
//! address, and stripped at ingress from everything else
//! ([`crate::claims`]).

use std::sync::OnceLock;

use sip_core::Request;

/// The header a leg to this node itself carries.
pub const OWN_LEG_HEADER: &str = "X-FCP-Own-Leg";

/// The tenant a leg to this node itself is placed for, when it is not a
/// call's the node has (a queue's agent, callback or campaign leg): kept
/// only beside this process's token (docs/TENANT_SCOPE.md §4b).
pub const OWN_LEG_TENANT_HEADER: &str = "X-FCP-Own-Leg-Tenant";

/// The tenant an own leg names, when it proves itself one.
pub fn tenant(request: &Request) -> Option<fcp_core::TenantId> {
    if !proves(request) {
        return None;
    }
    sip_parse::header(request.headers(), OWN_LEG_TENANT_HEADER)
        .and_then(|value| value.as_str().trim().parse().ok())
}

/// This process's token: 32 random bytes as hex, never logged.
pub fn token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
}

/// Whether `request` is a leg this node placed to itself.
pub fn proves(request: &Request) -> bool {
    sip_parse::header(request.headers(), OWN_LEG_HEADER).is_some_and(|value| {
        fcp_core::secret::constant_time_eq(value.trim().as_bytes(), token().as_bytes())
    })
}

/// This node's own SIP address, which the token is sent to and nowhere
/// else: the host and port of its local URI, and its bind address.
#[derive(Clone, Debug, Default)]
pub struct OwnAddress {
    addresses: Vec<(String, u16)>,
}

impl OwnAddress {
    /// The node at `host:port` (the local URI's), also reached at `bind`.
    pub fn new(host: &str, port: u16, bind: Option<std::net::SocketAddr>) -> Self {
        let mut addresses = vec![(host.to_ascii_lowercase(), port)];
        if let Some(bind) = bind {
            if !bind.ip().is_unspecified() {
                addresses.push((bind.ip().to_string(), bind.port()));
            }
            addresses.push(("127.0.0.1".to_string(), bind.port()));
        }
        Self { addresses }
    }

    /// Whether `target` (a SIP URI) is this node.
    pub fn is_target(&self, target: &str) -> bool {
        let Ok(uri) = sip_core::SipUri::parse(target) else {
            return false;
        };
        let host = uri.host().trim_matches(['[', ']']).to_ascii_lowercase();
        let port = uri
            .port()
            .unwrap_or(if uri.is_sips() { 5061 } else { 5060 });
        self.addresses
            .iter()
            .any(|(own_host, own_port)| *own_host == host && *own_port == port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_this_process_s_and_long() {
        assert_eq!(token(), token());
        assert_eq!(token().len(), 64);
    }

    #[test]
    fn only_this_node_s_address_is_a_target() {
        let own = OwnAddress::new("10.0.0.5", 5060, "0.0.0.0:5060".parse().ok());
        assert!(own.is_target("sip:1002@10.0.0.5:5060"));
        assert!(own.is_target("sip:1002@10.0.0.5"));
        assert!(own.is_target("sip:1002@127.0.0.1:5060"));
        assert!(!own.is_target("sip:1002@10.0.0.5:5070"));
        assert!(!own.is_target("sip:1002@10.0.0.6:5060"));
        assert!(!own.is_target("sip:+15551234567@carrier.example.com"));
        assert!(!own.is_target("not a uri"));
    }
}
