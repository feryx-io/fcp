//! RTP ports a node can put in an answer or an offer.

use crate::{Error, Result};
use forge_rtp::{PortPair, PortPool, RtpSocketConfig, RtpSocketPair};
use tracing::warn;

/// A port pair from `pool` with its RTP and RTCP sockets bound.
///
/// The pool knows only this node's pairs: another process on the host can
/// hold one, and a pair that cannot be bound must not go into an answer or
/// an offer — a caller answered on one sat in a call no room or queue held
/// (`docs/DEEP_TESTING.md` D6). Such a pair is passed over and given back
/// once a pair is bound, for a later try; a failed try costs two binds.
pub async fn bound_port_pair(pool: &PortPool) -> Result<(PortPair, RtpSocketPair)> {
    let mut taken = Vec::new();
    let bound = loop {
        let pair = match pool.allocate().await {
            Ok(pair) => pair,
            Err(e) if taken.is_empty() => break Err(Error::ForgeMedia(e.to_string())),
            Err(_) => {
                break Err(Error::ForgeMedia(format!(
                    "no RTP port pair could be bound: {} held outside this node",
                    taken.len()
                )))
            }
        };
        match RtpSocketPair::new(pair, RtpSocketConfig::default()).await {
            Ok(socket) => break Ok((pair, socket)),
            Err(e) => {
                warn!(
                    rtp_port = pair.rtp_port,
                    error = %e,
                    "RTP port pair held outside this node; trying another"
                );
                taken.push(pair);
            }
        }
    };
    for pair in taken {
        pool.deallocate(pair).await;
    }
    bound
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_rtp::PortPoolConfig;

    #[tokio::test]
    async fn a_pair_held_by_another_socket_is_passed_over_and_given_back() {
        // Fifty pairs (the pool's least), all but the first held here. The
        // range is shared with every test binding ports at once: one is
        // used only when both ports of its first pair are free and every
        // other port is held by this test, so the free pair is the one
        // expected. A port held elsewhere let another pair through.
        let (start, held) = loop {
            let start = 40000 + (rand::random::<u16>() % 5000) * 2;
            if ![start, start + 1]
                .iter()
                .all(|port| std::net::UdpSocket::bind(("0.0.0.0", *port)).is_ok())
            {
                continue;
            }
            let held: Vec<_> = (1..50)
                .flat_map(|pair| [start + 2 * pair, start + 2 * pair + 1])
                .filter_map(|port| std::net::UdpSocket::bind(("0.0.0.0", port)).ok())
                .collect();
            if held.len() == 98 {
                break (start, held);
            }
        };
        let pool = PortPool::new(PortPoolConfig {
            min_port: start,
            max_port: start + 100,
        });
        let (pair, _socket) = bound_port_pair(&pool).await.expect("the free pair");
        assert_eq!(pair.rtp_port, start);
        assert_eq!(
            pool.allocated_count().await,
            1,
            "the held pairs are given back"
        );
        drop(held);
    }
}
