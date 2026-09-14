//! The phone's media: one RTP port pair on loopback that plays tones and
//! DTMF at the negotiated codec and hears what comes back. Assertions
//! are about hearing — a tone detected at the far end says the whole
//! path (SDP, ports, codec, direction) works — with the packet count as
//! the fallback for paths that are one-way by design.

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use forge_dtmf::rfc2833::{Rfc2833Detector, Rfc2833Generator};
use forge_dtmf::{DtmfDigit, DtmfEventType};
use forge_rtp::{LatchPolicy, PortPair, RemoteEndpoint, RtpPacket, RtpSocketConfig, RtpSocketPair};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

/// Samples per 20 ms packet at 8 kHz.
const FRAME: usize = 160;
/// The telephone-event payload type the phone offers.
pub const TELEPHONE_EVENT_PT: u8 = 101;

/// An audio codec the phone speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Pcmu,
    Pcma,
}

impl Codec {
    pub fn payload_type(self) -> u8 {
        match self {
            Codec::Pcmu => 0,
            Codec::Pcma => 8,
        }
    }

    pub fn sdp_name(self) -> &'static str {
        match self {
            Codec::Pcmu => "PCMU",
            Codec::Pcma => "PCMA",
        }
    }

    /// One sample to the wire.
    pub fn encode(self, pcm: i16) -> u8 {
        match self {
            Codec::Pcmu => forge_codecs::g711::encode_ulaw(pcm),
            Codec::Pcma => forge_codecs::g711::encode_alaw(pcm),
        }
    }

    /// One wire byte to a sample.
    pub fn decode(self, byte: u8) -> i16 {
        match self {
            Codec::Pcmu => forge_codecs::g711::decode_ulaw(byte),
            Codec::Pcma => forge_codecs::g711::decode_alaw(byte),
        }
    }

    pub fn from_payload_type(pt: u8) -> Option<Self> {
        match pt {
            0 => Some(Codec::Pcmu),
            8 => Some(Codec::Pcma),
            _ => None,
        }
    }
}

/// Where the far end's audio is, from its SDP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAudio {
    pub addr: SocketAddr,
    pub codec: Codec,
    pub telephone_event_pt: Option<u8>,
    /// `a=sendonly` / `a=inactive` from the far end: it will not send.
    pub sending: bool,
}

/// The audio section of an SDP: address, port, codec, telephone-event.
pub fn parse_remote_audio(sdp: &str) -> Result<RemoteAudio> {
    let desc = sip_sdp::parse::parse_sdp(sdp).map_err(|e| anyhow!("SDP: {e}"))?;
    let audio = desc
        .media
        .iter()
        .find(|m| m.media_type == sip_sdp::MediaType::Audio)
        .context("SDP has no audio section")?;
    if audio.port == 0 {
        return Err(anyhow!("audio section rejected (port 0)"));
    }
    let ip: IpAddr = audio
        .connection
        .as_ref()
        .or(desc.connection.as_ref())
        .map(|c| c.connection_address.as_str())
        .context("SDP has no connection address")?
        .parse()
        .context("SDP connection address")?;
    let mut codec = None;
    let mut te = None;
    for fmt in &audio.formats {
        let pt: u8 = match fmt.parse() {
            Ok(pt) => pt,
            Err(_) => continue,
        };
        let name = audio
            .rtpmaps
            .get(&pt)
            .map(|r| r.encoding_name.to_ascii_uppercase());
        match (name.as_deref(), pt) {
            (Some("TELEPHONE-EVENT"), _) => te = te.or(Some(pt)),
            (Some("PCMU"), _) | (None, 0) => codec = codec.or(Some(Codec::Pcmu)),
            (Some("PCMA"), _) | (None, 8) => codec = codec.or(Some(Codec::Pcma)),
            _ => {}
        }
    }
    let codec = codec.context("no PCMU or PCMA in the audio section")?;
    let is_flag = |a: &sip_sdp::Attribute, name: &str| match a {
        sip_sdp::Attribute::Property(p) => p.eq_ignore_ascii_case(name),
        sip_sdp::Attribute::Value { .. } => false,
    };
    let direction_attr = |name: &str| {
        audio.attributes.iter().any(|a| is_flag(a, name))
            || desc.attributes.iter().any(|a| is_flag(a, name))
    };
    let sending = !(direction_attr("recvonly") || direction_attr("inactive"));
    Ok(RemoteAudio {
        addr: SocketAddr::new(ip, audio.port),
        codec,
        telephone_event_pt: te,
        sending,
    })
}

