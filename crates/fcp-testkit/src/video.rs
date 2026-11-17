//! A scripted phone's video: one camera section that sends numbered
//! packets and counts what arrives (`docs/SOFTPHONE_VIDEO.md` §4).
//!
//! Nothing here is a codec. A packet is an RTP header with payload type
//! 96 (H.264 in the SDP, or VP8) and a payload naming its sequence number,
//! which is all a relay that copies packets can be held to. A frame sent by
//! [`VideoLeg::send_frames`] starts with the payload descriptor of its codec,
//! a keyframe's or not, which is all a room that forwards it without
//! decoding reads (`docs/FORWARDING_ROOMS.md`). What arrives is counted by
//! SSRC, for a room that sends several streams in the section. A section
//! told to send simulcast ([`VideoLeg::set_simulcast`]) sends each frame
//! once per layer (RFC 8853): its own SSRC, its rid in the header
//! extension, a VP8 keyframe header saying its size, and the rid after the
//! payload's number so a receiver can tell which layer reached it. RTCP is
//! a PLI (RFC 4585 §6.3.1) sent on request and counted on receipt, and a
//! REMB sent on request. A section told to carry transport-cc
//! ([`VideoLeg::enable_transport_cc`]) offers it and writes feedback for
//! what arrives, and its receive side can be shaped
//! ([`VideoLeg::shape`]): every packet passes a simulated link that drops
//! it or says when it would have arrived, which is all a sender's
//! estimate sees (`docs/TRANSPORT_CC.md` §5). With SDES
//! the section is keyed as a phone keys it: its own key in an offer, or
//! its own under the offer's tag in an answer.

use anyhow::{anyhow, Context, Result};
use fcp_media::{SdesCryptoLine, SrtpOffer, SrtpSdpContext, VideoStream};
use forge_rtp::srtp::SrtpContext;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// The payload type the section offers, as H.264.
pub const VIDEO_PT: u8 = 96;

/// The SSRC this phone's video is sent with; a simulcast layer's is this
/// plus one more than its index.
const SSRC: u32 = 0x5644_4f30;

/// The one-byte header extension id the rid goes under (RFC 8852).
const RID_EXT_ID: u8 = 3;

/// The one-byte header extension id transport-cc goes under.
pub const TCC_EXT_ID: u8 = 5;

/// What every payload carries after its codec's first bytes.
const MARK: &[u8] = b"fcp-video";

/// What the section says it sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoCodec {
    #[default]
    H264,
    Vp8,
}

/// How a phone keys its video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoMode {
    /// Plain RTP (`RTP/AVPF`).
    Plain,
    /// SDES-SRTP (`RTP/SAVPF`), the phone's own key.
    Srtp,
}

/// Transport-cc on the receive side: the feedback writer and, when the
/// link is shaped, the simulated link every packet passes.
#[derive(Default)]
struct Receiving {
    writer: forge_bwe::FeedbackWriter,
    link: Option<forge_bwe::testing::Bottleneck>,
    /// Packets the shaped link dropped.
    dropped: u64,
    /// Feedback messages sent.
    feedback_sent: u64,
    /// Padding-only packets (a sender's probes) seen.
    padding: u64,
}

enum Keys {
    Plain,
    /// The key this phone sent, waiting for the far end's.
    Mine(SrtpOffer),
    Keyed(Box<SrtpContext>),
}

