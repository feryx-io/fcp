# Video Conferencing — Design

Status: **design, not implemented** (2026-09-04). This document is the
plan of record for adding video to FCP conferences. Nothing below exists
in code yet except the substrate noted in §14. `STATUS.md` tracks
progress; this document describes intent and the decisions behind it.

Read with `docs/DISTRIBUTED_CONFERENCING.md` (the audio conference
architecture this extends) and `docs/MULTI_REGION.md` (placement).

## 1. Decisions

Taken 2026-09-04, after surveying forge-media (no video anywhere: no
codecs, no payload formats, no RTCP feedback, SDP video sections rejected)
and FCP (audio-only conference server, WebRTC gateway, and SDP handling):

1. **The video conference is a mixer (MCU), not a forwarder (SFU).**
   Every participant receives one composited stream in their own codec,
   resolution and profile. Rationale in §2.
2. **Forwarding is a fast path inside the mixer**, used when a
   participant's view is a single source in a codec and profile they can
   decode. It is an optimisation, applied later, not the architecture.
3. **No video decoding or encoding is written from scratch.** Native
   codecs are bound behind cargo features; forge-media builds and its
   tests run without them.
4. **Quality over schedule.** Phases are sized to be reviewed; each ships
   with tests and capacity numbers measured, not estimated.

The questions raised by the first draft were settled the same day (§16):
15 fps and 720p by default, own tile included, OpenH264 built from
source, WebM recordings, screen share and hardware acceleration deferred.

## 2. Why mixing

| Concern | Mixer | Forwarder |
|---|---|---|
| SIP video phones (one `m=video`, one decoder) | Proper layout | Active speaker only, glitch at every switch |
| Mixed codecs in one room (H.264 phone + VP8/AV1 browsers) | Yes, transcoded at the edge | No; everyone shares one codec or is audio-only |
| Client bandwidth and CPU | One stream, any device | N streams; poor on mobile and weak links |
| Recording, compliance | The composite is the recording | Needs the full decode/compose/encode pipeline anyway |
| Cascaded nodes (`DISTRIBUTED_CONFERENCING.md`) | One composite per trunk direction, like audio | Per-source forwarding across nodes, N×M streams |
| Server CPU | ~1 core per 720p encode, ~⅓ core per decode | Near zero |
| Latency | +50–100 ms (decode, compose, encode) | +~0 |
| Quality | One transcoding generation | Lossless |
| Native libraries | libvpx, openh264, rav1e/dav1d | None |

FCP's endpoints are SIP phones first, the audio path is already an MCU
(mixer on a frame clock, one mixed stream per trunk), and recording is a
product requirement. The mixer's costs are CPU and latency, which are
engineering problems with known answers; the forwarder's costs are
product limitations.

## 3. Goals and non-goals

Goals, in delivery order (§15):

- A SIP endpoint offering audio + video joins a room and sees a layout of
  the other participants; audio-only participants appear as avatars.
- H.264 (constrained baseline / main / high, packetization-mode 1), VP8,
  VP9 and AV1 ingress and egress; each participant in the codec, profile
  and resolution it negotiated, up to a per-room cap.
- Layouts: grid, active speaker, spotlight, picture-in-picture; host
  controls (layout, pin, spotlight) via the conference API.
- Loss and bandwidth handling: PLI/FIR, NACK with retransmission, REMB /
  transport-cc driving per-output bitrate.
- Browsers via the WebRTC gateway (DTLS-SRTP, BUNDLE, one video section).
- Recording: composite video + mixed audio to a file.
- Cascaded conferences across nodes carrying video on the trunks.
- Capacity accounting so placement and admission know what a node can
  mix.

Non-goals for now: simulcast/SVC *egress* (we send one layer per
output; simulcast *ingress* from browsers is accepted and the best layer
is used), screen-share as a distinct stream type (a second video section;
after phase 4), hardware acceleration (an encoder trait boundary is kept
for it), H.265 encode (no permissively licensed encoder; decode is
optional), end-to-end encryption, per-participant private layouts beyond
"exclude self".

## 4. Architecture

