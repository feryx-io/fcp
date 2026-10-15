//! The per-source limit on SIP messages at a node's ports, and the sources
//! it does not apply to (`docs/DEEP_TESTING.md` D2c).
//!
//! siphon-rs drops, unread, what one address sends over a rate (UDP
//! packets, TCP/TLS/WebSocket frames). That defends a node against one
//! flooding device, but a peer that carries many parties' traffic by
//! design — an SBC in front of the node, a carrier trunk, another node of
//! the cluster — passes the same rate with ordinary load, and lost the
//! rest silently. Every binary sets the rate from its configuration and
//! names the peers it trusts.

use crate::emergency::TrustedEdges;
use crate::trunk_ingress::InboundTrunks;
use std::net::IpAddr;
use std::sync::Arc;
use tracing::{info, warn};

/// Messages a second one source may send when the configuration says
/// nothing: several times what a busy office behind one NAT sends.
pub const DEFAULT_MESSAGES_PER_SOURCE: u32 = 200;

/// For serde: [`DEFAULT_MESSAGES_PER_SOURCE`].
pub fn default_messages_per_source() -> u32 {
    DEFAULT_MESSAGES_PER_SOURCE
}

/// Whether a source address is one the limit does not apply to.
pub type SourceCheck = Arc<dyn Fn(IpAddr) -> bool + Send + Sync>;

/// Set this process's limit: `per_source` messages a second from one
/// address, `0` for none. Before the first listener starts; afterwards the
/// limit in force is kept, and said so.
pub fn set_limit(per_source: u32) -> anyhow::Result<()> {
    let config = if per_source == 0 {
        sip_transport::RateLimitConfig::disabled()
    } else {
        sip_transport::RateLimitConfig::new(per_source, 1)
            .map_err(|e| anyhow::anyhow!("max_messages_per_source = {per_source}: {e}"))?
            .with_idle_timeout(60)
    };
    let udp = sip_transport::set_udp_rate_limit(config.clone());
    let stream = sip_transport::set_stream_rate_limit(config);
    if udp && stream {
        info!(
            per_source,
            "SIP messages per source address limited (0 is none)"
        );
    } else {
        warn!(
            per_source,
            "The per-source SIP limit was set before; the first setting stays"
        );
    }
    Ok(())
}

/// The sources exempt from the limit, assembled from what a node trusts.
#[derive(Default)]
pub struct Exemptions {
    checks: Vec<SourceCheck>,
    named: Vec<&'static str>,
}

impl Exemptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Addresses and ranges (`trusted_sources`, `trusted_edges`).
    pub fn networks(mut self, what: &'static str, networks: TrustedEdges) -> Self {
        if !networks.is_empty() {
            self.named.push(what);
            self.checks
                .push(Arc::new(move |ip: IpAddr| networks.contains(ip)));
        }
        self
    }

    /// The addresses this node's trunks take calls from, as they are when
    /// asked: a trunk added later is exempt from then on.
    pub fn trunks(mut self, trunks: Arc<dyn InboundTrunks>) -> Self {
        self.named.push("trunk sources");
        self.checks.push(Arc::new(move |ip: IpAddr| {
            !trunks.inbound_from(unmap(ip)).is_empty()
        }));
        self
    }

    /// Any other set, asked as it is when a message would be dropped.
    pub fn check(mut self, what: &'static str, check: SourceCheck) -> Self {
        self.named.push(what);
        self.checks.push(check);
        self
    }

    /// Whether `ip` is exempt.
    pub fn contains(&self, ip: IpAddr) -> bool {
        self.checks.iter().any(|check| check(ip))
    }

    /// Make these the process's exemptions, replacing any before.
    pub fn install(self) {
        if self.checks.is_empty() {
            sip_transport::set_ingress_rate_exemption(None);
            return;
        }
        info!(exempt = ?self.named, "Sources exempt from the per-source SIP limit");
        let this = Arc::new(self);
        sip_transport::set_ingress_rate_exemption(Some(Arc::new(move |ip| this.contains(ip))));
    }
}

/// An IPv4 peer on a dual-stack socket arrives mapped into IPv6.
fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exempt_is_any_of_the_parts() {
        let exempt = Exemptions::new()
            .networks(
                "trusted_sources",
                TrustedEdges::parse(&["10.1.0.0/16"]).unwrap(),
            )
            .check(
                "cluster",
                Arc::new(|ip: IpAddr| ip == "192.0.2.9".parse::<IpAddr>().unwrap()),
            );
        assert!(exempt.contains("10.1.2.3".parse().unwrap()));
        assert!(exempt.contains("::ffff:10.1.2.3".parse().unwrap()));
        assert!(exempt.contains("192.0.2.9".parse().unwrap()));
        assert!(!exempt.contains("192.0.2.10".parse().unwrap()));
        assert!(!Exemptions::new().contains("10.1.2.3".parse().unwrap()));
    }

    #[test]
    fn an_empty_network_list_names_nothing() {
        let exempt = Exemptions::new().networks("trusted_sources", TrustedEdges::default());
        assert!(exempt.named.is_empty());
    }
}