/// One call's video section.
pub struct VideoLeg {
    rtp: Arc<UdpSocket>,
    rtcp: Arc<UdpSocket>,
    port: u16,
    advertised: IpAddr,
    mode: VideoMode,
    keys: Arc<Mutex<Keys>>,
    codec: VideoCodec,
    /// Packets received, by SSRC.
    by_ssrc: Arc<Mutex<std::collections::HashMap<u32, u64>>>,
    /// The layer each SSRC last carried, by the rid its payload named.
    layer_by_ssrc: Arc<Mutex<std::collections::HashMap<u32, String>>>,
    /// The simulcast layers it sends, `(rid, height)`, largest first;
    /// empty for one stream.
    simulcast: Mutex<Vec<(String, u32)>>,
    /// Each layer's next sequence number.
    layer_seq: Mutex<std::collections::HashMap<String, u16>>,
    /// Bytes a frame's packet is padded to ([`VideoLeg::set_packet_size`]).
    packet_size: std::sync::atomic::AtomicUsize,
    /// The `a=crypto` this phone puts in its section, once it has a key.
    crypto: Mutex<Option<SrtpSdpContext>>,
    remote: Arc<Mutex<Option<SocketAddr>>>,
    /// Transport-cc, once [`enable_transport_cc`](Self::enable_transport_cc).
    cc: Arc<Mutex<Option<Receiving>>>,
    /// Transport-cc feedback received (about what this phone sent).
    cc_feedback: Arc<AtomicU64>,
    received: Arc<AtomicU64>,
    plis: Arc<AtomicU64>,
    seq: AtomicU64,
    tasks: Vec<JoinHandle<()>>,
}

impl VideoLeg {
    /// A video section bound on `bind`, its SDP naming `advertised`.
    pub async fn open(bind: IpAddr, advertised: IpAddr, mode: VideoMode) -> Result<Self> {
        Self::open_as(bind, advertised, mode, VideoCodec::H264).await
    }

    /// [`open`](Self::open), saying it sends `codec`.
    pub async fn open_as(
        bind: IpAddr,
        advertised: IpAddr,
        mode: VideoMode,
        codec: VideoCodec,
    ) -> Result<Self> {
        let pair = crate::media::allocate_port_pair_at(bind);
        let rtp = Arc::new(UdpSocket::bind((bind, pair.rtp_port)).await?);
        let rtcp = Arc::new(UdpSocket::bind((bind, pair.rtp_port + 1)).await?);
        let keys = Arc::new(Mutex::new(match mode {
            VideoMode::Plain => Keys::Plain,
            VideoMode::Srtp => Keys::Mine(SrtpOffer::generate().map_err(|e| anyhow!("{e}"))?),
        }));
        let crypto = match &*keys.lock().unwrap() {
            Keys::Mine(offer) => Some(offer.sdp.clone()),
            _ => None,
        };
        let received = Arc::new(AtomicU64::new(0));
        let plis = Arc::new(AtomicU64::new(0));
        let by_ssrc = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let layer_by_ssrc = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let remote = Arc::new(Mutex::new(None));
        let cc = Arc::new(Mutex::new(None));
        let cc_feedback = Arc::new(AtomicU64::new(0));
        let tasks = [&rtp, &rtcp]
            .into_iter()
            .map(|socket| {
                tokio::spawn(read(
                    Arc::clone(socket),
                    Reader {
                        keys: Arc::clone(&keys),
                        received: Arc::clone(&received),
                        plis: Arc::clone(&plis),
                        by_ssrc: Arc::clone(&by_ssrc),
                        layer_by_ssrc: Arc::clone(&layer_by_ssrc),
                        cc: Arc::clone(&cc),
                        cc_feedback: Arc::clone(&cc_feedback),
                        remote: Arc::clone(&remote),
                        rtcp: Arc::clone(&rtcp),
                    },
                ))
            })
            .collect();
        Ok(Self {
            rtp,
            rtcp,
            port: pair.rtp_port,
            advertised,
            mode,
            keys,
            codec,
            by_ssrc,
            layer_by_ssrc,
            simulcast: Mutex::new(Vec::new()),
            layer_seq: Mutex::new(std::collections::HashMap::new()),
            packet_size: std::sync::atomic::AtomicUsize::new(0),
            crypto: Mutex::new(crypto),
            remote,
            cc,
            cc_feedback,
            received,
            plis,
            seq: AtomicU64::new(0),
            tasks,
        })
    }

