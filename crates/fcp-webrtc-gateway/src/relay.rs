//! The media relay between a browser's peer connection and a SIP leg.
//!
//! Both legs speak the same audio codec (the gateway offers the SIP side
//! only what the browser negotiated), so audio is re-tagged with the far
//! side's payload type and forwarded, telephone-event included. Video is
//! forwarded packet for packet: the coded payload, sequence number,
//! timestamp and marker are untouched, the payload type is mapped, and the
//! SSRC is the browser's towards the SIP side and the peer connection's
//! own towards the browser (the transport stamps it). A shared screen is
//! a second video section on both legs (§15.6), relayed the same way on
//! its own sockets and SSRC pair. RTCP feedback about video — NACK, PLI,
//! FIR, REMB — crosses in both directions with its SSRCs rewritten to
//! what each side knows, each section's to its own; reports (SR/RR) are
//! each leg's own business and are not relayed.
//!
//! The SIP side is plain RTP or SDES-SRTP per section; the browser side's
//! DTLS-SRTP is inside the peer connection.
//!
//! The SIP leg can be **swapped** while the relay runs: when the
//! conference node the session is on drains, the session dials another
//! node and hands the relay the new leg over a channel
//! (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5c). The relay takes it
//! between packets; the browser side is untouched.

use bytes::{Bytes, BytesMut};
use forge_rtp::rtcp::{PayloadFeedback, PsFeedback, RtpFeedback, TransportFeedback};
use forge_rtp::{RtcpPacket, RtpPacket, RtpSocketPair, SrtpContext};
use forge_webrtc::{AudioSender, PeerEvent, VideoSender};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, Notify};
use tracing::{debug, trace, warn};

use crate::forwarding::Forwarding;

/// The SIP side's audio section.
pub struct SipAudioLeg {
    pub socket: Arc<RtpSocketPair>,
    /// SDES SRTP for the section, `None` for plain RTP.
    pub srtp: Option<Arc<Mutex<SrtpContext>>>,
    /// The audio payload type on the SIP side.
    pub payload_type: u8,
    /// telephone-event on the SIP side, if negotiated.
    pub telephone_event_pt: Option<u8>,
}

/// The SIP side's video section.
pub struct SipVideoLeg {
    pub socket: Arc<RtpSocketPair>,
    pub srtp: Option<Arc<Mutex<SrtpContext>>>,
    /// The video payload type on the SIP side.
    pub payload_type: u8,
}

/// The SIP leg.
pub struct SipLeg {
    pub audio: SipAudioLeg,
    pub video: Option<SipVideoLeg>,
    /// The shared screen's section, when negotiated.
    pub content: Option<SipVideoLeg>,
}

/// The browser leg: the peer connection's event stream and senders.
pub struct BrowserLeg {
    pub events: mpsc::Receiver<PeerEvent>,
    pub audio: AudioSender,
    /// The audio payload type on the browser side.
    pub audio_payload_type: u8,
    /// telephone-event on the browser side, if negotiated.
    pub telephone_event_pt: Option<u8>,
    /// The video sender and the browser side's video payload type.
    pub video: Option<(VideoSender, u8)>,
    /// The shared screen's sender and payload type on the browser side.
    pub content: Option<(VideoSender, u8)>,
    /// Where the room forwards each person's stream: a sender (and the
    /// browser side's payload type) per send-only section the browser
    /// took, `None` where it rejected one, and the map of the room's SSRCs
    /// onto them. Empty: the room's one picture goes out on `video`.
    pub streams: Vec<Option<(VideoSender, u8)>>,
    pub forwarding: Option<Arc<std::sync::Mutex<Forwarding>>>,
    /// How long the browser may send nothing at all — no RTP, no RTCP —
    /// before it is taken as gone. A live browser reports every second or
    /// so even with its tracks muted; the peer connection itself does not
    /// enforce consent freshness, so a browser that vanished would
    /// otherwise hold its session open for good.
    pub inactivity_timeout: Duration,
}

