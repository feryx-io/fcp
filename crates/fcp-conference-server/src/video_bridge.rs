//! A participant's video leg (design §5.1, §5.3, §7, §12).
//!
//! The conference server owns the sockets and SRTP; forge-conference's
//! [`VideoRoom`] owns the codecs. This module is the glue: it decides
//! what a meeting lets a caller negotiate, attaches a caller to the room
//! (source and subscription), and runs the bridge task that moves RTP
//! and RTCP between the caller's second socket pair and the room.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use fcp_core::{Meeting, TenantId};
use fcp_events::{ConferenceEvent, EventPublisher, PlatformEvent};
use fcp_media::{video_codec_to_forge, NegotiatedVideo, SrtpSdpContext};
use forge_conference::video::{
    SubscribeRequest, VideoBackend, VideoRoom, VideoRoomEvent, VideoRoomSettings, VideoSubscription,
};
use forge_conference::ConferenceRoom;
use forge_rtp::rtcp::{PsFeedback, RtcpPacket};
use forge_rtp::{PortPair, RtpPacket, RtpSocketPair, SrtpContext};
use forge_video::frame::Resolution;
use forge_video::layout::Layout;
use tokio::sync::RwLock;
use tracing::{debug, info, info_span, trace, warn, Instrument};

/// Socket-level counters for a video leg.
#[derive(Debug, Default)]
pub struct VideoLegStats {
    pub packets_received: AtomicU64,
    pub packets_sent: AtomicU64,
    pub rtcp_received: AtomicU64,
    pub rtcp_sent: AtomicU64,
    pub srtp_errors: AtomicU64,
    pub last_rx_unix_ms: AtomicU64,
}

/// What a participant's video leg is made of, kept on the session for
/// teardown, re-INVITEs and the API.
pub struct VideoLeg {
    pub port_pair: PortPair,
    pub socket: Arc<RtpSocketPair>,
    pub srtp_context: Option<Arc<RwLock<SrtpContext>>>,
    /// Our SDES key for the answer, when the leg is SRTP.
    pub srtp_sdp: Option<SrtpSdpContext>,
    pub negotiated: NegotiatedVideo,
    pub settings: VideoRoomSettings,
    pub stats: Arc<VideoLegStats>,
    /// The bridge task, when it runs on its own. A participant admitted
    /// after waiting for a moderator runs it joined with the audio
    /// bridge, whose handle covers both.
    pub bridge_handle: Option<tokio::task::JoinHandle<()>>,
}

impl VideoLeg {
    /// The same leg without a bridge handle: what a task that runs the
    /// bridge itself needs.
    pub fn shared(&self) -> VideoLeg {
        VideoLeg {
            port_pair: self.port_pair,
            socket: Arc::clone(&self.socket),
            srtp_context: self.srtp_context.clone(),
            srtp_sdp: self.srtp_sdp.clone(),
            negotiated: self.negotiated.clone(),
            settings: self.settings.clone(),
            stats: Arc::clone(&self.stats),
            bridge_handle: None,
        }
    }
}