/// A free even RTP port with its odd RTCP neighbour, on loopback.
pub fn allocate_port_pair() -> PortPair {
    for _ in 0..200 {
        // 30000–59998, even.
        let port = 30000 + (rand_u16() % 15000) * 2;
        let rtp = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, port));
        let rtcp = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, port + 1));
        if rtp.is_ok() && rtcp.is_ok() {
            drop((rtp, rtcp));
            return PortPair::new(port).expect("even port");
        }
    }
    panic!("no free RTP port pair on loopback after 200 attempts");
}

fn rand_u16() -> u16 {
    let n = uuid::Uuid::new_v4().as_u128();
    (n & 0xFFFF) as u16
}

/// What the receiver has heard so far.
#[derive(Default)]
struct Heard {
    /// Consecutive frames in which each watched frequency dominated.
    tone_runs: HashMap<u32, u32>,
    /// The most consecutive frames each watched frequency ever had.
    tone_best: HashMap<u32, u32>,
    dtmf: Vec<char>,
}

enum Playing {
    Nothing,
    Tone { hz: f32, until: Instant, phase: f32 },
}

/// One media leg: the socket, what is being sent, what has been heard.
pub struct MediaLeg {
    socket: Arc<RtpSocketPair>,
    pub local_port: u16,
    remote: Arc<RwLock<Option<RemoteAudio>>>,
    playing: Arc<Mutex<Playing>>,
    watched: Arc<Mutex<HashSet<u32>>>,
    heard: Arc<Mutex<Heard>>,
    packets_received: Arc<AtomicU64>,
    packets_sent: Arc<AtomicU64>,
    dtmf_tx: mpsc::UnboundedSender<char>,
    stop: tokio::sync::watch::Sender<bool>,
}

impl MediaLeg {
    /// Bind a port pair; the sender and receiver tasks start now and
    /// wait for the far end.
    pub async fn open() -> Result<Self> {
        let pair = allocate_port_pair();
        let socket = Arc::new(
            RtpSocketPair::new(
                pair,
                RtpSocketConfig {
                    bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                    ..RtpSocketConfig::default()
                },
            )
            .await
            .context("RTP socket")?,
        );
        let (stop, stop_rx) = tokio::sync::watch::channel(false);
        let (dtmf_tx, dtmf_rx) = mpsc::unbounded_channel();
        let leg = Self {
            socket,
            local_port: pair.rtp_port,
            remote: Arc::new(RwLock::new(None)),
            playing: Arc::new(Mutex::new(Playing::Nothing)),
            watched: Arc::new(Mutex::new(HashSet::new())),
            heard: Arc::new(Mutex::new(Heard::default())),
            packets_received: Arc::new(AtomicU64::new(0)),
            packets_sent: Arc::new(AtomicU64::new(0)),
            dtmf_tx,
            stop,
        };
        leg.spawn_sender(dtmf_rx, stop_rx.clone());
        leg.spawn_receiver(stop_rx);
        Ok(leg)
    }

    /// The SDP the phone offers or answers with.
    pub fn sdp(&self, direction: &str) -> String {
        format!(
            "v=0\r\n\
             o=- {id} 1 IN IP4 127.0.0.1\r\n\
             s=fcp-testkit\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n\
             m=audio {port} RTP/AVP 0 8 {te}\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=rtpmap:{te} telephone-event/8000\r\n\
             a=fmtp:{te} 0-16\r\n\
             a=ptime:20\r\n\
             a={direction}\r\n",
            id = self.local_port,
            port = self.local_port,
            te = TELEPHONE_EVENT_PT,
        )
    }

    /// Point at the far end named by its SDP.
    pub async fn set_remote(&self, sdp: &str) -> Result<RemoteAudio> {
        let remote = parse_remote_audio(sdp)?;
        self.socket
            .set_remote_endpoint(RemoteEndpoint {
                rtp_addr: remote.addr,
                rtcp_addr: Some(SocketAddr::new(
                    remote.addr.ip(),
                    remote.addr.port().wrapping_add(1),
                )),
            })
            .await;
        let mut allowed = HashSet::new();
        allowed.insert(remote.addr.ip());
        self.socket
            .set_latch_policy(LatchPolicy::AllowedIps(allowed))
            .await;
        *self.remote.write().await = Some(remote.clone());
        Ok(remote)
    }

