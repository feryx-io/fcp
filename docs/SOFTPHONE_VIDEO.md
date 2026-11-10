# Video in the softphone

Designed 2026-11-08. Anvil (`thevoiceguy/anvil`) is audio-only by design: its
`docs/ARCHITECTURE.md` lists video as a non-goal, and `docs/SOFTPHONE.md` §5 sends
conferences to dial-in or the WebRTC join page. This project gives Anvil video
on two kinds of call:

- **one-to-one video calls** between users, through the call manager, like any
  other call;
- **video conferences**, with Anvil as a full client of FCP's video rooms: the room's
  picture, the people in it, camera and microphone controls, screen share sent and
  received, and a host's controls.

Questions are in §7, each with a recommendation. Every recommendation was agreed on
2026-11-08.

## 1. What exists

### 1.1 Anvil

- **Its own media pipeline**, not forge-engine: `anvil-core/src/media.rs` runs one
  audio stream per call on `forge-rtp` sockets, with its own SDP
  (`sip_sdp::SessionDescription`), SDES-SRTP (`srtp.rs`), RTCP (`rtcp.rs`, reports
  and round trip) and quality (`quality.rs`). It has STUN but no ICE; FCP latches.
- **Platform audio behind a trait**: `AudioHost` is supplied by `anvil-audio` (cpal,
  with echo cancellation) on desktops and by the platform layer on mobile. The core
  never opens a device.
- **A Flutter app** (`app/`) over `flutter_rust_bridge`, on Linux, macOS, Windows,
  iOS and Android, with the system's call screens (CallKit, `ConnectionService`).
- **No video anywhere.** It offers no `m=video`, so a video call reaches it as audio.

### 1.2 forge-media

It has the packet side of video. Nothing in it captures, renders or estimates
bandwidth.

- **Payload formats**, both ways (`forge-rtp/src/video/payload.rs`): H.264 (single
  NAL, STAP-A, FU-A), VP8, VP9, AV1 and HEVC. VP8's sender writes a minimal
  descriptor without a picture id.
- **Frame assembly** (`assembler.rs`): `FrameAssembler` reorders, closes frames,
  reports `needs_keyframe()` and the sequence numbers `missing()`. There is no
  playout clock and no lip sync.
- **Send side**: `RtxCache` answers NACKs on the same SSRC; `KeyframeRequestGate`
  rate-limits PLIs.
- **RTCP feedback** (`forge-rtp/src/rtcp.rs`): NACK, PLI, FIR and REMB, built and
  parsed. transport-cc is kept as raw bytes, and nothing estimates bandwidth.
- **Codecs** (`forge-video`, `forge-video-codecs`): `VideoEncoder` and
  `VideoDecoder` traits with `set_bitrate` and `ContentHint::{Camera, Screen}`, and
  libvpx, OpenH264, dav1d and SVT-AV1 behind features. They link system libraries
  through pkg-config. That suits a Linux server, not iOS or Android. Nothing uses a
  platform's hardware codecs.
- **SDP** (`forge-sdp/src/video.rs`): rtcp-fb, `H264Fmtp`, RFC 4796 `a=content`,
  `a=label`, `video_sections()`, `choose_video_codec`.
- **BFCP** (`forge-bfcp`): the message codec and its TCP and UDP transports, and a
  floor *server*. There is no client.
- **forge-webrtc**: a Rust `PeerConnection` with ICE, DTLS-SRTP and BUNDLE, and
  video sections, without codecs.

### 1.3 FCP's video rooms

Video conferencing is built (`docs/VIDEO_CONFERENCING.md`, phases 3–9).

- **A SIP caller dials the room's number.** The call manager places a
  `video_enabled` meeting on a node with video budget
  (`ConferenceVideoNeed`). The conference server answers the caller's offer: the
  first of the meeting's `video_codecs` (default H.264 then VP8) that the offer
  carries. H.264 must use `packetization-mode=1`, at constrained baseline or main.
  The rtcp-fb it allows is `nack`, `nack pli`, `ccm fir` and `goog-remb`.
- **One mixed picture comes down**, at the room's cap (default 720p at 15 fps).
  Every participant sees the same layout: grid, active speaker, spotlight or
  picture-in-picture, and presentation while someone shares. A spotlit source
  with a matching codec is forwarded rather than re-encoded.