/// Packet counters for a relay, both directions.
#[derive(Default)]
pub struct RelayStats {
    pub audio_to_sip: AtomicU64,
    pub audio_to_browser: AtomicU64,
    pub video_to_sip: AtomicU64,
    pub video_to_browser: AtomicU64,
    pub content_to_sip: AtomicU64,
    pub content_to_browser: AtomicU64,
    /// A forwarding room's streams, each on its section.
    pub streams_to_browser: AtomicU64,
    pub feedback_to_sip: AtomicU64,
    pub feedback_to_browser: AtomicU64,
    pub dropped: AtomicU64,
    pub srtp_errors: AtomicU64,
    /// When the browser last sent a screen packet, in milliseconds since
    /// the Unix epoch; 0 before the first.
    pub content_last_from_browser_ms: AtomicU64,
}

/// A point-in-time copy of [`RelayStats`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RelayStatsSnapshot {
    pub audio_to_sip: u64,
    pub audio_to_browser: u64,
    pub video_to_sip: u64,
    pub video_to_browser: u64,
    pub content_to_sip: u64,
    pub content_to_browser: u64,
    pub streams_to_browser: u64,
    pub feedback_to_sip: u64,
    pub feedback_to_browser: u64,
    pub dropped: u64,
    pub srtp_errors: u64,
}

impl RelayStats {
    pub fn snapshot(&self) -> RelayStatsSnapshot {
        RelayStatsSnapshot {
            audio_to_sip: self.audio_to_sip.load(Ordering::Relaxed),
            audio_to_browser: self.audio_to_browser.load(Ordering::Relaxed),
            video_to_sip: self.video_to_sip.load(Ordering::Relaxed),
            video_to_browser: self.video_to_browser.load(Ordering::Relaxed),
            content_to_sip: self.content_to_sip.load(Ordering::Relaxed),
            content_to_browser: self.content_to_browser.load(Ordering::Relaxed),
            streams_to_browser: self.streams_to_browser.load(Ordering::Relaxed),
            feedback_to_sip: self.feedback_to_sip.load(Ordering::Relaxed),
            feedback_to_browser: self.feedback_to_browser.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            srtp_errors: self.srtp_errors.load(Ordering::Relaxed),
        }
    }

