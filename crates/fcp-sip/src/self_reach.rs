//! Whether this host reaches itself at the address it advertises
//! (`docs/DEEP_TESTING.md` D8). On a cloud VM whose public address is a
//! 1:1 NAT at the provider's edge (GCE, an AWS elastic IP) no interface
//! holds that address, and nothing sent to it from the host comes back.
//! Every service on the host advertises it in Contact and SDP, so the
//! requests one service sends another inside a call — a BYE to voicemail,
//! a transfer, the media between them — are lost. Since v0.25.529 a
//! service that finds this tells a peer on this host the address the peer
//! is at instead (`crate::sibling`, #335); the address on the loopback
//! (`ip addr add <ip>/32 dev lo`) still mends it for anything else.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tracing::warn;

/// Whether a datagram sent to `ip` from this host arrives back on this
/// host, within `within`. A loopback or unspecified address is taken as
/// reachable.
pub async fn reaches_this_host(ip: IpAddr, within: Duration) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    let bind: SocketAddr = match ip {
        IpAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
        IpAddr::V6(_) => "[::]:0".parse().unwrap(),
    };
    let Ok(socket) = tokio::net::UdpSocket::bind(bind).await else {
        // Nothing to test with: say nothing rather than cry wolf.
        return true;
    };
    let Ok(port) = socket.local_addr().map(|a| a.port()) else {
        return true;
    };
    let probe = b"fcp-self-reach";
    if socket.send_to(probe, (ip, port)).await.is_err() {
        return false;
    }
    let mut buf = [0u8; 32];
    matches!(
        tokio::time::timeout(within, socket.recv_from(&mut buf)).await,
        Ok(Ok((n, _))) if &buf[..n] == probe
    )
}

/// [`reaches_this_host`], with a warning when it does not. Returns the
/// address when this host does not reach it, for
/// [`crate::transport::RealTransportDispatcher::advertise_to_siblings`].
pub async fn warn_if_unreachable(ip: &str, service: &str) -> Option<IpAddr> {
    let addr = ip.parse::<IpAddr>().ok()?;
    if reaches_this_host(addr, Duration::from_secs(1)).await {
        return None;
    }
    warn!(
        public_ip = %addr,
        service,
        "This host does not reach its own advertised address. SIP services on it are told the \
         local address for one another instead; for anything else on the host, put the address \
         on the loopback: ip addr add {addr}/32 dev lo"
    );
    Some(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_reaches_this_host_and_a_documentation_address_does_not() {
        assert!(reaches_this_host("127.0.0.1".parse().unwrap(), Duration::from_millis(500)).await);
        // An address of one of this host's own interfaces comes back.
        let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        if probe.connect("192.0.2.1:9").is_ok() {
            let own = probe.local_addr().unwrap().ip();
            if !own.is_unspecified() {
                assert!(
                    reaches_this_host(own, Duration::from_millis(500)).await,
                    "{own}"
                );
            }
        }
        // TEST-NET-3: routed nowhere, so nothing comes back.
        assert!(
            !reaches_this_host("203.0.113.7".parse().unwrap(), Duration::from_millis(300)).await
        );
    }
}