    /// This phone's camera section, to append to its SDP.
    pub fn section(&self, direction: &str) -> String {
        let profile = match self.mode {
            VideoMode::Plain => "RTP/AVPF",
            VideoMode::Srtp => "RTP/SAVPF",
        };
        let codec = match self.codec {
            VideoCodec::H264 => format!(
                "a=rtpmap:{pt} H264/90000\r\n\
                 a=fmtp:{pt} profile-level-id=42e01f;packetization-mode=1\r\n",
                pt = VIDEO_PT
            ),
            VideoCodec::Vp8 => format!("a=rtpmap:{VIDEO_PT} VP8/90000\r\n"),
        };
        let mut s = format!(
            "m=video {port} {profile} {pt}\r\n\
             c=IN IP4 {ip}\r\n\
             {codec}\
             a=rtcp-fb:{pt} nack\r\n\
             a=rtcp-fb:{pt} nack pli\r\n\
             a=rtcp-fb:{pt} ccm fir\r\n\
             a=rtcp-fb:{pt} goog-remb\r\n\
             a={direction}\r\n",
            port = self.port,
            ip = self.advertised,
            pt = VIDEO_PT,
        );
        let simulcast = self.simulcast.lock().unwrap();
        if !simulcast.is_empty() {
            s.push_str(&format!(
                "a=extmap:{RID_EXT_ID} urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id\r\n"
            ));
            for (rid, _) in simulcast.iter() {
                s.push_str(&format!("a=rid:{rid} send\r\n"));
            }
            let rids: Vec<&str> = simulcast.iter().map(|(r, _)| r.as_str()).collect();
            s.push_str(&format!("a=simulcast:send {}\r\n", rids.join(";")));
        }
        drop(simulcast);
        if self.cc.lock().unwrap().is_some() {
            s.push_str(&format!(
                "a=extmap:{TCC_EXT_ID} {}\r\na=rtcp-fb:{VIDEO_PT} transport-cc\r\n",
                forge_rtp::transport_cc::TRANSPORT_CC_URI
            ));
        }
        if let Some(c) = &*self.crypto.lock().unwrap() {
            s.push_str(&format!(
                "a=crypto:{} {} {}\r\n",
                c.tag, c.suite, c.local_key_b64
            ));
        }
        s
    }

    /// The far end's SDP, offer or answer: where its video goes, and its
    /// key. An offer's key is answered with one of this phone's own.
    pub fn set_remote(&self, sdp: &str) -> Result<()> {
        *self.remote.lock().unwrap() = fcp_media::sdp::stream_endpoint(sdp, VideoStream::Camera);
        let theirs: Option<SdesCryptoLine> =
            fcp_media::sdp::stream_sdes_crypto(sdp, VideoStream::Camera)
                .into_iter()
                .next();
        let mut keys = self.keys.lock().unwrap();
        let next = match (std::mem::replace(&mut *keys, Keys::Plain), theirs) {
            (Keys::Mine(mine), Some(theirs)) => {
                Keys::Keyed(Box::new(mine.accept(&theirs).map_err(|e| anyhow!("{e}"))?))
            }
            // Offered a key: answer with ours, under its tag.
            (Keys::Plain | Keys::Keyed(_), Some(theirs)) if self.mode == VideoMode::Srtp => {
                let mine = SrtpOffer::answering(&theirs).map_err(|e| anyhow!("{e}"))?;
                *self.crypto.lock().unwrap() = Some(mine.sdp.clone());
                Keys::Keyed(Box::new(mine.accept(&theirs).map_err(|e| anyhow!("{e}"))?))
            }
            (Keys::Keyed(ctx), None) if self.mode == VideoMode::Srtp => Keys::Keyed(ctx),
            (other, _) => other,
        };
        *keys = next;
        Ok(())
    }

    /// The port this phone's video is on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Where this phone's video goes, once the far end said.
    pub fn remote(&self) -> Option<SocketAddr> {
        *self.remote.lock().unwrap()
    }

    /// Whether this phone and the far end agreed on a key.
    pub fn keyed(&self) -> bool {
        matches!(&*self.keys.lock().unwrap(), Keys::Keyed(_))
    }