/// A participant's video for the API (design §12): `{codec, resolution,
/// fps, bitrate_kbps, keyframes_sent, nacks, plis, state}` plus the leg's
/// socket counters. `{"state": "off"}` when no video was negotiated.
pub fn participant_video_json(
    room: &ConferenceRoom,
    participant_id: &str,
    leg: Option<&VideoLeg>,
) -> serde_json::Value {
    let Some(leg) = leg else {
        return serde_json::json!({ "state": "off" });
    };
    let info = room.video().and_then(|v| v.participant(participant_id));
    let mut json = serde_json::json!({
        "state": info.as_ref().map(|i| i.state.name()).unwrap_or("off"),
        "codec": leg.negotiated.codec.sdp_name(),
        "payload_type": leg.negotiated.payload_type,
        "srtp": leg.srtp_context.is_some(),
        "rtp_port": leg.port_pair.rtp_port,
        "packets_received": leg.stats.packets_received.load(Ordering::Relaxed),
        "packets_sent": leg.stats.packets_sent.load(Ordering::Relaxed),
        "rtcp_received": leg.stats.rtcp_received.load(Ordering::Relaxed),
        "rtcp_sent": leg.stats.rtcp_sent.load(Ordering::Relaxed),
        "srtp_errors": leg.stats.srtp_errors.load(Ordering::Relaxed),
        "is_inactive": leg.stats.last_rx_age().map(|a| a > VIDEO_INACTIVE_AFTER).unwrap_or(true),
    });
    if let Some(info) = info {
        if let Some(src) = info.source {
            json["resolution"] = serde_json::json!(src.resolution.to_string());
            json["fps"] = serde_json::json!(src.fps);
            json["bitrate_kbps"] = serde_json::json!(src.bitrate_kbps);
            json["frames_decoded"] = serde_json::json!(src.frames_decoded);
            json["frames_lost"] = serde_json::json!(src.frames_lost);
            json["frames_dropped"] = serde_json::json!(src.frames_dropped);
            json["decode_errors"] = serde_json::json!(src.decode_errors);
            json["nacks_sent"] = serde_json::json!(src.nacks_sent);
            json["plis_sent"] = serde_json::json!(src.plis_sent);
        }
        if let Some(sub) = info.subscription {
            json["flavor"] = serde_json::json!(sub.flavor.to_string());
            json["ssrc"] = serde_json::json!(sub.ssrc);
            json["keyframes_sent"] = serde_json::json!(sub.keyframes_sent);
            json["frames_sent"] = serde_json::json!(sub.frames_sent);
            json["nacks"] = serde_json::json!(sub.nacks_received);
            json["plis"] = serde_json::json!(sub.plis_received);
            json["packets_retransmitted"] = serde_json::json!(sub.packets_retransmitted);
            json["packets_dropped"] = serde_json::json!(sub.packets_dropped);
            json["remb_kbps"] = serde_json::json!(sub.remb_kbps);
        }
        json["speaking"] = serde_json::json!(info.speaking);
    }
    json
}

