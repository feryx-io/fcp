//! Video relayed through an anchored call (`docs/SOFTPHONE_VIDEO.md` §2.2).
//!
//! An anchored call terminates each leg's media at the node. Audio goes
//! through forge-engine; video has no engine to go through, so each video
//! section of the call — the camera, a shared screen — gets a relay: a
//! socket pair per leg, and packets copied from one to the other. Nothing
//! is decoded. Each leg is keyed on its own, so one phone's SDES key never
//! reaches the other, and RTCP (PLI, FIR, NACK, REMB, reports) passes
//! through with its SSRCs, so the phones adapt to each other as if they
//! were connected directly.
//!
//! A relay has two *ends*, each with its socket, its peer and its keys.
//! A leg names the end its phone is on. A phone moved between legs keeps
//! its end, and so its port, as audio's do (`docs/NAT_AND_TLS.md`).

use crate::backend::Leg;
use crate::sdp::{SrtpOffer, SrtpSdpContext, VideoStream};
use crate::Result;
use forge_rtp::srtp::SrtpContext;
use forge_rtp::{PortPair, PortPool, RtpSocketPair};
use parking_lot::Mutex;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use tokio::task::JoinHandle;
use tracing::{debug, info, trace, warn};

/// The largest datagram a relay reads: an Ethernet MTU's worth and more.
const MAX_DATAGRAM: usize = 2048;

/// Where one leg's phone is, as its SDP or its first packet says.
#[derive(Debug, Default, Clone)]
struct Peer {
    /// Where its RTP goes; `None` until known.
    rtp: Option<SocketAddr>,
    /// Where its RTCP goes, when it said or sent from somewhere other
    /// than the RTP port plus one.
    rtcp: Option<SocketAddr>,
    /// RTP and RTCP on one port (RFC 5761).
    mux: bool,
    /// The address its first packets are learned from instead of its SDP:
    /// a phone behind NAT.
    learn_from: Option<IpAddr>,
}

impl Peer {
    /// Whether a packet from `source` comes from this peer; a peer being
    /// learned takes its address from the first one.
    fn admit(&mut self, source: SocketAddr, rtcp: bool) -> bool {
        if let Some(ip) = self.learn_from {
            if source.ip() != ip {
                return false;
            }
            if rtcp && !self.mux {
                self.rtcp.get_or_insert(source);
            } else {
                self.rtp.get_or_insert(source);
            }
            return true;
        }
        self.rtp.is_some_and(|rtp| rtp.ip() == source.ip())
    }

    /// Where a packet for this peer goes.
    fn destination(&self, rtcp: bool) -> Option<SocketAddr> {
        let rtp = self.rtp?;
        if !rtcp || self.mux {
            return Some(rtp);
        }
        Some(
            self.rtcp
                .unwrap_or_else(|| SocketAddr::new(rtp.ip(), rtp.port().wrapping_add(1))),
        )
    }
}

/// How the node keys one leg's section.
enum Keying {
    /// Plain RTP.
    Plain,
    /// The node offered this key and waits for the answer's; nothing is
    /// forwarded either way until then.
    Offered(SrtpOffer),
    /// SRTP both ways.
    Keyed(Box<SrtpContext>),
    /// The node offered a key and the answer gave none it can use: the
    /// leg's video is not relayed, rather than relayed in the clear.
    Refused,
}

/// One end of a relay: the node's socket pair for one phone.
struct End {
    pair: PortPair,
    socket: Arc<RtpSocketPair>,
    peer: Mutex<Peer>,
    keying: Mutex<Keying>,
}

impl End {
    /// Clear what the end knew of its phone: no peer, plain, until an SDP
    /// says otherwise.
    fn forget(&self) {
        *self.peer.lock() = Peer::default();
        *self.keying.lock() = Keying::Plain;
    }
}

/// What a relay has done.
#[derive(Debug, Default)]
struct Counters {
    rtp: AtomicU64,
    rtcp: AtomicU64,
    refused: AtomicU64,
    srtp_errors: AtomicU64,
}

