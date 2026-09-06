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
5. **GPU is a first-class backend, designed in now, built after the CPU
   path.** Frames know where they live (host or device), the whole
   pipeline runs on one device, and nodes advertise what they can mix.
   The CPU path is the reference implementation the GPU path is tested
   against. Rationale in §9.1.

The questions raised by the first draft were settled the same day (§16):
15 fps and 720p by default, own tile included, OpenH264 built from
source, WebM recordings, screen share deferred, GPU designed in.

## 2. Why mixing

| Concern | Mixer | Forwarder |
|---|---|---|
| SIP video phones (one `m=video`, one decoder) | Proper layout | Active speaker only, glitch at every switch |
| Mixed codecs in one room (H.264 phone + VP8/AV1 browsers) | Yes, transcoded at the edge | No; everyone shares one codec or is audio-only |
| Client bandwidth and CPU | One stream, any device | N streams; poor on mobile and weak links |
| Recording, compliance | The composite is the recording | Needs the full decode/compose/encode pipeline anyway |
| Cascaded nodes (`DISTRIBUTED_CONFERENCING.md`) | One composite per trunk direction, like audio | Per-source forwarding across nodes, N×M streams |
| Server CPU | measured (§9.4): 8–17 H.264 or 3–6 VP8 720p30 encodes per core, 20–45 decodes per core | Near zero |
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
after phase 4), H.265 encode on the CPU path (no permissively licensed
encoder; decode is optional; hardware H.265 encode comes with the GPU
backend), end-to-end encryption, per-participant private layouts beyond
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
| `forge-video` (new) | `VideoFrame` (host I420 or a device-resident handle; §5.5), scaling and colour conversion (own Rust, SIMD later), `Compositor` + `Layout` engine, `VideoClock`, flavor management, the `VideoDecoder` / `VideoEncoder` / `Scaler` / `Compositor` traits with device affinity, the `MediaDevice` abstraction, the raw test codec |
| `forge-video-codecs` (new) | Bindings behind features: `vpx` (VP8/VP9, libvpx), `openh264` (H.264), `dav1d` (AV1 decode), `svt-av1` (AV1 encode), and `hwaccel` (FFmpeg hardware contexts: VAAPI / QuickSync, NVENC / NVDEC, AMF) providing device codecs, scaling and compositing on the device. Each implements the traits |
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

### 5.5 Devices and frame residency

Every stage of the pipeline is bound to a **media device**: the host
(CPU), or a GPU (`vaapi:/dev/dri/renderD128`, `cuda:0`, …). A
`VideoFrame` is either host I420 or a handle to memory on one device,
and a stage only accepts frames resident where it runs. A room's video
pipeline is placed on **one device end to end** — decode, frame slot,
compositor, scaler, encoder — so on a GPU node nothing but RTP crosses
the PCIe bus. Mixing devices within a room (a GPU decode feeding a CPU
compositor) is a copy the design allows but the scheduler avoids; it is
what happens when a GPU's decoder cannot handle a codec (an AV1 source on
a card without AV1 decode falls back to dav1d on the host, and its
frames are uploaded once per tick).

The host device is the reference implementation: a GPU device must
produce the same layouts within a PSNR tolerance and the same RTP shape,
and the golden-image tests run against both when hardware is present.

