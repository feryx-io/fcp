//! Egress HMR dispatcher — wraps `TransportDispatcher` to apply header
//! manipulation rules before sending SIP messages on the wire.

use crate::hmr::HeaderManipulationEngine;
use bytes::Bytes;
use fcp_sbc_core::TrunkConfig;
use sip_transaction::{TransportContext, TransportDispatcher};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tracing::debug;

/// Transport dispatcher wrapper that applies egress HMR rules.
///
/// On dispatch, parses the outbound bytes as a SIP request or response,
/// applies egress HMR rules, reserializes, and delegates to the inner
/// dispatcher. If parsing fails, the payload is forwarded unchanged.
pub struct HmrTransportDispatcher {
    inner: Arc<dyn TransportDispatcher>,
    engine: Arc<HeaderManipulationEngine>,
    /// Maps peer IP to trunk name for trunk identification on egress.
    peer_trunk_map: HashMap<IpAddr, String>,
}

impl HmrTransportDispatcher {
    /// Create a new HMR egress dispatcher.
    ///
    /// Builds a peer-IP-to-trunk-name mapping from the trunk configurations
    /// by resolving each trunk's host entries to IP addresses (best-effort).
    pub fn new(
        inner: Arc<dyn TransportDispatcher>,
        engine: Arc<HeaderManipulationEngine>,
        trunks: &[TrunkConfig],
    ) -> Self {
        let peer_trunk_map = build_peer_trunk_map(trunks);
        Self {
            inner,
            engine,
            peer_trunk_map,
        }
    }
}

#[async_trait::async_trait]
impl TransportDispatcher for HmrTransportDispatcher {
    async fn dispatch(&self, ctx: &TransportContext, payload: Bytes) -> anyhow::Result<()> {
        let trunk_name = self
            .peer_trunk_map
            .get(&ctx.peer().ip())
            .map(|s| s.as_str());

        // Try parse as request
        if let Some(mut req) = sip_parse::parse_request(&payload) {
            self.engine.apply_request_egress(&mut req, trunk_name);
            let rewritten = sip_parse::serialize_request(&req);
            return self.inner.dispatch(ctx, rewritten).await;
        }

        // Try parse as response
        if let Some(mut resp) = sip_parse::parse_response(&payload) {
            self.engine.apply_response_egress(&mut resp, trunk_name);
            let rewritten = sip_parse::serialize_response(&resp);
            return self.inner.dispatch(ctx, rewritten).await;
        }

        // Unparseable — pass through unchanged
        debug!("HMR egress: unparseable payload, passing through");
        self.inner.dispatch(ctx, payload).await
    }
}

/// Build a mapping from peer IP addresses to trunk names.
///
/// For each trunk, attempts to parse each host entry as an IP address.
/// FQDN hosts are resolved via DNS at startup (best-effort).
pub(crate) fn build_peer_trunk_map(trunks: &[TrunkConfig]) -> HashMap<IpAddr, String> {
    use std::net::ToSocketAddrs;

    let mut map = HashMap::new();
    for trunk in trunks {
        for host in &trunk.hosts {
            if let Ok(ip) = host.parse::<IpAddr>() {
                map.insert(ip, trunk.name.clone());
            } else {
                // DNS resolution for FQDNs (best-effort at startup)
                if let Ok(addrs) = (host.as_str(), 5060u16).to_socket_addrs() {
                    for addr in addrs {
                        map.insert(addr.ip(), trunk.name.clone());
                    }
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_trunk_mapping() {
        let trunks = vec![
            TrunkConfig {
                name: "carrier-a".into(),
                hosts: vec!["10.0.0.1".into(), "sip.carrier.com".into()],
                ..Default::default()
            },
            TrunkConfig {
                name: "carrier-b".into(),
                hosts: vec!["192.168.1.100".into()],
                ..Default::default()
            },
        ];

        let map = build_peer_trunk_map(&trunks);
        assert_eq!(
            map.get(&"10.0.0.1".parse::<IpAddr>().unwrap()),
            Some(&"carrier-a".to_string())
        );
        assert_eq!(
            map.get(&"192.168.1.100".parse::<IpAddr>().unwrap()),
            Some(&"carrier-b".to_string())
        );
        // FQDN not in map
        assert!(!map.values().any(|v| v == "sip.carrier.com"));
    }

    #[test]
    fn test_peer_trunk_mapping_empty() {
        let map = build_peer_trunk_map(&[]);
        assert!(map.is_empty());
    }
}