/// What a relay has done, read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VideoRelayStats {
    /// RTP packets relayed, both ways.
    pub rtp: u64,
    /// RTCP packets relayed, both ways.
    pub rtcp: u64,
    /// Packets from an address that is neither leg's phone.
    pub refused: u64,
    /// Packets that failed to decrypt or encrypt.
    pub srtp_errors: u64,
}

/// One video section of an anchored call, relayed between its legs.
pub struct VideoRelay {
    stream: VideoStream,
    /// The end each leg's phone is on: A's first.
    legs: Mutex<[Arc<End>; 2]>,
    paused: AtomicBool,
    counters: Counters,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    pool: Arc<PortPool>,
}

impl std::fmt::Debug for VideoRelay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoRelay")
            .field("stream", &self.stream)
            .field("ports", &[self.port(Leg::A), self.port(Leg::B)])
            .finish()
    }
}

fn index(leg: Leg) -> usize {
    match leg {
        Leg::A => 0,
        Leg::B => 1,
    }
}

impl VideoRelay {
    /// A relay for `stream`, with a bound port pair per leg from `pool`.
    pub async fn open(stream: VideoStream, pool: Arc<PortPool>) -> Result<Arc<Self>> {
        let (pair_a, socket_a) = crate::ports::bound_port_pair(&pool).await?;
        let (pair_b, socket_b) = match crate::ports::bound_port_pair(&pool).await {
            Ok(b) => b,
            Err(e) => {
                pool.deallocate(pair_a).await;
                return Err(e);
            }
        };
        let end = |pair, socket| {
            Arc::new(End {
                pair,
                socket: Arc::new(socket),
                peer: Mutex::new(Peer::default()),
                keying: Mutex::new(Keying::Plain),
            })
        };
        let relay = Arc::new(Self {
            stream,
            legs: Mutex::new([end(pair_a, socket_a), end(pair_b, socket_b)]),
            paused: AtomicBool::new(false),
            counters: Counters::default(),
            tasks: Mutex::new(Vec::new()),
            pool,
        });
        let ends = relay.legs.lock().clone();
        let tasks = ends
            .iter()
            .flat_map(|end| {
                [false, true].map(|rtcp| {
                    let socket = if rtcp {
                        end.socket.rtcp_socket()
                    } else {
                        end.socket.rtp_socket()
                    };
                    tokio::spawn(pump(
                        Arc::downgrade(&relay),
                        Arc::downgrade(end),
                        socket,
                        rtcp,
                    ))
                })
            })
            .collect();
        *relay.tasks.lock() = tasks;
        info!(
            stream = ?stream,
            port_a = pair_a.rtp_port,
            port_b = pair_b.rtp_port,
            "Video relay opened"
        );
        Ok(relay)
    }

    /// The section this relay carries.
    pub fn stream(&self) -> VideoStream {
        self.stream
    }

    /// The RTP port `leg`'s phone is told.
    pub fn port(&self, leg: Leg) -> u16 {
        self.legs.lock()[index(leg)].pair.rtp_port
    }

    fn end(&self, leg: Leg) -> Arc<End> {
        Arc::clone(&self.legs.lock()[index(leg)])
    }

    /// Where `leg`'s phone is, from an SDP it sent: the section's address,
    /// or, for a phone behind NAT (`latched`), the first packets from that
    /// address. A section it turned off (port 0, or absent) leaves the leg
    /// with no peer.
    pub fn set_peer(&self, leg: Leg, sdp: &str, latched: Option<IpAddr>) {
        let end = self.end(leg);
        let mut peer = end.peer.lock();
        let Some(endpoint) = crate::sdp::stream_endpoint(sdp, self.stream) else {
            *peer = Peer::default();
            debug!(stream = ?self.stream, ?leg, "Video section off on this leg");
            return;
        };
        let mux = crate::sdp::stream_rtcp_mux(sdp, self.stream);
        *peer = match latched {
            // The same phone, re-offering: what was learned stands.
            Some(ip) if peer.learn_from == Some(ip) => Peer {
                mux,
                ..peer.clone()
            },
            Some(ip) => Peer {
                mux,
                learn_from: Some(ip),
                ..Peer::default()
            },
            None => Peer {
                rtp: Some(endpoint),
                rtcp: None,
                mux,
                learn_from: None,
            },
        };
    }