    /// Whether the browser sent a screen packet within `window`.
    pub fn content_sent_within(&self, window: Duration) -> bool {
        let last = self.content_last_from_browser_ms.load(Ordering::Relaxed);
        last != 0 && unix_ms().saturating_sub(last) <= window.as_millis() as u64
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Why a relay stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEnd {
    /// Told to stop.
    Stopped,
    /// The browser's connection closed or its event stream ended.
    BrowserClosed,
    /// The browser's transport failed, or it went silent for longer than
    /// [`BrowserLeg::inactivity_timeout`].
    BrowserFailed(String),
    /// A SIP-side socket failed.
    SipSocketError(String),
}

/// The far side's payload type for one of ours: audio stays audio,
/// telephone-event stays telephone-event, anything else is dropped.
pub fn map_payload_type(
    pt: u8,
    from_audio: u8,
    from_te: Option<u8>,
    to_audio: u8,
    to_te: Option<u8>,
) -> Option<u8> {
    if pt == from_audio {
        Some(to_audio)
    } else if from_te == Some(pt) {
        to_te
    } else {
        None
    }
}

/// `packet` with its payload type replaced (the marker bit kept).
pub fn retag(packet: &[u8], payload_type: u8) -> BytesMut {
    let mut out = BytesMut::from(packet);
    if out.len() >= 2 {
        out[1] = (out[1] & 0x80) | (payload_type & 0x7f);
    }
    out
}

/// A feedback packet re-addressed for the other leg: `sender` becomes the
/// SSRC the other side knows the reporter by, `media` the SSRC it sends
/// the stream with. Reports, SDES and BYE yield `None`: each leg has its
/// own.
pub fn rewrite_feedback(packet: &RtcpPacket, sender: u32, media: u32) -> Option<RtcpPacket> {
    match packet {
        RtcpPacket::TransportFeedback(fb) => match &fb.kind {
            TransportFeedback::Nack(entries) => Some(RtcpPacket::TransportFeedback(RtpFeedback {
                sender_ssrc: sender,
                media_ssrc: media,
                kind: TransportFeedback::Nack(entries.clone()),
            })),
            TransportFeedback::Other { .. } => None,
        },
        RtcpPacket::PayloadFeedback(fb) => {
            let kind = match &fb.kind {
                PayloadFeedback::Pli => PayloadFeedback::Pli,
                PayloadFeedback::Fir(entries) => PayloadFeedback::Fir(
                    entries
                        .iter()
                        .map(|e| forge_rtp::rtcp::FirEntry {
                            ssrc: media,
                            seq: e.seq,
                        })
                        .collect(),
                ),
                PayloadFeedback::Remb { bitrate_bps, .. } => PayloadFeedback::Remb {
                    bitrate_bps: *bitrate_bps,
                    ssrcs: vec![media],
                },
                PayloadFeedback::Other { .. } => return None,
            };
            Some(RtcpPacket::PayloadFeedback(PsFeedback {
                sender_ssrc: sender,
                media_ssrc: media,
                kind,
            }))
        }
        _ => None,
    }
}

/// The SSRC feedback from the SIP side is about, as the browser knows
/// it: the one it names when that is one of the browser's own streams (a
/// simulcast layer), else `fallback`, the stream the browser sends.
pub fn feedback_target(packet: &RtcpPacket, fallback: u32, streams: &[u32]) -> u32 {
    packet
        .feedback_media_ssrc()
        .filter(|ssrc| streams.contains(ssrc))
        .unwrap_or(fallback)
}

/// The most SSRCs a browser's camera is expected to send with: a stream
/// per simulcast layer, and one more after a restart.
const MAX_CAMERA_STREAMS: usize = 8;

async fn protect(
    srtp: &Option<Arc<Mutex<SrtpContext>>>,
    plain: &[u8],
    rtcp: bool,
    stats: &RelayStats,
) -> Option<Vec<u8>> {
    match srtp {
        Some(ctx) => {
            let result = {
                let mut guard = ctx.lock().await;
                if rtcp {
                    guard.protect_rtcp(plain)
                } else {
                    guard.protect_rtp(plain)
                }
            };
            match result {
                Ok(bytes) => Some(bytes),
                Err(e) => {
                    trace!(error = %e, rtcp, "SIP-side SRTP protect failed");
                    stats.srtp_errors.fetch_add(1, Ordering::Relaxed);
                    None
                }
            }
        }
        None => Some(plain.to_vec()),
    }
}

async fn unprotect(
    srtp: &Option<Arc<Mutex<SrtpContext>>>,
    raw: &Bytes,
    rtcp: bool,
    stats: &RelayStats,
) -> Option<Bytes> {
    match srtp {
        Some(ctx) => {
            let result = {
                let mut guard = ctx.lock().await;
                if rtcp {
                    guard.unprotect_rtcp(raw)
                } else {
                    guard.unprotect_rtp(raw)
                }
            };
            match result {
                Ok(bytes) => Some(Bytes::from(bytes)),
                Err(e) => {
                    trace!(error = %e, rtcp, "SIP-side SRTP unprotect failed");
                    stats.srtp_errors.fetch_add(1, Ordering::Relaxed);
                    None
                }
            }
        }
        None => Some(raw.clone()),
    }
}

fn ssrc_of(packet: &[u8]) -> Option<u32> {
    (packet.len() >= 12 && packet[0] >> 6 == 2)
        .then(|| u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]))
}

