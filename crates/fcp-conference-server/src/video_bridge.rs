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

use crate::rooms::RoomKey;
use bytes::Bytes;
use fcp_core::{Meeting, TenantId};
use fcp_events::{ConferenceEvent, EventPublisher, PlatformEvent};
use fcp_media::{video_codec_to_forge, NegotiatedVideo, SrtpSdpContext};
use forge_conference::video::{
    ContentEvent, ContentGate, ContentInfo, ContentLayout, ForwardRequest, OutputScope,
    PassthroughMode, Simulcast, StreamKind, StreamTarget, StreamWant, SubscribeRequest,
    VideoBackend, VideoRoom, VideoRoomEvent, VideoRoomSettings, VideoSubscription, View,
    ViewRequest,
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
/// teardown, re-INVITEs and the API. A participant has one for their
/// camera and, when their offer carried an `a=content:slides` section,
/// one for their shared screen (§15.6).
pub struct VideoLeg {
    /// Which of the participant's streams this leg carries.
    pub kind: StreamKind,
    pub port_pair: PortPair,
    pub socket: Arc<RtpSocketPair>,
    pub srtp_context: Option<Arc<RwLock<SrtpContext>>>,
    /// Our SDES key for the answer, when the leg is SRTP.
    pub srtp_sdp: Option<SrtpSdpContext>,
    pub negotiated: NegotiatedVideo,
    pub settings: VideoRoomSettings,
    pub stats: Arc<VideoLegStats>,
    /// A camera leg that takes each person's own stream rather than the
    /// composite (`docs/FORWARDING_ROOMS.md` §2.1): the participant asked
    /// (`Recv-Info: fcp-view`) and the meeting's `video_mode` is `auto`.
    pub forward: bool,
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
            kind: self.kind,
            port_pair: self.port_pair,
            socket: Arc::clone(&self.socket),
            srtp_context: self.srtp_context.clone(),
            srtp_sdp: self.srtp_sdp.clone(),
            negotiated: self.negotiated.clone(),
            settings: self.settings.clone(),
            stats: Arc::clone(&self.stats),
            forward: self.forward,
            bridge_handle: None,
        }
    }
}

/// The socket-level half of a leg's JSON: what was negotiated and what
/// went over the wire.
fn leg_json(leg: &VideoLeg, state: &str) -> serde_json::Value {
    serde_json::json!({
        "state": state,
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
    })
}

/// The ingress side of a stream, as the room measures it.
fn source_json(json: &mut serde_json::Value, src: &forge_conference::video::VideoSourceInfo) {
    // Where the stream is decoded: the room's device, or the host when
    // the device has no decoder for the codec (§15.7).
    json["device"] = serde_json::json!(src.device.to_string());
    json["resolution"] = serde_json::json!(src.resolution.to_string());
    json["fps"] = serde_json::json!(src.fps);
    json["bitrate_kbps"] = serde_json::json!(src.bitrate_kbps);
    json["frames_decoded"] = serde_json::json!(src.frames_decoded);
    json["frames_lost"] = serde_json::json!(src.frames_lost);
    json["frames_dropped"] = serde_json::json!(src.frames_dropped);
    json["decode_errors"] = serde_json::json!(src.decode_errors);
    json["nacks_sent"] = serde_json::json!(src.nacks_sent);
    json["plis_sent"] = serde_json::json!(src.plis_sent);
    // Not while nothing in the room composes (`docs/FORWARDING_ROOMS.md`
    // §2.1): every subscriber forwarded, no recording.
    json["decoding"] = serde_json::json!(src.decoding);
    // A simulcast sender's layers: which one the room decodes, and what
    // each is sending (`docs/FORWARDING_ROOMS.md` §2.4).
    if src.layers.len() > 1 {
        json["layers"] = serde_json::Value::Array(
            src.layers
                .iter()
                .map(|l| {
                    serde_json::json!({
                        "rid": l.rid,
                        "ssrc": l.ssrc,
                        "resolution": l.resolution.to_string(),
                        "bitrate_kbps": l.bitrate_kbps,
                        "live": l.live,
                        "decoding": l.decoding,
                    })
                })
                .collect(),
        );
    }
}

/// The egress side of a channel: the flavor, the rung, the counters.
fn subscription_json(
    json: &mut serde_json::Value,
    sub: &forge_conference::video::VideoSubscriberInfo,
) {
    json["flavor"] = serde_json::json!(sub.flavor.to_string());
    json["view"] = serde_json::json!(sub.view.name());
    // The ladder rung this participant is on, and the rate it is
    // encoded at, without a caller having to parse the flavor
    // (§7). Below the room's own size means their link asked.
    json["rung"] = serde_json::json!(sub.flavor.resolution.to_string());
    json["rung_kbps"] = serde_json::json!(sub.flavor.max_kbps);
    json["ssrc"] = serde_json::json!(sub.ssrc);
    json["keyframes_sent"] = serde_json::json!(sub.keyframes_sent);
    json["frames_sent"] = serde_json::json!(sub.frames_sent);
    json["nacks"] = serde_json::json!(sub.nacks_received);
    json["plis"] = serde_json::json!(sub.plis_received);
    json["packets_retransmitted"] = serde_json::json!(sub.packets_retransmitted);
    json["packets_dropped"] = serde_json::json!(sub.packets_dropped);
    json["remb_kbps"] = serde_json::json!(sub.remb_kbps);
    // On the fast path this participant is watching one sender's
    // own stream rather than a composite (§5.6).
    if let Some(source) = &sub.forwarding {
        json["forwarding"] = serde_json::json!(source.to_string());
    }
}

/// What a forwarding subscriber is being sent (`docs/FORWARDING_ROOMS.md`
/// §2.1): each stream — whose, which layer, whether held back — and the
/// budget its REMB gives the allocator.
fn viewer_json(json: &mut serde_json::Value, v: &forge_conference::video::VideoViewerInfo) {
    json["forwarded"] = serde_json::json!({
        "remb_kbps": v.remb_kbps,
        "packets_sent": v.packets_sent,
        "packets_dropped": v.packets_dropped,
        "nacks": v.nacks_received,
        "packets_retransmitted": v.packets_retransmitted,
        "plis": v.plis_received,
        "streams": v.streams.iter().map(|s| serde_json::json!({
            "ssrc": s.ssrc,
            "participant": s.participant,
            "speaker": matches!(s.target, forge_conference::video::StreamTarget::Speaker),
            "rid": s.rid,
            "resolution": s.resolution.to_string(),
            "paused": s.paused,
            "packets_sent": s.packets_sent,
        })).collect::<Vec<_>>(),
    });
}

/// A participant's video for the API (design §12): `{codec, resolution,
/// fps, bitrate_kbps, keyframes_sent, nacks, plis, state}` plus the leg's
/// socket counters, and — when the participant's offer carried a
/// shared screen's section (§15.6) — a `content` object of the same
/// shape for it, with `channel` saying whether they receive the room's
/// content on that section, and `presenting` saying whether their screen
/// is the one the room shows. `{"state": "off"}` when no video was
/// negotiated.
pub fn participant_video_json(
    room: &ConferenceRoom,
    participant_id: &str,
    leg: Option<&VideoLeg>,
    content_leg: Option<&VideoLeg>,
) -> serde_json::Value {
    let info = room.video().and_then(|v| v.participant(participant_id));
    let mut json = match leg {
        Some(leg) => leg_json(leg, info.as_ref().map(|i| i.state.name()).unwrap_or("off")),
        None => serde_json::json!({ "state": "off" }),
    };
    if let Some(info) = &info {
        if leg.is_some() {
            if let Some(src) = &info.source {
                source_json(&mut json, src);
            }
            if let Some(sub) = &info.subscription {
                subscription_json(&mut json, sub);
            }
            if let Some(v) = &info.viewer {
                viewer_json(&mut json, v);
            }
        }
        json["speaking"] = serde_json::json!(info.speaking);
        if let Some(cleg) = content_leg {
            let mut c = leg_json(cleg, info.content_state.name());
            if let Some(src) = &info.content {
                source_json(&mut c, src);
            }
            if let Some(sub) = &info.content_subscription {
                subscription_json(&mut c, sub);
            }
            c["channel"] = serde_json::json!(info.content_subscription.is_some());
            json["content"] = c;
        }
        json["presenting"] = serde_json::json!(info.presenting);
    } else if let Some(cleg) = content_leg {
        json["content"] = leg_json(cleg, "off");
    }
    json
}

/// A room's video for the API (design §12): `{layout, spotlight, fps,
/// outputs: [...]}` and more; `null` when the room has no video.
/// How an output's scope reads over the API.
fn scope_json(scope: &OutputScope) -> serde_json::Value {
    match scope {
        OutputScope::All => serde_json::json!({ "kind": "all" }),
        OutputScope::Excluding(id) => serde_json::json!({ "kind": "excluding", "participant": id }),
        OutputScope::LocalOnly => serde_json::json!({ "kind": "local_only" }),
    }
}