    /// `leg`'s phone is learned from its first packets from `source`.
    pub fn latch(&self, leg: Leg, source: IpAddr) {
        let end = self.end(leg);
        let mut peer = end.peer.lock();
        *peer = Peer {
            mux: peer.mux,
            learn_from: Some(source),
            ..Peer::default()
        };
    }

    /// The key for the section of an offer the node sends `leg`: fresh,
    /// when the leg is keyed (`secure`), else none and the leg is plain.
    pub fn offer_key(&self, leg: Leg, secure: bool) -> Result<Option<SrtpSdpContext>> {
        let end = self.end(leg);
        let mut keying = end.keying.lock();
        if !secure {
            *keying = Keying::Plain;
            return Ok(None);
        }
        // An offer to the same leg before its answer — another of the
        // callee's devices — carries the same key, as audio's does: the
        // one that answers must hold the key the node encrypts with.
        if let Keying::Offered(offer) = &*keying {
            return Ok(Some(offer.sdp.clone()));
        }
        let offer = SrtpOffer::generate()?;
        let sdp = offer.sdp.clone();
        *keying = Keying::Offered(offer);
        Ok(Some(sdp))
    }

    /// `leg` answered the node's offer: its key, if the node offered one.
    /// An answer without one is plain, as audio's is: one of a callee's
    /// devices offered plain answered. An answer with a key the node
    /// cannot use leaves the leg's video unrelayed.
    pub fn answered(&self, leg: Leg, answer: &str) {
        let end = self.end(leg);
        let mut keying = end.keying.lock();
        let Keying::Offered(_) = &*keying else {
            return;
        };
        let Keying::Offered(offer) = std::mem::replace(&mut *keying, Keying::Plain) else {
            unreachable!()
        };
        let crypto = crate::sdp::stream_sdes_crypto(answer, self.stream);
        *keying = match crypto.first().map(|line| offer.accept(line)) {
            Some(Ok(context)) => Keying::Keyed(Box::new(context)),
            Some(Err(e)) => {
                warn!(stream = ?self.stream, ?leg, error = %e, "Video answered with a key the node cannot use; not relayed");
                Keying::Refused
            }
            None => Keying::Plain,
        };
    }

    /// The key the node answers `leg`'s offer with: the offer's own key
    /// for what it sends, a fresh one of the node's, under the offer's tag.
    /// An offer without a key is answered plain.
    pub fn answer_key(&self, leg: Leg, offer: &str) -> Result<Option<SrtpSdpContext>> {
        let end = self.end(leg);
        let mut keying = end.keying.lock();
        let Some(offered) = crate::sdp::stream_sdes_crypto(offer, self.stream)
            .into_iter()
            .next()
        else {
            *keying = Keying::Plain;
            return Ok(None);
        };
        let ours = SrtpOffer::answering(&offered)?;
        let sdp = ours.sdp.clone();
        *keying = Keying::Keyed(Box::new(ours.accept(&offered)?));
        Ok(Some(sdp))
    }

    /// Put the phone on `from` on `to`, with its port and keys, and leave
    /// `from` with no phone until an SDP names its next one (a parked
    /// party moved to A), as audio's [`crate::MediaManager::move_peer`].
    pub fn move_peer(&self, from: Leg, to: Leg) {
        if from == to {
            return;
        }
        let mut legs = self.legs.lock();
        legs.swap(index(from), index(to));
        legs[index(from)].forget();
    }

    /// Leave `leg` with no phone until an SDP names its next one.
    pub fn release(&self, leg: Leg) {
        self.end(leg).forget();
    }

    /// Stop relaying (a held call), or go on.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// What the relay has done.
    pub fn stats(&self) -> VideoRelayStats {
        let c = &self.counters;
        VideoRelayStats {
            rtp: c.rtp.load(Ordering::Relaxed),
            rtcp: c.rtcp.load(Ordering::Relaxed),
            refused: c.refused.load(Ordering::Relaxed),
            srtp_errors: c.srtp_errors.load(Ordering::Relaxed),
        }
    }