### 5.6 Passthrough fast path (later)

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
| AV1 encode | **SVT-AV1** (libsvtav1 ≥ 2.3) | BSD-2 + patent grant | own bindgen against the system header (the published `svt-av1-sys` crate pins a bindgen that conflicts with libvpx's) | rav1e was the first choice and measured at 1.8 fps for 720p on one core at its fastest preset (§9.4): not a real-time encoder. SVT-AV1 preset 12 in low-delay CBR mode is (2–4 streams per core, §9.4). Its packet API blocks in low-delay mode, so the encoder gets a reader thread. AV1 egress stays optional on the CPU path and comes for free on GPUs with AV1 engines |
| AV1 decode | dav1d | BSD-2 | `dav1d` crate (system lib) | Fast; multi-threaded |
| H.265 | — | — | — | CPU path: decode only if a permissive decoder is adopted later; no encode. GPU path: encode and decode where the hardware has it |
| GPU (all of the above) | FFmpeg hardware acceleration: VAAPI and oneVPL/QuickSync (Intel), NVENC/NVDEC (NVIDIA, via `nv-codec-headers`, MIT), AMF (AMD) | FFmpeg LGPL 2.1+ (dynamic link, built without GPL parts such as x264); vendor drivers proprietary at runtime | `ffmpeg-next` / `ffmpeg-sys-next` behind the `hwaccel` feature | One API over every vendor, with device frame contexts and hardware scale/overlay filters for composition. Direct vendor SDKs are the fallback if FFmpeg's overlay path proves too slow |

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

## 9. Threading, CPU, GPU and capacity

### 9.1 Why the GPU matters here

Composition is cheap on either side: scaling and blitting a 720p canvas
is memory bandwidth. The cost is codecs, and a mixer pays a decode per
participant and an encode per flavor. Fixed-function video engines
change that by an order of magnitude. Expected density for 720p30 H.264,
from vendor material and public benchmarks, **to be replaced by phase 0
measurements**:

| Device | Encode streams | Decode streams |
|---|---|---|
| One CPU core, software | ~1 | ~3–5 |
| Consumer NVIDIA GeForce | driver-capped at 8 sessions | ~30+ |
| Intel iGPU / Arc (QuickSync), one media engine | ~20–40 | ~60–100 |
| Datacenter (NVIDIA T4 / L4 / A10, Intel Flex 140 / 170) | ~40–100+ | ~100–250 |

Caveats the design carries: hardware encoders are worse per bit than
software (expect 20–30 % more bitrate for the same quality; the ladder
absorbs it), GeForce parts are session-capped by the driver whatever
their throughput, and cloud GPU instances are a specific SKU. Hence: GPU
is an advertised capability, the CPU path is always there, and placement
decides.

### 9.2 Threading

- Decoders and encoders run on a **dedicated codec thread pool** (size =
  cores − 2 by default), never on tokio workers; the async side hands
  frames over through bounded channels and drops rather than queues when
  behind (video is real time: a late frame is worthless).
- One compositor task per room, on the pool, ticked by the video clock.
- Frame budgets: the compositor must finish within one tick; if it
  overruns three ticks in a row the room's fps is halved and an event is
  emitted.

### 9.3 Capacity model

Each node advertises a video budget **per device** in "work units"
alongside `active_calls` in cluster membership: `video_budget: [{device,
codecs, left, total}]`. Costs (measured in phase 0 on reference hardware,
re-measured per node and per device at start with a 30-second
self-benchmark):

```
decode_cost(device, codec, W×H, fps)   ≈ k_dec(device, codec) × W×H×fps
encode_cost(device, codec, W×H, fps)   ≈ k_enc(device, codec) × W×H×fps
compose_cost(device, tiles, W×H, fps)  ≈ k_cmp(device) × W×H×fps × (1 + 0.1×tiles)
room_cost(device) = Σ decode + Σ_flavors encode + Σ_outputs compose
```

Admission: a join that would push every device on the node over budget
is refused with the same 503 path as the participant cap. Placement
(`choose_conference_node`) prefers, among live conference nodes in the
caller's region, the one with the most budget left on a device that has
the room's codecs; a GPU node therefore takes video rooms before a CPU
node does, and CPU nodes are the overflow. `conference_node_capacity`
remains the participant count cap; the video budget is the second
constraint. A room stays on the device it started on.

The dev machine (12 cores, AVX2, **2 GB RAM**) is enough for the phase 0
benchmarks at 720p; memory, not CPU, bounds how many 1080p encoders can
be measured at once here.

### 9.4 Phase 0 results (2026-09-04)

Measured with the throwaway `forge-video-spike` crate (forge-media branch
`spike/video-codecs`): 1280×720 at 30 fps, 150 synthetic frames, one
thread per codec, 1.2 Mb/s target, on this VM (Intel Core Ultra 9 275HX,
AVX2, 12 cores, 2 GB). Two sources bracket real cameras: *easy* (smooth
gradient, one moving block, ±8 noise) and *hard* (three moving blocks,
scrolling checker texture, ±32 noise; frame skipping off so every frame
is encoded). `ns/px` is `k` in §9.3; "streams" is 720p30 streams per
core.

| Stage | easy ns/px | hard ns/px | streams per core (easy / hard) |
|---|---|---|---|
| H.264 encode, OpenH264 baseline, complexity low | 2.1 | 4.4 | 17 / 8 |
| H.264 decode, OpenH264 | 0.8 | 1.4 | 45 / 26 |
| VP8 encode, libvpx cpu-used 8 (4 is no slower) | 5.8 | 11.7 | 6 / 3 |
| VP8 decode, libvpx | 1.1 | 1.8 | 33 / 21 |
| VP9 encode, libvpx cpu-used 8 | 6.9 | 9.1 | 5 / 4 |
| VP9 decode, libvpx | 1.2 | 1.7 | 31 / 22 |
| AV1 encode, rav1e speed 10 | 596 | — | 0.06 (not real time) |
| AV1 encode, SVT-AV1 2.3 preset 12 (maps to 11), low delay, CBR — *CPU time* | 9.5 | 17.2 | 3.8 / 2.1 |
| AV1 encode, SVT-AV1 preset 10 — *CPU time* | 13.5 | 21.7 | 2.7 / 1.7 |
| AV1 encode, SVT-AV1 preset 8 — *CPU time* | 20.5 | 75.6 | 1.8 / 0.5 |
| AV1 decode, dav1d, 1 thread, of SVT-AV1 output | 1.1 | 1.9 | 32 / 19 |
| compose 3×3 grid of 720p sources → 720p, scalar Rust bilinear | 1.8 | 1.9 | 19 canvases |
| compose 4×4 | 2.0 | 2.0 | 18 canvases |
| 4 encoders in parallel | 3.9× one encoder's throughput | 3.9× | cores add up |

What this changes:

- **CPU mixing is cheaper than the design assumed.** "One core per 720p
  encode" is off by 3–17× for H.264 and 3–6× for VP8. A 12-core node
  mixes on the order of 100 easy or 50 hard H.264 outputs before anything
  else, or 20–35 VP8 outputs. The GPU case (§9.1) remains one of density
  and power, not of feasibility.
- **Composition is a rounding error**: a 3×3 canvas costs about the same
  as one H.264 encode of easy content and a fifth of a VP8 encode, before
  any SIMD. §9.3's `k_cmp` term is real but small.
- **Decode is cheap**: 20–45 streams per core for every codec but AV1.
- **rav1e is out for encoding; SVT-AV1 is in.** rav1e manages 1.8 fps on
  one core at its fastest preset. SVT-AV1 preset 12 in low-delay CBR mode
  costs 9.5–17 CPU-ns/px — 2–4 720p30 streams per core, about half of
  VP8 — with 1.2 Mb/s held on the easy source (wall-clock throughput is
  ~3× higher because the library threads internally whatever `lp` says,
  so its constant is measured in CPU time). Preset 10 buys little; preset
  8 is not real time on hard content. dav1d decodes SVT-AV1 output at
  19–32 streams per core; the 4-per-core figure above came from rav1e's
  17 Mb/s bitstream, not from AV1 itself.
- **Rate control needs tuning per codec**: VP9 at cpu-used 8 overshot
  1.2 Mb/s by 25 %; OpenH264 cannot hold a bitrate without frame
  skipping, which the pipeline must expect (a skipped frame is a repeated
  frame, not a stall).
- The synthetic sources are a bracket, not camera footage; phase 2's
  benchmark uses real clips. Absolute numbers are for this CPU; every
  node self-benchmarks at start (§9.3).

Build notes for phase 2: `env-libvpx-sys` needs its `generate` feature
(bindgen) against libvpx ≥ 1.14, since its bundled bindings stop at 1.13;
`openh264` with the `source` feature builds cleanly with nasm and a C++
compiler; `dav1d` needs `libdav1d-dev` via pkg-config; the libvpx config
struct must be initialised by `vpx_codec_enc_config_default`, never
zeroed (its bit-depth enum has no zero variant).

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
0. *Shipped in 3a:* `fcp_media::construct_answer` answers one section
per offered section (`AnswerOptions` / `VideoAnswer` carry the allowed
codecs and video port; `Answer.video` reports what was agreed),
`rewrite_for_anchoring` takes a port per section and disables the rest,
`filter_sdp_codecs` touches only the audio section,
`remote_audio_endpoint` / `remote_video_endpoint` read each section's
own `c=`, and `parse_video_offer` summarises what a peer offered. The
B2BUA relays a video section untouched when it passes media through and
disables it (port 0) when it anchors; hold and resume apply to every RTP
section; an offer with only a video section is no longer refused.

**WebRTC** (forge-webrtc, gateway): BUNDLE with `audio` and `video` mids
on one DTLS-SRTP transport, `rtcp-mux`, feedback attributes, simulcast
ingress accepted (highest layer used). The gateway gains a direct
"join room" API so browsers do not round-trip through the B2BUA.

**Meeting model** (fcp-core, migration, API, admin UI): `video_enabled`,
`video_layout`, `video_max_participants`, `video_max_resolution`
(`180p`…`1080p`), `video_fps`, `video_exclude_self`, `video_codecs`
(preference order). As implemented, the defaults and ranges are:
`video_enabled` false; `video_layout` `grid` (one of `grid`,
`active_speaker`, `spotlight`, `pip`); `video_max_participants` 16
(1–16 tiles in the composite); `video_max_resolution` `720p` (one of
`180p`, `360p`, `720p`, `1080p`); `video_fps` 15 (1–30);
`video_exclude_self` false; `video_codecs` `["H264","VP8"]` (SDP names,
at least one, no repeats) — `Meeting::validate_video_settings` enforces
these and every create/update API returns 400 with the message.

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
  With the `hwaccel` feature on a GPU host: the same round trips per
  device, and layout parity against the host compositor within a PSNR
  threshold.
- **Conference server**: SIP-level tests with `m=audio + m=video` offers
  (fixtures beside `minimal_sdp()`): accept, reject-properly, re-INVITE
  add/remove, and a loopback RTP test that a raw-codec participant's
  frames reach a second participant's socket composited.
- **Browser**: the existing `tests/browser` page gains `video: true`.
- **Fuzz**: depacketizers and the H.264/VP8/VP9/AV1 inspectors.

The substrate shipped in phase 1 (forge-media `v2026.09.05`): RTCP
feedback packets (PLI, FIR, NACK, REMB, compound parsing), per-codec
keyframe inspection, packetizers and depacketizers, the frame assembler,
the retransmission cache and keyframe gate, the stream rewriter,
`VideoCodec`, and the SDP video helpers — each with unit tests in its
crate (`forge-rtp` 111, `forge-sdp` 90, `forge-core` 22).

## 15. Phases

| Phase | Deliverable | Where | Rough size |
|---|---|---|---|
| 0 | **Spike and numbers** — CPU half done 2026-09-04 (§9.4): libvpx, OpenH264, dav1d, SVT-AV1 build and are measured; rav1e dropped. Open: on a machine with an Intel or NVIDIA GPU (this VM has none), FFmpeg hardware encode/decode through `ffmpeg-next` and the same constants per device | forge-media (`spike/video-codecs`) | done except GPU |
| 1 | **Substrate** — done 2026-09-05, forge-media `v2026.09.05` (forge-rtp 0.4.0, forge-core 0.2.1, forge-sdp 0.2.1): RTCP feedback, payload formats for H.264, H.265, VP8, VP9, AV1, frame assembler, retransmission cache, keyframe gate, stream rewriter, SDP video negotiation (`answer_video`, `reject_section`). Open for phase 3: sip-sdp lacks an `RTP/AVPF` protocol variant | forge-rtp, forge-sdp, forge-core | done |
| 2 | **Mixer core** — done 2026-09-05, forge-media `v2026.09.05.1` (forge-video 0.1.0, forge-video-codecs 0.1.0, forge-core 0.2.2): frames with device residency, host scaling and the `Scaler` trait, the four layouts, the `Compositor` trait and `HostCompositor`, bitmap font, `VideoClock`, flavors and the bitrate ladder, codec traits and registry, raw codec, PSNR; libvpx, OpenH264, dav1d and SVT-AV1 bindings behind features with round-trip, bitrate and speed tests, built in CI. Open: real camera clips for the benchmark; SIMD scaling if the numbers ask | forge-media | done |
| 3 | **Conference video (SIP)**: fcp-media/fcp-sip SDP per section, meeting model + API + events, conference server ingress/egress, layouts and host controls, capacity accounting and placement | FCP | 3–4 weeks |
| 4 | **WebRTC**: video section in forge-webrtc, gateway join-room, browser test | forge-media, FCP | 2 weeks |
| 5 | **Recording and cascade**: WebM/MP4 muxing, trunk video, cross-node speaker state | both | 2–3 weeks |
| 6 | **Passthrough fast path, bitrate ladder, polish**: single-source forwarding, ladder switching, overload handling, fuzzing | both | 2 weeks |
| 7 | **GPU backend**: `hwaccel` feature (FFmpeg device contexts), device codecs, device scale/overlay composition, per-device budgets and self-benchmark, placement by capability, PSNR parity tests against the host path | forge-media, FCP | 3–4 weeks + a GPU box |

Each phase is one PR per repository (forge-media release tag, then the
FCP bump), reviewed before the next starts. Sizes are for one person and
will be corrected after phase 0.

### 15.1 Phase 3 plan (2026-09-05)

Phase 3 is the first FCP-side phase and, at three to four weeks, too
large for one review. It is delivered as three PRs in order, each
complete and shippable on its own; the FCP surveys that produced this
plan found more wrong with today's SDP handling than §12 assumed, which
is why signaling gets a block of its own.

**What the surveys found.** `fcp-media` answers every offer with a
single synthesised `m=audio` (no `m=video 0`, the RFC 3264 violation);
its anchoring rewrite points a video section's `c=` at the server while
leaving the video port alone; its codec filter strips video `rtpmap`s
but keeps the video formats; `fcp-sip` rejects any offer without audio
and applies hold/resume to the first audio section only; the conference
server's codec picker and endpoint parser see only audio, and the
endpoint parser takes the IP from the *last* `c=` line, which a video
section would supply; `fcp-core`'s `VideoCodec` gives VP9 payload type
101, the telephone-event type, and lacks AV1; and `sip-sdp` parses
`RTP/AVPF` as `RTP/AVP` followed by a stray character, so a
feedback-capable offer is mis-read today.

| Block | Deliverable | Where |
|---|---|---|
| 3a | **Signaling foundations** — implemented 2026-09-05 (FCP v0.25.96; siphon-rs sip-sdp 0.3.2). `sip-sdp`: `RTP/AVPF`. `fcp-media`: one answer section per offered section, in order (`answer_video` / `reject_section` from forge-sdp), a `VideoPolicy` input (codecs allowed, or none), section-aware anchoring with a port per section, section-scoped codec filtering, per-section remote endpoints. `fcp-sip`: accept any offer with a usable section, hold/resume per section, the B2BUA relays a video section untouched when it is not anchoring media and rejects it properly (`m=video 0`) when it is. Conference server: answers per section (rejecting video until 3b) and reads the audio endpoint from the audio section. `fcp-core`: `VideoCodec` aligned with forge-core's five codecs, no fixed payload types (types come from the offer); `Meeting.video_*` settings (§12) with migration, API DTOs and validation, admin UI settings, `has_video` on `participant_joined`. Fixtures with `m=audio + m=video` beside `minimal_sdp()` | siphon-rs, fcp-media, fcp-sip, fcp-core, fcp-storage-postgres, fcp-api-core, fcp-admin-ui, fcp-conference-server |
| 3b | **Conference video** — implemented 2026-09-06 (FCP v0.25.98; forge-media `v2026.09.06`: forge-conference 0.5.0, forge-mixer 0.4.0). forge-conference `video` module: `CodecPool` (dedicated threads, `cores − 2`), `VideoRoom` beside the audio room (`ConferenceRoom::enable_video`), per-participant sources (assembler → decoder on the pool → frame slot), the clock task composing one canvas per layout output and encoding once per flavor, subscribers with their own SSRC / sequence space / retransmission cache, keyframe coalescing, NACK-then-PLI per source through gates, REMB to the slowest receiver, decoder limits that fail a source not the room, tile ordering per layout, `ActiveSpeaker` hysteresis on the mixer's new `ParticipantMetadata::energy`, `forge_conference_video_*` metrics. Conference server: the video leg (`video_bridge`: second port pair and socket, its own SDES key, `run_video_bridge` moving RTP and RTCP both ways), meeting settings → `VideoRoomSettings`, join / re-INVITE add and remove / UPDATE / BYE, `video_state_changed` and `video_overload` events, `video` in participant and room JSON, a loopback test through real sockets on the raw codec. *As built:* the room is transport-free like the audio room (the server drains each subscriber's packets from a bounded channel); the retransmission cache is per subscriber, since sequence numbers are; a source with three coded frames waiting for the pool drops the frame and re-keys rather than queueing; a participant admitted after waiting for a moderator runs the video bridge joined with the audio bridge; a room with `enable_srtp` refuses plaintext video; `conference.video_native_codecs_only` (default on) offers callers only codecs with a native binding compiled in (`video-vpx`, `video-openh264`, `video-dav1d` + `video-svt-av1` features on fcp-conference-server), the raw test codec being CI's; layout, pin and spotlight exist on the room and get their API in 3c; trunks, dial-out and WebRTC stay audio-only until phases 4–5 | forge-media (release), fcp-conference-server, fcp-events, fcp-media |
| 3c | **Controls and capacity.** Layout / pin / spotlight / participant video on-off endpoints with the `/internal` mirror and admin UI live controls, `video_layout_changed`; the self-benchmark and `video_budget_left` in cluster membership, admission (503) and placement by budget (§9.3) | fcp-conference-server, fcp-cluster, fcp-call-manager, fcp-admin-ui |