pub fn room_video_json(room: &ConferenceRoom) -> serde_json::Value {
    let Some(video) = room.video() else {
        return serde_json::Value::Null;
    };
    let st = video.status();
    serde_json::json!({
        "layout": st.layout.name(),
        "content_layout": st.content_layout.name(),
        // The share under way (§15.6): who holds the floor and what their
        // screen arrives as. `null` while nobody is sharing.
        "content": st.content.as_ref().map(|c| serde_json::json!({
            "participant_id": c.participant_id,
            "display_name": c.display_name,
            "remote": c.remote,
            "held_for_secs": c.held_for.as_secs(),
            "codec": c.codec.sdp_name(),
            "resolution": c.resolution.to_string(),
            "fps": c.fps,
            "live": c.live,
        })),
        "spotlight": st.spotlight,
        "pinned": st.pinned,
        "active_speaker": st.active_speaker,
        "fps": st.fps,
        "target_fps": st.target_fps,
        "resolution": st.resolution.to_string(),
        // The device the room is placed on; each output, flavor and
        // source says where it actually runs (§15.7).
        "device": st.device.to_string(),
        "sources": st.sources,
        "encoders": st.encoders,
        "ticks": st.ticks,
        "overruns": st.overruns,
        "outputs": st.outputs.iter().map(|o| serde_json::json!({
            // Whose tiles the composite is drawn from: everyone, all but
            // one (`exclude_self`), or this node's callers only (what a
            // trunk carries).
            "scope": scope_json(&o.scope),
            // The composite, the cameras alone, or the content alone (§15.6).
            "view": o.view.name(),
            "resolution": o.resolution.to_string(),
            "device": o.device.to_string(),
            "flavors": o.flavors.iter().map(|f| serde_json::json!({
                "device": f.device.to_string(),
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

// ---- runtime controls (design §8, §12) ---------------------------------------

/// Why a video control could not be applied on this node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoControlError {
    /// The participant is not in the room on this node; it may be on
    /// another node hosting the room.
    NoSuchParticipant(String),
    /// The room is active here but nobody has video yet, so there is no
    /// video room to control.
    NoVideo,
    /// Not one of `grid`, `active_speaker`, `spotlight`, `pip`.
    UnknownLayout(String),
    /// Nobody is sharing a screen here.
    NoContent,
    /// Not one of `presentation`, `content_only`.
    UnknownContentLayout(String),
}

impl std::fmt::Display for VideoControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchParticipant(id) => write!(f, "Participant '{id}' not in room"),
            Self::NoVideo => f.write_str("Room has no video"),
            Self::UnknownLayout(l) => write!(
                f,
                "Unknown layout '{l}' (one of grid, active_speaker, spotlight, pip)"
            ),
            Self::NoContent => f.write_str("Nobody is sharing a screen in this room"),
            Self::UnknownContentLayout(l) => write!(
                f,
                "Unknown content layout '{l}' (one of presentation, content_only)"
            ),
        }
    }
}

impl std::error::Error for VideoControlError {}

fn video_of(room: &ConferenceRoom) -> Result<Arc<VideoRoom>, VideoControlError> {
    room.video().ok_or(VideoControlError::NoVideo)
}

fn participant_in(room: &ConferenceRoom, participant_id: &str) -> Result<(), VideoControlError> {
    if room.participants().iter().any(|p| p == participant_id) {
        Ok(())
    } else {
        Err(VideoControlError::NoSuchParticipant(
            participant_id.to_string(),
        ))
    }
}

/// `{layout, pinned, spotlight}` after a control, for the response.
fn layout_json(status: &str, video: &VideoRoom) -> serde_json::Value {
    let st = video.status();
    serde_json::json!({
        "status": status,
        "layout": st.layout.name(),
        "content_layout": st.content_layout.name(),
        "pinned": st.pinned,
        "spotlight": st.spotlight,
    })
}

/// A host ends the share under way (§15.6). The presenter cannot take
/// the floor back by just sending; a fresh content section, or
/// [`set_participant_video`] on their content, lets them. `NoContent`
/// when nobody is sharing on this node.
pub fn stop_content(room: &ConferenceRoom) -> Result<serde_json::Value, VideoControlError> {
    let video = video_of(room)?;
    match video.stop_content() {
        Some(pid) => Ok(serde_json::json!({
            "status": "content_stopped",
            "participant": pid,
        })),
        None => Err(VideoControlError::NoContent),
    }
}

/// How the composite shows a shared screen: `presentation` or
/// `content_only`, changeable while one is on.
pub fn set_content_layout(
    room: &ConferenceRoom,
    layout: &str,
) -> Result<serde_json::Value, VideoControlError> {
    let parsed = ContentLayout::parse(layout)
        .ok_or_else(|| VideoControlError::UnknownContentLayout(layout.to_string()))?;
    let video = video_of(room)?;
    video.set_content_layout(parsed);
    Ok(layout_json("content_layout_set", &video))
}

/// Set the room's layout on this node.
pub fn set_layout(
    room: &ConferenceRoom,
    layout: &str,
) -> Result<serde_json::Value, VideoControlError> {
    let parsed = Layout::parse(layout)
        .ok_or_else(|| VideoControlError::UnknownLayout(layout.to_string()))?;
    let video = video_of(room)?;
    video.set_layout(parsed);
    Ok(layout_json("layout_set", &video))
}

/// Keep a participant in the first tile, or clear the pin with `None`.
/// Pinning needs the participant here; clearing only needs the room.
pub fn pin(
    room: &ConferenceRoom,
    participant_id: Option<&str>,
) -> Result<serde_json::Value, VideoControlError> {
    if let Some(pid) = participant_id {
        participant_in(room, pid)?;
    }
    let video = video_of(room)?;
    video.pin(participant_id);
    let mut json = layout_json(
        if participant_id.is_some() {
            "pinned"
        } else {
            "unpinned"
        },
        &video,
    );
    if let Some(pid) = participant_id {
        json["participant"] = serde_json::json!(pid);
    }
    Ok(json)
}

/// Put one participant full-canvas (this switches the layout to
/// `spotlight` unless it is already `spotlight` or `pip`), or clear the
/// spotlight with `None`. Clearing keeps the layout: set it explicitly to
/// go back to a grid.
pub fn spotlight(
    room: &ConferenceRoom,
    participant_id: Option<&str>,
) -> Result<serde_json::Value, VideoControlError> {
    if let Some(pid) = participant_id {
        participant_in(room, pid)?;
    }
    let video = video_of(room)?;
    video.spotlight(participant_id);
    let mut json = layout_json(
        if participant_id.is_some() {
            "spotlighted"
        } else {
            "unspotlighted"
        },
        &video,
    );
    if let Some(pid) = participant_id {
        json["participant"] = serde_json::json!(pid);
    }
    Ok(json)
}

/// Host control: hide (`false`) or show (`true`) a participant's video.
/// Hidden, the participant's tile draws an avatar and its state reads
/// `disabled`; the room's state machine emits `video_state_changed`.
pub fn set_participant_video(
    room: &ConferenceRoom,
    participant_id: &str,
    enabled: bool,
) -> Result<serde_json::Value, VideoControlError> {
    participant_in(room, participant_id)?;
    let video = video_of(room)?;
    video.set_participant_video_enabled(participant_id, enabled);
    Ok(serde_json::json!({
        "status": if enabled { "video_enabled" } else { "video_disabled" },
        "participant": participant_id,
    }))
}

impl std::fmt::Debug for VideoLeg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoLeg")
            .field("rtp_port", &self.port_pair.rtp_port)
            .field("codec", &self.negotiated.codec)
            .finish()
    }
}

/// The video codecs the node can actually run, and where.
///
/// A codec is usable when the node decodes it somewhere and encodes it
/// somewhere: on the room's device, or on the host — a room on a GPU
/// decodes VP8 on NVDEC and encodes it on the host (design §15.7,
/// decision 5). Under `native_only`, the host counts only codecs with a
/// native binding compiled in; the device's own codecs always count.
/// Without a binding the registry still answers with the raw test codec,
/// which is what CI uses and what a real phone must never be offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCodecs {
    /// What the node offers callers: decodable and encodable somewhere.
    pub usable: Vec<forge_core::VideoCodec>,
    /// Per device, the room's device first and the host after it when
    /// they differ: what it encodes and decodes of `usable`.
    pub devices: Vec<DeviceCodecs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodecs {
    pub device: forge_video::frame::MediaDevice,
    /// Codecs this device encodes: what its budget advertises.
    pub encodes: Vec<forge_core::VideoCodec>,
    pub decodes: Vec<forge_core::VideoCodec>,
}

impl NodeCodecs {
    #[allow(dead_code)]
    pub fn on(&self, device: &forge_video::frame::MediaDevice) -> Option<&DeviceCodecs> {
        self.devices.iter().find(|d| &d.device == device)
    }
}

/// The host's codecs with a native binding compiled in.
fn native_host_codecs() -> Vec<forge_core::VideoCodec> {
    let backends = forge_video_codecs::enabled_backends();
    let mut native: Vec<forge_core::VideoCodec> = backends
        .iter()
        .flat_map(|b| match *b {
            "vpx" => vec![forge_core::VideoCodec::VP8, forge_core::VideoCodec::VP9],
            "openh264" => vec![forge_core::VideoCodec::H264],
            _ => Vec::new(),
        })
        .collect();
    // AV1 needs both the decoder and the encoder binding.
    if backends.contains(&"dav1d") && backends.contains(&"svt-av1") {
        native.push(forge_core::VideoCodec::AV1);
    }
    native
}

/// What the node runs, per device.
pub fn node_codecs(backend: &VideoBackend, native_only: bool) -> NodeCodecs {
    use forge_video::frame::MediaDevice;
    let reg = &backend.registry;
    let native = native_host_codecs();
    let host_allowed = |c: &forge_core::VideoCodec| !native_only || native.contains(c);
    let host = DeviceCodecs {
        device: MediaDevice::Host,
        encodes: reg
            .encodable_on(&MediaDevice::Host)
            .into_iter()
            .filter(host_allowed)
            .collect(),
        decodes: reg
            .decodable_on(&MediaDevice::Host)
            .into_iter()
            .filter(host_allowed)
            .collect(),
    };
    let mut devices = Vec::new();
    if !backend.device.is_host() {
        devices.push(DeviceCodecs {
            device: backend.device.clone(),
            encodes: reg.encodable_on(&backend.device),
            decodes: reg.decodable_on(&backend.device),
        });
    }
    devices.push(host);
    let usable: Vec<forge_core::VideoCodec> = forge_core::VideoCodec::ALL
        .iter()
        .copied()
        .filter(|c| {
            devices.iter().any(|d| d.decodes.contains(c))
                && devices.iter().any(|d| d.encodes.contains(c))
        })
        .collect();
    for d in &mut devices {
        d.encodes.retain(|c| usable.contains(c));
        d.decodes.retain(|c| usable.contains(c));
    }
    NodeCodecs { usable, devices }
}

/// The video codecs the node offers callers: see [`node_codecs`].
pub fn usable_codecs(backend: &VideoBackend, native_only: bool) -> Vec<forge_core::VideoCodec> {
    node_codecs(backend, native_only).usable
}

/// Whether a participant may take the room's content floor under the
/// meeting's `video_content_share` policy: nobody under `off`, hosts
/// under `hosts`, everyone otherwise (§15.6).
pub fn may_share(policy: &str, is_host: bool) -> bool {
    match policy {
        "off" => false,
        "hosts" => is_host,
        _ => true,
    }
}

/// The meeting's video settings as a room would run them, and the codecs
/// to offer, in the meeting's preference order restricted to what the
/// node can run. `None` when the meeting has video off or nothing usable.
/// `content_idle` is the node's `[conference] content_idle_secs`.
pub fn plan_video(
    meeting: &Meeting,
    usable: &[forge_core::VideoCodec],
    content_idle: Duration,
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
        // A meeting that says nothing keeps `Auto`: forward only while
        // the room is showing one participant full canvas, where the
        // missing name banner is least missed (§5.6).
        passthrough: PassthroughMode::parse(&meeting.video_passthrough).unwrap_or_default(),
        // A shared screen (§15.6): whether the room takes one at all, how
        // the composite shows it, and how long a silent presenter keeps
        // the floor. Who may take it is decided per participant when
        // their content leg is attached.
        content: meeting.video_content_share != "off",
        content_layout: ContentLayout::parse(&meeting.video_content_layout).unwrap_or_default(),
        content_idle,
        ..VideoRoomSettings::default()
    };
    Some((settings, codecs))
}

impl VideoEventSink {
    /// The room the events are about.
    pub fn key(&self) -> RoomKey {
        RoomKey::new(self.tenant_id, self.room_number.clone())
    }
}

/// Where the room's video events go.
#[derive(Clone)]
pub struct VideoEventSink {
    pub event_bus: Arc<dyn EventPublisher>,
    pub tenant_id: TenantId,
    pub room_number: String,
    /// This node, named on the events it is the one to emit.
    pub node_id: String,
    /// Rooms with a pump running, so two callers joining at once start
    /// one pump, not two.
    pub pumps: Arc<dashmap::DashMap<RoomKey, ()>>,
    /// Whether the node can afford a screen share (§15.6): the video
    /// budget, asked before the room grants the floor. `None` admits all.
    pub content_gate: Option<Arc<dyn ContentGate>>,
    /// Who tells the participants that can hear it about the floor
    /// (§15.6, 7c): the conference server, which knows which of them
    /// joined through a gateway. `None` tells nobody.
    pub floor: Option<Arc<dyn FloorNotifier>>,
}

/// The media type of the SIP `INFO` that carries a [`FloorNotice`] to a
/// browser's gateway session.
pub use fcp_core::CONTENT_INFO_TYPE;

/// What the room's content floor just did, for the participants that
/// can hear it (§15.6, 7c). The conference server sends one to every
/// participant that joined with a join token — a browser through the
/// gateway — as a SIP `INFO` of [`CONTENT_INFO_TYPE`], adding `mine`
/// for the recipient.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FloorNotice {
    /// `granted` (someone took the floor), `released` (they gave it up
    /// or lost it) or `refused` (their share was turned down).
    pub floor: &'static str,
    /// The participant the notice is about.
    pub participant_id: String,
    /// The presenter's display name while someone holds the floor:
    /// the new presenter on `granted`, the holder on a `refused` that
    /// was refused because the floor is held.
    pub presenter: Option<String>,
    /// Why, on `released` (the [`ContentStop`](forge_conference::video::ContentStop)'s
    /// name) and `refused` (the [`ContentRefusal`](forge_conference::video::ContentRefusal)'s).
    pub reason: Option<String>,
}

/// Where floor notices go.
pub trait FloorNotifier: Send + Sync {
    /// The floor of `room_number` changed. A `refused` notice concerns
    /// only the participant it names.
    fn floor_changed(&self, key: &RoomKey, notice: FloorNotice);
}

/// The notice one of the room's content events is, if any. `content`
/// is the room's floor as it stands after the event.
pub fn floor_notice(
    participant_id: &str,
    event: &ContentEvent,
    content: Option<&ContentInfo>,
) -> FloorNotice {
    let presenter = content.map(|c| c.display_name.clone());
    match event {
        ContentEvent::Started => FloorNotice {
            floor: "granted",
            participant_id: participant_id.to_string(),
            presenter: presenter.or_else(|| Some(participant_id.to_string())),
            reason: None,
        },
        ContentEvent::Stopped(why) => FloorNotice {
            floor: "released",
            participant_id: participant_id.to_string(),
            presenter: None,
            reason: Some(why.name().to_string()),
        },
        ContentEvent::Refused(why) => FloorNotice {
            floor: "refused",
            participant_id: participant_id.to_string(),
            presenter,
            reason: Some(why.name().to_string()),
        },
    }
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
    video.set_local_node(&sink.node_id);
    video.set_content_gate(sink.content_gate.clone());
    if sink.pumps.insert(sink.key(), ()).is_none() {
        spawn_event_pump(&video, sink);
    }
    if let Some(name) = display_name {
        video.set_display_name(participant_id, name);
    }
    // The sender's own fmtp travels with the source: forwarding its
    // stream to somebody else depends on it (§5.6).
    let profile = leg.negotiated.fmtp.clone().unwrap_or_default();
    // A camera sending simulcast is a layer per rid, of which the room
    // decodes the smallest that fills its tile (`docs/FORWARDING_ROOMS.md`
    // §2.4).
    let simulcast = leg.negotiated.simulcast.as_ref().map(|layers| Simulcast {
        rids: layers.rids.iter().map(|r| r.id.to_string()).collect(),
        rid_extension: layers.rid_extension,
    });
    if let Err(e) = video.add_source_layered(
        participant_id,
        StreamKind::Camera,
        leg.negotiated.codec,
        &profile,
        simulcast.as_ref(),
    ) {
        warn!(participant = %participant_id, error = %e, "could not add video source");
        return None;
    }
    // A caller with a content section of their own — attached first, so
    // it is known here — gets people on this section and the slides on
    // the other; a caller with one section gets the composite, which
    // follows a share into the presentation (§15.6).
    // A forwarding participant is sent each person's own stream, all in
    // this one section, as the room's allocator picks them
    // (`docs/FORWARDING_ROOMS.md` §2.2).
    if leg.forward {
        let fwd = match video.forward(
            participant_id,
            ForwardRequest {
                codec: leg.negotiated.codec,
                profile: leg.negotiated.fmtp.clone().unwrap_or_default(),
                payload_type: leg.negotiated.payload_type,
            },
        ) {
            Ok(f) => f,
            Err(e) => {
                warn!(participant = %participant_id, error = %e, "could not forward room video");
                video.remove_source(participant_id);
                return None;
            }
        };
        return Some(run_video_bridge(
            video,
            participant_id.to_string(),
            StreamKind::Camera,
            Arc::clone(&leg.socket),
            leg.srtp_context.clone(),
            Outbound::Streams(fwd.packets),
            leg.negotiated.clone(),
            Arc::clone(&leg.stats),
        ));
    }
    let view = video
        .has_subscriber_kind(participant_id, StreamKind::Content)
        .then_some(forge_conference::video::View::Cameras);
    let sub = match video.subscribe(
        participant_id,
        SubscribeRequest {
            codec: leg.negotiated.codec,
            profile: leg.negotiated.fmtp.clone().unwrap_or_default(),
            payload_type: leg.negotiated.payload_type,
            resolution: None,
            fps: None,
            max_kbps: None,
            // A caller sees the whole room, this node's callers and the
            // peers' tiles alike; the room's `exclude_self` decides
            // whether their own tile is in it.
            scope: None,
            view,
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
        StreamKind::Camera,
        Arc::clone(&leg.socket),
        leg.srtp_context.clone(),
        sub.into(),
        leg.negotiated.clone(),
        Arc::clone(&leg.stats),
    ))
}

/// Put a participant's shared screen on the room's video (§15.6): a
/// content source, decoded but shown only once its packets take the
/// floor, and a content channel back, on which the participant receives
/// whoever's screen the room is showing (never their own). `may_share`
/// is the meeting's policy for this participant: a guest under `hosts`
/// still has the section answered — a phone that offered one expects an
/// answer — but is never given the floor, and is told so once.
pub fn attach_content(
    room: &Arc<ConferenceRoom>,
    backend: &VideoBackend,
    participant_id: &str,
    leg: &VideoLeg,
    sink: VideoEventSink,
    may_share: bool,
) -> Option<impl Future<Output = ()> + Send + 'static> {
    let video = room.enable_video(leg.settings.clone(), backend);
    video.set_local_node(&sink.node_id);
    video.set_content_gate(sink.content_gate.clone());
    if sink.pumps.insert(sink.key(), ()).is_none() {
        spawn_event_pump(&video, sink);
    }
    let profile = leg.negotiated.fmtp.clone().unwrap_or_default();
    if let Err(e) = video.add_source_kind(
        participant_id,
        StreamKind::Content,
        leg.negotiated.codec,
        &profile,
    ) {
        warn!(participant = %participant_id, error = %e, "could not add content source");
        return None;
    }
    if !may_share {
        video.set_participant_content_enabled(participant_id, false);
    }
    let sub = match video.subscribe_kind(
        participant_id,
        StreamKind::Content,
        SubscribeRequest {
            codec: leg.negotiated.codec,
            profile,
            payload_type: leg.negotiated.payload_type,
            resolution: None,
            fps: None,
            max_kbps: None,
            scope: None,
            // The content alone, as a content section expects.
            view: None,
        },
    ) {
        Ok(s) => s,
        Err(e) => {
            warn!(participant = %participant_id, error = %e, "could not subscribe to room content");
            video.remove_source_kind(participant_id, StreamKind::Content);
            return None;
        }
    };
    Some(run_video_bridge(
        video,
        participant_id.to_string(),
        StreamKind::Content,
        Arc::clone(&leg.socket),
        leg.srtp_context.clone(),
        sub.into(),
        leg.negotiated.clone(),
        Arc::clone(&leg.stats),
    ))
}