- **One camera stream goes up.** Neither SIP nor WebRTC sends simulcast.
- **Screen share** is a second `m=video` with `a=content:slides`. The floor is taken
  by the first content packets, or through BFCP. An endpoint offering BFCP must use
  it, and BFCP is refused by the B2BUA and the SBC when they anchor. The WebRTC
  gateway hears of the floor by INFO (`application/vnd.fcp.content+json`).
- **Adaptation**: PLI and FIR bring a keyframe, at most one a second. NACK is answered
  from about 500 ms of cache. **REMB is the only bandwidth signal**: a subscriber
  moves between 1080p 2.5 Mb/s, 720p 1.2 Mb/s, 360p 500 kb/s and 180p 200 kb/s on it,
  without a re-INVITE.
- **Controls are HTTP only**, on the conference server and proxied by the admin
  (`/api/v1/conferences/live/{room}/…`): layout, pin, spotlight, a participant's
  video, mute, kick. An owner's `/me/conferences/{id}` has no live controls. In the
  call, there is only the DTMF menu (`*6` mute).
- **The roster over SIP is thin.** RFC 4575 `conference` SUBSCRIBE works, but each
  NOTIFY is `state="full" version="1"` and lists participant ids with no names,
  media, mute or video state.
- **Join tokens** (`X-FCP-Join`) let a caller skip the PIN.

### 1.4 The gap in the call manager

**An anchored call has no video.** Every anchored leg is rewritten by
`rewrite_sdp_for_anchoring` with `AnchorPorts { video: None, content: None }`
(`fcp-media/src/sdp.rs`), which sets the video section to port 0, "since nothing would
relay it". A call is anchored when its caller is behind NAT, is recorded, needs DTMF
detected, is an emergency call or needs transcoding, and whenever the phone is a
softphone on TLS with SRTP (`secure_device_offer`). **Every Anvil call is one of
those.** Only a call the B2BUA passes through keeps its video, and no test places a
video call between two phones.

The same rewrite runs on a call that reaches a conference room through the call
manager, so Anvil dialing a room today would join with audio only.

## 2. The model

### 2.1 One path: SIP through the call manager

Anvil makes and takes video calls, and joins rooms, as SIP calls through the call
manager, the same as its audio calls. Nothing needs a second stack, and every call
keeps its record, its policy (class of service, recording and consent) and its
features. A room is a number Anvil dials, as a desk phone does.

The other path, a native WebRTC client through the gateway (`forge-webrtc` is Rust),
would bring ICE, but it would also mean a second media stack in the app. It would
bypass the B2BUA, and the gateway's audio is G.711 and G.722 only, never Opus. It
also does nothing for one-to-one calls. It is §7 Q2, which also says when WebRTC
becomes the right path: when FCP's rooms forward each participant's stream. That
future is why Anvil's video must not depend on SIP (§2.4).

### 2.2 FCP relays video on an anchored call

The call manager relays each video section of an anchored call packet for packet: a
socket pair per leg per section, as audio has (`docs/NAT_AND_TLS.md`). It never
decodes or transcodes, so a video call costs the node copies, not encodes.

- **Negotiation end to end.** The B2BUA offers the callee the caller's video codecs
  and answers the caller with what the callee chose, with ports rewritten to the
  node's. If the two have no codec in common, the callee's answer has video at port
  0, and the call goes on with audio.
- **Each leg keyed on its own.** SDES keys are made per leg and per section, as audio
  already is. A plaintext leg is allowed only where audio's would be.
- **RTCP passes through**, SSRCs unchanged: PLI, FIR, NACK and REMB from one phone
  reach the other, so the phones adapt to each other as if they were directly
  connected. The node reads what passes, for the call's quality record.
- **Latching** works as for audio, so a phone behind NAT gets video back.
- **Screen share** (`a=content:slides`) is relayed the same way, on its own section.
- **What changes a call's media also changes its video.**
  - Hold: video goes `inactive` with audio, and resumes with it.
  - Music on hold and park: the held party's video is stopped.
  - Transfer and retrieve: the moved leg is re-offered video toward its new party.
  - A flow's prompt or menu: video is stopped while the flow has the call.
  - Recording: audio only, as today (§6).