    /// Stop the relay and give its ports back.
    pub async fn close(&self) {
        for task in self.tasks.lock().drain(..) {
            task.abort();
        }
        let pairs = self.legs.lock().clone().map(|end| end.pair);
        for pair in pairs {
            self.pool.deallocate(pair).await;
        }
        info!(stream = ?self.stream, stats = ?self.stats(), "Video relay closed");
    }

    /// Relay one datagram that arrived on `from`.
    fn relay(&self, from: &Arc<End>, data: &[u8], source: SocketAddr, rtcp: bool) {
        let to = {
            let legs = self.legs.lock();
            if Arc::ptr_eq(&legs[0], from) {
                Arc::clone(&legs[1])
            } else {
                Arc::clone(&legs[0])
            }
        };
        if !from.peer.lock().admit(source, rtcp) {
            self.counters.refused.fetch_add(1, Ordering::Relaxed);
            trace!(stream = ?self.stream, %source, "Video packet from neither phone");
            return;
        }
        if self.paused.load(Ordering::Relaxed) {
            return;
        }
        let plain = match &mut *from.keying.lock() {
            Keying::Plain => data.to_vec(),
            Keying::Offered(_) | Keying::Refused => return,
            Keying::Keyed(ctx) => {
                let plain = if rtcp {
                    ctx.unprotect_rtcp(data)
                } else {
                    ctx.unprotect_rtp(data)
                };
                match plain {
                    Ok(p) => p,
                    Err(e) => {
                        self.counters.srtp_errors.fetch_add(1, Ordering::Relaxed);
                        trace!(stream = ?self.stream, error = %e, "Video SRTP decrypt failed");
                        return;
                    }
                }
            }
        };
        let out = match &mut *to.keying.lock() {
            Keying::Plain => plain,
            Keying::Offered(_) | Keying::Refused => return,
            Keying::Keyed(ctx) => {
                let out = if rtcp {
                    ctx.protect_rtcp(&plain)
                } else {
                    ctx.protect_rtp(&plain)
                };
                match out {
                    Ok(p) => p,
                    Err(e) => {
                        self.counters.srtp_errors.fetch_add(1, Ordering::Relaxed);
                        trace!(stream = ?self.stream, error = %e, "Video SRTP encrypt failed");
                        return;
                    }
                }
            }
        };
        let (destination, mux) = {
            let peer = to.peer.lock();
            (peer.destination(rtcp), peer.mux)
        };
        let Some(destination) = destination else {
            return;
        };
        let socket = if rtcp && !mux {
            to.socket.rtcp_socket()
        } else {
            to.socket.rtp_socket()
        };
        match socket.try_send_to(&out, destination) {
            Ok(_) => {
                let counter = if rtcp {
                    &self.counters.rtcp
                } else {
                    &self.counters.rtp
                };
                counter.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => trace!(stream = ?self.stream, %destination, error = %e, "Video send failed"),
        }
    }
}

impl Drop for VideoRelay {
    fn drop(&mut self) {
        for task in self.tasks.lock().drain(..) {
            task.abort();
        }
    }
}

/// Read one socket of `end` and relay what arrives. RTP and RTCP on one
/// port (RFC 5761) are told apart by the RTCP packet types' range.
async fn pump(
    relay: Weak<VideoRelay>,
    end: Weak<End>,
    socket: Arc<tokio::net::UdpSocket>,
    rtcp_socket: bool,
) {
    let mut buf = vec![0u8; MAX_DATAGRAM];
    loop {
        let (len, source) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                // An ICMP unreachable from a phone that went away reads as
                // an error on some systems: keep reading.
                trace!(error = %e, "Video socket read failed");
                continue;
            }
        };
        let (Some(relay), Some(end)) = (relay.upgrade(), end.upgrade()) else {
            return;
        };
        let data = &buf[..len];
        let rtcp = rtcp_socket || is_rtcp(data);
        relay.relay(&end, data, source, rtcp);
    }
}