/// Put an inter-node trunk on the room's video: the peer's composite
/// becomes one remote tile here, and what goes back down the trunk is
/// this node's callers only.
///
/// The two halves are the anti-echo rule of the audio mix, drawn: a
/// composite sent to a peer must carry no tile that came from a peer, or
/// a room on three nodes would show the same caller twice — once
/// directly and once forwarded (`docs/DISTRIBUTED_CONFERENCING.md`).
pub fn attach_trunk_video(
    room: &Arc<ConferenceRoom>,
    backend: &VideoBackend,
    participant_id: &str,
    peer_node: &str,
    leg: &VideoLeg,
    sink: VideoEventSink,
) -> Option<impl Future<Output = ()> + Send + 'static> {
    let video = room.enable_video(leg.settings.clone(), backend);
    // Named so an exact tie between this node's caller and a peer's
    // claim is settled the same way here and there (§15.4, 5c).
    video.set_local_node(&sink.node_id);
    if sink.pumps.insert(sink.key(), ()).is_none() {
        spawn_event_pump(&video, sink);
    }
    // The tile has to exist before it can be named: unlike a caller,
    // nothing put it in the room ahead of its video.
    let profile = leg.negotiated.fmtp.clone().unwrap_or_default();
    if let Err(e) = video.add_remote(participant_id, leg.negotiated.codec, &profile) {
        warn!(trunk = %participant_id, error = %e, "could not add the peer's video tile");
        return None;
    }
    video.set_display_name(participant_id, peer_node);
    let sub = match video.subscribe(
        participant_id,
        SubscribeRequest {
            codec: leg.negotiated.codec,
            profile: leg.negotiated.fmtp.clone().unwrap_or_default(),
            payload_type: leg.negotiated.payload_type,
            resolution: None,
            fps: None,
            max_kbps: None,
            scope: Some(OutputScope::LocalOnly),
            view: None,
        },
    ) {
        Ok(s) => s,
        Err(e) => {
            warn!(trunk = %participant_id, error = %e, "could not compose for the trunk");
            video.remove_remote(participant_id);
            return None;
        }
    };
    Some(run_video_bridge(
        video,
        participant_id.to_string(),
        StreamKind::Camera,
        Arc::clone(&leg.socket),
        leg.srtp_context.clone(),
        sub.into(),
        leg.negotiated.clone(),
        Arc::clone(&leg.stats),
    ))
}

/// Put a trunk's content section on the room's video (§15.6, 7d): the
/// peer's shared screen becomes a remote content source under the
/// trunk's tile id — taking the floor here as a caller's would — and
/// what goes back down the trunk is this node's own presenter's screen,
/// `LocalOnly`, so a screen that came from a peer is never sent back.
pub fn attach_trunk_content(
    room: &Arc<ConferenceRoom>,
    backend: &VideoBackend,
    participant_id: &str,
    leg: &VideoLeg,
    sink: VideoEventSink,
) -> Option<impl Future<Output = ()> + Send + 'static> {
    let video = room.enable_video(leg.settings.clone(), backend);
    video.set_local_node(&sink.node_id);
    video.set_content_gate(sink.content_gate.clone());
    if sink.pumps.insert(sink.key(), ()).is_none() {
        spawn_event_pump(&video, sink);
    }
    let profile = leg.negotiated.fmtp.clone().unwrap_or_default();
    if let Err(e) = video.add_remote_content(participant_id, leg.negotiated.codec, &profile) {
        warn!(trunk = %participant_id, error = %e, "could not add the peer's content source");
        return None;
    }
    let sub = match video.subscribe_kind(
        participant_id,
        StreamKind::Content,
        SubscribeRequest {
            codec: leg.negotiated.codec,
            profile,
            payload_type: leg.negotiated.payload_type,
            resolution: None,
            fps: None,
            max_kbps: None,
            scope: Some(OutputScope::LocalOnly),
            view: Some(View::Content),
        },
    ) {
        Ok(s) => s,
        Err(e) => {
            warn!(trunk = %participant_id, error = %e, "could not send content down the trunk");
            video.remove_remote_content(participant_id);
            return None;
        }
    };
    Some(run_video_bridge(
        video,
        participant_id.to_string(),
        StreamKind::Content,
        Arc::clone(&leg.socket),
        leg.srtp_context.clone(),
        sub.into(),
        leg.negotiated.clone(),
        Arc::clone(&leg.stats),
    ))
}

/// One share a room hosted on several nodes agrees on (§15.6, 7d): of
/// two claims to the floor, the earlier wins, and an exact tie falls to
/// the lower node id, then the lower participant id — the same answer on
/// every node, whichever order the claims arrived in.
pub fn claim_beats(challenger: (u64, &str, &str), holder: (u64, &str, &str)) -> bool {
    challenger < holder
}

/// Take the participant's shared screen off the room's video (a
/// re-INVITE removed the section, or the leg is torn down). Ends their
/// share if it was on.
pub fn detach_content(room: &ConferenceRoom, participant_id: &str) {
    if let Some(video) = room.video() {
        video.unsubscribe_kind(participant_id, StreamKind::Content);
        video.remove_source_kind(participant_id, StreamKind::Content);
    }
}

/// Take the participant off the room's video (a re-INVITE removed the
/// section, or the leg is torn down before the participant leaves). A
/// trunk's remote tile goes the same way, tile and all.
pub fn detach_video(room: &ConferenceRoom, participant_id: &str) {
    if let Some(video) = room.video() {
        video.unsubscribe(participant_id);
        if video.is_remote(participant_id) {
            video.remove_remote(participant_id);
        } else {
            video.remove_source(participant_id);
        }
    }
}