- **Policy.** A tenant setting `video.calls` (on by default) and a class-of-service
  flag decide whether a call may carry video. A call that may not has its video set
  to port 0 where it is today.

Conference calls are the same relay: Anvil's leg to the call manager, then the call
manager's leg to the room.

### 2.3 Anvil's video, in the core and the host

The boundary mirrors audio's. `anvil-core` owns the network. A new `VideoHost` trait,
supplied by the platform, owns the camera, the codecs and the screen. They pass
**coded frames**, not pictures, because the codec that matters on each platform is
the hardware one:
- VideoToolbox on iOS and macOS;
- MediaCodec on Android;
- Media Foundation on Windows.

Each of those works on the platform's own buffers. Moving raw frames across the
interface 30 times a second would cost more than encoding them.

```rust
pub trait VideoHost: Send + Sync {
    fn codecs(&self) -> Vec<VideoCodecCaps>;             // what it encodes and decodes
    fn cameras(&self) -> Vec<CameraInfo>;
    // A camera or a screen; one encoding today, several (simulcast) later.
    fn start_send(&self, cfg: SendConfig) -> Result<Box<dyn VideoSource>>;
    // One per incoming stream: the room's picture today, a participant's later.
    fn start_receive(&self, stream: StreamId, cfg: ReceiveConfig)
        -> Result<Box<dyn VideoSink>>;
}
pub trait VideoSource: Send {                            // capture + encode, on the host
    async fn next_frame(&mut self) -> Option<(EncodingId, CodedFrame)>;
    fn request_keyframe(&self);
    fn set_target(&self, kbps: u32, max: Resolution, fps: u32);
}
pub trait VideoSink: Send {                              // decode + render, on the host
    fn frame(&mut self, frame: CodedFrame, at: Instant); // when to show it
    fn lost(&mut self);                                  // freeze until the next keyframe
}
```

**The core**, per video stream:
- SDP:
  - offers H.264 constrained baseline with `packetization-mode=1` (FCP's rooms take
    it, and every platform has it in hardware), then VP8 where the host has it;
  - adds rtcp-fb `nack`, `nack pli`, `ccm fir` and `goog-remb`;
  - adds `a=content:slides` for a second section, and the sections' directions.
- Sending: packetizing (`forge-rtp`), an `RtxCache`, SRTP and RTCP sender reports.
- Receiving: a `FrameAssembler`, NACK for what is missing, then PLI (gated) when
  the assembler needs a keyframe.
- **Bandwidth both ways.**
  - Down: the core makes a REMB from what it receives, using delay and loss, a
    simple GCC-style estimator. Without one, FCP's rooms never move a
    subscriber.
  - Up: it obeys the REMB it gets by calling `set_target`.
- **Timing and lip sync**: a short playout delay (60–150 ms, grown on jitter). Audio
  and video are aligned by the sender reports' NTP times, and each frame goes to the
  sink with the moment to show it.
- **Events**: `VideoStarted`, `VideoStopped`, `VideoStats` (resolution, fps, kbps,
  loss), `ContentStarted` and `ContentStopped`, and the conference events of §2.6.
- **API**: `place_call(target, CallMedia { video })`, `set_video(call, on)` (a
  re-INVITE turning a call's video on or off, or upgrading an audio call), `set_camera`,
  `share_screen(call, source)` and `stop_share`.
- **The test host**: `anvil-core` ships a `SyntheticVideoHost` that sends numbered
  coded frames and records what it receives, so the core, the CLI and CI need no
  camera.

**The hosts:**
- **Desktop**: a new `anvil-video` crate, like `anvil-audio`:
  - macOS: AVFoundation capture, VideoToolbox, ScreenCaptureKit;
  - Windows: Media Foundation capture and codecs, Windows.Graphics.Capture;
  - Linux: PipeWire (camera and the screen-cast portal), with OpenH264 and libvpx
    (§7 Q4).
- **iOS**: AVCaptureSession and VideoToolbox, in Swift through the bridge.
- **Android**: CameraX and MediaCodec, in Kotlin through the bridge.
- **Rendering**: decoded frames go straight into a Flutter `Texture` through the
  platform's texture registry (a `CVPixelBuffer`, a `SurfaceTexture`, a pixel buffer
  on Windows and Linux). Pictures never cross into Dart.

### 2.4 Independent of the transport (a requirement)