/// Run the relay until one side ends it or `stop` is signalled. A leg
/// arriving on `swap` replaces the SIP side from then on.
pub async fn run_relay(
    mut browser: BrowserLeg,
    mut sip: SipLeg,
    stats: Arc<RelayStats>,
    stop: Arc<Notify>,
    swap: mpsc::Receiver<SipLeg>,
) -> RelayEnd {
    let mut swap = Some(swap);
    // What each side sends each video section with, learned from the
    // first packet each way, so feedback can be re-addressed.
    let mut browser_video_ssrc: Option<u32> = None;
    // Every SSRC the browser's camera has sent with: one, or a stream per
    // simulcast layer, which reach the SIP side under their own SSRCs.
    let mut browser_video_ssrcs: Vec<u32> = Vec::new();
    let mut sip_video_ssrc: Option<u32> = None;
    let our_video_ssrc = browser.video.as_ref().map(|(v, _)| v.ssrc());
    let mut browser_content_ssrc: Option<u32> = None;
    let mut sip_content_ssrc: Option<u32> = None;
    let our_content_ssrc = browser.content.as_ref().map(|(v, _)| v.ssrc());
    // Where the room forwards: our SSRC on each section, so feedback about
    // one reaches the room's stream on it.
    let stream_ssrcs: Vec<Option<u32>> = browser
        .streams
        .iter()
        .map(|s| s.as_ref().map(|(v, _)| v.ssrc()))
        .collect();
    let forwarding = browser
        .forwarding
        .clone()
        .filter(|_| !browser.streams.is_empty());
    let mut last_browser_activity = Instant::now();
    let mut liveness = tokio::time::interval(Duration::from_secs(1));
    liveness.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = stop.notified() => return RelayEnd::Stopped,

            new_leg = next_swap(&mut swap) => {
                // The far end's SSRCs start over with the new leg.
                sip = new_leg;
                sip_video_ssrc = None;
                sip_content_ssrc = None;
                if let Some(f) = &forwarding {
                    f.lock().unwrap_or_else(|e| e.into_inner()).clear();
                }
                debug!("SIP leg swapped; relaying to the new far end");
            }

            _ = liveness.tick() => {
                if let Some(f) = &forwarding {
                    f.lock().unwrap_or_else(|e| e.into_inner()).expire(Instant::now());
                }
                if last_browser_activity.elapsed() > browser.inactivity_timeout {
                    return RelayEnd::BrowserFailed(format!(
                        "no media or RTCP from the browser for {} s",
                        browser.inactivity_timeout.as_secs()
                    ));
                }
            }

            event = browser.events.recv() => {
                let Some(event) = event else { return RelayEnd::BrowserClosed };
                if matches!(event, PeerEvent::Rtp(_) | PeerEvent::VideoRtp(_) | PeerEvent::ContentRtp(_) | PeerEvent::StreamRtp(..) | PeerEvent::Rtcp(_)) {
                    last_browser_activity = Instant::now();
                }
                match event {
                    PeerEvent::Rtp(pkt) => {
                        let Some(pt) = map_payload_type(
                            pkt.header.payload_type(),
                            browser.audio_payload_type,
                            browser.telephone_event_pt,
                            sip.audio.payload_type,
                            sip.audio.telephone_event_pt,
                        ) else {
                            stats.dropped.fetch_add(1, Ordering::Relaxed);
                            continue;
                        };
                        let plain = retag(&pkt.to_bytes(), pt);
                        if let Some(out) = protect(&sip.audio.srtp, &plain, false, &stats).await {
                            match sip.audio.socket.send_rtp_raw(&out).await {
                                Ok(_) => { stats.audio_to_sip.fetch_add(1, Ordering::Relaxed); }
                                Err(e) => trace!(error = %e, "audio send to SIP failed"),
                            }
                        }
                    }
                    PeerEvent::VideoRtp(pkt) => {
                        let Some(video) = &sip.video else {
                            stats.dropped.fetch_add(1, Ordering::Relaxed);
                            continue;
                        };
                        browser_video_ssrc = Some(pkt.header.ssrc);
                        let ssrc = pkt.header.ssrc;
                        if !browser_video_ssrcs.contains(&ssrc)
                            && browser_video_ssrcs.len() < MAX_CAMERA_STREAMS
                        {
                            browser_video_ssrcs.push(ssrc);
                        }
                        video_to_sip(video, &pkt, &stats, &stats.video_to_sip, "video").await;
                    }
                    PeerEvent::ContentRtp(pkt) => {
                        let Some(content) = &sip.content else {
                            stats.dropped.fetch_add(1, Ordering::Relaxed);
                            continue;
                        };
                        browser_content_ssrc = Some(pkt.header.ssrc);
                        stats.content_last_from_browser_ms.store(unix_ms(), Ordering::Relaxed);
                        video_to_sip(content, &pkt, &stats, &stats.content_to_sip, "content").await;
                    }
                    // The browser's sections for a forwarding room only
                    // receive: nothing arrives on them.
                    PeerEvent::StreamRtp(..) => {}
                    PeerEvent::Rtcp(packets) => {
                        // Feedback about what we send the browser on each
                        // section goes to that section's far end; a REMB,
                        // which is about the whole transport, to the camera's.
                        if let (Some(video), Some(f)) = (&sip.video, &forwarding) {
                            // A forwarding room: feedback about the stream
                            // on one of its sections is about the room's
                            // SSRC there; a REMB is the viewer's budget.
                            let sender = browser_video_ssrc.unwrap_or(0);
                            let rewritten: Vec<RtcpPacket> = {
                                let map = f.lock().unwrap_or_else(|e| e.into_inner());
                                packets
                                    .iter()
                                    .filter_map(|p| {
                                        let room = p
                                            .feedback_media_ssrc()
                                            .and_then(|ours| stream_ssrcs.iter().position(|s| *s == Some(ours)))
                                            .and_then(|i| map.ssrc_on(i));
                                        match room {
                                            Some(media) => rewrite_feedback(p, sender, media),
                                            None if is_remb(p) => rewrite_feedback(p, sender, 0),
                                            None => None,
                                        }
                                    })
                                    .collect()
                            };
                            feedback_to_sip(video, &rewritten, &stats).await;
                        } else if let (Some(video), Some(media)) = (&sip.video, sip_video_ssrc) {
                            let sender = browser_video_ssrc.unwrap_or(0);
                            let rewritten: Vec<RtcpPacket> = packets
                                .iter()
                                .filter(|p| our_video_ssrc.is_none() || p.feedback_media_ssrc() == our_video_ssrc || is_remb(p))
                                .filter_map(|p| rewrite_feedback(p, sender, media))
                                .collect();
                            feedback_to_sip(video, &rewritten, &stats).await;
                        }
                        if let (Some(content), Some(media), Some(ours)) = (&sip.content, sip_content_ssrc, our_content_ssrc) {
                            let sender = browser_content_ssrc.unwrap_or(0);
                            let rewritten: Vec<RtcpPacket> = packets
                                .iter()
                                .filter(|p| p.feedback_media_ssrc() == Some(ours) && !is_remb(p))
                                .filter_map(|p| rewrite_feedback(p, sender, media))
                                .collect();
                            feedback_to_sip(content, &rewritten, &stats).await;
                        }
                    }
                    PeerEvent::Failed(why) => return RelayEnd::BrowserFailed(why),
                    PeerEvent::Closed => return RelayEnd::BrowserClosed,
                    PeerEvent::LocalCandidate(_)
                    | PeerEvent::GatheringComplete
                    | PeerEvent::IceConnected { .. }
                    | PeerEvent::Connected => {}
                }
            }

            received = sip.audio.socket.recv_rtp_raw() => {
                let (raw, _) = match received {
                    Ok(r) => r,
                    Err(e) => return RelayEnd::SipSocketError(format!("audio: {e}")),
                };
                let Some(plain) = unprotect(&sip.audio.srtp, &raw, false, &stats).await else { continue };
                let Ok(pkt) = RtpPacket::parse(plain) else {
                    stats.dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let Some(pt) = map_payload_type(
                    pkt.header.payload_type(),
                    sip.audio.payload_type,
                    sip.audio.telephone_event_pt,
                    browser.audio_payload_type,
                    browser.telephone_event_pt,
                ) else {
                    stats.dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let marker = pkt.header.marker();
                let timestamp = pkt.header.timestamp;
                match browser.audio.send_rtp(pt, marker, timestamp, pkt.payload).await {
                    Ok(()) => { stats.audio_to_browser.fetch_add(1, Ordering::Relaxed); }
                    Err(e) => trace!(error = %e, "audio send to browser failed"),
                }
            }

            received = recv_rtp_on(&sip.video) => {
                let raw = match received {
                    Ok(r) => r,
                    Err(e) => return RelayEnd::SipSocketError(format!("video: {e}")),
                };
                if let (Some(video), Some(f)) = (&sip.video, &forwarding) {
                    stream_to_browser(video, &browser.streams, f, raw, &stats).await;
                    continue;
                }
                let (Some(video), Some((sender, browser_pt))) = (&sip.video, &browser.video) else { continue };
                if let Some(ssrc) = video_to_browser(video, sender, *browser_pt, raw, &stats, &stats.video_to_browser, "video").await {
                    sip_video_ssrc = Some(ssrc);
                }
            }

            received = recv_rtcp_on(&sip.video) => {
                let raw = match received {
                    Ok(r) => r,
                    Err(e) => return RelayEnd::SipSocketError(format!("video rtcp: {e}")),
                };
                let (Some(video), Some((sender, _)), Some(media), Some(ours)) =
                    (&sip.video, &browser.video, browser_video_ssrc, our_video_ssrc) else { continue };
                feedback_to_browser(video, sender, ours, media, &browser_video_ssrcs, raw, &stats).await;
            }

            received = recv_rtp_on(&sip.content) => {
                let raw = match received {
                    Ok(r) => r,
                    Err(e) => return RelayEnd::SipSocketError(format!("content: {e}")),
                };
                let (Some(content), Some((sender, browser_pt))) = (&sip.content, &browser.content) else { continue };
                if let Some(ssrc) = video_to_browser(content, sender, *browser_pt, raw, &stats, &stats.content_to_browser, "content").await {
                    sip_content_ssrc = Some(ssrc);
                }
            }

            received = recv_rtcp_on(&sip.content) => {
                let raw = match received {
                    Ok(r) => r,
                    Err(e) => return RelayEnd::SipSocketError(format!("content rtcp: {e}")),
                };
                let (Some(content), Some((sender, _)), Some(media), Some(ours)) =
                    (&sip.content, &browser.content, browser_content_ssrc, our_content_ssrc) else { continue };
                feedback_to_browser(content, sender, ours, media, &[], raw, &stats).await;
            }
        }
    }
}