/// Forward the room's video events to the platform bus until the room's
/// video stops.
fn spawn_event_pump(video: &Arc<VideoRoom>, sink: VideoEventSink) {
    let mut events = video.events();
    // Weak, so the pump never keeps the room alive: it ends when the
    // room's event channel closes, which is when the room goes.
    let room = Arc::downgrade(video);
    tokio::spawn(async move {
        struct Done(Arc<dashmap::DashMap<RoomKey, ()>>, RoomKey);
        impl Drop for Done {
            fn drop(&mut self) {
                self.0.remove(&self.1);
            }
        }
        let _done = Done(Arc::clone(&sink.pumps), sink.key());
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(room = %sink.room_number, skipped = n, "video events lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            // The room's rate as it stands: a shed leaves it alone, and
            // the event says so.
            let fps = room.upgrade().map(|r| r.fps()).unwrap_or(0);
            let content = room.upgrade().and_then(|r| r.content());
            if let (
                Some(notifier),
                VideoRoomEvent::Content {
                    participant_id,
                    event,
                },
            ) = (&sink.floor, &ev)
            {
                notifier.floor_changed(
                    &sink.key(),
                    floor_notice(participant_id, event, content.as_ref()),
                );
            }
            if let Some(event) = conference_event(ev, &sink, fps, content.as_ref()) {
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

/// What one of the room's video events is on the platform bus, if
/// anything. `room_fps` is the room's frame rate as it stands.
fn conference_event(
    ev: VideoRoomEvent,
    sink: &VideoEventSink,
    room_fps: u32,
    content: Option<&forge_conference::video::ContentInfo>,
) -> Option<ConferenceEvent> {
    match ev {
        // A trunk's tile is infrastructure, not a participant: it is left
        // out of the participant-facing events, as a trunk is left out of
        // join and leave.
        VideoRoomEvent::ParticipantState {
            participant_id,
            kind,
            state,
        } if !participant_id.starts_with("__") => Some(ConferenceEvent::VideoStateChanged {
            room_number: sink.room_number.clone(),
            participant_id,
            state: state.name().to_string(),
            stream: kind.name().to_string(),
            tenant_id: sink.tenant_id,
            timestamp: SystemTime::now(),
        }),
        VideoRoomEvent::ParticipantState { .. } => None,
        // The floor changing hands (§15.6). A refusal is the sender's
        // business, not the room's: 7c tells a browser through its
        // gateway session, and a phone with BFCP hears it there (7e).
        VideoRoomEvent::Content {
            participant_id,
            event: ContentEvent::Started,
        } if !participant_id.starts_with("__") => Some(ConferenceEvent::ContentStarted {
            room_number: sink.room_number.clone(),
            participant_id,
            node_id: sink.node_id.clone(),
            codec: content
                .map(|c| c.codec.sdp_name().to_string())
                .unwrap_or_default(),
            tenant_id: sink.tenant_id,
            timestamp: SystemTime::now(),
        }),
        VideoRoomEvent::Content {
            participant_id,
            event: ContentEvent::Stopped(reason),
        } if !participant_id.starts_with("__") => Some(ConferenceEvent::ContentStopped {
            room_number: sink.room_number.clone(),
            participant_id,
            node_id: sink.node_id.clone(),
            reason: reason.name().to_string(),
            tenant_id: sink.tenant_id,
            timestamp: SystemTime::now(),
        }),
        VideoRoomEvent::Content {
            participant_id,
            event,
        } => {
            debug!(room = %sink.room_number, participant = %participant_id, ?event, "screen share");
            None
        }
        // The last resort: shedding had nothing left to give (§9.2).
        VideoRoomEvent::FpsChanged {
            from,
            to,
            overload: true,
        } => Some(ConferenceEvent::VideoOverload {
            room_number: sink.room_number.clone(),
            action: "fps".to_string(),
            from_fps: from,
            to_fps: to,
            output: None,
            from_resolution: None,
            to_resolution: None,
            tenant_id: sink.tenant_id,
            timestamp: SystemTime::now(),
        }),
        VideoRoomEvent::FpsChanged { from, to, .. } => {
            info!(room = %sink.room_number, from, to, "video frame rate restored");
            None
        }
        // A rung given back once the room keeps up again: a recovery,
        // logged as a restored frame rate is rather than published.
        VideoRoomEvent::Shed { output, from, to } if to.height >= from.height => {
            info!(room = %sink.room_number, %output, %from, %to, "video output given a rung back");
            None
        }
        // What the room gives up first (§9.2): one composite drops a rung
        // of the ladder and everyone's motion is left alone.
        VideoRoomEvent::Shed { output, from, to } => Some(ConferenceEvent::VideoOverload {
            room_number: sink.room_number.clone(),
            action: "shed".to_string(),
            from_fps: room_fps,
            to_fps: room_fps,
            output: Some(output),
            from_resolution: Some(from.to_string()),
            to_resolution: Some(to.to_string()),
            tenant_id: sink.tenant_id,
            timestamp: SystemTime::now(),
        }),
        // One emitter per change: the node the speaker is on. A speaker
        // reached through a remote tile is a peer's caller, and that peer
        // announces them, so a listener sees one change rather than one
        // per hosting node (§15.4, 5c).
        VideoRoomEvent::ActiveSpeaker {
            participant_id,
            via,
        } => {
            debug!(room = %sink.room_number, speaker = ?participant_id, ?via, "active speaker");
            match (participant_id, via) {
                (Some(participant_id), None) => Some(ConferenceEvent::ActiveSpeakerChanged {
                    room_number: sink.room_number.clone(),
                    participant_id,
                    node_id: sink.node_id.clone(),
                    tenant_id: sink.tenant_id,
                    timestamp: SystemTime::now(),
                }),
                _ => None,
            }
        }
        // What a forwarding participant is sent: their roster says, from
        // the room as it stands, at its next update.
        VideoRoomEvent::Streams {
            participant_id,
            streams,
        } => {
            debug!(room = %sink.room_number, participant = %participant_id,
                   streams = streams.len(), "forwarded streams changed");
            None
        }
        VideoRoomEvent::LayoutChanged {
            layout,
            pinned,
            spotlight,
            content_layout: _,
        } => Some(ConferenceEvent::VideoLayoutChanged {
            room_number: sink.room_number.clone(),
            layout: layout.name().to_string(),
            pinned,
            spotlight,
            tenant_id: sink.tenant_id,
            timestamp: SystemTime::now(),
        }),
    }
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
    kind: StreamKind,
    socket: Arc<RtpSocketPair>,
    srtp_context: Option<Arc<RwLock<SrtpContext>>>,
    outbound: Outbound,
    negotiated: NegotiatedVideo,
    stats: Arc<VideoLegStats>,
) {
    let (ssrc, mut packets) = match outbound {
        Outbound::Composite(sub) => (Some(sub.ssrc), sub.packets),
        Outbound::Streams(packets) => (None, packets),
    };
    let span = info_span!(
        "video_bridge",
        participant = %participant_id,
        room = %room.id(),
        stream = %kind,
        codec = %negotiated.codec,
        srtp = srtp_context.is_some(),
    );
    async move {
        info!(
            payload_type = negotiated.payload_type,
            pli = negotiated.pli,
            fir = negotiated.fir,
            nack = negotiated.nack,
            ssrc = ?ssrc,
            forwarding = ssrc.is_none(),
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
                    for fb in room.push_rtp_kind(&participant_id, kind, packet) {
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
                        room.handle_feedback_kind(&participant_id, kind, &packet);
                    }
                }

                // Outbound: the composite for this participant.
                next = packets.recv() => {
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

/// The room's form of a participant's `fcp-view` INFO.
pub fn view_request(view: &fcp_core::video_view::View) -> ViewRequest {
    ViewRequest {
        streams: view.streams.as_ref().map(|streams| {
            streams
                .iter()
                .map(|s| StreamWant {
                    target: match &s.participant {
                        Some(p) => StreamTarget::Participant(p.clone()),
                        None => StreamTarget::Speaker,
                    },
                    max_height: s.max_height,
                    priority: s.priority,
                })
                .collect()
        }),
        max_streams: view.max_streams,
    }
}

/// What a video leg sends: the composite under its one SSRC, or a
/// forwarding participant's streams (`docs/FORWARDING_ROOMS.md` §2.2).
pub enum Outbound {
    Composite(VideoSubscription),
    Streams(tokio::sync::mpsc::Receiver<Bytes>),
}

impl From<VideoSubscription> for Outbound {
    fn from(sub: VideoSubscription) -> Self {
        Outbound::Composite(sub)
    }
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
    use fcp_events::{EventFilter, EventSubscriber, InMemoryEventBus};
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
            video_passthrough: fcp_core::meeting::default_video_passthrough(),
            video_content_share: "anyone".to_string(),
            video_content_layout: "presentation".to_string(),
            video_mode: fcp_core::meeting::default_video_mode(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn a_plan_follows_the_meeting_and_the_usable_codecs() {
        let usable = vec![forge_core::VideoCodec::VP8, forge_core::VideoCodec::VP9];
        let (settings, codecs) =
            plan_video(&meeting(true), &usable, Duration::from_secs(30)).expect("video");
        assert_eq!(codecs, vec![forge_core::VideoCodec::VP8]);
        assert_eq!(settings.layout, Layout::ActiveSpeaker);
        assert_eq!(settings.resolution, Resolution::new(640, 360));
        assert_eq!(settings.fps, 30);
        assert!(plan_video(&meeting(false), &usable, Duration::from_secs(30)).is_none());
        assert!(
            plan_video(
                &meeting(true),
                &[forge_core::VideoCodec::H265],
                Duration::from_secs(30)
            )
            .is_none(),
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
    fn a_device_s_codecs_count_where_it_runs_them_and_the_host_fills_in() {
        use forge_video::device::DeviceBackend;
        use forge_video::testing::{fake_registry, FakeBackend};
        // A device that decodes three codecs and encodes one, like an
        // NVIDIA GPU (NVDEC has VP8 and VP9, NVENC has neither).
        let hw = Arc::new(FakeBackend::new("0"));
        let device = hw.device();
        let registry = fake_registry(
            &device,
            &[
                forge_core::VideoCodec::H264,
                forge_core::VideoCodec::VP8,
                forge_core::VideoCodec::VP9,
            ],
            &[forge_core::VideoCodec::H264],
        );
        let backend = VideoBackend::on_device(
            registry,
            forge_conference::video::CodecPool::new(1),
            hw as Arc<dyn forge_video::device::DeviceBackend>,
        );
        let codecs = node_codecs(&backend, false);
        assert_eq!(
            codecs.usable.len(),
            5,
            "the host's raw codec fills in for the rest"
        );
        assert_eq!(codecs.devices.len(), 2);
        let dev = codecs.on(&device).unwrap();
        assert_eq!(dev.encodes, vec![forge_core::VideoCodec::H264]);
        assert_eq!(dev.decodes.len(), 3);
        let host = codecs.on(&MediaDevice::Host).unwrap();
        assert_eq!(host.encodes.len(), 5);
        // Native only: the host's raw codec is out; what is left is the
        // device's own codecs where it both decodes and encodes them —
        // and, in this test build without native bindings, nothing else.
        let native = node_codecs(&backend, true);
        if forge_video_codecs::enabled_backends().is_empty() {
            assert_eq!(native.usable, vec![forge_core::VideoCodec::H264]);
            assert!(native.on(&MediaDevice::Host).unwrap().encodes.is_empty());
            assert_eq!(
                native.on(&device).unwrap().decodes,
                vec![forge_core::VideoCodec::H264]
            );
        }
    }

    #[test]
    fn a_shed_is_an_overload_and_a_rung_given_back_is_not() {
        let sink = VideoEventSink {
            event_bus: Arc::new(InMemoryEventBus::new()) as Arc<dyn EventPublisher>,
            tenant_id: TenantId::new(),
            room_number: "1000".into(),
            node_id: "conf-a".into(),
            pumps: Arc::new(dashmap::DashMap::new()),
            content_gate: None,
            floor: None,
        };
        let hd = Resolution::new(1280, 720);
        let sd = Resolution::new(640, 360);

        // What the room gives up first: one composite a rung smaller, and
        // the room's frame rate left where it was.
        let shed = VideoRoomEvent::Shed {
            output: "all".into(),
            from: hd,
            to: sd,
        };
        match conference_event(shed, &sink, 15, None) {
            Some(ConferenceEvent::VideoOverload {
                action,
                from_fps,
                to_fps,
                output,
                from_resolution,
                to_resolution,
                ..
            }) => {
                assert_eq!(action, "shed");
                assert_eq!((from_fps, to_fps), (15, 15));
                assert_eq!(output.as_deref(), Some("all"));
                assert_eq!(from_resolution.as_deref(), Some("1280x720"));
                assert_eq!(to_resolution.as_deref(), Some("640x360"));
            }
            other => panic!("a shed is a video_overload, got {other:?}"),
        }

        // The same event on the way back up is a recovery, and recoveries
        // are logged rather than published.
        let back = VideoRoomEvent::Shed {
            output: "all".into(),
            from: sd,
            to: hd,
        };
        assert!(conference_event(back, &sink, 15, None).is_none());

        // Once shedding has nothing left, the rate halves: no composite named.
        let halved = VideoRoomEvent::FpsChanged {
            from: 15,
            to: 7,
            overload: true,
        };
        match conference_event(halved, &sink, 7, None) {
            Some(ConferenceEvent::VideoOverload {
                action,
                from_fps,
                to_fps,
                output,
                ..
            }) => {
                assert_eq!(action, "fps");
                assert_eq!((from_fps, to_fps), (15, 7));
                assert!(output.is_none());
            }
            other => panic!("a halved rate is a video_overload, got {other:?}"),
        }
        let restored = VideoRoomEvent::FpsChanged {
            from: 7,
            to: 15,
            overload: false,
        };
        assert!(conference_event(restored, &sink, 15, None).is_none());
    }

    #[test]
    fn feedback_is_adapted_to_what_the_peer_negotiated() {
        let mut n = NegotiatedVideo {
            label: None,
            codec: forge_core::VideoCodec::VP8,
            payload_type: 97,
            fmtp: None,
            remote: None,
            pli: false,
            fir: true,
            nack: false,
            remote_crypto: None,
            simulcast: None,
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

    /// A phone that sends solid raw frames of one size and shade forever.
    fn send_frames(
        phone: Arc<RtpSocketPair>,
        width: u32,
        height: u32,
        luma: u8,
        ssrc: u32,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let enc_settings = EncoderSettings {
                codec: forge_core::VideoCodec::VP8,
                resolution: Resolution::new(width, height),
                fps: 15,
                bitrate_kbps: 300,
                keyframe_interval: 30,
                profile: String::new(),
                content: Default::default(),
            };
            let mut enc: Box<dyn VideoEncoder> = raw_registry()
                .encoder(&enc_settings, &MediaDevice::Host)
                .unwrap();
            let mut seq = 1u16;
            for i in 0u32.. {
                let f = HostFrame::solid(width, height, luma, 128, 128).with_pts(i * 6000);
                for c in enc.encode(&VideoFrame::Host(f), false).unwrap() {
                    let payloads = packetize(forge_core::VideoCodec::VP8, &c, 1200).unwrap();
                    let n = payloads.len();
                    for (k, p) in payloads.into_iter().enumerate() {
                        let pkt = RtpPacket::build(97, seq, c.timestamp, ssrc, p, k + 1 == n);
                        seq = seq.wrapping_add(1);
                        if phone.send_rtp(&pkt).await.is_err() {
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(33)).await;
            }
        })
    }

    /// Decode what a phone receives until a frame passes `check`.
    async fn frame_matching(
        phone: &Arc<RtpSocketPair>,
        what: &str,
        check: impl Fn(&HostFrame) -> bool,
    ) -> HostFrame {
        let mut asm = FrameAssembler::new(forge_core::VideoCodec::VP8);
        let mut dec: Box<dyn VideoDecoder> = raw_registry()
            .decoder(forge_core::VideoCodec::VP8, &MediaDevice::Host)
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while tokio::time::Instant::now() < deadline {
            let (bytes, _) =
                match tokio::time::timeout(Duration::from_secs(1), phone.recv_rtp_raw()).await {
                    Ok(Ok(r)) => r,
                    _ => continue,
                };
            let pkt = RtpPacket::parse(bytes).unwrap();
            for ev in asm.push(pkt) {
                if let AssemblerEvent::Frame(f) = ev {
                    if let Some(v) = dec.decode(&f).unwrap() {
                        let h = v.into_host().unwrap();
                        if check(&h) {
                            return h;
                        }
                    }
                }
            }
        }
        panic!("no frame {what} in time");
    }

    /// One layer of a simulcast camera: solid raw frames of one size and
    /// shade under `ssrc`, the rid in the header extension on the first
    /// frames' packets, as a simulcast sender names its streams.
    fn send_layer(
        phone: Arc<RtpSocketPair>,
        size: (u32, u32),
        luma: u8,
        ssrc: u32,
        rid: &'static str,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let (width, height) = size;
            let mut enc: Box<dyn VideoEncoder> = raw_registry()
                .encoder(
                    &EncoderSettings {
                        codec: forge_core::VideoCodec::VP8,
                        resolution: Resolution::new(width, height),
                        fps: 15,
                        bitrate_kbps: 300,
                        keyframe_interval: 30,
                        profile: String::new(),
                        content: Default::default(),
                    },
                    &MediaDevice::Host,
                )
                .unwrap();
            let mut seq = 1u16;
            for i in 0u32.. {
                let f = HostFrame::solid(width, height, luma, 128, 128).with_pts(i * 6000);
                for c in enc.encode(&VideoFrame::Host(f), false).unwrap() {
                    let payloads = packetize(forge_core::VideoCodec::VP8, &c, 1200).unwrap();
                    let n = payloads.len();
                    for (k, p) in payloads.into_iter().enumerate() {
                        let mut pkt = RtpPacket::build(97, seq, c.timestamp, ssrc, p, k + 1 == n);
                        if i < 3 {
                            pkt.header.version_flags |= 0x10;
                            pkt.extension =
                                Some(forge_rtp::RtpExtension::one_byte(&[(4, rid.as_bytes())]));
                        }
                        seq = seq.wrapping_add(1);
                        if phone.send_rtp(&pkt).await.is_err() {
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(33)).await;
            }
        })
    }

    /// A camera that negotiated simulcast is a source with a layer per rid
    /// (`docs/FORWARDING_ROOMS.md` §2.4): both layers bound to their SSRCs
    /// from the rid on their first packets, only one decoded, and the
    /// composite drawn from it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_simulcast_camera_is_drawn_from_one_of_its_layers() {
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
                m.video_passthrough = "off".into();
                m
            },
            &usable_codecs(&backend, false),
            Duration::from_secs(30),
        )
        .unwrap();
        let sink = VideoEventSink {
            event_bus: Arc::new(InMemoryEventBus::new()) as Arc<dyn EventPublisher>,
            tenant_id: TenantId::new(),
            room_number: "1000".into(),
            node_id: "conf-a".into(),
            pumps: Arc::new(dashmap::DashMap::new()),
            content_gate: None,
            floor: None,
        };
        let lo = |p: u16| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), p);
        let (alice_server, bob_server, alice_phone, bob_phone) =
            (free_pair(), free_pair(), free_pair(), free_pair());
        let alice_srv = socket_pair(alice_server).await;
        let bob_srv = socket_pair(bob_server).await;
        let alice_ph = socket_pair(alice_phone).await;
        let bob_ph = socket_pair(bob_phone).await;
        for (srv, phone) in [(&alice_srv, alice_phone), (&bob_srv, bob_phone)] {
            srv.set_remote_endpoint(RemoteEndpoint {
                rtp_addr: lo(phone),
                rtcp_addr: Some(lo(phone + 1)),
            })
            .await;
        }
        alice_ph
            .set_remote_endpoint(RemoteEndpoint {
                rtp_addr: lo(alice_server),
                rtcp_addr: Some(lo(alice_server + 1)),
            })
            .await;

        // What answering Alice's simulcast offer agreed, as the handler
        // answers a participant's camera.
        let answer = fcp_media::construct_answer(
            "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
             m=audio 4000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n\
             m=video 4002 RTP/AVP 97\r\na=rtpmap:97 VP8/90000\r\n\
             a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id\r\n\
             a=rid:l send\r\na=rid:h send\r\na=simulcast:send l;h\r\n",
            &fcp_media::AnswerOptions {
                server_ip: "127.0.0.1",
                audio_rtp_port: 5000,
                audio_srtp: None,
                video: Some(fcp_media::VideoAnswer {
                    rtp_port: alice_server,
                    codecs: &[forge_core::VideoCodec::VP8],
                    srtp: None,
                    simulcast: 3,
                }),
                content: None,
                bfcp: None,
            },
        )
        .unwrap();
        assert!(
            answer.sdp.contains("a=simulcast:recv l;h"),
            "{}",
            answer.sdp
        );
        let layers = answer.video.and_then(|v| v.simulcast);
        assert!(layers.is_some());
        let plain = NegotiatedVideo {
            label: None,
            codec: forge_core::VideoCodec::VP8,
            payload_type: 97,
            fmtp: None,
            remote: None,
            pli: true,
            fir: false,
            nack: true,
            remote_crypto: None,
            simulcast: None,
        };
        let mut tasks = Vec::new();
        for (pid, srv, pair, simulcast) in [
            ("alice", &alice_srv, alice_server, layers),
            ("bob", &bob_srv, bob_server, None),
        ] {
            let leg = VideoLeg {
                kind: StreamKind::Camera,
                port_pair: PortPair::new(pair).unwrap(),
                socket: Arc::clone(srv),
                srtp_context: None,
                srtp_sdp: None,
                negotiated: NegotiatedVideo {
                    simulcast,
                    ..plain.clone()
                },
                settings: settings.clone(),
                stats: Arc::new(VideoLegStats::default()),
                forward: false,
                bridge_handle: None,
            };
            let fut = attach_video(&room, &backend, pid, Some(pid), &leg, sink.clone()).unwrap();
            tasks.push(tokio::spawn(fut));
        }

        // The large layer bright, the small one dark. The small one is
        // listed first, so the room starts on it; once it has measured both
        // it ranks them by bitrate (the raw codec has no header to read a
        // size from) and moves up to the large one for Alice's tile.
        let high = send_layer(Arc::clone(&alice_ph), (160, 90), 200, 0x1111, "h");
        let low = send_layer(Arc::clone(&alice_ph), (80, 46), 40, 0x2222, "l");
        // A 2×1 grid at 320×180: Alice's tile on the left.
        frame_matching(&bob_ph, "drawn from the large layer", |f| {
            f.luma(80, 90) == 200
        })
        .await;

        let info = room.video().unwrap().participant("alice").unwrap();
        let src = info.source.expect("alice sends");
        let bound: Vec<(Option<&str>, u32, bool)> = src
            .layers
            .iter()
            .map(|l| (l.rid.as_deref(), l.ssrc, l.decoding))
            .collect();
        assert_eq!(
            bound,
            [(Some("l"), 0x2222, false), (Some("h"), 0x1111, true)]
        );
        let mut json = serde_json::json!({});
        source_json(&mut json, &src);
        assert_eq!(json["layers"][0]["rid"], "l");
        assert_eq!(json["layers"][1]["decoding"], true);
        assert_eq!(json["layers"][1]["ssrc"], 0x1111);

        high.abort();
        low.abort();
        for t in tasks {
            t.abort();
        }
    }

    /// A shared screen through real sockets (§15.6): Alice's screen takes
    /// the floor; Bob's one-section phone gets the presentation; Carol's
    /// two-section phone gets people on one section and the slides —
    /// Alice's own stream, forwarded — on the other, while her own content
    /// section, a guest's under `hosts`, is answered but never given the
    /// floor.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_shared_screen_reaches_every_kind_of_endpoint() {
        let room = Arc::new(
            ConferenceRoom::new(
                "1001",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        room.add_participant("alice", true).unwrap();
        room.add_participant("bob", false).unwrap();
        room.add_participant("carol", false).unwrap();
        let backend = VideoBackend::raw();
        let meeting = {
            let mut m = meeting(true);
            m.video_layout = "grid".into();
            m.video_max_resolution = "180p".into();
            m.video_codecs = vec![VideoCodec::Vp8];
            m.video_passthrough = "auto".into();
            m.video_content_share = "hosts".into();
            m
        };
        let (settings, _) = plan_video(
            &meeting,
            &usable_codecs(&backend, false),
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(settings.content);
        assert!(may_share(&meeting.video_content_share, true));
        assert!(!may_share(&meeting.video_content_share, false));
        let bus = Arc::new(InMemoryEventBus::new());
        // What the conference server would turn into floor INFOs (7c).
        #[derive(Default)]
        struct Notices(std::sync::Mutex<Vec<(String, FloorNotice)>>);
        impl FloorNotifier for Notices {
            fn floor_changed(&self, key: &RoomKey, notice: FloorNotice) {
                self.0.lock().unwrap().push((key.room.clone(), notice));
            }
        }
        let notices = Arc::new(Notices::default());
        let sink = VideoEventSink {
            event_bus: bus.clone() as Arc<dyn EventPublisher>,
            tenant_id: TenantId::new(),
            room_number: "1001".into(),
            node_id: "conf-a".into(),
            pumps: Arc::new(dashmap::DashMap::new()),
            content_gate: None,
            floor: Some(notices.clone() as Arc<dyn FloorNotifier>),
        };
        let lo = |p: u16| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), p);

        // Five legs: Alice's camera and screen, Bob's camera, Carol's
        // camera and screen. Each has a server socket and a phone.
        struct Wire {
            pid: &'static str,
            kind: StreamKind,
            server: u16,
            srv: Arc<RtpSocketPair>,
            phone: Arc<RtpSocketPair>,
        }
        let mut wires = Vec::new();
        for (pid, kind) in [
            ("alice", StreamKind::Content),
            ("carol", StreamKind::Content),
            ("alice", StreamKind::Camera),
            ("bob", StreamKind::Camera),
            ("carol", StreamKind::Camera),
        ] {
            let server = free_pair();
            let phone_port = free_pair();
            let srv = socket_pair(server).await;
            let phone = socket_pair(phone_port).await;
            srv.set_remote_endpoint(RemoteEndpoint {
                rtp_addr: lo(phone_port),
                rtcp_addr: Some(lo(phone_port + 1)),
            })
            .await;
            phone
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: lo(server),
                    rtcp_addr: Some(lo(server + 1)),
                })
                .await;
            wires.push(Wire {
                pid,
                kind,
                server,
                srv,
                phone,
            });
        }
        let negotiated = NegotiatedVideo {
            label: None,
            codec: forge_core::VideoCodec::VP8,
            payload_type: 97,
            fmtp: None,
            remote: None,
            pli: true,
            fir: false,
            nack: true,
            remote_crypto: None,
            simulcast: None,
        };
        // Content legs are attached before camera legs, as the handler
        // does, so a two-section phone's main section is the cameras.
        let mut bridges = Vec::new();
        for w in &wires {
            let leg = VideoLeg {
                kind: w.kind,
                port_pair: PortPair::new(w.server).unwrap(),
                socket: Arc::clone(&w.srv),
                srtp_context: None,
                srtp_sdp: None,
                negotiated: negotiated.clone(),
                settings: settings.clone(),
                stats: Arc::new(VideoLegStats::default()),
                forward: false,
                bridge_handle: None,
            };
            let fut: std::pin::Pin<Box<dyn Future<Output = ()> + Send>> = match w.kind {
                StreamKind::Content => Box::pin(
                    attach_content(
                        &room,
                        &backend,
                        w.pid,
                        &leg,
                        sink.clone(),
                        may_share(&meeting.video_content_share, w.pid == "alice"),
                    )
                    .unwrap(),
                ),
                StreamKind::Camera => Box::pin(
                    attach_video(&room, &backend, w.pid, Some(w.pid), &leg, sink.clone()).unwrap(),
                ),
            };
            bridges.push((tokio::spawn(fut), leg));
        }
        let video = room.video().expect("video room");
        let phone_of = |pid: &str, kind: StreamKind| {
            Arc::clone(
                &wires
                    .iter()
                    .find(|w| w.pid == pid && w.kind == kind)
                    .unwrap()
                    .phone,
            )
        };
        // Carol's main section shows the cameras; Bob's the composite.
        assert_eq!(
            video
                .participant("carol")
                .unwrap()
                .subscription
                .unwrap()
                .view,
            forge_conference::video::View::Cameras
        );
        assert_eq!(
            video.participant("bob").unwrap().subscription.unwrap().view,
            forge_conference::video::View::Composite
        );

        let senders = [
            send_frames(phone_of("alice", StreamKind::Camera), 128, 72, 200, 0xA11CE),
            send_frames(phone_of("alice", StreamKind::Content), 160, 90, 90, 0x5C4EE),
            send_frames(phone_of("carol", StreamKind::Content), 160, 90, 30, 0xCA401),
        ];

        // Bob's one section: the presentation, Alice's screen in the main
        // region and no screen in the strip.
        let bob_phone = phone_of("bob", StreamKind::Camera);
        let frame = frame_matching(&bob_phone, "with the screen", |h| {
            h.resolution() == Resolution::new(320, 180) && h.luma(118, 90) == 90
        })
        .await;
        assert_ne!(
            frame.luma(280, 90),
            90,
            "the strip holds cameras, not the screen"
        );
        assert_eq!(video.content_holder().as_deref(), Some("alice"));
        assert!(video.participant("alice").unwrap().presenting);

        // Carol's content section: Alice's own stream, at its own size.
        let carol_content = phone_of("carol", StreamKind::Content);
        let frame = frame_matching(&carol_content, "on carol's content section", |h| {
            h.luma(80, 45) == 90
        })
        .await;
        assert_eq!(
            frame.resolution(),
            Resolution::new(160, 90),
            "forwarded, not composed"
        );
        // Carol's main section: the cameras in a grid, never the screen.
        let carol_main = phone_of("carol", StreamKind::Camera);
        let frame = frame_matching(&carol_main, "with alice's camera", |h| {
            h.resolution() == Resolution::new(320, 180) && h.luma(79, 44) == 200
        })
        .await;
        assert_ne!(frame.luma(118, 90), 90);

        // Carol's own screen never took the floor: a guest under `hosts`.
        assert_eq!(
            video.request_content("carol"),
            Err(forge_conference::video::ContentRefusal::ParticipantDisabled)
        );
        assert_eq!(
            video.participant("carol").unwrap().content_state,
            forge_conference::video::VideoState::Disabled
        );

        // The API's view of it.
        let alice_json =
            participant_video_json(&room, "alice", Some(&bridges[2].1), Some(&bridges[0].1));
        assert_eq!(alice_json["presenting"], serde_json::json!(true));
        assert_eq!(alice_json["content"]["state"], serde_json::json!("on"));
        assert_eq!(alice_json["content"]["codec"], serde_json::json!("VP8"));
        assert_eq!(alice_json["content"]["channel"], serde_json::json!(true));
        let carol_json =
            participant_video_json(&room, "carol", Some(&bridges[4].1), Some(&bridges[1].1));
        assert_eq!(carol_json["presenting"], serde_json::json!(false));
        assert_eq!(carol_json["view"], serde_json::json!("cameras"));
        assert_eq!(
            carol_json["content"]["state"],
            serde_json::json!("disabled")
        );
        assert_eq!(
            carol_json["content"]["forwarding"],
            serde_json::json!("alice#content")
        );
        let bob_json = participant_video_json(&room, "bob", Some(&bridges[3].1), None);
        assert!(bob_json.get("content").is_none());
        assert_eq!(bob_json["view"], serde_json::json!("composite"));
        let room_json = room_video_json(&room);
        assert_eq!(
            room_json["content"]["participant_id"],
            serde_json::json!("alice")
        );
        assert_eq!(room_json["content"]["live"], serde_json::json!(true));
        assert_eq!(
            room_json["content_layout"],
            serde_json::json!("presentation")
        );

        // A host ends it: the floor is free and the composite is a grid again.
        let stopped = stop_content(&room).unwrap();
        assert_eq!(stopped["participant"], serde_json::json!("alice"));
        assert_eq!(stop_content(&room), Err(VideoControlError::NoContent));
        frame_matching(&bob_phone, "without the screen", |h| {
            h.resolution() == Resolution::new(320, 180) && h.luma(118, 90) != 90
        })
        .await;
        assert!(video.content().is_none());

        // The floor's story, as gateway sessions would hear it: Alice
        // granted, Carol refused (hers alone), Alice released by the host.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let heard = loop {
            let heard = notices.0.lock().unwrap().clone();
            if heard.iter().any(|(_, n)| n.floor == "released") {
                break heard;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no released notice; heard {heard:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(heard.iter().all(|(room, _)| room == "1001"));
        // Carol is refused for her packets and again for asking — each
        // once per floor — and which of hers comes before Alice's grant
        // is the scheduler's; what matters is who was told what.
        let of = |floor: &str, who: &str| -> Vec<&FloorNotice> {
            heard
                .iter()
                .filter(|(_, n)| n.floor == floor && n.participant_id == who)
                .map(|(_, n)| n)
                .collect()
        };
        assert_eq!(of("granted", "alice").len(), 1, "{heard:?}");
        assert_eq!(
            of("granted", "alice")[0].presenter.as_deref(),
            Some("alice")
        );
        assert!(!of("refused", "carol").is_empty(), "{heard:?}");
        assert!(of("refused", "carol")
            .iter()
            .all(|n| n.reason.as_deref() == Some("participant_disabled")));
        assert_eq!(of("released", "alice").len(), 1, "{heard:?}");
        assert_eq!(of("released", "alice")[0].reason.as_deref(), Some("host"));
        // After the host's stop her phone is still sending, and the room
        // may refuse her once for it before the senders stop: fine, and
        // the only refusal she can draw. Carol is never granted.
        assert!(of("refused", "alice")
            .iter()
            .all(|n| n.reason.as_deref() == Some("participant_disabled")));
        assert!(of("granted", "carol").is_empty(), "{heard:?}");
        assert!(
            heard.iter().rev().any(|(_, n)| n.floor == "released"),
            "{heard:?}"
        );
        assert_eq!(
            set_content_layout(&room, "content_only").unwrap()["content_layout"],
            serde_json::json!("content_only")
        );
        assert!(matches!(
            set_content_layout(&room, "mosaic"),
            Err(VideoControlError::UnknownContentLayout(_))
        ));

        for s in senders {
            s.abort();
        }
        for (b, _) in bridges {
            b.abort();
        }
    }

    #[test]
    fn share_events_reach_the_bus_with_the_presenter_and_reason() {
        use forge_conference::video::{ContentInfo, ContentStop, VideoState};
        let bus = Arc::new(InMemoryEventBus::new());
        let sink = VideoEventSink {
            event_bus: bus as Arc<dyn EventPublisher>,
            tenant_id: TenantId::new(),
            room_number: "1000".into(),
            node_id: "conf-a".into(),
            pumps: Arc::new(dashmap::DashMap::new()),
            content_gate: None,
            floor: None,
        };
        let info = ContentInfo {
            participant_id: "alice".into(),
            display_name: "Alice".into(),
            remote: false,
            held_for: Duration::from_secs(1),
            codec: forge_core::VideoCodec::VP8,
            resolution: Resolution::new(1280, 720),
            fps: 5,
            live: true,
        };
        let started = VideoRoomEvent::Content {
            participant_id: "alice".into(),
            event: ContentEvent::Started,
        };
        match conference_event(started, &sink, 15, Some(&info)) {
            Some(ConferenceEvent::ContentStarted {
                participant_id,
                node_id,
                codec,
                ..
            }) => {
                assert_eq!(participant_id, "alice");
                assert_eq!(node_id, "conf-a");
                assert_eq!(codec, "VP8");
            }
            other => panic!("expected content_started, got {other:?}"),
        }
        let stopped = VideoRoomEvent::Content {
            participant_id: "alice".into(),
            event: ContentEvent::Stopped(ContentStop::Host),
        };
        match conference_event(stopped, &sink, 15, None) {
            Some(ConferenceEvent::ContentStopped { reason, .. }) => assert_eq!(reason, "host"),
            other => panic!("expected content_stopped, got {other:?}"),
        }
        // A refusal is the sender's business, not the bus's.
        let refused = VideoRoomEvent::Content {
            participant_id: "bob".into(),
            event: ContentEvent::Refused(forge_conference::video::ContentRefusal::Held(
                "alice".into(),
            )),
        };
        assert!(conference_event(refused, &sink, 15, None).is_none());
        // What a gateway session is told of each (7c): the presenter's
        // name on a grant, the holder's on a refusal for being held, the
        // reason on a stop.
        let notice = floor_notice("alice", &ContentEvent::Started, Some(&info));
        assert_eq!(
            notice,
            FloorNotice {
                floor: "granted",
                participant_id: "alice".into(),
                presenter: Some("Alice".into()),
                reason: None,
            }
        );
        let notice = floor_notice("alice", &ContentEvent::Stopped(ContentStop::Idle), None);
        assert_eq!(
            (notice.floor, notice.reason.as_deref()),
            ("released", Some("idle"))
        );
        assert!(notice.presenter.is_none());
        let notice = floor_notice(
            "bob",
            &ContentEvent::Refused(forge_conference::video::ContentRefusal::Held(
                "alice".into(),
            )),
            Some(&info),
        );
        assert_eq!(notice.floor, "refused");
        assert_eq!(notice.reason.as_deref(), Some("held"));
        assert_eq!(notice.presenter.as_deref(), Some("Alice"));
        assert_eq!(
            serde_json::to_value(&notice).unwrap(),
            serde_json::json!({
                "floor": "refused",
                "participant_id": "bob",
                "presenter": "Alice",
                "reason": "held",
            })
        );
        // A trunk's remote content is infrastructure, like its tile.
        let trunk = VideoRoomEvent::Content {
            participant_id: "__trunk__node-b".into(),
            event: ContentEvent::Started,
        };
        assert!(conference_event(trunk, &sink, 15, Some(&info)).is_none());
        // A shared screen's state names its stream.
        let state = VideoRoomEvent::ParticipantState {
            participant_id: "alice".into(),
            kind: StreamKind::Content,
            state: VideoState::On,
        };
        match conference_event(state, &sink, 15, None) {
            Some(ConferenceEvent::VideoStateChanged { stream, state, .. }) => {
                assert_eq!((stream.as_str(), state.as_str()), ("content", "on"));
            }
            other => panic!("expected video_state_changed, got {other:?}"),
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
                // This test is about composition, and it spotlights
                // Alice half way through — which under the default
                // `auto` would put Bob on the passthrough fast path and
                // send him Alice's own frame instead of a canvas (§5.6).
                // That interaction has its own test in forge-media.
                m.video_passthrough = "off".into();
                m
            },
            &usable_codecs(&backend, false),
            Duration::from_secs(30),
        )
        .unwrap();
        let bus = Arc::new(InMemoryEventBus::new());
        let pumps = Arc::new(dashmap::DashMap::new());
        let sink = VideoEventSink {
            event_bus: bus.clone() as Arc<dyn EventPublisher>,
            tenant_id: TenantId::new(),
            room_number: "1000".into(),
            node_id: "conf-a".into(),
            pumps: Arc::clone(&pumps),
            content_gate: None,
            floor: None,
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
            label: None,
            codec: forge_core::VideoCodec::VP8,
            payload_type: 97,
            fmtp: None,
            remote: Some(lo(alice_phone)),
            pli: true,
            fir: false,
            nack: true,
            remote_crypto: None,
            simulcast: None,
        };
        let mut legs = Vec::new();
        for (pid, srv, pair) in [
            ("alice", &alice_srv, alice_server),
            ("bob", &bob_srv, bob_server),
        ] {
            let leg = VideoLeg {
                kind: StreamKind::Camera,
                port_pair: PortPair::new(pair).unwrap(),
                socket: Arc::clone(srv),
                srtp_context: None,
                srtp_sdp: None,
                negotiated: negotiated.clone(),
                settings: settings.clone(),
                stats: Arc::new(VideoLegStats::default()),
                forward: false,
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
            content: Default::default(),
        };
        let mut enc: Box<dyn VideoEncoder> = raw_registry()
            .encoder(&enc_settings, &MediaDevice::Host)
            .unwrap();
        let sender = {
            let alice_ph = Arc::clone(&alice_ph);
            tokio::spawn(async move {
                let mut seq = 1u16;
                for i in 0..240u32 {
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

        // A host spotlights Alice (design §8, §12): the layout switches to
        // spotlight, the event reaches the bus through the pump, and the
        // next composites Bob receives are Alice full-canvas.
        let mut events = bus.subscribe(EventFilter::default()).await.unwrap();
        let answer = spotlight(&room, Some("alice")).unwrap();
        assert_eq!(answer["status"], "spotlighted");
        assert_eq!(answer["layout"], "spotlight");
        assert_eq!(answer["spotlight"], "alice");
        let ev = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match events.next().await {
                    Some(PlatformEvent::Conference(ConferenceEvent::VideoLayoutChanged {
                        room_number,
                        layout,
                        pinned,
                        spotlight,
                        ..
                    })) => return (room_number, layout, pinned, spotlight),
                    Some(_) => continue,
                    None => panic!("bus closed"),
                }
            }
        })
        .await
        .expect("video_layout_changed reached the bus");
        assert_eq!(
            ev,
            (
                "1000".to_string(),
                "spotlight".to_string(),
                None,
                Some("alice".to_string())
            )
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        let mut full_canvas = false;
        while tokio::time::Instant::now() < deadline {
            let (bytes, _) =
                match tokio::time::timeout(Duration::from_secs(1), bob_ph.recv_rtp_raw()).await {
                    Ok(Ok(r)) => r,
                    _ => continue,
                };
            let pkt = RtpPacket::parse(bytes).unwrap();
            for ev in asm.push(pkt) {
                if let AssemblerEvent::Frame(f) = ev {
                    if let Some(v) = dec.decode(&f).unwrap() {
                        let h = v.into_host().unwrap();
                        if h.luma(80, 90) == 180 && h.luma(240, 90) == 180 {
                            full_canvas = true;
                        }
                    }
                }
            }
            if full_canvas {
                break;
            }
        }
        assert!(full_canvas, "after the spotlight, alice fills bob's canvas");
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

    /// The next conference event on the bus that `pick` accepts.
    async fn next_event<T>(
        events: &mut fcp_events::EventStream,
        pick: impl Fn(&ConferenceEvent) -> Option<T>,
    ) -> T {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match events.next().await {
                    Some(PlatformEvent::Conference(e)) => {
                        if let Some(t) = pick(&e) {
                            return t;
                        }
                    }
                    Some(_) => continue,
                    None => panic!("bus closed"),
                }
            }
        })
        .await
        .expect("event reached the bus")
    }

    fn layout_of(e: &ConferenceEvent) -> Option<(String, Option<String>, Option<String>)> {
        match e {
            ConferenceEvent::VideoLayoutChanged {
                layout,
                pinned,
                spotlight,
                ..
            } => Some((layout.clone(), pinned.clone(), spotlight.clone())),
            _ => None,
        }
    }

    /// Layout, pin, spotlight and per-participant video controls apply to
    /// the room, answer what the API returns, refuse what they should, and
    /// their events reach the platform bus (design §8, §12).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn controls_apply_to_the_room_and_reach_the_bus() {
        let room = Arc::new(
            ConferenceRoom::new(
                "2000",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        room.add_participant("alice", true).unwrap();
        room.add_participant("bob", false).unwrap();

        // Without video the room refuses layout controls but still tells a
        // missing participant apart (that answer is what makes a peer node
        // get asked).
        assert_eq!(set_layout(&room, "grid"), Err(VideoControlError::NoVideo));
        assert_eq!(
            pin(&room, Some("carol")),
            Err(VideoControlError::NoSuchParticipant("carol".into()))
        );
        assert_eq!(
            set_layout(&room, "sideways"),
            Err(VideoControlError::UnknownLayout("sideways".into()))
        );

        let backend = VideoBackend::raw();
        let (settings, _) = plan_video(
            &meeting(true),
            &usable_codecs(&backend, false),
            Duration::from_secs(30),
        )
        .unwrap();
        let video = room.enable_video(settings, &backend);
        let bus = Arc::new(InMemoryEventBus::new());
        let pumps = Arc::new(dashmap::DashMap::new());
        let tenant = TenantId::new();
        pumps.insert(RoomKey::new(tenant, "2000"), ());
        spawn_event_pump(
            &video,
            VideoEventSink {
                event_bus: bus.clone() as Arc<dyn EventPublisher>,
                tenant_id: tenant,
                room_number: "2000".into(),
                node_id: "conf-a".into(),
                pumps: Arc::clone(&pumps),
                content_gate: None,
                floor: None,
            },
        );
        let mut events = bus.subscribe(EventFilter::default()).await.unwrap();

        let answer = set_layout(&room, "active_speaker").unwrap();
        assert_eq!(answer["status"], "layout_set");
        assert_eq!(answer["layout"], "active_speaker");
        assert_eq!(
            next_event(&mut events, layout_of).await,
            ("active_speaker".to_string(), None, None)
        );

        let answer = pin(&room, Some("alice")).unwrap();
        assert_eq!(answer["status"], "pinned");
        assert_eq!(answer["participant"], "alice");
        assert_eq!(answer["pinned"], "alice");
        assert_eq!(
            next_event(&mut events, layout_of).await,
            ("active_speaker".to_string(), Some("alice".into()), None)
        );

        // Spotlighting switches the layout; clearing the spotlight keeps it.
        let answer = spotlight(&room, Some("bob")).unwrap();
        assert_eq!(answer["status"], "spotlighted");
        assert_eq!(answer["layout"], "spotlight");
        assert_eq!(answer["spotlight"], "bob");
        assert_eq!(
            next_event(&mut events, layout_of).await,
            (
                "spotlight".to_string(),
                Some("alice".into()),
                Some("bob".into())
            )
        );
        let answer = spotlight(&room, None).unwrap();
        assert_eq!(answer["status"], "unspotlighted");
        assert_eq!(answer["layout"], "spotlight");
        assert!(answer["spotlight"].is_null());
        assert_eq!(
            next_event(&mut events, layout_of).await,
            ("spotlight".to_string(), Some("alice".into()), None)
        );

        // A host hides Alice's video: her tile reads `disabled` on the next
        // tick and the state change is published; showing it again lifts it.
        video
            .add_source("alice", forge_core::VideoCodec::VP8, "")
            .unwrap();
        let answer = set_participant_video(&room, "alice", false).unwrap();
        assert_eq!(answer["status"], "video_disabled");
        assert_eq!(answer["participant"], "alice");
        let state = next_event(&mut events, |e| match e {
            ConferenceEvent::VideoStateChanged {
                participant_id,
                state,
                ..
            } if participant_id == "alice" && state == "disabled" => Some(state.clone()),
            _ => None,
        })
        .await;
        assert_eq!(state, "disabled");
        assert_eq!(
            video.participant("alice").unwrap().state,
            forge_conference::video::VideoState::Disabled
        );
        let answer = set_participant_video(&room, "alice", true).unwrap();
        assert_eq!(answer["status"], "video_enabled");
        next_event(&mut events, |e| match e {
            ConferenceEvent::VideoStateChanged {
                participant_id,
                state,
                ..
            } if participant_id == "alice" && state != "disabled" => Some(()),
            _ => None,
        })
        .await;
        assert_eq!(
            set_participant_video(&room, "carol", false),
            Err(VideoControlError::NoSuchParticipant("carol".into()))
        );

        // The pinned participant leaves: the pin clears and says so.
        room.remove_participant("alice").unwrap();
        assert_eq!(
            next_event(&mut events, layout_of).await,
            ("spotlight".to_string(), None, None)
        );
        let answer = pin(&room, None).unwrap();
        assert_eq!(answer["status"], "unpinned");

        drop(video);
        room.disable_video();
    }
    /// A trunk's video half (§15.4, 5b): the peer's composite becomes one
    /// remote tile here, and what goes back down the trunk is this
    /// node's callers only — the anti-echo rule of the audio mix, drawn.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_trunk_takes_a_remote_tile_and_sends_back_local_callers_only() {
        let room = Arc::new(
            ConferenceRoom::new(
                "2000",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        room.add_participant("alice", true).unwrap();
        // The trunk's audio side, exactly as `attach_trunk` makes it.
        let trunk_id = crate::trunks::trunk_participant_id("node-b");
        room.add_virtual_participant(&trunk_id).unwrap();

        let backend = VideoBackend::raw();
        let (settings, _) = plan_video(
            &{
                let mut m = meeting(true);
                m.video_max_resolution = "180p".into();
                m.video_codecs = vec![VideoCodec::Vp8];
                m
            },
            &usable_codecs(&backend, false),
            Duration::from_secs(30),
        )
        .unwrap();
        let bus = Arc::new(InMemoryEventBus::new());
        let pumps = Arc::new(dashmap::DashMap::new());
        let sink = || VideoEventSink {
            event_bus: bus.clone() as Arc<dyn EventPublisher>,
            tenant_id: TenantId::new(),
            room_number: "2000".into(),
            node_id: "conf-a".into(),
            pumps: Arc::clone(&pumps),
            content_gate: None,
            floor: None,
        };
        let leg = |port: u16| VideoLeg {
            kind: StreamKind::Camera,
            port_pair: PortPair::new(port).unwrap(),
            socket: Arc::new(futures::executor::block_on(async {
                RtpSocketPair::new(
                    PortPair::new(port).unwrap(),
                    RtpSocketConfig {
                        bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                        ..RtpSocketConfig::default()
                    },
                )
                .await
                .unwrap()
            })),
            srtp_context: None,
            srtp_sdp: None,
            negotiated: NegotiatedVideo {
                label: None,
                codec: forge_core::VideoCodec::VP8,
                payload_type: 97,
                fmtp: None,
                remote: None,
                pli: true,
                fir: false,
                nack: true,
                remote_crypto: None,
                simulcast: None,
            },
            settings: settings.clone(),
            stats: Arc::new(VideoLegStats::default()),
            forward: false,
            bridge_handle: None,
        };

        // A caller with video, and the trunk.
        let alice_leg = leg(free_pair());
        let alice_task = tokio::spawn(
            attach_video(&room, &backend, "alice", Some("Alice"), &alice_leg, sink()).unwrap(),
        );
        let trunk_leg = leg(free_pair());
        let trunk_task = tokio::spawn(
            attach_trunk_video(&room, &backend, &trunk_id, "node-b", &trunk_leg, sink()).unwrap(),
        );

        let video = room.video().expect("the room has video");
        // The trunk is a tile, not a caller: it draws, and it is named
        // for the node behind it.
        assert!(video.is_remote(&trunk_id));
        assert!(!video.is_remote("alice"));
        let tile = video.participant(&trunk_id).expect("the peer's tile");
        assert!(tile.remote);
        assert_eq!(tile.display_name, "node-b");
        assert!(tile.source.is_some(), "the peer's composite is decoded");

        // Two composites: the room's, and the one going back to the peer
        // with no remote tile in it.
        let status = video.status();
        let scopes: Vec<String> = status
            .outputs
            .iter()
            .map(|o| format!("{:?}", o.scope))
            .collect();
        assert_eq!(status.outputs.len(), 2, "{scopes:?}");
        assert!(
            status
                .outputs
                .iter()
                .any(|o| o.scope == OutputScope::LocalOnly),
            "the trunk composes local callers only: {scopes:?}"
        );
        assert!(status.outputs.iter().any(|o| o.scope == OutputScope::All));

        // The room's video events skip the trunk: it is infrastructure,
        // like its join and leave.
        let json = room_video_json(&room);
        assert_eq!(json["outputs"].as_array().unwrap().len(), 2);

        // The trunk goes: tile, decoder and its composite go with it.
        alice_task.abort();
        trunk_task.abort();
        detach_video(&room, &trunk_id);
        assert!(!video.is_remote(&trunk_id));
        assert!(video.participant(&trunk_id).is_none());
        assert_eq!(
            video.status().outputs.len(),
            1,
            "the local-only composite went with the trunk"
        );
        drop(video);
        room.disable_video();
    }
    #[test]
    fn the_earlier_claim_wins_and_ties_fall_to_the_lower_node_then_id() {
        assert!(claim_beats(
            (100, "conf-b", "bob"),
            (200, "conf-a", "alice")
        ));
        assert!(!claim_beats(
            (300, "conf-a", "alice"),
            (200, "conf-b", "bob")
        ));
        assert!(claim_beats(
            (200, "conf-a", "zed"),
            (200, "conf-b", "alice")
        ));
        assert!(claim_beats(
            (200, "conf-a", "alice"),
            (200, "conf-a", "bob")
        ));
        assert!(!claim_beats((200, "conf-a", "bob"), (200, "conf-a", "bob")));
    }

    /// A shared screen across a trunk (§15.6, 7d): two rooms on two nodes,
    /// each with a caller; Alice's screen on node A takes the floor there,
    /// goes down the trunk local-only, takes the floor on node B as a
    /// remote content source, and reaches Bob's content channel; Bob's own
    /// screen is refused while it does; the node releasing the remote
    /// holder — what a withdrawn or expired claim does — frees the floor
    /// for Bob, whose screen then crosses the other way.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_shared_screen_crosses_a_trunk_and_the_floor_is_one_room_wide() {
        let backend = VideoBackend::raw();
        let meeting = {
            let mut m = meeting(true);
            m.video_max_resolution = "180p".into();
            m.video_codecs = vec![VideoCodec::Vp8];
            m.video_content_share = "anyone".into();
            m
        };
        let (settings, _) = plan_video(
            &meeting,
            &usable_codecs(&backend, false),
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(settings.content);
        let lo = |p: u16| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), p);
        let bus = Arc::new(InMemoryEventBus::new());
        let negotiated = NegotiatedVideo {
            label: None,
            codec: forge_core::VideoCodec::VP8,
            payload_type: 97,
            fmtp: None,
            remote: None,
            pli: true,
            fir: false,
            nack: true,
            remote_crypto: None,
            simulcast: None,
        };
        let leg = |socket: &Arc<RtpSocketPair>, port: u16| VideoLeg {
            kind: StreamKind::Content,
            port_pair: PortPair::new(port).unwrap(),
            socket: Arc::clone(socket),
            srtp_context: None,
            srtp_sdp: None,
            negotiated: negotiated.clone(),
            settings: settings.clone(),
            stats: Arc::new(VideoLegStats::default()),
            forward: false,
            bridge_handle: None,
        };
        // A socket pair pointed at another.
        let wired = |port: u16, peer: u16| async move {
            let s = socket_pair(port).await;
            s.set_remote_endpoint(RemoteEndpoint {
                rtp_addr: lo(peer),
                rtcp_addr: Some(lo(peer + 1)),
            })
            .await;
            s
        };

        // Two nodes, each hosting the room with one caller who has a
        // content leg (a screen to send and a channel to receive on).
        let node = |name: &'static str, caller: &'static str| {
            let room = Arc::new(
                ConferenceRoom::new(
                    "4000",
                    AudioFormat::pcm_mono(),
                    160,
                    forge_mixer::MixerOptions::default(),
                )
                .unwrap(),
            );
            room.add_participant(caller, false).unwrap();
            let sink = VideoEventSink {
                event_bus: bus.clone() as Arc<dyn EventPublisher>,
                tenant_id: TenantId::new(),
                room_number: "4000".into(),
                node_id: name.into(),
                pumps: Arc::new(dashmap::DashMap::new()),
                content_gate: None,
                floor: None,
            };
            (room, sink)
        };
        let (room_a, sink_a) = node("conf-a", "alice");
        let (room_b, sink_b) = node("conf-b", "bob");
        let trunk_a = crate::trunks::trunk_participant_id("conf-b"); // B, seen from A
        let trunk_b = crate::trunks::trunk_participant_id("conf-a"); // A, seen from B
        room_a.add_virtual_participant(&trunk_a).unwrap();
        room_b.add_virtual_participant(&trunk_b).unwrap();

        // Alice's and Bob's content legs, each with a phone behind it.
        let (a_srv, a_phone_port) = (free_pair(), free_pair());
        let (b_srv, b_phone_port) = (free_pair(), free_pair());
        let alice_srv = wired(a_srv, a_phone_port).await;
        let alice_phone = wired(a_phone_port, a_srv).await;
        let bob_srv = wired(b_srv, b_phone_port).await;
        let bob_phone = wired(b_phone_port, b_srv).await;
        // The trunk's content section: A's socket and B's, pointed at each other.
        let (ta, tb) = (free_pair(), free_pair());
        let trunk_a_sock = wired(ta, tb).await;
        let trunk_b_sock = wired(tb, ta).await;

        let alice_leg = leg(&alice_srv, a_srv);
        let bob_leg = leg(&bob_srv, b_srv);
        let ta_leg = leg(&trunk_a_sock, ta);
        let tb_leg = leg(&trunk_b_sock, tb);
        let tasks = [
            tokio::spawn(
                attach_content(&room_a, &backend, "alice", &alice_leg, sink_a.clone(), true)
                    .unwrap(),
            ),
            tokio::spawn(
                attach_content(&room_b, &backend, "bob", &bob_leg, sink_b.clone(), true).unwrap(),
            ),
            tokio::spawn(
                attach_trunk_content(&room_a, &backend, &trunk_a, &ta_leg, sink_a.clone()).unwrap(),
            ),
            tokio::spawn(
                attach_trunk_content(&room_b, &backend, &trunk_b, &tb_leg, sink_b.clone()).unwrap(),
            ),
        ];
        let video_a = room_a.video().unwrap();
        let video_b = room_b.video().unwrap();
        assert!(video_a.is_remote(&trunk_a));
        assert!(video_b.is_remote(&trunk_b));
        // Each trunk sends the content view local-only: a peer's screen
        // never goes back down a trunk.
        for video in [&video_a, &video_b] {
            assert!(video.status().outputs.iter().any(|o| {
                o.scope == OutputScope::LocalOnly
                    && o.view == forge_conference::video::View::Content
            }));
        }

        // Alice shares: the floor on A is hers, on B it is the trunk's,
        // and Bob's channel carries her screen.
        let alice_screen = send_frames(Arc::clone(&alice_phone), 160, 90, 90, 0xA5C4);
        frame_matching(&bob_phone, "with alice's screen on node B", |h| {
            h.resolution() == Resolution::new(160, 90) && h.luma(80, 45) == 90
        })
        .await;
        assert_eq!(video_a.content_holder().as_deref(), Some("alice"));
        assert_eq!(video_b.content_holder().as_deref(), Some(trunk_b.as_str()));
        assert!(video_b.content().unwrap().remote);
        assert!(!video_a.content().unwrap().remote);

        // Bob tries to share while she does: refused, the floor is held.
        let bob_screen = send_frames(Arc::clone(&bob_phone), 160, 90, 30, 0xB0B5);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            video_b.request_content("bob"),
            Err(forge_conference::video::ContentRefusal::Held(
                trunk_b.clone()
            ))
        );
        assert_eq!(video_b.content_holder().as_deref(), Some(trunk_b.as_str()));

        // Alice stops (her track ended; the section stays); node B hears
        // it (a withdrawn claim, or an expired one) and releases the
        // remote holder: the floor is Bob's on the next packet, and his
        // screen crosses to Alice's channel.
        alice_screen.abort();
        assert!(video_a.release_content("alice"));
        assert!(video_a.content_holder().is_none());
        assert!(video_b.release_content_with(&trunk_b, forge_conference::video::ContentStop::Ended));
        // Aborting her sender does not empty the pipes: a frame of hers
        // still in node A takes node B's floor back for the trunk the
        // moment it lands, and a content packet is all a claim is. Here
        // that stale hold would stand until the 30 s idle expiry, so
        // withdraw it again until Bob's own claim is the one that sticks
        // — what the peers tell each other for real (7d).
        let settled = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if video_b.content_holder().as_deref() == Some("bob")
                    && video_a.content_holder().as_deref() == Some(trunk_a.as_str())
                {
                    return;
                }
                video_b.release_content_with(&trunk_b, forge_conference::video::ContentStop::Ended);
                video_a.release_content("alice");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(
            settled.is_ok(),
            "the floor never passed to bob: node B holds {:?}, node A {:?}",
            video_b.content_holder(),
            video_a.content_holder()
        );
        frame_matching(&alice_phone, "with bob's screen on node A", |h| {
            h.resolution() == Resolution::new(160, 90) && h.luma(80, 45) == 30
        })
        .await;
        assert_eq!(video_b.content_holder().as_deref(), Some("bob"));
        assert_eq!(video_a.content_holder().as_deref(), Some(trunk_a.as_str()));

        bob_screen.abort();
        for t in tasks {
            t.abort();
        }
        drop((video_a, video_b));
        room_a.disable_video();
        room_b.disable_video();
    }

    /// One emitter per change (§15.4, 5c): the node the speaker is on
    /// announces them, and a node following a peer's claim stays quiet,
    /// so a listener sees one `active_speaker_changed` per change rather
    /// than one per hosting node.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_the_speaker_s_own_node_announces_the_change() {
        let room = Arc::new(
            ConferenceRoom::new(
                "3000",
                AudioFormat::pcm_mono(),
                160,
                forge_mixer::MixerOptions::default(),
            )
            .unwrap(),
        );
        room.add_participant("alice", true).unwrap();
        let trunk_id = crate::trunks::trunk_participant_id("node-b");
        room.add_virtual_participant(&trunk_id).unwrap();

        let backend = VideoBackend::raw();
        let (settings, _) = plan_video(
            &meeting(true),
            &usable_codecs(&backend, false),
            Duration::from_secs(30),
        )
        .unwrap();
        let video = room.enable_video(settings, &backend);
        video.set_local_node("conf-a");
        video
            .add_remote(&trunk_id, forge_core::VideoCodec::VP8, "")
            .unwrap();

        let bus = Arc::new(InMemoryEventBus::new());
        let pumps = Arc::new(dashmap::DashMap::new());
        let tenant = TenantId::new();
        pumps.insert(RoomKey::new(tenant, "3000"), ());
        spawn_event_pump(
            &video,
            VideoEventSink {
                event_bus: bus.clone() as Arc<dyn EventPublisher>,
                tenant_id: tenant,
                room_number: "3000".into(),
                node_id: "conf-a".into(),
                pumps: Arc::clone(&pumps),
                content_gate: None,
                floor: None,
            },
        );
        let mut events = bus.subscribe(EventFilter::default()).await.unwrap();

        // A caller here takes the floor: this node announces them.
        let speaking = tokio::spawn({
            let room = Arc::clone(&room);
            async move {
                for _ in 0..200 {
                    room.write_audio("alice", &[1_500i16; 160]).ok();
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        });
        let announced = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(PlatformEvent::Conference(ConferenceEvent::ActiveSpeakerChanged {
                    room_number,
                    participant_id,
                    node_id,
                    ..
                })) = events.next().await
                {
                    return (room_number, participant_id, node_id);
                }
            }
        })
        .await
        .expect("the speaker's own node announced them");
        assert_eq!(
            announced,
            (
                "3000".to_string(),
                "alice".to_string(),
                "conf-a".to_string()
            )
        );

        // Node B now says its caller is louder. This node follows —
        // `active_speaker` becomes Bob — but says nothing, because Bob
        // is node B's to announce.
        video.set_remote_speaker(
            &trunk_id,
            Some(forge_conference::video::RemoteSpeaker {
                node: "node-b".to_string(),
                participant_id: "bob".to_string(),
                display_name: "Bob".to_string(),
                energy: 30_000.0,
                speaking: true,
            }),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        while video.active_speaker().as_deref() != Some("bob")
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(video.active_speaker().as_deref(), Some("bob"));
        assert_eq!(
            video.active_speaker_via().as_deref(),
            Some(trunk_id.as_str())
        );

        let quiet = tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                if let Some(PlatformEvent::Conference(ConferenceEvent::ActiveSpeakerChanged {
                    participant_id,
                    ..
                })) = events.next().await
                {
                    return participant_id;
                }
            }
        })
        .await;
        assert!(
            quiet.is_err(),
            "a node following a peer's claim announces nothing: {quiet:?}"
        );

        speaking.abort();
        drop(video);
        room.disable_video();
    }
}