Decisions taken with this plan, each open to reversal:

- **Three PRs, not one.** Reviewability over the letter of "one PR per
  phase".
- **`VideoRoom` lives in forge-conference**, not the server, so the
  WebRTC gateway (phase 4) and the raw-codec tests reuse it; 3b therefore
  starts with a forge-media release.
- **`fcp-core::VideoCodec` is aligned with forge-core's model** rather
  than replaced: fcp-core has no forge dependency by design, so it keeps
  its own enum (five codecs, SDP names, conversion in fcp-media) and
  loses the fixed payload types.
- **Video through an anchored B2BUA is rejected, not relayed, in 3a.**
  A video relay (forward coded RTP between two legs, no decode) is a
  later block; until then a room's video is reachable when the B2BUA
  passes media through, which is how conference calls are placed by
  default.
- **`video_enabled` defaults to off** per meeting until 3c's capacity
  accounting exists.
- **Trunk video stays in phase 5** (§10, §15), as designed.

## 16. Resolved questions

Raised with the first draft and decided 2026-09-04, each as recommended.

| Question | Decision |
|---|---|
| Default frame rate | **15 fps** by default; 30 fps per meeting. Half the CPU, and what phone-heavy rooms want |
| Self view | **Own tile included** by default so encoders are shared; `exclude_self` per meeting, charged in the capacity model |
| H.264 library | **OpenH264 built from source**, feature-gated. Reproducible; the C++ toolchain is only needed with the feature on |
| Recording container | **WebM** (VP8/VP9 + Opus) by default, every part permissively licensed; MP4 (H.264 + Opus) as an option |
| Screen share | Section handling designed in phase 3 so a second video section (RFC 4796 `a=content:slides`) is additive; **implemented after phase 6** |
| Hardware acceleration | Revised after review: **designed in now** (device-resident frames, pipeline per device, per-device budgets, placement by capability; §5.5, §9), **built as phase 7** on FFmpeg hardware contexts once the CPU path is the proven reference |
| Maximum resolution | **720p** by default; 1080p per meeting |
| Dev machine memory | Phase 0 measures **relative** constants here (12 cores, 2 GB, a Microsoft VM with no GPU device); absolute capacity is re-measured once on a production-shaped node before phase 3, GPU constants on a GPU box, and every node self-benchmarks at start |

## 17. References

RFC 3550 (RTP), RFC 4585 (RTCP feedback: PLI, NACK), RFC 5104 (FIR,
TMMBR), RFC 6184 (H.264 RTP), RFC 7798 (H.265 RTP), RFC 7741 (VP8 RTP),
RFC 9628 (VP9 RTP), AV1 RTP payload specification (AOM), REMB
(draft-alvestrand-rmcat-remb), transport-cc
(draft-holmer-rmcat-transport-wide-cc-extensions), RFC 3264 (offer/
answer), RFC 4796 (content attribute), RFC 8829 (JSEP), RFC 8834 (WebRTC
media transport).