FCP's rooms mix today. They will one day **forward each participant's stream**
instead, as Zoom, Meet and Teams do, and that will most likely be over WebRTC
(§7 Q2). Anvil's video is built now so that change adds a transport and a
layout, and rewrites nothing. A1 is not done until these hold:

- **A transport beneath the streams.** SIP's SDP, `forge-rtp` sockets and SDES are
  one `MediaTransport`. A WebRTC one (`forge-webrtc`: ICE, DTLS-SRTP, BUNDLE,
  demultiplexing by `mid` and SSRC) can sit beside it later. Nothing above the
  transport reads SDP or opens a socket.
- **Streams, not "the call's video".** A call has any number of incoming video
  streams, each with:
  - its own SSRC;
  - its own assembler, NACK and PLI state;
  - its own playout;
  - its own `VideoSink`.

  It has a camera stream and a content stream going out. Today a room sends one
  picture and a share, but nothing may assume one.
- **Several encodings per source.** `VideoSource` already names an encoding, so
  simulcast later means asking the host for more, not a new interface. A1
  implements one.
- **Bandwidth estimation behind an interface.** It is REMB both ways today.
  Send-side transport-cc, which forwarding servers use, plugs in later.
- **A stream belongs to someone.** The roster maps a stream to a participant:
  RFC 4575's media `src-id` today, `mid` and `msid` over WebRTC.
- **A layout the app builds.** The call screen lays out a list of streams, each
  with whose it is. A room's composite is one tile today; a participant's stream
  will be one tile each.
- **Proved.** A1's tests include a call with two incoming video streams and a
  second transport stub, so the assumptions stay honest before anything needs
  them.

### 2.5 The app

- **Calls**: a video button on the dialer and on contacts. While on a call: a video
  toggle and a camera switch (front and back on mobile, a device picker on
  desktop). The self-view is a draggable corner tile.
- **Incoming calls**: an incoming video call says so, and can be answered with audio
  only (video `recvonly`, or port 0).
- **Mobile**:
  - CallKit `hasVideo` and `ConnectionService` video state;
  - the camera is stopped (`recvonly`) when the app goes to the background, and
    resumed when it returns;
  - picture-in-picture: AVKit's video-call PiP on iOS, PiP mode on Android;
  - permission prompts asked at the first video call, not at install.
- **Data**: a setting to send video only on Wi-Fi, which caps the send ladder to
  360p on cellular.

### 2.6 Anvil in a room