    pub async fn remote(&self) -> Option<RemoteAudio> {
        self.remote.read().await.clone()
    }

    /// Play a sine at `hz` for `for_ms` (the sender paces it 20 ms at a
    /// time at the negotiated codec).
    pub fn play_tone(&self, hz: f32, for_ms: u64) {
        *self.playing.lock().unwrap() = Playing::Tone {
            hz,
            until: Instant::now() + Duration::from_millis(for_ms),
            phase: 0.0,
        };
    }

    pub fn stop_tone(&self) {
        *self.playing.lock().unwrap() = Playing::Nothing;
    }

    /// Send one RFC 2833 digit: a start, continues over 100 ms, the
    /// three end packets.
    pub fn send_dtmf(&self, digit: char) {
        let _ = self.dtmf_tx.send(digit);
    }

    /// Wait until `hz` has dominated the received audio for a few
    /// consecutive frames; `false` at the deadline.
    pub async fn heard(&self, hz: u32, within: Duration) -> bool {
        self.watched.lock().unwrap().insert(hz);
        let deadline = Instant::now() + within;
        loop {
            {
                let heard = self.heard.lock().unwrap();
                if heard.tone_best.get(&hz).copied().unwrap_or(0) >= 3 {
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The most consecutive frames `hz` dominated so far, for negative
    /// assertions after a wait.
    pub fn tone_frames(&self, hz: u32) -> u32 {
        self.heard
            .lock()
            .unwrap()
            .tone_best
            .get(&hz)
            .copied()
            .unwrap_or(0)
    }

    /// The next digit whose end packet arrived; `None` at the deadline.
    pub async fn heard_dtmf(&self, within: Duration) -> Option<char> {
        let deadline = Instant::now() + within;
        loop {
            {
                let mut heard = self.heard.lock().unwrap();
                if !heard.dtmf.is_empty() {
                    return Some(heard.dtmf.remove(0));
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn packets_received(&self) -> u64 {
        self.packets_received.load(Ordering::Relaxed)
    }

    pub fn packets_sent(&self) -> u64 {
        self.packets_sent.load(Ordering::Relaxed)
    }

    /// Wait until at least `n` packets have arrived; `false` at the deadline.
    pub async fn received_at_least(&self, n: u64, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while self.packets_received() < n {
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        true
    }

    fn spawn_sender(
        &self,
        mut dtmf_rx: mpsc::UnboundedReceiver<char>,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) {
        let socket = Arc::clone(&self.socket);
        let remote = Arc::clone(&self.remote);
        let playing = Arc::clone(&self.playing);
        let sent = Arc::clone(&self.packets_sent);
        tokio::spawn(async move {
            let ssrc = rand_u16() as u32 * 7919 + 1;
            let mut seq: u16 = rand_u16();
            let mut ts: u32 = rand_u16() as u32 * 160;
            let mut ticker = tokio::time::interval(Duration::from_millis(20));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut dtmf_gen = Rfc2833Generator::new(8000, 20);
            let mut dtmf_left: Option<(u32, u32)> = None; // (packets to go, timestamp)
            let mut pending_dtmf: Vec<char> = Vec::new();
            let mut first = true;
            loop {
                tokio::select! {
                    _ = stop.changed() => return,
                    Some(d) = dtmf_rx.recv() => { pending_dtmf.push(d); }
                    _ = ticker.tick() => {}
                }
                let Some(remote) = remote.read().await.clone() else {
                    continue;
                };
                // A digit in progress, or the next one.
                if dtmf_left.is_none() {
                    if let Some(d) = pending_dtmf.first().copied() {
                        if let (Some(te), Some(digit)) =
                            (remote.telephone_event_pt, DtmfDigit::from_char(d).ok())
                        {
                            pending_dtmf.remove(0);
                            let ev = dtmf_gen.start_digit(digit);
                            let pkt = RtpPacket::build(
                                te,
                                seq,
                                ts,
                                ssrc,
                                Bytes::from(ev.to_bytes()),
                                true,
                            );
                            seq = seq.wrapping_add(1);
                            if socket.send_rtp(&pkt).await.is_ok() {
                                sent.fetch_add(1, Ordering::Relaxed);
                            }
                            dtmf_left = Some((5, ts));
                            continue;
                        } else {
                            pending_dtmf.remove(0);
                        }
                    }
                }
                if let Some((left, start_ts)) = dtmf_left {
                    let te = remote.telephone_event_pt.unwrap_or(TELEPHONE_EVENT_PT);
                    if left > 0 {
                        if let Some(ev) = dtmf_gen.continue_digit() {
                            let pkt = RtpPacket::build(
                                te,
                                seq,
                                start_ts,
                                ssrc,
                                Bytes::from(ev.to_bytes()),
                                false,
                            );
                            seq = seq.wrapping_add(1);
                            let _ = socket.send_rtp(&pkt).await;
                            sent.fetch_add(1, Ordering::Relaxed);
                        }
                        dtmf_left = Some((left - 1, start_ts));
                    } else {
                        if let Some(ends) = dtmf_gen.end_digit() {
                            for ev in ends {
                                let pkt = RtpPacket::build(
                                    te,
                                    seq,
                                    start_ts,
                                    ssrc,
                                    Bytes::from(ev.to_bytes()),
                                    false,
                                );
                                seq = seq.wrapping_add(1);
                                let _ = socket.send_rtp(&pkt).await;
                                sent.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        dtmf_left = None;
                    }
                    ts = ts.wrapping_add(FRAME as u32);
                    continue;
                }
                // Audio: the tone, or nothing.
                let frame: Option<Vec<i16>> = {
                    let mut p = playing.lock().unwrap();
                    match &mut *p {
                        Playing::Tone { hz, until, phase } if Instant::now() < *until => {
                            Some(tone_frame(*hz, phase))
                        }
                        Playing::Tone { .. } => {
                            *p = Playing::Nothing;
                            None
                        }
                        Playing::Nothing => None,
                    }
                };
                if let Some(samples) = frame {
                    let payload: Vec<u8> =
                        samples.iter().map(|&s| remote.codec.encode(s)).collect();
                    let pkt = RtpPacket::build(
                        remote.codec.payload_type(),
                        seq,
                        ts,
                        ssrc,
                        Bytes::from(payload),
                        first,
                    );
                    first = false;
                    seq = seq.wrapping_add(1);
                    if socket.send_rtp(&pkt).await.is_ok() {
                        sent.fetch_add(1, Ordering::Relaxed);
                    }
                }
                ts = ts.wrapping_add(FRAME as u32);
            }
        });
    }

    fn spawn_receiver(&self, mut stop: tokio::sync::watch::Receiver<bool>) {
        let socket = Arc::clone(&self.socket);
        let remote = Arc::clone(&self.remote);
        let watched = Arc::clone(&self.watched);
        let heard = Arc::clone(&self.heard);
        let received = Arc::clone(&self.packets_received);
        tokio::spawn(async move {
            let mut detector = Rfc2833Detector::new(8000);
            loop {
                let pkt = tokio::select! {
                    _ = stop.changed() => return,
                    r = socket.recv_rtp() => match r {
                        Ok((pkt, _)) => pkt,
                        Err(_) => return,
                    },
                };
                received.fetch_add(1, Ordering::Relaxed);
                let pt = pkt.header.payload_type();
                let te = remote
                    .read()
                    .await
                    .as_ref()
                    .and_then(|r| r.telephone_event_pt)
                    .unwrap_or(TELEPHONE_EVENT_PT);
                if pt == te {
                    if let Ok(events) =
                        detector.process_with_timestamp(&pkt.payload, pkt.header.timestamp)
                    {
                        for ev in events {
                            if ev.event_type == DtmfEventType::End {
                                let c = ev.digit.to_string().chars().next().unwrap_or('?');
                                heard.lock().unwrap().dtmf.push(c);
                            }
                        }
                    }
                    continue;
                }
                let Some(codec) = Codec::from_payload_type(pt) else {
                    continue;
                };
                let samples: Vec<i16> = pkt.payload.iter().map(|&b| codec.decode(b)).collect();
                let freqs: Vec<u32> = watched.lock().unwrap().iter().copied().collect();
                if freqs.is_empty() {
                    continue;
                }
                let energy: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
                let rms = (energy / samples.len().max(1) as f64).sqrt();
                let mut heard = heard.lock().unwrap();
                for hz in freqs {
                    let dominant = rms > 200.0 && goertzel_ratio(&samples, hz) > 0.5;
                    let run = heard.tone_runs.entry(hz).or_insert(0);
                    if dominant {
                        *run += 1;
                        let run = *run;
                        let best = heard.tone_best.entry(hz).or_insert(0);
                        *best = (*best).max(run);
                    } else {
                        *run = 0;
                    }
                }
            }
        });
    }
}

impl Drop for MediaLeg {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

/// A 20 ms frame of a sine at `hz` (8 kHz, 160 samples), continuing
/// from `phase`, which it advances: what the phone's sender plays.
pub fn tone_frame(hz: f32, phase: &mut f32) -> Vec<i16> {
    let step = 2.0 * std::f32::consts::PI * hz / 8000.0;
    (0..FRAME)
        .map(|_| {
            let s = (phase.sin() * 0.7 * i16::MAX as f32) as i16;
            *phase += step;
            if *phase > 2.0 * std::f32::consts::PI {
                *phase -= 2.0 * std::f32::consts::PI;
            }
            s
        })
        .collect()
}

/// Hears one frequency in a stream of 8 kHz frames, the way a
/// [`MediaLeg`] does: `heard` once the tone has dominated three
/// consecutive frames.
pub struct ToneDetector {
    hz: u32,
    run: u32,
    best: u32,
}

impl ToneDetector {
    pub fn new(hz: u32) -> Self {
        Self {
            hz,
            run: 0,
            best: 0,
        }
    }

    pub fn feed(&mut self, samples: &[i16]) {
        let energy: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let rms = (energy / samples.len().max(1) as f64).sqrt();
        if rms > 200.0 && goertzel_ratio(samples, self.hz) > 0.5 {
            self.run += 1;
            self.best = self.best.max(self.run);
        } else {
            self.run = 0;
        }
    }

    pub fn heard(&self) -> bool {
        self.best >= 3
    }
}

/// How much of a frame's energy sits at `hz`: about 1 for a pure tone
/// there, about 0 for anything else.
pub fn goertzel_ratio(samples: &[i16], hz: u32) -> f64 {
    let n = samples.len();
    if n == 0 {
        return 0.0;
    }
    let k = (n as f64 * hz as f64 / 8000.0).round();
    let w = 2.0 * std::f64::consts::PI * k / n as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    let energy: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    if energy <= 0.0 {
        return 0.0;
    }
    power / (energy * n as f64 / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                ((2.0 * std::f32::consts::PI * hz * i as f32 / 8000.0).sin() * 10_000.0) as i16
            })
            .collect()
    }

    #[test]
    fn goertzel_tells_a_tone_from_another() {
        let a = sine(440.0, 160);
        assert!(goertzel_ratio(&a, 440) > 0.8, "{}", goertzel_ratio(&a, 440));
        assert!(
            goertzel_ratio(&a, 1000) < 0.1,
            "{}",
            goertzel_ratio(&a, 1000)
        );
        assert_eq!(goertzel_ratio(&[0; 160], 440), 0.0);
    }

    #[test]
    fn an_answer_is_read_for_its_audio() {
        let sdp = "v=0\r\no=- 1 1 IN IP4 10.0.0.9\r\ns=-\r\nc=IN IP4 10.0.0.9\r\nt=0 0\r\nm=audio 4000 RTP/AVP 8 0 96\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:96 telephone-event/8000\r\na=sendrecv\r\n";
        let r = parse_remote_audio(sdp).unwrap();
        assert_eq!(r.addr, "10.0.0.9:4000".parse().unwrap());
        assert_eq!(r.codec, Codec::Pcma, "the first format wins");
        assert_eq!(r.telephone_event_pt, Some(96));
        assert!(r.sending);
        let held = sdp.replace("a=sendrecv", "a=recvonly");
        assert!(!parse_remote_audio(&held).unwrap().sending);
        assert!(parse_remote_audio(&sdp.replace("m=audio 4000", "m=audio 0")).is_err());
    }

    #[tokio::test]
    async fn two_legs_hear_each_other_and_pass_a_digit() {
        let a = MediaLeg::open().await.unwrap();
        let b = MediaLeg::open().await.unwrap();
        a.set_remote(&b.sdp("sendrecv")).await.unwrap();
        b.set_remote(&a.sdp("sendrecv")).await.unwrap();
        a.play_tone(440.0, 400);
        assert!(b.heard(440, Duration::from_secs(3)).await);
        assert!(
            !a.heard(440, Duration::from_millis(200)).await,
            "a hears nothing"
        );
        b.send_dtmf('7');
        assert_eq!(a.heard_dtmf(Duration::from_secs(3)).await, Some('7'));
        assert!(a.packets_received() > 0 && b.packets_received() > 0);
    }
}