```
                 ┌────────────────────── conference node ───────────────────────┐
 SIP/RTP  ──►  RTP in ─► SRTP ─► depacketize ─► frame assembler ─► DECODE ─► frame slot ─┐
 (per          (video socket pair per participant)                 (codec pool)          │
  participant)                                                                            ▼
                                        ┌──────────────── ROOM ────────────────────────────────┐
                                        │ video clock (fps)  ──► compositor(layout) ──► canvas │
                                        │ active-speaker ◄── audio mixer energy                │
                                        └───────────────────────────┬──────────────────────────┘
                                                                    ▼
                                             flavor = (codec, profile, WxH, fps, bitrate cap)
                                             ENCODE per flavor (codec pool) ─► RTP packetize
                                                                    │
 SIP/RTP  ◄──  RTP out ◄─ SRTP ◄─ retransmission cache ◄────────────┘   (participants subscribe
                                                                          to a flavor)
 RTCP in (PLI/FIR/NACK/REMB/TWCC) ─► feedback router ─► encoder keyframe / bitrate, NACK cache
 RTCP out (PLI to senders on decoder loss, RR/SR as today)
```

### 4.1 Where it lives

forge-media (new or extended crates):

| Crate | Role |
|---|---|
| `forge-rtp` (extend) | RTCP feedback (PLI, FIR, NACK, REMB, transport-cc), per-codec payload inspection (frame start, keyframe), depacketizers and packetizers (RFC 6184 H.264, RFC 7798 H.265, RFC 7741 VP8, RFC 9628 VP9, AV1 RTP), frame-aware jitter buffer / assembler, retransmission cache, stream rewriter for the passthrough path |
| `forge-video` (new) | `VideoFrame` (I420, owned planes, stride, pts), scaling and colour conversion (own Rust, SIMD later), `Compositor` + `Layout` engine, `VideoClock`, flavor management, the `VideoDecoder` / `VideoEncoder` traits, the raw test codec |
| `forge-video-codecs` (new) | Bindings behind features: `vpx` (VP8/VP9, libvpx), `openh264` (H.264), `av1` (rav1e encode, dav1d decode). Each implements the traits |
| `forge-sdp` (extend) | Video sections, `a=rtcp-fb`, H.264 fmtp, codec choice |
| `forge-webrtc` (extend) | A video section in offers and answers, BUNDLE with two mids, video SRTP, feedback on the shared RTCP path |
| `forge-conference` (extend) | `VideoRoom` beside the audio room: participant video state, subscriptions, layout state, the video clock task |
| `forge-recorder` (extend) | Composite + mixed audio muxing (WebM first, MP4 second) |
| `forge-core` (extend) | `VideoCodec`, `VideoProfile`, resolution and frame-rate types, config |

FCP:

| Crate | Change |
|---|---|
| `fcp-media` | SDP per media type: parse video sections, answer them (or reject with `m=video 0`, which today's answer builder fails to do), anchoring and codec filtering per section |
| `fcp-sip` | Stop requiring audio-only shapes where video is now allowed; pass video sections through the B2BUA correctly; `MediaProfileBuilder::audio_video()` for video-capable offers |
| `fcp-conference-server` | Negotiate video on join and re-INVITE, a video socket pair per participant, the video bridge task, feedback handling, layout API, capacity accounting, trunk video |
| `fcp-core` | `Meeting.video` settings, `VideoCodec` fixes (the existing enum's payload types clash with telephone-event), conference video events |
| `fcp-api-core` / admin UI | Meeting video settings, live layout controls, participant video state |
| `fcp-cluster` | Placement weight: video cost in the load figure conference nodes advertise |
| `fcp-webrtc-gateway` | Video in browser sessions; join rooms directly (today browsers can only reach a room via the B2BUA) |

## 5. Media pipeline

### 5.1 Ingress

Per participant with an accepted video section:

1. A second `PortPair` / `RtpSocketPair` (or the BUNDLE'd transport for
   WebRTC). SRTP as for audio.
2. **Depacketizer** for the negotiated codec reassembles RTP into coded
   frames. It is frame-aware: it knows frame boundaries (marker bit plus
   the codec's own start indication), keyframes, and gaps. It tolerates
   reordering within a small window and declares a frame lost when a gap
   is not filled in time.
3. **Loss handling**: a gap raises a NACK immediately (RFC 4585 early
   feedback, rate-limited); a frame given up on marks the decoder as
   needing a keyframe and sends a PLI upstream (rate-limited to one per
   ~500 ms per source). Until the keyframe arrives, the participant's
   tile shows their last good frame (frozen) then their avatar after 2 s.
4. **Decoder** (codec pool, §9) produces `VideoFrame`s into the
   participant's **frame slot**: a single-slot mailbox holding the latest
   decoded frame. The compositor reads the slot at its own clock; a
   participant sending 30 fps into a room composited at 15 fps costs
   decode work but no extra composition.
5. The negotiated maximum resolution is enforced: frames larger than the
   room cap are rejected at the depacketizer (SPS / keyframe header
   inspection) and the sender is asked for a lower level via REMB or a
   re-INVITE with a tighter `profile-level-id`.

### 5.2 Room clock and composition

Each room with at least one video participant runs a **video clock** at
the room's frame rate (default 15 fps for phone-heavy rooms, 30 fps
configurable). On every tick the compositor renders the current layout
onto a **canvas** per *layout output* (§5.3). Composition is plain I420
work: scale each source frame into its tile (letterboxed, never
stretched), blit, draw name labels and the speaking indicator, and draw
avatars for audio-only participants.

Active-speaker state comes from the audio mixer's per-participant energy
(already computed) with hysteresis: a speaker must dominate for 800 ms to
take the spotlight and keeps it for at least 2 s, so the layout does not
flap on cross-talk.

### 5.3 Flavors and encoders

A **flavor** is what a receiver can consume: `(codec, profile/fmtp,
width×height, fps, max bitrate)`. Participants **subscribe to a flavor of
a layout output**; participants with the same needs share one encoder.
A ten-participant room of identical H.264 phones is one encode; a room
with three phones and two browsers is two or three.

The canvas is rendered once per distinct `(layout, resolution)` and fed
to every encoder of that resolution. Encoders run on the codec pool; the
packetizer emits RTP into a per-flavor **retransmission cache** (last
~500 ms of packets) that answers NACKs, then per-subscriber SRTP and
send. Sequence numbers and timestamps are per subscriber (each receiver
sees its own continuous stream), which is what lets a subscriber move
between flavors without a re-INVITE.

**Self view.** By default a participant's own tile is included, because
it is what makes flavor sharing possible. `exclude_self = true` on a
meeting gives each participant a private layout output and therefore a
private encode; the capacity model (§9) charges for it.

### 5.4 Keyframes and joining

An encoder emits a keyframe on start, on a PLI/FIR from any subscriber
(coalesced, at most one per second per encoder), when a new subscriber
attaches, and on a fixed maximum interval (default 10 s) so late joiners
and lossy links recover on their own. Because subscribers share an
encoder, a new subscriber costs every other subscriber of that flavor one
keyframe; that is the price of sharing and is bounded by the rate limit.

### 5.5 Passthrough fast path (later)

When a layout output is a single source (spotlight, or active speaker
with one video participant) and a subscriber's flavor matches the source
(same codec, forwardable profile per `H264Fmtp::forwardable_to`,
resolution within cap), the source's coded frames are forwarded instead
of decoded and re-encoded: the **stream rewriter** keeps one continuous
SSRC / sequence / timestamp for the receiver and switches sources only at
a keyframe, requesting one with PLI. This removes an encode and a decode
and ~40 ms of latency in the common two-party and "presenter" cases. It
is transparent to signaling and can be turned off per room.

## 6. Codecs and licensing

| Codec | Library | License | Binding | Notes |
|---|---|---|---|---|
| VP8, VP9 | libvpx | BSD-3 | `env-libvpx-sys` (system lib via pkg-config) | Mature; VP8 is the WebRTC baseline codec |
| H.264 | OpenH264 (built from source) | BSD-2 | `openh264` crate (`source` feature) | Cisco's patent grant covers only Cisco's binary; building from source means H.264 royalties are the deployer's matter. Documented in deployment notes. Encoder is constrained baseline/main only (no B-frames), which is what SIP phones want anyway |
| AV1 encode | rav1e | BSD-2 | `rav1e` (pure Rust) | Speed presets 8–10 for real time; CPU-heavy, browsers only |
| AV1 decode | dav1d | BSD-2 | `dav1d` crate (system lib) | Fast; multi-threaded |
| H.265 | — | — | — | Decode only if a permissive decoder is adopted later; no encode |

Each binding lives behind its own cargo feature in `forge-video-codecs`;
`forge-video` depends on none of them and ships a **raw I420 test codec**
(`codec = "raw"`) so the pipeline, layouts and the conference server are
tested in CI without native libraries. The CI image installs the native
libraries and runs the codec crate's own tests. Exact crate versions and
build prerequisites are settled in the spike (§15, phase 0).

Colour: everything internal is I420 (4:2:0, limited range, BT.709 for
≥720p, BT.601 below). Sources in other formats are converted at decode.

## 7. Feedback, loss and adaptation

| Signal | Direction | Action |
|---|---|---|
| PLI / FIR from a receiver | in | Keyframe from that subscriber's encoder (rate-limited); on the passthrough path, forwarded to the source |
| NACK from a receiver | in | Retransmit from the flavor's cache (RTX-less, same SSRC, per RFC 4585); ignore if older than the cache |
| REMB / transport-cc from a receiver | in | Subscriber's allowed bitrate; the flavor's encoder targets the **minimum** over its subscribers, and a subscriber far below the others is moved to a lower-resolution flavor (a bitrate ladder per resolution: e.g. 1080p 2.5 Mb/s, 720p 1.2 Mb/s, 360p 500 kb/s, 180p 200 kb/s) |
| Gap at our depacketizer | out | NACK to the source, then PLI if the frame is lost |
| Our decoder errors | out | PLI to the source |
| RR/SR | both | As today; jitter and loss feed the participant stats |

Every feedback type is negotiated in SDP (`a=rtcp-fb`). A peer that offers
none gets keyframes on the fixed interval only and no retransmission;
SIP phones commonly offer `nack pli` and `ccm fir`.

## 8. Layouts

Layouts are defined declaratively: a layout is a function from
`(tiles: ordered participant list, canvas size) → tile rectangles`, and
the compositor is the same for all of them.

| Layout | Shape |
|---|---|
| `grid` | N tiles in the tightest grid (1, 2×1, 2×2, 3×2, 3×3, 4×3, 4×4), up to a cap; overflow participants are audio-only tiles cycled by recent speech |
| `active_speaker` | The speaker large, up to 5 others in a strip |
| `spotlight` | One pinned participant full canvas (host sets; e.g. a presenter) |
| `pip` | Spotlight plus the active speaker in a corner |

Tile order is stable (join order) except the speaker strip, which is
recency of speech. Each tile draws the display name, a speaking indicator,
and a muted glyph; audio-only participants draw an avatar (initials).
Fonts and drawing are a small bitmap-font rasteriser in `forge-video`; no
freetype dependency.

Per meeting: default layout, `max_video_participants` (tiles), canvas
resolution cap (default 720p, up to 1080p), fps (default 15, up to 30),
`exclude_self` (default off). At runtime the host (or the API)
changes layout, pins, spotlights, and can turn a participant's video off.

## 9. Threading, CPU and capacity

- Decoders and encoders run on a **dedicated codec thread pool** (size =
  cores − 2 by default), never on tokio workers; the async side hands
  frames over through bounded channels and drops rather than queues when
  behind (video is real time: a late frame is worthless).
- One compositor task per room, on the pool, ticked by the video clock.
- Frame budgets: the compositor must finish within one tick; if it
  overruns three ticks in a row the room's fps is halved and an event is
  emitted.

**Capacity model.** Each node advertises a video budget in "work units"
alongside `active_calls` in cluster membership. Costs (measured in
phase 0 on reference hardware, re-measured per node at start with a
30-second self-benchmark):

```
decode_cost(codec, W×H, fps)   ≈ k_dec(codec) × W×H×fps
encode_cost(codec, W×H, fps)   ≈ k_enc(codec) × W×H×fps
compose_cost(tiles, W×H, fps)  ≈ k_cmp × W×H×fps × (1 + 0.1×tiles)
room_cost = Σ decode + Σ_flavors encode + Σ_outputs compose
```

Admission: a join that would push the node over budget is refused with
the same 503 path as the participant cap, and placement
(`choose_conference_node`) prefers the node with the most video budget
left among those in the caller's region. `conference_node_capacity`
remains the participant count cap; the video budget is the second
constraint.

The dev machine (12 cores, AVX2, **2 GB RAM**) is enough for the phase 0
benchmarks at 720p; memory, not CPU, bounds how many 1080p encoders can
be measured at once here.

## 10. Cascading across nodes

Follows the audio design: the trunk between two nodes hosting a room
carries **one composite per direction** as a video section on the trunk
INVITE (same mutual-TLS INVITE, an extra `m=video`). On the receiving
node the trunk is one participant whose tile is the far side's
composite; active-speaker state crosses through the existing cross-node
control channel so both sides agree on the speaker. This gives correct
audio+video behaviour with N×(N−1) trunk encodes for N hosting nodes,
which is fine for the two- and three-node rooms placement produces.
Per-participant tiles across nodes (forwarding decoded or coded frames
over the trunk) are a later refinement.

## 11. Recording

Composite (the room's default layout at its capped resolution) plus mixed
audio, muxed to **WebM** (VP8 or VP9 + Opus; every part permissively
licensed) by default; **MP4** (H.264 + Opus-in-MP4, or AAC if an encoder
is ever adopted) as an option for players that need it. The recorder is
one more subscriber of a dedicated flavor, so recording never changes what
participants see. Per-participant video recording is not planned.

## 12. Signaling and platform surface

**SDP** (fcp-media, fcp-sip): handle media sections per type. A video
section is answered when the room allows video and a codec is agreed;
otherwise it is *rejected properly* (`m=video 0 …` with the offer's
formats), which fixes the RFC 3264 violation the survey found. Anchoring
and codec filtering operate per section. Adding video mid-call is a
re-INVITE with a new video section; removing it is a re-INVITE with port
0.

**WebRTC** (forge-webrtc, gateway): BUNDLE with `audio` and `video` mids
on one DTLS-SRTP transport, `rtcp-mux`, feedback attributes, simulcast
ingress accepted (highest layer used). The gateway gains a direct
"join room" API so browsers do not round-trip through the B2BUA.

**Meeting model** (fcp-core, migration, API, admin UI): `video_enabled`,
`video_layout`, `video_max_participants`, `video_max_resolution`
(`180p`…`1080p`), `video_fps`, `video_exclude_self`, `video_codecs`
(preference order).

**Conference API**: participant JSON gains `video: {codec, resolution,
fps, bitrate_kbps, keyframes_sent, nacks, plis, state}`; room JSON gains
`video: {layout, spotlight, fps, outputs: [...]}`; new endpoints
`PUT /conferences/{room}/video/layout`, `POST …/video/pin/{pid}`,
`POST …/video/spotlight/{pid}`, `POST …/participants/{pid}/video/{on|off}`,
mirrored on the internal cross-node router.

**Events**: `conference.participant_joined` gains `has_video`;
new `conference.video_layout_changed`, `conference.video_state_changed`
(participant video on/off/lost), `conference.video_overload` (fps halved).

**Placement**: conference nodes advertise `video_budget_left`; the
selector uses it (§9).

## 13. Security and robustness

- SRTP for video as for audio (SDES on SIP, DTLS-SRTP on WebRTC); a room
  with `enable_srtp` refuses plaintext video.
- Decoders are the largest new attack surface: they run behind size and
  rate limits (max coded frame size, max resolution, max fps), inputs are
  fuzzed in `forge-media/fuzz`, and a decoder failure disables that
  participant's video rather than the room.
- Feedback is rate-limited per source and per subscriber (PLI floods must
  not become keyframe floods); NACKs beyond the cache are ignored.
- Trunk video inherits the trunk's mutual TLS and SRTP.

## 14. Testing

- **Without native codecs** (every CI job): the raw I420 test codec drives
  the whole pipeline: depacketize → assemble → "decode" → compose →
  "encode" → packetize, with golden-image assertions on layouts (small
  canvases, exact pixel checks), clock and overrun behaviour, flavor
  sharing, keyframe and NACK logic, feedback rate limits, the stream
  rewriter.
- **With native codecs** (the codec crate's tests, one CI job with the
  libraries installed): encode → decode round trips per codec with PSNR
  thresholds, keyframe-on-demand, bitrate targeting, and the phase 0
  benchmark as a test that records numbers rather than asserting them.
- **Conference server**: SIP-level tests with `m=audio + m=video` offers
  (fixtures beside `minimal_sdp()`): accept, reject-properly, re-INVITE
  add/remove, and a loopback RTP test that a raw-codec participant's
  frames reach a second participant's socket composited.
- **Browser**: the existing `tests/browser` page gains `video: true`.
- **Fuzz**: depacketizers and the H.264/VP8/VP9/AV1 inspectors.

Substrate already written (unmerged, forge-media branch
`feature/video-foundation`): RTCP feedback packets (PLI, FIR, NACK, REMB,
compound parsing), per-codec keyframe inspection, the stream rewriter,
`VideoCodec`, and SDP video helpers. It is kept for phase 1.

## 15. Phases

| Phase | Deliverable | Where | Rough size |
|---|---|---|---|
| 0 | **Spike and numbers**: install build prerequisites, get libvpx, OpenH264, rav1e/dav1d building through their crates, write the benchmark, record `k_dec` / `k_enc` / `k_cmp` for this machine; confirm crate choices and pin versions | forge-media (throwaway branch) | 1 week |
| 1 | **Substrate**: RTCP feedback, payload formats (depacketize/packetize) for H.264, VP8, VP9, AV1, frame assembler, retransmission cache, SDP video, `VideoCodec`; the branch from §14 finished | forge-rtp, forge-sdp, forge-core | 2 weeks |
| 2 | **forge-video**: frames, scaling, compositor + layouts, clock, flavors, codec traits, raw codec; **forge-video-codecs** with the four bindings | forge-media | 3–4 weeks |
| 3 | **Conference video (SIP)**: fcp-media/fcp-sip SDP per section, meeting model + API + events, conference server ingress/egress, layouts and host controls, capacity accounting and placement | FCP | 3–4 weeks |
| 4 | **WebRTC**: video section in forge-webrtc, gateway join-room, browser test | forge-media, FCP | 2 weeks |
| 5 | **Recording and cascade**: WebM/MP4 muxing, trunk video, cross-node speaker state | both | 2–3 weeks |
| 6 | **Passthrough fast path, bitrate ladder, polish**: single-source forwarding, ladder switching, overload handling, fuzzing | both | 2 weeks |

Each phase is one PR per repository (forge-media release tag, then the
FCP bump), reviewed before the next starts. Sizes are for one person and
will be corrected after phase 0.

## 16. Resolved questions

Raised with the first draft and decided 2026-09-04, each as recommended.

| Question | Decision |
|---|---|
| Default frame rate | **15 fps** by default; 30 fps per meeting. Half the CPU, and what phone-heavy rooms want |
| Self view | **Own tile included** by default so encoders are shared; `exclude_self` per meeting, charged in the capacity model |
| H.264 library | **OpenH264 built from source**, feature-gated. Reproducible; the C++ toolchain is only needed with the feature on |
| Recording container | **WebM** (VP8/VP9 + Opus) by default, every part permissively licensed; MP4 (H.264 + Opus) as an option |
| Screen share | Section handling designed in phase 3 so a second video section (RFC 4796 `a=content:slides`) is additive; **implemented after phase 6** |
| Hardware acceleration | **No work now.** The encoder trait boundary is kept so VAAPI / NVENC via ffmpeg can be added |
| Maximum resolution | **720p** by default; 1080p per meeting |
| Dev machine memory | Phase 0 measures **relative** constants here (12 cores, 2 GB); absolute capacity is re-measured once on a production-shaped node before phase 3, and every node self-benchmarks at start |

## 17. References

RFC 3550 (RTP), RFC 4585 (RTCP feedback: PLI, NACK), RFC 5104 (FIR,
TMMBR), RFC 6184 (H.264 RTP), RFC 7798 (H.265 RTP), RFC 7741 (VP8 RTP),
RFC 9628 (VP9 RTP), AV1 RTP payload specification (AOM), REMB
(draft-alvestrand-rmcat-remb), transport-cc
(draft-holmer-rmcat-transport-wide-cc-extensions), RFC 3264 (offer/
answer), RFC 4796 (content attribute), RFC 8829 (JSEP), RFC 8834 (WebRTC
media transport).