/// The next SIP leg to swap in; never, once the sender is gone (the
/// session keeps the leg it has).
async fn next_swap(swap: &mut Option<mpsc::Receiver<SipLeg>>) -> SipLeg {
    if let Some(rx) = swap {
        if let Some(leg) = rx.recv().await {
            return leg;
        }
        *swap = None;
    }
    std::future::pending().await
}

fn is_remb(p: &RtcpPacket) -> bool {
    matches!(
        p,
        RtcpPacket::PayloadFeedback(PsFeedback {
            kind: PayloadFeedback::Remb { .. },
            ..
        })
    )
}

/// RTP on a video section's socket, or never when there is no section.
async fn recv_rtp_on(leg: &Option<SipVideoLeg>) -> forge_core::Result<Bytes> {
    match leg {
        Some(v) => v.socket.recv_rtp_raw().await.map(|(raw, _)| raw),
        None => std::future::pending().await,
    }
}

/// RTCP on a video section's socket, or never when there is no section.
async fn recv_rtcp_on(leg: &Option<SipVideoLeg>) -> forge_core::Result<Bytes> {
    match leg {
        Some(v) => v.socket.recv_rtcp().await.map(|(raw, _)| raw),
        None => std::future::pending().await,
    }
}

/// A browser's video packet onto a SIP section: re-tagged, protected.
async fn video_to_sip(
    leg: &SipVideoLeg,
    pkt: &RtpPacket,
    stats: &RelayStats,
    counter: &AtomicU64,
    what: &str,
) {
    let plain = retag(&pkt.to_bytes(), leg.payload_type);
    if let Some(out) = protect(&leg.srtp, &plain, false, stats).await {
        match leg.socket.send_rtp_raw(&out).await {
            Ok(_) => {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => trace!(error = %e, "{what} send to SIP failed"),
        }
    }
}

/// A SIP section's packet to the browser: unprotected, re-tagged, sent
/// under the peer connection's SSRC for the section. Returns the SIP
/// side's SSRC when the packet was forwarded.
async fn video_to_browser(
    leg: &SipVideoLeg,
    sender: &VideoSender,
    browser_pt: u8,
    raw: Bytes,
    stats: &RelayStats,
    counter: &AtomicU64,
    what: &str,
) -> Option<u32> {
    let plain = unprotect(&leg.srtp, &raw, false, stats).await?;
    if plain.len() < 12 || (plain[1] & 0x7f) != leg.payload_type {
        stats.dropped.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let ssrc = ssrc_of(&plain);
    let out = retag(&plain, browser_pt).freeze();
    match sender.send_packet(out).await {
        Ok(()) => {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => trace!(error = %e, "{what} send to browser failed"),
    }
    ssrc
}

/// A forwarding room's packet to the browser: on the section its SSRC
/// holds, or takes when it is new; dropped when every section is taken.
async fn stream_to_browser(
    leg: &SipVideoLeg,
    senders: &[Option<(VideoSender, u8)>],
    forwarding: &std::sync::Mutex<Forwarding>,
    raw: Bytes,
    stats: &RelayStats,
) {
    let Some(plain) = unprotect(&leg.srtp, &raw, false, stats).await else {
        return;
    };
    let ssrc = match ssrc_of(&plain) {
        Some(ssrc) if plain.len() >= 12 && (plain[1] & 0x7f) == leg.payload_type => ssrc,
        _ => {
            stats.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let slot = forwarding
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .slot_for(ssrc, Instant::now());
    let Some((sender, browser_pt)) = slot.and_then(|i| senders.get(i)?.as_ref()) else {
        stats.dropped.fetch_add(1, Ordering::Relaxed);
        return;
    };
    match sender
        .send_packet(retag(&plain, *browser_pt).freeze())
        .await
    {
        Ok(()) => {
            stats.streams_to_browser.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => trace!(error = %e, "stream send to browser failed"),
    }
}

/// Re-addressed feedback onto a SIP section, as one compound packet.
async fn feedback_to_sip(leg: &SipVideoLeg, rewritten: &[RtcpPacket], stats: &RelayStats) {
    if rewritten.is_empty() {
        return;
    }
    let mut plain = Vec::with_capacity(64);
    for p in rewritten {
        plain.extend_from_slice(&p.to_bytes());
    }
    if let Some(out) = protect(&leg.srtp, &plain, true, stats).await {
        match leg.socket.send_rtcp(&out).await {
            Ok(_) => {
                stats
                    .feedback_to_sip
                    .fetch_add(rewritten.len() as u64, Ordering::Relaxed);
            }
            Err(e) => trace!(error = %e, "feedback send to SIP failed"),
        }
    }
}

/// A SIP section's feedback to the browser, re-addressed: from `ours`
/// (the SSRC the browser knows the section by) about `media` (what the
/// browser sends it with), unless it names one of `streams` — a
/// simulcast layer the browser sends under its own SSRC, which the far
/// side knows it by, and which is what it means.
#[allow(clippy::too_many_arguments)]
async fn feedback_to_browser(
    leg: &SipVideoLeg,
    sender: &VideoSender,
    ours: u32,
    media: u32,
    streams: &[u32],
    raw: Bytes,
    stats: &RelayStats,
) {
    let Some(plain) = unprotect(&leg.srtp, &raw, true, stats).await else {
        return;
    };
    let rewritten: Vec<RtcpPacket> = RtcpPacket::parse_compound(&plain)
        .iter()
        .filter_map(|p| rewrite_feedback(p, ours, feedback_target(p, media, streams)))
        .collect();
    if rewritten.is_empty() {
        return;
    }
    match sender.send_rtcp(&rewritten).await {
        Ok(()) => {
            stats
                .feedback_to_browser
                .fetch_add(rewritten.len() as u64, Ordering::Relaxed);
        }
        Err(e) => trace!(error = %e, "feedback send to browser failed"),
    }
}

/// Log a relay's end at the level it deserves.
pub fn log_relay_end(session_id: &str, end: &RelayEnd) {
    match end {
        RelayEnd::Stopped | RelayEnd::BrowserClosed => {
            debug!(session = %session_id, ?end, "relay ended")
        }
        RelayEnd::BrowserFailed(_) | RelayEnd::SipSocketError(_) => {
            warn!(session = %session_id, ?end, "relay ended")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_types_map_audio_and_telephone_event_only() {
        assert_eq!(map_payload_type(9, 9, Some(110), 9, Some(101)), Some(9));
        assert_eq!(map_payload_type(110, 9, Some(110), 9, Some(101)), Some(101));
        // The far side without telephone-event drops it.
        assert_eq!(map_payload_type(110, 9, Some(110), 9, None), None);
        // Comfort noise and anything else are dropped.
        assert_eq!(map_payload_type(13, 9, Some(110), 9, Some(101)), None);
    }

    #[test]
    fn retag_keeps_the_marker_bit() {
        let pkt = RtpPacket::build(96, 7, 900, 0x1234, Bytes::from_static(&[1, 2, 3]), true);
        let out = retag(&pkt.to_bytes(), 120);
        let back = RtpPacket::parse(out.freeze()).unwrap();
        assert_eq!(back.header.payload_type(), 120);
        assert!(back.header.marker());
        assert_eq!({ back.header.sequence_number }, 7);
        assert_eq!({ back.header.ssrc }, 0x1234);
    }

    #[test]
    fn feedback_about_a_simulcast_layer_keeps_its_ssrc() {
        // The browser sends layers under 10, 11 and 12; the room asks for a
        // keyframe on 11, the one it is switching to.
        let streams = [10, 11, 12];
        let pli = RtcpPacket::PayloadFeedback(PsFeedback::pli(99, 11));
        assert_eq!(feedback_target(&pli, 12, &streams), 11);
        match rewrite_feedback(&pli, 7, feedback_target(&pli, 12, &streams)) {
            Some(RtcpPacket::PayloadFeedback(fb)) => {
                assert_eq!((fb.sender_ssrc, fb.media_ssrc), (7, 11))
            }
            other => panic!("{other:?}"),
        }
        // One the browser never sent with goes to its latest stream, as
        // before there were layers.
        let stranger = RtcpPacket::PayloadFeedback(PsFeedback::pli(99, 55));
        assert_eq!(feedback_target(&stranger, 12, &streams), 12);
        assert_eq!(feedback_target(&pli, 12, &[]), 12);
    }

    #[test]
    fn feedback_is_readdressed_and_reports_are_not() {
        let nack = RtcpPacket::TransportFeedback(RtpFeedback::nack(1, 2, &[10, 11]));
        match rewrite_feedback(&nack, 100, 200).unwrap() {
            RtcpPacket::TransportFeedback(fb) => {
                assert_eq!((fb.sender_ssrc, fb.media_ssrc), (100, 200));
                assert!(
                    matches!(fb.kind, TransportFeedback::Nack(ref e) if e[0].lost() == vec![10, 11])
                );
            }
            other => panic!("{other:?}"),
        }
        let fir = RtcpPacket::PayloadFeedback(PsFeedback::fir(1, 2, 5));
        match rewrite_feedback(&fir, 100, 200).unwrap() {
            RtcpPacket::PayloadFeedback(fb) => {
                assert_eq!(fb.media_ssrc, 200);
                assert!(
                    matches!(fb.kind, PayloadFeedback::Fir(ref e) if e[0].ssrc == 200 && e[0].seq == 5)
                );
            }
            other => panic!("{other:?}"),
        }
        let remb = RtcpPacket::PayloadFeedback(PsFeedback::remb(1, 800_000, vec![2]));
        match rewrite_feedback(&remb, 100, 200).unwrap() {
            RtcpPacket::PayloadFeedback(fb) => {
                assert!(
                    matches!(fb.kind, PayloadFeedback::Remb { bitrate_bps: 800_000, ref ssrcs } if *ssrcs == vec![200])
                );
            }
            other => panic!("{other:?}"),
        }
        let rr = RtcpPacket::ReceiverReport(forge_rtp::ReceiverReport::new(1));
        assert!(rewrite_feedback(&rr, 100, 200).is_none());
    }
}