    /// Send `n` packets, numbered on from the last.
    pub async fn send(&self, n: u16) -> Result<()> {
        let to = self.remote().context("no far end for video")?;
        for _ in 0..n {
            let seq = self.seq.fetch_add(1, Ordering::Relaxed) as u16;
            let mut packet = vec![0x80, VIDEO_PT];
            packet.extend_from_slice(&seq.to_be_bytes());
            packet.extend_from_slice(&(u32::from(seq) * 3000).to_be_bytes());
            packet.extend_from_slice(&SSRC.to_be_bytes());
            packet.extend_from_slice(b"fcp-video");
            packet.extend_from_slice(&seq.to_be_bytes());
            let out = self.protect(&packet, false)?;
            self.rtp.send_to(&out, to).await?;
        }
        Ok(())
    }

    /// Send `n` frames of one packet each, numbered on from the last, every
    /// `keyframe_every`-th a keyframe (the first always is): the payload
    /// starts as the codec's does, so a room that forwards without
    /// decoding can tell where pictures start.
    pub async fn send_frames(&self, n: u16, keyframe_every: u16) -> Result<()> {
        let rids: Vec<String> = self
            .simulcast
            .lock()
            .unwrap()
            .iter()
            .map(|(r, _)| r.clone())
            .collect();
        if !rids.is_empty() {
            let rids: Vec<&str> = rids.iter().map(String::as_str).collect();
            return self.send_layers(n, keyframe_every, &rids).await;
        }
        let to = self.remote().context("no far end for video")?;
        for i in 0..n {
            let seq = self.seq.fetch_add(1, Ordering::Relaxed) as u16;
            let key = i % keyframe_every.max(1) == 0;
            let mut packet = vec![0x80, 0x80 | VIDEO_PT];
            packet.extend_from_slice(&seq.to_be_bytes());
            packet.extend_from_slice(&(u32::from(seq) * 3000).to_be_bytes());
            packet.extend_from_slice(&SSRC.to_be_bytes());
            match self.codec {
                // An IDR slice, or a P slice.
                VideoCodec::H264 => packet.push(if key { 0x65 } else { 0x41 }),
                // The start of partition 0, then the payload header's P bit.
                VideoCodec::Vp8 => packet.extend_from_slice(&[0x10, if key { 0x00 } else { 0x01 }]),
            }
            packet.extend_from_slice(MARK);
            packet.extend_from_slice(&seq.to_be_bytes());
            let size = self.packet_size.load(Ordering::Relaxed);
            if packet.len() < size {
                // Zeros, which no rid begins with.
                packet.resize(size, 0);
            }
            let out = self.protect(&packet, false)?;
            self.rtp.send_to(&out, to).await?;
        }
        Ok(())
    }

    /// Packets of this harness's received, by the SSRC they came under.
    pub fn received_by_ssrc(&self) -> std::collections::HashMap<u32, u64> {
        self.by_ssrc.lock().unwrap().clone()
    }

    /// The simulcast layer each SSRC last carried, by rid.
    pub fn layer_by_ssrc(&self) -> std::collections::HashMap<u32, String> {
        self.layer_by_ssrc.lock().unwrap().clone()
    }

    /// Pad each packet [`send_frames`](Self::send_frames) sends to `bytes`
    /// of RTP, as a camera's packets are: what a load run measures a
    /// forwarding room's per-packet cost with.
    pub fn set_packet_size(&self, bytes: usize) {
        self.packet_size.store(bytes, Ordering::Relaxed);
    }

    /// Send simulcast: `layers` as `(rid, height)`, largest first (RFC
    /// 8853). Set before the section goes into an offer.
    pub fn set_simulcast(&self, layers: &[(&str, u32)]) {
        *self.simulcast.lock().unwrap() = layers
            .iter()
            .map(|(rid, h)| (rid.to_string(), *h))
            .collect();
    }