Anvil joins a room by dialing its number, or from a join link (`fcp://join/...` or
the gateway's `join_url`). A link's token is sent as `X-FCP-Join`, so the PIN is
skipped.

- **The picture**: the room's composite in the main section, at the rung REMB
  gives it. While someone shares, the shared screen comes in its own section, because
  Anvil offers the content section from the start. The app shows the share large and
  the cameras beside it.
- **The people**: Anvil subscribes to the room's `conference` event package
  (RFC 4575), in the dialog. FCP's NOTIFYs become real ones (F2):
  - names, audio and video state, mute, speaking and the presenter;
  - versioned partial updates after a full first one.

  Anvil shows a roster and the active speaker from them.
- **Own controls**: mute is local, plus a mute the room sees. Camera off is the
  video section turned `recvonly`: the room shows the avatar and the roster says
  so. Hand raise is §6.
- **Screen share sent** (desktop first; mobile is A5): a second section sending
  `a=content:slides`. The floor is taken implicitly, as a non-BFCP endpoint does
  today. Anvil learns it was given, refused or revoked from the INFO the gateway
  already gets (`application/vnd.fcp.content+json`); FCP sends it to a SIP
  endpoint that says it supports it (F2). BFCP is not needed (§7 Q6).
- **A host's controls**: layout, pin, spotlight, mute a participant, stop their
  video, remove them, stop a share. They are on new `/me/conferences/live/{room}/…`
  routes, under the app session, for a participant whose join made them a host
  (the meeting's owner, or a host join token). They reuse the conference server's
  routes through the admin's existing proxy.
- **Meetings**: the app lists the user's own meetings (`/me/conferences`), joins
  them, and copies a join link to send. FCP has no scheduling or invitations, and
  this project adds none (§6).

### 2.7 What a user is told

`/me/softphone` gains a `video` block:
- `enabled`, from the tenant setting and the user's class of service;
- `codecs`;
- `max_send` (resolution and kbps), the tenant's cap for a phone's camera;
- `content_share`, whether the user may share in rooms they don't host.

Anvil turns video off where it is not enabled, and no video button shows.

## 3. Capacity

- A one-to-one call relays packets. At 720p it adds about 1.2 Mb/s each way per
  leg to the node, and no encoding.
- A participant in a room costs what any SIP video participant costs today
  (`docs/VIDEO_CONFERENCING.md` §9.4, `docs/GPU.md`). With identical settings,
  participants share one encode and each costs one decode. A room placed by
  budget refuses an over-budget join with 503, and Anvil says the room is full.

## 4. How it is proved

- **FCP**: `fcp_testkit::Phone` gains a video section that sends numbered synthetic
  coded frames (no codec) and checks what arrives.
  - `calls.rs`: two phones with video through an anchored call (SRTP, NAT, hold,
    transfer, park, a flow, no common codec).
  - The relay of PLI, NACK and REMB.
  - Policy turning video off.
  - `rooms.rs`: a SIP phone through the call manager into a video room, its
    composite received, the roster's NOTIFYs, the content INFO, a host's control
    through `/me`.
- **Anvil**:
  - Anvil to Anvil with `SyntheticVideoHost`: offer, answer and upgrade, NACK, PLI,
    the REMB loop under shaped loss, lip sync from SR.
  - Against FCP in the existing "against FCP" job on the nightly images: a video
    call through the call manager, and a room joined with a share.
  - Each platform host has a device test where CI has the device (macOS and Windows
    runners have no camera, so their capture is tested with a virtual source). The
    app's integration tests run on the Android emulator's virtual camera.

## 5. Phases

Each phase is one PR in its repository, as before.

### F1: video through an anchored call (fcp)

- The video relay of §2.2: sockets, SDES per leg, RTCP passed through, latching.
- Hold, music, park, transfer, flows.
- `video.calls` and the class-of-service flag.
- `/me/softphone`'s `video` block.
- The testkit phone's video; the `calls.rs` tests.

In two parts:
- **F1a, done (v0.25.539).** `fcp_media::VideoRelay`:
  - a bound socket pair per leg per video section;
  - SDES per leg, with the other leg's key never crossing;
  - RTCP passed through, with RTCP on the RTP port told apart;
  - latching, and a stranger refused.

  The SDP side is `AnchorKeys`, `splice_relayed_video` and `with_video_sdes`, with
  `MediaManager::{anchored_offer, video_answered, anchored_answer,
  secure_video_offer}`. They are used for:
  - the B-leg offer, and a TLS callee's;
  - the callee's answer, serial and forked;
  - re-INVITEs both ways.

  `fcp_testkit::video::VideoLeg` and `Phone::enable_video` back the `calls.rs`
  `video_*` tests.
- **F1b, done (v0.25.541).** The rest:
  - **Hold, music and park.** A hold the node answers keeps the holder's
    video on the relay, answered `inactive` (`with_video_direction`). Every
    change of a call's held leg pauses the relay or resumes it
    (`MediaManager::set_video_paused`, from the hold hook), so a held or
    parked party sees nothing and a resume brings the video back.
  - **Transfer and retrieve.** A node-run transfer offers its target the
    transferred party's video on the referrer's relay, and a party taken
    off hold is re-offered its own. A retrieve or `Replaces` answers the
    taker with the party's video. A flow redirect is a transfer, so it is
    covered: a flow answers audio alone, so video stops while it has the
    call.
  - **Find-me and forwarding on no answer** offer each target the caller's
    video, as the first callee was.
  - **The policy.** The tenant's `video` (`calls`, `max_send_height`,
    `max_send_kbps`, `content_share`; migration 139) and a class's `video`
    flag. Before the call's first offer the node asks whether the call may
    carry video: the tenant, then the caller's and the callee's classes. The
    answer is kept on the media session (`MediaSession::set_video_allowed`),
    and every later offer follows it, so a call that may not carry video
    has its video at port 0.
  - **`/me/softphone`'s `video`**: `enabled`, `codecs` (H.264, then VP8),
    `max_send` and `content_share`.
  - The admin UI's tenant and class-of-service pages show the settings.
  - The `calling.rs` `video_*` tests and `api_postgres.rs`
    `the_softphone_is_told_whether_its_calls_carry_video` prove it.

### A1: video in the core (anvil)

- `VideoHost`, the SDP, sending and receiving, NACK, RTX and PLI.
- The REMB estimator and the response to REMB.
- Playout and lip sync.
- `set_video` and upgrades.
- The transport independence of §2.4, with its tests.
- `SyntheticVideoHost`, the CLI's `--video synthetic`, and the FFI events.
- Anvil-to-Anvil and against-FCP tests.
- `ARCHITECTURE.md`'s non-goal rewritten.

### A2: desktop video (anvil)

- `anvil-video` for macOS, Windows and Linux.
- Flutter textures.
- The call screen, self-view, camera picker and video toggle.

### A3: mobile video (anvil)

- iOS and Android hosts.
- CallKit and `ConnectionService` video.
- Background camera handling, picture-in-picture and permissions.

### F2: rooms for a SIP client (fcp)

F2 is split in two PRs: F2a, the roster, and F2b, the rest.

- RFC 4575 NOTIFYs with names, media, mute, speaker and presenter; full then partial,
  versioned. **F2a, done (v0.25.542):**
  - The conference server's roster (`fcp-conference-server/src/roster.rs`) lists each
    caller on the node. Each entry has the caller's name, a host or participant role,
    when they joined, and their audio media: `sendrecv`, `recvonly` while muted, or
    `inactive` on hold.
  - Each entry also has the video media with the camera's state as
    `fcp:camera`, the content media while they present, and
    `fcp:speaking` and `fcp:presenting`. These are in FCP's namespace,
    `urn:fcp:params:xml:ns:conference-info`.
  - The first NOTIFY is `state="full"`. Each one after it is `state="partial"` and
    carries only the users that changed (a user who left is `state="deleted"`). The
    `version` rises by one each time and keeps rising across a refresh.
  - The rosters are compared with what each subscriber was last told every 500 ms,
    and at once on a room event, so a mute or a speaker change is caught without an
    event of its own.
  - A participant subscribes inside their call, or out of it naming the call in
    `Target-Dialog` (RFC 4538). Anyone else gets 403. Before this, anyone could
    subscribe to a room that had a participant.
  - NOTIFYs go in the subscription's dialog, with `Subscription-State`. The
    subscription ends with `terminated` when it lapses, when its participant leaves,
    or when the room ends (`noresource`).
  - Callers on a peer node are not listed yet; each node lists its own.
- `X-FCP-Join` carried through the B2BUA to the conference node.
- The content INFO to SIP endpoints that support it.
- `/me/conferences/live/{room}/…` for hosts.
- `rooms.rs` tests through the call manager.

### A4: Anvil in a room (anvil)

- Joining by number and by link.
- The roster, the active speaker, the content section.
- Desktop screen share with the floor by INFO.
- A host's controls.
- The against-FCP room test.

### A5: mobile screen share (anvil)

- iOS: a ReplayKit broadcast extension.
- Android: MediaProjection.

The phases run in order, with one exception: A1 can start alongside F1, because
Anvil-to-Anvil needs no FCP. A2 and A3 can run in parallel.

## 6. What this does not do

- **Recording video.** A one-to-one call's recording stays audio. A room's recording
  already includes its video (`recording_format`).
- **Rooms that forward each participant's stream**, and with them simulcast, a
  personal layout and a WebRTC transport in Anvil. They are a server project first
  (the conference server and forge-conference), and Anvil's side of them is
  prepared by §2.4. Until then every participant sees the room's layout, which a
  host can change for everyone.
- **ICE.** FCP's latching serves video as it serves audio. Strict NATs remain a
  separate question (`docs/SOFTPHONE.md` §5).
- **Scheduling, invitations and calendars.** FCP has none.
- **Hand raise, chat and reactions.** FCP's rooms have none.
- **Video to or from the PSTN, and video voicemail.**

## 7. Questions

Each has a recommendation; all were agreed on 2026-11-08.

1. **Conferences too, or one-to-one only?** *Recommendation:* both (agreed when the
   design was asked for). The rooms exist, and one-to-one needs the same relay.
2. **SIP through the call manager, or WebRTC through the gateway, for rooms?**
   *Recommendation:* SIP for the rooms FCP has. Make Anvil's video independent of
   the transport (§2.4), because forwarding rooms will want WebRTC.

   **Over either transport, Anvil gets the same room.** FCP's rooms mix everyone
   into one picture on the server, so either way Anvil receives:
   - the same composite;
   - the same layout;
   - the same adaptation, which is REMB only, with neither path having
     transport-cc or simulcast.

   **What SIP does better:**
   - *Audio*: the gateway takes G.711 and G.722 and refuses Opus. Over SIP Anvil
     keeps Opus, which matters most in a room.
   - *One stack*: calls and rooms share one media path. The relay of §2.2 is needed
     for one-to-one calls anyway.
   - *The call*: a room joined through the call manager keeps its record, policy,
     recording consent, and transfer from a call into a room.

   **What WebRTC does better: getting through networks.**
   - ICE with TURN over TCP or TLS on 443 passes firewalls that block UDP, and
     strict NATs.
   - It can survive a phone moving from Wi-Fi to cellular. forge-webrtc has no ICE
     restart yet.
   - FCP's SIP path latches, which serves most networks but not locked-down ones.

   That gap is the same for one-to-one calls, so it is a question of its own: ICE
   on Anvil's SIP legs (RFC 8445), or TURN for both. It is not a reason to change
   the rooms' transport.

   **What would change the experience is forwarding, not the transport.** A server
   that forwards each participant's stream, at two or three qualities, lets the app:
   - build its own layout, with a personal pin and gallery;
   - show each tile sharp, as its sender sent it, not as a slice of a re-encoded
     composite;
   - draw names and speaking indicators itself;
   - adapt each tile on its own.

   It also spends far less server encoding, so rooms grow larger. FCP's rooms mix,
   with a forwarding fast path only for a spotlight
   (`docs/VIDEO_CONFERENCING.md`), and list simulcast and SVC egress as non-goals.
   If FCP's rooms forward one day, WebRTC is the natural transport for them
   (BUNDLE, `mid` and `rid`, simulcast, transport-cc). Anvil then adds a
   `forge-webrtc` transport and a layout of its own, on the core this project
   builds.

   Agreed 2026-11-08: per-participant streams are to be supported in the future,
   so §2.4 is a requirement of A1.
3. **Where encoding happens.** *Recommendation:* in the platform host, as coded
   frames across the boundary (§2.3). Hardware codecs, battery and heat decide it on
   phones and laptops. forge-video-codecs is for the server.
4. **Codecs, and H.264's licence on Linux.** *Recommendation:* H.264 constrained
   baseline everywhere, and VP8 where it is cheap.
   - Platforms with hardware H.264: the OS vendor's licence covers it.
   - Linux has none. It uses libvpx (VP8, BSD, no royalties) and downloads Cisco's
     OpenH264 binary at first use, as Firefox does, since only Cisco's binary
     carries Cisco's patent licence.
   - FCP's rooms default to H.264 then VP8, so a Linux Anvil without OpenH264 still
     joins.
5. **Whether video is on by default.** *Recommendation:* on for every tenant
   (`video.calls`), with a class-of-service flag to take it away. The cost is
   bandwidth, not server encoding.
6. **BFCP for Anvil's screen share.** *Recommendation:* no. Take the floor
   implicitly, and learn its state from FCP's content INFO, which the gateway
   already uses. BFCP would need a client written and BFCP relayed through an
   anchored B2BUA, for no gain between FCP and its own client. Room systems keep
   BFCP.
7. **A host's controls through `/me`, or in the dialog?** *Recommendation:* `/me`
   routes under the app session (§2.6). They reuse the conference server's routes
   and the admin's proxy, and say who is acting, which a SIP request from the node
   would not.
8. **The roster: RFC 4575, or FCP's own events?** *Recommendation:* RFC 4575,
   brought up to the RFC (§2.6). Anvil already subscribes over SIP, and room systems
   gain the same roster.
9. **Video on cellular.** *Recommendation:* sent by default, capped at 360p, with a
   setting to send only on Wi-Fi. Received video always follows REMB.
10. **Where the design lives.** *Recommendation:* here, as `docs/SOFTPHONE.md` set
    out. Anvil's `ROADMAP.md` and `ARCHITECTURE.md` link to it in A1.