/// An RTCP packet on an RTP port (RFC 5761 §4): packet types 192–223.
fn is_rtcp(data: &[u8]) -> bool {
    data.len() >= 2 && (192..=223).contains(&data[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdp::SdesCryptoLine;
    use forge_rtp::PortPoolConfig;
    use std::time::Duration;
    use tokio::net::UdpSocket;

    fn pool() -> Arc<PortPool> {
        let start = 46000 + (rand::random::<u16>() % 4000) * 2;
        Arc::new(PortPool::new(
            PortPoolConfig::new(start, start + 200).unwrap(),
        ))
    }

    fn rtp(seq: u16) -> Vec<u8> {
        let mut p = vec![0x80, 96];
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&(u32::from(seq) * 3000).to_be_bytes());
        p.extend_from_slice(&0x1234_5678u32.to_be_bytes());
        p.extend_from_slice(b"frame");
        p.extend_from_slice(&seq.to_be_bytes());
        p
    }

    /// A PLI (RFC 4585 §6.3.1) from `sender` about `media`.
    fn pli(sender: u32, media: u32) -> Vec<u8> {
        let mut p = vec![0x81, 206, 0, 2];
        p.extend_from_slice(&sender.to_be_bytes());
        p.extend_from_slice(&media.to_be_bytes());
        p
    }

    fn sdp(c: &str, port: u16, extra: &str) -> String {
        format!(
            "v=0\r\no=- 1 1 IN IP4 {c}\r\ns=-\r\nc=IN IP4 {c}\r\nt=0 0\r\n\
             m=audio 4000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n\
             m=video {port} RTP/AVPF 96\r\na=rtpmap:96 H264/90000\r\n{extra}"
        )
    }

    fn crypto(ctx: &SrtpSdpContext) -> String {
        format!(
            "a=crypto:{} {} {}\r\n",
            ctx.tag, ctx.suite, ctx.local_key_b64
        )
    }

    fn line(ctx: &SrtpSdpContext) -> SdesCryptoLine {
        SdesCryptoLine {
            tag: ctx.tag,
            suite: ctx.suite.clone(),
            key_params: ctx.local_key_b64.clone(),
        }
    }

    async fn phone() -> UdpSocket {
        UdpSocket::bind("127.0.0.1:0").await.unwrap()
    }

    async fn recv(socket: &UdpSocket) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 2048];
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buf))
            .await
            .ok()?
            .ok()?;
        buf.truncate(n);
        Some(buf)
    }

    fn node(relay: &VideoRelay, leg: Leg) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], relay.port(leg)))
    }

    /// A call between `a` and `b`, plain: B offered by the node, A
    /// answered by it.
    async fn plain_call(relay: &VideoRelay, a: &UdpSocket, b: &UdpSocket) {
        let a_sdp = sdp("127.0.0.1", a.local_addr().unwrap().port(), "");
        let b_sdp = sdp("127.0.0.1", b.local_addr().unwrap().port(), "");
        assert!(relay.offer_key(Leg::B, false).unwrap().is_none());
        relay.set_peer(Leg::B, &b_sdp, None);
        relay.answered(Leg::B, &b_sdp);
        relay.set_peer(Leg::A, &a_sdp, None);
        assert!(relay.answer_key(Leg::A, &a_sdp).unwrap().is_none());
    }

    #[tokio::test]
    async fn video_is_relayed_both_ways_unchanged() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let (a, b) = (phone().await, phone().await);
        plain_call(&relay, &a, &b).await;

        a.send_to(&rtp(1), node(&relay, Leg::A)).await.unwrap();
        assert_eq!(recv(&b).await, Some(rtp(1)), "A's packet reaches B as sent");
        b.send_to(&rtp(7), node(&relay, Leg::B)).await.unwrap();
        assert_eq!(recv(&a).await, Some(rtp(7)), "B's packet reaches A as sent");
        assert_eq!(relay.stats().rtp, 2);
        relay.close().await;
    }

    #[tokio::test]
    async fn each_leg_is_keyed_on_its_own() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let (a, b) = (phone().await, phone().await);

        // A offers its key; the node answers with its own.
        let a_key = SrtpOffer::generate().unwrap();
        let a_sdp = sdp(
            "127.0.0.1",
            a.local_addr().unwrap().port(),
            &crypto(&a_key.sdp),
        );
        relay.set_peer(Leg::A, &a_sdp, None);
        let node_a = relay.answer_key(Leg::A, &a_sdp).unwrap().expect("A keyed");
        let mut a_ctx = a_key.clone().accept(&line(&node_a)).unwrap();

        // The node offers B a key of its own; B answers with its.
        let node_b = relay.offer_key(Leg::B, true).unwrap().expect("B keyed");
        let b_key = SrtpOffer::answering(&line(&node_b)).unwrap();
        let b_sdp = sdp(
            "127.0.0.1",
            b.local_addr().unwrap().port(),
            &crypto(&b_key.sdp),
        );
        relay.set_peer(Leg::B, &b_sdp, None);
        relay.answered(Leg::B, &b_sdp);
        let mut b_ctx = b_key.accept(&line(&node_b)).unwrap();
        assert_ne!(node_a.local_key_b64, node_b.local_key_b64);
        assert_ne!(node_a.local_key_b64, a_key.sdp.local_key_b64);

        let sent = a_ctx.protect_rtp(&rtp(3)).unwrap();
        a.send_to(&sent, node(&relay, Leg::A)).await.unwrap();
        let got = recv(&b).await.expect("relayed");
        assert_ne!(got, sent, "re-encrypted for B, not passed through");
        assert_eq!(b_ctx.unprotect_rtp(&got).unwrap(), rtp(3));

        let sent = b_ctx.protect_rtcp(&pli(2, 0x1234_5678)).unwrap();
        b.send_to(&sent, node(&relay, Leg::B)).await.ok();
        // B sends RTCP to the RTP port plus one by default; A's own RTCP
        // address is its RTP port plus one too, which this phone does not
        // read, so the RTCP path is proved with rtcp-mux below.
        relay.close().await;
    }

    #[tokio::test]
    async fn rtcp_on_the_rtp_port_is_relayed_as_rtcp() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let (a, b) = (phone().await, phone().await);
        let a_sdp = sdp(
            "127.0.0.1",
            a.local_addr().unwrap().port(),
            "a=rtcp-mux\r\n",
        );
        let b_sdp = sdp(
            "127.0.0.1",
            b.local_addr().unwrap().port(),
            "a=rtcp-mux\r\n",
        );
        relay.offer_key(Leg::B, false).unwrap();
        relay.set_peer(Leg::B, &b_sdp, None);
        relay.answered(Leg::B, &b_sdp);
        relay.set_peer(Leg::A, &a_sdp, None);
        relay.answer_key(Leg::A, &a_sdp).unwrap();

        // B asks A for a keyframe.
        b.send_to(&pli(2, 0x1234_5678), node(&relay, Leg::B))
            .await
            .unwrap();
        assert_eq!(recv(&a).await, Some(pli(2, 0x1234_5678)));
        assert_eq!(relay.stats().rtcp, 1);
        relay.close().await;
    }

    #[tokio::test]
    async fn a_stranger_is_not_relayed() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let (a, b) = (phone().await, phone().await);
        // A said it was elsewhere: its packets are a stranger's.
        let a_sdp = sdp("127.0.0.9", a.local_addr().unwrap().port(), "");
        let b_sdp = sdp("127.0.0.1", b.local_addr().unwrap().port(), "");
        relay.offer_key(Leg::B, false).unwrap();
        relay.set_peer(Leg::B, &b_sdp, None);
        relay.answered(Leg::B, &b_sdp);
        relay.set_peer(Leg::A, &a_sdp, None);
        relay.answer_key(Leg::A, &a_sdp).unwrap();

        a.send_to(&rtp(1), node(&relay, Leg::A)).await.unwrap();
        assert_eq!(recv(&b).await, None);
        assert_eq!(relay.stats().refused, 1);
        relay.close().await;
    }

    #[tokio::test]
    async fn a_phone_behind_nat_is_learned_from_its_first_packet() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let (a, b) = (phone().await, phone().await);
        // A's SDP names its private address; its packets come from the
        // NAT's, which the call already latched audio to.
        let a_sdp = sdp("10.9.8.7", 5004, "");
        let b_sdp = sdp("127.0.0.1", b.local_addr().unwrap().port(), "");
        relay.offer_key(Leg::B, false).unwrap();
        relay.set_peer(Leg::B, &b_sdp, None);
        relay.answered(Leg::B, &b_sdp);
        relay.set_peer(Leg::A, &a_sdp, Some("127.0.0.1".parse().unwrap()));
        relay.answer_key(Leg::A, &a_sdp).unwrap();

        // Nothing goes to A before it is heard from.
        b.send_to(&rtp(1), node(&relay, Leg::B)).await.unwrap();
        assert_eq!(recv(&a).await, None);

        a.send_to(&rtp(2), node(&relay, Leg::A)).await.unwrap();
        assert_eq!(recv(&b).await, Some(rtp(2)));
        b.send_to(&rtp(3), node(&relay, Leg::B)).await.unwrap();
        assert_eq!(
            recv(&a).await,
            Some(rtp(3)),
            "B's video goes where A's came from"
        );
        relay.close().await;
    }

    #[tokio::test]
    async fn a_moved_phone_keeps_its_port_and_the_leg_it_left_has_none() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let (a, b) = (phone().await, phone().await);
        plain_call(&relay, &a, &b).await;
        let (port_a, port_b) = (relay.port(Leg::A), relay.port(Leg::B));

        // B is parked onto A: its port goes with it.
        relay.move_peer(Leg::B, Leg::A);
        assert_eq!(relay.port(Leg::A), port_b);
        assert_eq!(relay.port(Leg::B), port_a);

        // The old A, now on B with no phone, is refused.
        a.send_to(&rtp(1), node(&relay, Leg::B)).await.unwrap();
        assert_eq!(recv(&b).await, None);
        assert_eq!(relay.stats().refused, 1);
        relay.close().await;
    }

    #[tokio::test]
    async fn every_device_offered_before_the_answer_gets_one_key() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let first = relay.offer_key(Leg::B, true).unwrap().unwrap();
        let second = relay.offer_key(Leg::B, true).unwrap().unwrap();
        assert_eq!(first.local_key_b64, second.local_key_b64);
        relay.close().await;
    }

    #[tokio::test]
    async fn a_held_relay_passes_nothing() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let (a, b) = (phone().await, phone().await);
        plain_call(&relay, &a, &b).await;
        relay.set_paused(true);
        a.send_to(&rtp(1), node(&relay, Leg::A)).await.unwrap();
        assert_eq!(recv(&b).await, None);
        relay.set_paused(false);
        a.send_to(&rtp(2), node(&relay, Leg::A)).await.unwrap();
        assert_eq!(recv(&b).await, Some(rtp(2)));
        relay.close().await;
    }

    #[tokio::test]
    async fn an_answer_with_a_key_the_node_cannot_use_is_not_relayed() {
        let relay = VideoRelay::open(VideoStream::Camera, pool()).await.unwrap();
        let (a, b) = (phone().await, phone().await);
        let a_sdp = sdp("127.0.0.1", a.local_addr().unwrap().port(), "");
        let b_sdp = sdp("127.0.0.1", b.local_addr().unwrap().port(), "");
        relay.offer_key(Leg::B, true).unwrap().expect("offered");
        let b_sdp = format!("{b_sdp}a=crypto:1 AES_CM_128_HMAC_SHA1_32 inline:other\r\n");
        relay.set_peer(Leg::B, &b_sdp, None);
        relay.answered(Leg::B, &b_sdp);
        relay.set_peer(Leg::A, &a_sdp, None);
        relay.answer_key(Leg::A, &a_sdp).unwrap();

        a.send_to(&rtp(1), node(&relay, Leg::A)).await.unwrap();
        assert_eq!(recv(&b).await, None);
        relay.close().await;
    }
}