    /// Send `n` frames on the simulcast layers named by `rids` alone,
    /// every `keyframe_every`-th a keyframe (the first always is): what
    /// a sender does when its uplink stops carrying its largest layer.
    pub async fn send_layers(&self, n: u16, keyframe_every: u16, rids: &[&str]) -> Result<()> {
        let to = self.remote().context("no far end for video")?;
        let layers: Vec<(usize, String, u32)> = self
            .simulcast
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, (rid, _))| rids.contains(&rid.as_str()))
            .map(|(i, (rid, h))| (i, rid.clone(), *h))
            .collect();
        for i in 0..n {
            let frame = self.seq.fetch_add(1, Ordering::Relaxed) as u32;
            let key = i % keyframe_every.max(1) == 0;
            for (index, rid, height) in &layers {
                let seq = {
                    let mut seqs = self.layer_seq.lock().unwrap();
                    let s = seqs.entry(rid.clone()).or_default();
                    let now = *s;
                    *s = s.wrapping_add(1);
                    now
                };
                // RTP with the one-byte header extension (RFC 8285): the
                // rid, padded to a word.
                let mut packet = vec![0x90, 0x80 | VIDEO_PT];
                packet.extend_from_slice(&seq.to_be_bytes());
                packet.extend_from_slice(&(frame * 3000).to_be_bytes());
                packet.extend_from_slice(&(SSRC + 1 + *index as u32).to_be_bytes());
                let mut ext = vec![(RID_EXT_ID << 4) | (rid.len() as u8 - 1)];
                ext.extend_from_slice(rid.as_bytes());
                while ext.len() % 4 != 0 {
                    ext.push(0);
                }
                packet.extend_from_slice(&[0xBE, 0xDE]);
                packet.extend_from_slice(&((ext.len() / 4) as u16).to_be_bytes());
                packet.extend_from_slice(&ext);
                packet.push(0x10); // VP8: the start of partition 0
                if key {
                    // A key frame's tag, start code and size (RFC 6386 §9.1).
                    let width = (height * 16 / 9) as u16;
                    packet.extend_from_slice(&[0x00, 0x00, 0x00, 0x9d, 0x01, 0x2a]);
                    packet.extend_from_slice(&width.to_le_bytes());
                    packet.extend_from_slice(&(*height as u16).to_le_bytes());
                } else {
                    packet.extend_from_slice(&[0x01, 0x00, 0x00]);
                }
                packet.extend_from_slice(MARK);
                packet.extend_from_slice(&seq.to_be_bytes());
                packet.extend_from_slice(rid.as_bytes());
                let out = self.protect(&packet, false)?;
                self.rtp.send_to(&out, to).await?;
            }
        }
        Ok(())
    }

    /// Offer transport-cc on this section and write feedback for what
    /// arrives numbered under it. Before the section is offered.
    pub fn enable_transport_cc(&self) {
        self.cc
            .lock()
            .unwrap()
            .get_or_insert_with(Receiving::default);
    }

    /// Shape what this phone receives to `rate_bps` (a 64 KB queue, 20 ms
    /// one way), or stop shaping (`None`). Needs transport-cc.
    pub fn shape(&self, rate_bps: Option<u64>) {
        if let Some(r) = self.cc.lock().unwrap().as_mut() {
            match (rate_bps, r.link.as_mut()) {
                (Some(bps), Some(link)) => link.set_rate(bps),
                (Some(bps), None) => {
                    r.link = Some(forge_bwe::testing::Bottleneck::new(
                        bps,
                        64 * 1024,
                        Duration::from_millis(20),
                    ))
                }
                (None, _) => r.link = None,
            }
        }
    }

    /// Transport-cc feedback sent, packets the shaped link dropped, and
    /// padding-only packets (probes) seen.
    pub fn transport_cc_stats(&self) -> (u64, u64, u64) {
        self.cc
            .lock()
            .unwrap()
            .as_ref()
            .map_or((0, 0, 0), |r| (r.feedback_sent, r.dropped, r.padding))
    }

    /// Transport-cc feedback received about what this phone sent.
    pub fn transport_cc_feedback_received(&self) -> u64 {
        self.cc_feedback.load(Ordering::Relaxed)
    }

    /// Tell the far end what this phone can take: a REMB
    /// (draft-alvestrand-rmcat-remb) of `kbps`.
    pub async fn send_remb(&self, kbps: u32) -> Result<()> {
        let to = self.remote().context("no far end for video")?;
        let mut mantissa = u64::from(kbps) * 1000;
        let mut exp = 0u8;
        while mantissa >= 1 << 18 {
            mantissa >>= 1;
            exp += 1;
        }
        let mut remb = vec![0x8F, 206, 0, 5];
        remb.extend_from_slice(&SSRC.to_be_bytes());
        remb.extend_from_slice(&0u32.to_be_bytes());
        remb.extend_from_slice(b"REMB");
        remb.push(1);
        remb.push((exp << 2) | ((mantissa >> 16) as u8 & 0x03));
        remb.push((mantissa >> 8) as u8);
        remb.push(mantissa as u8);
        remb.extend_from_slice(&0u32.to_be_bytes());
        let out = self.protect(&remb, true)?;
        self.rtcp
            .send_to(&out, SocketAddr::new(to.ip(), to.port() + 1))
            .await?;
        Ok(())
    }

    /// Ask the far end for a keyframe.
    pub async fn send_pli(&self) -> Result<()> {
        let to = self.remote().context("no far end for video")?;
        let mut pli = vec![0x81, 206, 0, 2];
        pli.extend_from_slice(&SSRC.to_be_bytes());
        pli.extend_from_slice(&0u32.to_be_bytes());
        let out = self.protect(&pli, true)?;
        self.rtcp
            .send_to(&out, SocketAddr::new(to.ip(), to.port() + 1))
            .await?;
        Ok(())
    }

    fn protect(&self, packet: &[u8], rtcp: bool) -> Result<Vec<u8>> {
        match &mut *self.keys.lock().unwrap() {
            Keys::Keyed(ctx) => if rtcp {
                ctx.protect_rtcp(packet)
            } else {
                ctx.protect_rtp(packet)
            }
            .map_err(|e| anyhow!("SRTP: {e}")),
            Keys::Mine(_) => Err(anyhow!("video is not keyed yet")),
            Keys::Plain => Ok(packet.to_vec()),
        }
    }

    /// Video packets received that read as this harness's.
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    /// Wait until at least `n` video packets have arrived.
    pub async fn received_at_least(&self, n: u64, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        while tokio::time::Instant::now() < deadline {
            if self.received() >= n {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.received() >= n
    }

    /// PLIs received.
    pub fn plis(&self) -> u64 {
        self.plis.load(Ordering::Relaxed)
    }

    /// Wait for a PLI.
    pub async fn pli_received(&self, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        while tokio::time::Instant::now() < deadline {
            if self.plis() > 0 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.plis() > 0
    }
}

impl Drop for VideoLeg {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Read one socket: RTP from this harness counted, PLIs counted.
/// What a section's reading task shares with it.
struct Reader {
    keys: Arc<Mutex<Keys>>,
    received: Arc<AtomicU64>,
    plis: Arc<AtomicU64>,
    by_ssrc: Arc<Mutex<std::collections::HashMap<u32, u64>>>,
    layer_by_ssrc: Arc<Mutex<std::collections::HashMap<u32, String>>>,
    cc: Arc<Mutex<Option<Receiving>>>,
    cc_feedback: Arc<AtomicU64>,
    remote: Arc<Mutex<Option<SocketAddr>>>,
    rtcp: Arc<UdpSocket>,
}

impl Reader {
    /// Transport-cc for one arriving RTP packet: through the shaped link,
    /// noted by the writer. `false` when the link dropped it. Feedback
    /// that is due goes out.
    async fn transport_cc(&self, plain: &[u8]) -> bool {
        let feedback = {
            let mut cc = self.cc.lock().unwrap();
            let Some(r) = cc.as_mut() else {
                return true;
            };
            let Ok(p) = forge_rtp::RtpPacket::parse(bytes::Bytes::copy_from_slice(plain)) else {
                return true;
            };
            if p.payload.is_empty() && p.header.padding() {
                r.padding += 1;
            }
            let now = std::time::Instant::now();
            let at = match r.link.as_mut() {
                // UDP and IP on the wire too.
                Some(link) => match link.send(plain.len() + 28, now) {
                    Some(at) => at,
                    None => {
                        r.dropped += 1;
                        return false;
                    }
                },
                None => now,
            };
            if let Some(seq) = forge_rtp::transport_cc::transport_sequence(&p, TCC_EXT_ID) {
                r.writer.on_packet(seq, at);
            }
            let fb = r.writer.poll(now);
            if fb.is_some() {
                r.feedback_sent += 1;
            }
            fb
        };
        if let Some(fb) = feedback {
            let packet = forge_rtp::RtcpPacket::TransportFeedback(forge_rtp::RtpFeedback {
                sender_ssrc: SSRC,
                media_ssrc: 0,
                kind: forge_rtp::rtcp::TransportFeedback::TransportCc(fb),
            })
            .to_bytes();
            let out = match &mut *self.keys.lock().unwrap() {
                Keys::Keyed(ctx) => ctx.protect_rtcp(&packet).ok(),
                Keys::Plain => Some(packet),
                Keys::Mine(_) => None,
            };
            let to = *self.remote.lock().unwrap();
            if let (Some(out), Some(to)) = (out, to) {
                let _ = self
                    .rtcp
                    .send_to(&out, SocketAddr::new(to.ip(), to.port() + 1))
                    .await;
            }
        }
        true
    }
}

async fn read(socket: Arc<UdpSocket>, r: Reader) {
    let Reader {
        ref keys,
        ref received,
        ref plis,
        ref by_ssrc,
        ref layer_by_ssrc,
        ..
    } = r;
    let mut buf = vec![0u8; 2048];
    loop {
        let Ok((n, _)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        let data = &buf[..n];
        let rtcp = data.len() >= 2 && (192..=223).contains(&data[1]);
        let plain = match &mut *keys.lock().unwrap() {
            Keys::Keyed(ctx) => {
                let p = if rtcp {
                    ctx.unprotect_rtcp(data)
                } else {
                    ctx.unprotect_rtp(data)
                };
                match p {
                    Ok(p) => p,
                    Err(_) => continue,
                }
            }
            Keys::Mine(_) => continue,
            Keys::Plain => data.to_vec(),
        };
        if rtcp {
            if plain.len() >= 2 && plain[1] == 206 && plain[0] & 0x1f == 1 {
                plis.fetch_add(1, Ordering::Relaxed);
            }
            for p in forge_rtp::RtcpPacket::parse_compound(&plain) {
                if let forge_rtp::RtcpPacket::TransportFeedback(forge_rtp::RtpFeedback {
                    kind: forge_rtp::rtcp::TransportFeedback::TransportCc(_),
                    ..
                }) = p
                {
                    r.cc_feedback.fetch_add(1, Ordering::Relaxed);
                }
            }
        } else if plain.len() > 12 && plain[1] & 0x7f == VIDEO_PT {
            if !r.transport_cc(&plain).await {
                continue;
            }
            // After the header: the payload as sent, behind its codec's
            // first bytes when it was sent as frames.
            let head = &plain[12..plain.len().min(12 + 48)];
            let Some(at) = head.windows(MARK.len()).position(|w| w == MARK) else {
                continue;
            };
            received.fetch_add(1, Ordering::Relaxed);
            let ssrc = u32::from_be_bytes([plain[8], plain[9], plain[10], plain[11]]);
            *by_ssrc.lock().unwrap().entry(ssrc).or_default() += 1;
            let rest = &plain[12 + at + MARK.len()..];
            if rest.len() > 2 {
                let rid = &rest[2..];
                let rid = &rid[..rid.iter().position(|b| *b == 0).unwrap_or(rid.len())];
                if let Ok(rid) = std::str::from_utf8(rid) {
                    if !rid.is_empty() {
                        layer_by_ssrc.lock().unwrap().insert(ssrc, rid.to_string());
                    }
                }
            }
        }
    }
}
