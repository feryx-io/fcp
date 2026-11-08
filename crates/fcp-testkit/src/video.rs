//! A scripted phone's video: one camera section that sends numbered
//! packets and counts what arrives (`docs/SOFTPHONE_VIDEO.md` §4).
//!
//! Nothing here is a codec. A packet is an RTP header with payload type
//! 96 (H.264 in the SDP) and a payload naming its sequence number, which
//! is all a relay that copies packets can be held to. RTCP is a PLI
//! (RFC 4585 §6.3.1) sent on request and counted on receipt. With SDES
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

/// The SSRC this phone's video is sent with.
const SSRC: u32 = 0x5644_4f30;

/// How a phone keys its video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoMode {
    /// Plain RTP (`RTP/AVPF`).
    Plain,
    /// SDES-SRTP (`RTP/SAVPF`), the phone's own key.
    Srtp,
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
    /// The `a=crypto` this phone puts in its section, once it has a key.
    crypto: Mutex<Option<SrtpSdpContext>>,
    remote: Mutex<Option<SocketAddr>>,
    received: Arc<AtomicU64>,
    plis: Arc<AtomicU64>,
    seq: AtomicU64,
    tasks: Vec<JoinHandle<()>>,
}

impl VideoLeg {
    /// A video section bound on `bind`, its SDP naming `advertised`.
    pub async fn open(bind: IpAddr, advertised: IpAddr, mode: VideoMode) -> Result<Self> {
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
        let tasks = vec![
            tokio::spawn(read(
                Arc::clone(&rtp),
                Arc::clone(&keys),
                Arc::clone(&received),
                Arc::clone(&plis),
            )),
            tokio::spawn(read(
                Arc::clone(&rtcp),
                Arc::clone(&keys),
                Arc::clone(&received),
                Arc::clone(&plis),
            )),
        ];
        Ok(Self {
            rtp,
            rtcp,
            port: pair.rtp_port,
            advertised,
            mode,
            keys,
            crypto: Mutex::new(crypto),
            remote: Mutex::new(None),
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
        let mut s = format!(
            "m=video {port} {profile} {pt}\r\n\
             c=IN IP4 {ip}\r\n\
             a=rtpmap:{pt} H264/90000\r\n\
             a=fmtp:{pt} profile-level-id=42e01f;packetization-mode=1\r\n\
             a=rtcp-fb:{pt} nack\r\n\
             a=rtcp-fb:{pt} nack pli\r\n\
             a=rtcp-fb:{pt} ccm fir\r\n\
             a=rtcp-fb:{pt} goog-remb\r\n\
             a={direction}\r\n",
            port = self.port,
            ip = self.advertised,
            pt = VIDEO_PT,
        );
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
async fn read(
    socket: Arc<UdpSocket>,
    keys: Arc<Mutex<Keys>>,
    received: Arc<AtomicU64>,
    plis: Arc<AtomicU64>,
) {
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
        } else if plain.len() > 12
            && plain[1] & 0x7f == VIDEO_PT
            && plain[12..].starts_with(b"fcp-video")
        {
            received.fetch_add(1, Ordering::Relaxed);
        }
    }
}