/// A room's video for the API (design §12): `{layout, spotlight, fps,
/// outputs: [...]}` and more; `null` when the room has no video.
pub fn room_video_json(room: &ConferenceRoom) -> serde_json::Value {
    let Some(video) = room.video() else {
        return serde_json::Value::Null;
    };
    let st = video.status();
    serde_json::json!({
        "layout": st.layout.name(),
        "spotlight": st.spotlight,
        "pinned": st.pinned,
        "active_speaker": st.active_speaker,
        "fps": st.fps,
        "target_fps": st.target_fps,
        "resolution": st.resolution.to_string(),
        "sources": st.sources,
        "encoders": st.encoders,
        "ticks": st.ticks,
        "overruns": st.overruns,
        "outputs": st.outputs.iter().map(|o| serde_json::json!({
            "exclude": o.exclude,
            "resolution": o.resolution.to_string(),
            "flavors": o.flavors.iter().map(|f| serde_json::json!({
                "codec": f.flavor.codec.sdp_name(),
                "profile": f.flavor.profile,
                "resolution": f.flavor.resolution.to_string(),
                "fps": f.flavor.fps,
                "max_kbps": f.flavor.max_kbps,
                "target_kbps": f.target_kbps,
                "subscribers": f.subscribers,
                "keyframes": f.keyframes,
                "encode_errors": f.encode_errors,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

impl std::fmt::Debug for VideoLeg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoLeg")
            .field("rtp_port", &self.port_pair.rtp_port)
            .field("codec", &self.negotiated.codec)
            .finish()
    }
}

/// The video codecs the node can actually run: every codec the backend's
/// registry serves, and — when `native_only` — only those with a native
/// binding compiled in. Without a binding the registry still answers
/// with the raw test codec, which is what CI uses and what a real phone
/// must never be offered.
pub fn usable_codecs(backend: &VideoBackend, native_only: bool) -> Vec<forge_core::VideoCodec> {
    let on_device = backend.registry.codecs_on(&backend.device);
    if !native_only {
        return on_device;
    }
    let native: Vec<forge_core::VideoCodec> = forge_video_codecs::enabled_backends()
        .into_iter()
        .flat_map(|b| match b {
            "vpx" => vec![forge_core::VideoCodec::VP8, forge_core::VideoCodec::VP9],
            "openh264" => vec![forge_core::VideoCodec::H264],
            // AV1 needs both the decoder and the encoder binding.
            _ => Vec::new(),
        })
        .collect();
    let av1 = {
        let b = forge_video_codecs::enabled_backends();
        b.contains(&"dav1d") && b.contains(&"svt-av1")
    };
    on_device
        .into_iter()
        .filter(|c| native.contains(c) || (av1 && *c == forge_core::VideoCodec::AV1))
        .collect()
}

/// The meeting's video settings as a room would run them, and the codecs
/// to offer, in the meeting's preference order restricted to what the
/// node can run. `None` when the meeting has video off or nothing usable.
pub fn plan_video(
    meeting: &Meeting,
    usable: &[forge_core::VideoCodec],
) -> Option<(VideoRoomSettings, Vec<forge_core::VideoCodec>)> {
    if !meeting.video_enabled {
        return None;
    }
    let codecs: Vec<forge_core::VideoCodec> = meeting
        .video_codecs
        .iter()
        .map(|c| video_codec_to_forge(*c))
        .filter(|c| usable.contains(c))
        .collect();
    if codecs.is_empty() {
        return None;
    }
    let settings = VideoRoomSettings {
        layout: Layout::parse(&meeting.video_layout).unwrap_or(Layout::Grid),
        max_tiles: (meeting.video_max_participants as usize).clamp(1, 16),
        resolution: Resolution::parse(&meeting.video_max_resolution)
            .unwrap_or(Resolution::new(1280, 720)),
        fps: meeting.video_fps.clamp(1, 30),
        exclude_self: meeting.video_exclude_self,
        codecs: codecs.clone(),
        ..VideoRoomSettings::default()
    };
    Some((settings, codecs))
}

/// Where the room's video events go.
#[derive(Clone)]
pub struct VideoEventSink {
    pub event_bus: Arc<dyn EventPublisher>,
    pub tenant_id: TenantId,
    pub room_number: String,
    /// Rooms with a pump running, so two callers joining at once start
    /// one pump, not two.
    pub pumps: Arc<dashmap::DashMap<String, ()>>,
}

/// Put the participant on the room's video: enable the room's video if
/// this is its first video participant (and start pumping its events),
/// add the participant as a source and a subscriber, and return the
/// bridge future to run. `None` when the room refused (no codec, the
/// participant is not in the room).
pub fn attach_video(
    room: &Arc<ConferenceRoom>,
    backend: &VideoBackend,
    participant_id: &str,
    display_name: Option<&str>,
    leg: &VideoLeg,
    sink: VideoEventSink,
) -> Option<impl Future<Output = ()> + Send + 'static> {
    let video = room.enable_video(leg.settings.clone(), backend);
    if sink.pumps.insert(sink.room_number.clone(), ()).is_none() {
        spawn_event_pump(&video, sink);
    }
    if let Some(name) = display_name {
        video.set_display_name(participant_id, name);
    }
    if let Err(e) = video.add_source(participant_id, leg.negotiated.codec) {
        warn!(participant = %participant_id, error = %e, "could not add video source");
        return None;
    }
    let sub = match video.subscribe(
        participant_id,
        SubscribeRequest {
            codec: leg.negotiated.codec,
            profile: leg.negotiated.fmtp.clone().unwrap_or_default(),
            payload_type: leg.negotiated.payload_type,
            resolution: None,
            fps: None,
            max_kbps: None,
        },
    ) {
        Ok(s) => s,
        Err(e) => {
            warn!(participant = %participant_id, error = %e, "could not subscribe to room video");
            video.remove_source(participant_id);
            return None;
        }
    };
    Some(run_video_bridge(
        video,
        participant_id.to_string(),
        Arc::clone(&leg.socket),
        leg.srtp_context.clone(),
        sub,
        leg.negotiated.clone(),
        Arc::clone(&leg.stats),
    ))
}

/// Take the participant off the room's video (a re-INVITE removed the
/// section, or the leg is torn down before the participant leaves).
pub fn detach_video(room: &ConferenceRoom, participant_id: &str) {
    if let Some(video) = room.video() {
        video.remove_source(participant_id);
        video.unsubscribe(participant_id);
    }
}

/// Forward the room's video events to the platform bus until the room's
/// video stops.
fn spawn_event_pump(video: &Arc<VideoRoom>, sink: VideoEventSink) {
    let mut events = video.events();
    tokio::spawn(async move {
        struct Done(Arc<dashmap::DashMap<String, ()>>, String);
        impl Drop for Done {
            fn drop(&mut self) {
                self.0.remove(&self.1);
            }
        }
        let _done = Done(Arc::clone(&sink.pumps), sink.room_number.clone());
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(room = %sink.room_number, skipped = n, "video events lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let event = match ev {
                VideoRoomEvent::ParticipantState {
                    participant_id,
                    state,
                } => Some(ConferenceEvent::VideoStateChanged {
                    room_number: sink.room_number.clone(),
                    participant_id,
                    state: state.name().to_string(),
                    tenant_id: sink.tenant_id,
                    timestamp: SystemTime::now(),
                }),
                VideoRoomEvent::FpsChanged { from, to, overload } => {
                    if overload {
                        Some(ConferenceEvent::VideoOverload {
                            room_number: sink.room_number.clone(),
                            from_fps: from,
                            to_fps: to,
                            tenant_id: sink.tenant_id,
                            timestamp: SystemTime::now(),
                        })
                    } else {
                        info!(room = %sink.room_number, from, to, "video frame rate restored");
                        None
                    }
                }
                VideoRoomEvent::ActiveSpeaker { participant_id } => {
                    debug!(room = %sink.room_number, speaker = ?participant_id, "active speaker");
                    None
                }
                // Layout controls and their event arrive with phase 3c.
                VideoRoomEvent::LayoutChanged { .. } => None,
            };
            if let Some(event) = event {
                if let Err(e) = sink
                    .event_bus
                    .publish(PlatformEvent::Conference(event))
                    .await
                {
                    warn!(error = %e, "Failed to publish video event");
                }
            }
        }
    });
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The bridge: RTP in → room; RTCP in → room feedback; the room's
/// feedback for the source → RTCP out; the subscription's packets → RTP
/// out. Ends when the subscription closes (the participant left the room's
/// video) or the socket fails.
pub async fn run_video_bridge(
    room: Arc<VideoRoom>,
    participant_id: String,
    socket: Arc<RtpSocketPair>,
    srtp_context: Option<Arc<RwLock<SrtpContext>>>,
    mut sub: VideoSubscription,
    negotiated: NegotiatedVideo,
    stats: Arc<VideoLegStats>,
) {
    let span = info_span!(
        "video_bridge",
        participant = %participant_id,
        room = %room.id(),
        codec = %negotiated.codec,
        srtp = srtp_context.is_some(),
    );
    async move {
        info!(
            payload_type = negotiated.payload_type,
            pli = negotiated.pli,
            fir = negotiated.fir,
            nack = negotiated.nack,
            ssrc = sub.ssrc,
            "Video bridge started"
        );
        let mut fir_seq: u8 = 0;

        loop {
            tokio::select! {
                // Inbound RTP: the participant's camera.
                result = socket.recv_rtp_raw() => {
                    let (raw, _addr) = match result {
                        Ok(r) => r,
                        Err(e) => {
                            warn!(error = %e, "video RTP receive failed; bridge stopping");
                            break;
                        }
                    };
                    stats.packets_received.fetch_add(1, Ordering::Relaxed);
                    stats.last_rx_unix_ms.store(unix_ms(), Ordering::Relaxed);
                    let plain = match unprotect(&srtp_context, &raw, false).await {
                        Some(p) => p,
                        None => {
                            stats.srtp_errors.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    };
                    let packet = match RtpPacket::parse(plain) {
                        Ok(p) => p,
                        Err(e) => {
                            trace!(error = %e, "video RTP parse error");
                            continue;
                        }
                    };
                    if packet.header.payload_type() != negotiated.payload_type {
                        trace!(pt = packet.header.payload_type(), "video packet with another payload type");
                        continue;
                    }
                    for fb in room.push_rtp(&participant_id, packet) {
                        let fb = match adapt_feedback(fb, &negotiated, &mut fir_seq) {
                            Some(fb) => fb,
                            None => continue,
                        };
                        send_rtcp(&socket, &srtp_context, &fb, &stats).await;
                    }
                }

                // Inbound RTCP: the participant's receiver talking about the
                // composite we send (PLI, FIR, NACK, REMB) — or reports.
                result = socket.recv_rtcp() => {
                    let (raw, _addr) = match result {
                        Ok(r) => r,
                        Err(e) => {
                            warn!(error = %e, "video RTCP receive failed; bridge stopping");
                            break;
                        }
                    };
                    stats.rtcp_received.fetch_add(1, Ordering::Relaxed);
                    let plain = match unprotect(&srtp_context, &raw, true).await {
                        Some(p) => p,
                        None => {
                            stats.srtp_errors.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    };
                    for packet in RtcpPacket::parse_compound(&plain) {
                        room.handle_feedback(&participant_id, &packet);
                    }
                }

                // Outbound: the composite for this participant.
                next = sub.packets.recv() => {
                    let Some(bytes) = next else {
                        debug!("video subscription closed; bridge stopping");
                        break;
                    };
                    let out = if let Some(ref ctx) = srtp_context {
                        let protected = {
                            let mut guard = ctx.write().await;
                            guard.protect_rtp(&bytes)
                        };
                        match protected {
                            Ok(p) => Bytes::from(p),
                            Err(e) => {
                                trace!(error = %e, "video SRTP protect error");
                                stats.srtp_errors.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                        }
                    } else {
                        bytes
                    };
                    match socket.send_rtp_raw(&out).await {
                        Ok(_) => {
                            stats.packets_sent.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => trace!(error = %e, "video RTP send error"),
                    }
                }
            }
        }

        info!(
            rx = stats.packets_received.load(Ordering::Relaxed),
            tx = stats.packets_sent.load(Ordering::Relaxed),
            rtcp_rx = stats.rtcp_received.load(Ordering::Relaxed),
            rtcp_tx = stats.rtcp_sent.load(Ordering::Relaxed),
            "Video bridge stopped"
        );
    }
    .instrument(span)
    .await
}

/// Only send what the peer negotiated: a PLI to a peer that offered FIR
/// but not PLI becomes a FIR; NACK to a peer without `nack` is dropped.
fn adapt_feedback(
    fb: RtcpPacket,
    negotiated: &NegotiatedVideo,
    fir_seq: &mut u8,
) -> Option<RtcpPacket> {
    match fb {
        RtcpPacket::PayloadFeedback(ps) if ps.is_pli_kind() => {
            if negotiated.pli {
                Some(RtcpPacket::PayloadFeedback(ps))
            } else if negotiated.fir {
                *fir_seq = fir_seq.wrapping_add(1);
                Some(RtcpPacket::PayloadFeedback(PsFeedback::fir(
                    ps.sender_ssrc,
                    ps.media_ssrc,
                    *fir_seq,
                )))
            } else {
                None
            }
        }
        RtcpPacket::TransportFeedback(_) if !negotiated.nack => None,
        other => Some(other),
    }
}

trait PliKind {
    fn is_pli_kind(&self) -> bool;
}

impl PliKind for PsFeedback {
    fn is_pli_kind(&self) -> bool {
        matches!(self.kind, forge_rtp::rtcp::PayloadFeedback::Pli)
    }
}

async fn unprotect(
    srtp: &Option<Arc<RwLock<SrtpContext>>>,
    raw: &Bytes,
    rtcp: bool,
) -> Option<Bytes> {
    match srtp {
        Some(ctx) => {
            let result = {
                let mut guard = ctx.write().await;
                if rtcp {
                    guard.unprotect_rtcp(raw)
                } else {
                    guard.unprotect_rtp(raw)
                }
            };
            match result {
                Ok(d) => Some(Bytes::from(d)),
                Err(e) => {
                    trace!(error = %e, rtcp, "video SRTP unprotect error");
                    None
                }
            }
        }
        None => Some(raw.clone()),
    }
}

async fn send_rtcp(
    socket: &RtpSocketPair,
    srtp: &Option<Arc<RwLock<SrtpContext>>>,
    packet: &RtcpPacket,
    stats: &VideoLegStats,
) {
    let bytes = packet.to_bytes();
    let out = if let Some(ctx) = srtp {
        let protected = {
            let mut guard = ctx.write().await;
            guard.protect_rtcp(&bytes)
        };
        match protected {
            Ok(p) => p,
            Err(e) => {
                trace!(error = %e, "video SRTCP protect error");
                stats.srtp_errors.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
    } else {
        bytes
    };
    match socket.send_rtcp(&out).await {
        Ok(_) => {
            stats.rtcp_sent.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => trace!(error = %e, "video RTCP send error"),
    }
}

/// How long a video leg may go without packets before the API calls it
/// inactive.
pub const VIDEO_INACTIVE_AFTER: Duration = Duration::from_secs(30);

impl VideoLegStats {
    pub fn last_rx_age(&self) -> Option<Duration> {
        let last = self.last_rx_unix_ms.load(Ordering::Relaxed);
        if last == 0 {
            return None;
        }
        Some(Duration::from_millis(unix_ms().saturating_sub(last)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::VideoCodec;
    use fcp_events::InMemoryEventBus;
    use forge_conference::AudioFormat;
    use forge_rtp::video::payload::packetize;
    use forge_rtp::{AssemblerEvent, FrameAssembler, RemoteEndpoint, RtpSocketConfig};
    use forge_video::codec::{EncoderSettings, VideoDecoder, VideoEncoder};
    use forge_video::frame::{HostFrame, VideoFrame};
    use forge_video::raw::raw_registry;
    use forge_video::MediaDevice;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn meeting(enabled: bool) -> Meeting {
        let now = SystemTime::now();
        Meeting {
            id: fcp_core::MeetingId::new(),
            tenant_id: TenantId::new(),
            room_number: "1000".to_string(),
            name: "Video room".to_string(),
            guest_pin: None,
            host_pin: None,
            require_guest_pin: false,
            max_participants: 0,
            wait_for_moderator: false,
            enabled: true,
            auto_record: false,
            default_locked: false,
            permanent: false,
            owner_id: None,
            pin_prompt_sound: None,
            invalid_pin_sound: None,
            moh_sound: None,
            pin_prompt_tts: None,
            invalid_pin_tts: None,
            welcome_tts: None,
            play_entry_tone: false,
            play_exit_tone: false,
            join_sound: None,
            exit_sound: None,
            video_enabled: enabled,
            video_layout: "active_speaker".to_string(),
            video_max_participants: 16,
            video_max_resolution: "360p".to_string(),
            video_fps: 30,
            video_exclude_self: false,
            video_codecs: vec![VideoCodec::H264, VideoCodec::Vp8, VideoCodec::Av1],
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn a_plan_follows_the_meeting_and_the_usable_codecs() {
        let usable = vec![forge_core::VideoCodec::VP8, forge_core::VideoCodec::VP9];
        let (settings, codecs) = plan_video(&meeting(true), &usable).expect("video");
        assert_eq!(codecs, vec![forge_core::VideoCodec::VP8]);
        assert_eq!(settings.layout, Layout::ActiveSpeaker);
        assert_eq!(settings.resolution, Resolution::new(640, 360));
        assert_eq!(settings.fps, 30);
        assert!(plan_video(&meeting(false), &usable).is_none());
        assert!(
            plan_video(&meeting(true), &[forge_core::VideoCodec::H265]).is_none(),
            "nothing the node can run"
        );
    }

    #[test]
    fn usable_codecs_are_raw_only_when_allowed() {
        let backend = VideoBackend::raw();
        let all = usable_codecs(&backend, false);
        assert_eq!(all.len(), 5, "the raw registry serves every codec");
        let native = usable_codecs(&backend, true);
        // No native binding is compiled into this test build unless a
        // feature is on; the list is then exactly the bound codecs.
        for c in &native {
            assert!(all.contains(c));
        }
        if forge_video_codecs::enabled_backends().is_empty() {
            assert!(native.is_empty());
        }
    }

    #[test]
    fn feedback_is_adapted_to_what_the_peer_negotiated() {
        let mut n = NegotiatedVideo {
            codec: forge_core::VideoCodec::VP8,
            payload_type: 97,
            fmtp: None,
            remote: None,
            pli: false,
            fir: true,
            nack: false,
            remote_crypto: None,
        };
        let pli = RtcpPacket::PayloadFeedback(PsFeedback::pli(1, 2));
        let mut seq = 0;
        match adapt_feedback(pli.clone(), &n, &mut seq) {
            Some(RtcpPacket::PayloadFeedback(ps)) => {
                assert!(matches!(ps.kind, forge_rtp::rtcp::PayloadFeedback::Fir(_)));
            }
            other => panic!("expected FIR, got {other:?}"),
        }
        assert_eq!(seq, 1);
        let nack = RtcpPacket::TransportFeedback(forge_rtp::rtcp::RtpFeedback::nack(1, 2, &[5]));
        assert!(adapt_feedback(nack.clone(), &n, &mut seq).is_none());
        n.pli = true;
        n.nack = true;
        assert!(matches!(
            adapt_feedback(pli, &n, &mut seq),
            Some(RtcpPacket::PayloadFeedback(_))
        ));
        assert!(adapt_feedback(nack, &n, &mut seq).is_some());
        n.pli = false;
        n.fir = false;
        assert!(adapt_feedback(
            RtcpPacket::PayloadFeedback(PsFeedback::pli(1, 2)),
            &n,
            &mut seq
        )
        .is_none());
    }

    async fn socket_pair(port: u16) -> Arc<RtpSocketPair> {
        let cfg = RtpSocketConfig {
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..RtpSocketConfig::default()
        };
        Arc::new(
            RtpSocketPair::new(PortPair::new(port).unwrap(), cfg)
                .await
                .unwrap(),
        )
    }

    /// Two free even ports on loopback, each with its RTCP neighbour.
    fn free_pair() -> u16 {
        loop {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let p = s.local_addr().unwrap().port();
            if p.is_multiple_of(2)
                && p < u16::MAX - 1
                && std::net::UdpSocket::bind(("127.0.0.1", p + 1)).is_ok()
            {
                return p;
            }
        }
    }

    /// The loopback test of design §14: a raw-codec participant's frames go
    /// through a real socket pair into the room, and the composite reaches
    /// a second participant's socket, where it is reassembled and decoded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn frames_reach_a_second_participants_socket_composited() {
        let room = Arc::new(
            ConferenceRoom::new(
                "1000",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        room.add_participant("alice", true).unwrap();
        room.add_participant("bob", false).unwrap();
        let backend = VideoBackend::raw();
        let (settings, _) = plan_video(
            &{
                let mut m = meeting(true);
                m.video_layout = "grid".into();
                m.video_max_resolution = "180p".into();
                m.video_codecs = vec![VideoCodec::Vp8];
                m
            },
            &usable_codecs(&backend, false),
        )
        .unwrap();
        let bus = Arc::new(InMemoryEventBus::new());
        let pumps = Arc::new(dashmap::DashMap::new());
        let sink = VideoEventSink {
            event_bus: bus.clone() as Arc<dyn EventPublisher>,
            tenant_id: TenantId::new(),
            room_number: "1000".into(),
            pumps: Arc::clone(&pumps),
        };

        // Server-side sockets for the two legs, and the "phones".
        let alice_server = free_pair();
        let bob_server = free_pair();
        let alice_phone = free_pair();
        let bob_phone = free_pair();
        let alice_srv = socket_pair(alice_server).await;
        let bob_srv = socket_pair(bob_server).await;
        let alice_ph = socket_pair(alice_phone).await;
        let bob_ph = socket_pair(bob_phone).await;
        let lo = |p: u16| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), p);
        for (srv, phone) in [(&alice_srv, alice_phone), (&bob_srv, bob_phone)] {
            srv.set_remote_endpoint(RemoteEndpoint {
                rtp_addr: lo(phone),
                rtcp_addr: Some(lo(phone + 1)),
            })
            .await;
        }
        for (phone, srv) in [(&alice_ph, alice_server), (&bob_ph, bob_server)] {
            phone
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: lo(srv),
                    rtcp_addr: Some(lo(srv + 1)),
                })
                .await;
        }

        let negotiated = NegotiatedVideo {
            codec: forge_core::VideoCodec::VP8,
            payload_type: 97,
            fmtp: None,
            remote: Some(lo(alice_phone)),
            pli: true,
            fir: false,
            nack: true,
            remote_crypto: None,
        };
        let mut legs = Vec::new();
        for (pid, srv, pair) in [
            ("alice", &alice_srv, alice_server),
            ("bob", &bob_srv, bob_server),
        ] {
            let leg = VideoLeg {
                port_pair: PortPair::new(pair).unwrap(),
                socket: Arc::clone(srv),
                srtp_context: None,
                srtp_sdp: None,
                negotiated: negotiated.clone(),
                settings: settings.clone(),
                stats: Arc::new(VideoLegStats::default()),
                bridge_handle: None,
            };
            let fut = attach_video(&room, &backend, pid, Some(pid), &leg, sink.clone()).unwrap();
            legs.push((tokio::spawn(fut), leg));
        }
        assert!(room.video().is_some());
        assert_eq!(pumps.len(), 1, "two attaches, one event pump");

        // Alice's phone sends solid frames; Bob's phone decodes what it gets.
        let enc_settings = EncoderSettings {
            codec: forge_core::VideoCodec::VP8,
            resolution: Resolution::new(160, 90),
            fps: 15,
            bitrate_kbps: 300,
            keyframe_interval: 30,
            profile: String::new(),
        };
        let mut enc: Box<dyn VideoEncoder> = raw_registry()
            .encoder(&enc_settings, &MediaDevice::Host)
            .unwrap();
        let sender = {
            let alice_ph = Arc::clone(&alice_ph);
            tokio::spawn(async move {
                let mut seq = 1u16;
                for i in 0..90u32 {
                    let f = HostFrame::solid(160, 90, 180, 128, 128).with_pts(i * 6000);
                    for c in enc.encode(&VideoFrame::Host(f), false).unwrap() {
                        let payloads = packetize(forge_core::VideoCodec::VP8, &c, 1200).unwrap();
                        let n = payloads.len();
                        for (k, p) in payloads.into_iter().enumerate() {
                            let pkt =
                                RtpPacket::build(97, seq, c.timestamp, 0xA11CE, p, k + 1 == n);
                            seq = seq.wrapping_add(1);
                            alice_ph.send_rtp(&pkt).await.unwrap();
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(33)).await;
                }
            })
        };

        let mut asm = FrameAssembler::new(forge_core::VideoCodec::VP8);
        let mut dec: Box<dyn VideoDecoder> = raw_registry()
            .decoder(forge_core::VideoCodec::VP8, &MediaDevice::Host)
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut composite = None;
        while tokio::time::Instant::now() < deadline {
            let (bytes, _) =
                match tokio::time::timeout(Duration::from_secs(1), bob_ph.recv_rtp_raw()).await {
                    Ok(Ok(r)) => r,
                    _ => continue,
                };
            let pkt = RtpPacket::parse(bytes).unwrap();
            assert_eq!(pkt.header.payload_type(), 97);
            for ev in asm.push(pkt) {
                if let AssemblerEvent::Frame(f) = ev {
                    if let Some(v) = dec.decode(&f).unwrap() {
                        let h = v.into_host().unwrap();
                        // A 2×1 grid at 320×180: Alice's tile on the left.
                        if h.luma(80, 90) == 180 {
                            composite = Some(h);
                        }
                    }
                }
            }
            if composite.is_some() {
                break;
            }
        }
        let composite = composite.expect("bob's phone received alice composited");
        assert_eq!(composite.resolution(), Resolution::new(320, 180));
        assert_ne!(composite.luma(240, 90), 180, "bob's own tile is an avatar");
        sender.abort();

        // The bridge sent RTCP feedback to Alice's phone (the start-up PLI),
        // and the counters are visible.
        let (raw, _) = tokio::time::timeout(Duration::from_secs(2), alice_ph.recv_rtcp())
            .await
            .expect("feedback reached alice's phone")
            .unwrap();
        assert!(RtcpPacket::parse_compound(&raw)
            .iter()
            .any(|p| p.is_keyframe_request()));
        let alice_stats = &legs[0].1.stats;
        assert!(alice_stats.packets_received.load(Ordering::Relaxed) > 0);
        assert!(alice_stats.rtcp_sent.load(Ordering::Relaxed) > 0);
        assert!(legs[1].1.stats.packets_sent.load(Ordering::Relaxed) > 0);
        let video = room.video().unwrap();
        assert_eq!(
            video.participant("alice").unwrap().state,
            forge_conference::video::VideoState::On
        );

        // Leaving the room ends the bridge on its own.
        room.remove_participant("bob").unwrap();
        let (bob_task, _) = legs.pop().unwrap();
        tokio::time::timeout(Duration::from_secs(2), bob_task)
            .await
            .expect("bob's bridge ended when his subscription closed")
            .unwrap();
        // Stopping the room's video ends the pump once the last holder of
        // the room (here: this test's own handle, and alice's bridge) lets go.
        drop(video);
        room.disable_video();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(pumps.is_empty(), "the pump ended with the room's video");
    }
}
