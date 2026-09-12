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
is used), two simultaneous screen shares or a separate content track in
recordings (screen share itself is phase 7, §15.6), H.265 encode on the
CPU path (no permissively licensed
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

*Shipped in 6a (2026-09-08, forge-media `v2026.09.08.3`).* As designed, decided per subscriber per
tick: an output that comes to exactly one tile with a picture, and a
source whose stream that subscriber can already decode (same codec, an
H.264 profile `forwardable_to` its own, a frame no larger than it asked
for), is forwarded through `StreamRewriter` under the subscriber's own
SSRC. Forwarded packets go out **on ingress**, not on the compose clock —
a tick of latency would undo the point — and an output every subscriber
has been forwarded away from is not rendered or encoded at all, which is
where the saving is. The retransmission cache holds forwarded packets as
it holds encoded ones; a PLI from the receiver goes to the source.

Two things the section did not say. First, a forwarded frame carries **no
name banner, no speaking border and no avatar**, because nothing
composited it — right for a presenter at full canvas, wrong for a grid —
so `Meeting.video_passthrough` is `auto` (forward only while the layout
is `spotlight`), `on` or `off`, and `auto` reads the layout as it changes.
Second, the **decode still runs**: the tile state and the freeze timeout
are read off decoded frames, so a source stops being decoded only when
nothing draws it, which is a further step. Per §9.4 the encode is two to
three times the decode, so the bulk of the saving is here.

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

*The ladder shipped in 6b (2026-09-08, forge-media `v2026.09.08.4`).* `forge_video::ladder` holds the
rungs at or below a room's own size and the rule: below 70% of the
current rung go down, above 120% of the next one up climb, and a link
that has collapsed goes straight to the rung it can carry rather than
walking down a step at a time. `VideoRoom` moves whoever has been asking
long enough — five seconds down, fifteen up, since a picture nobody can
decode is worse than a small one and climbing only to fall back is the
worst of both. The move keeps the subscriber's SSRC, payload type and
sequence, so it is a resolution change and a keyframe on the wire and
nothing at all in signaling; the rung it left is released when it was the
last one there. A subscriber on the passthrough fast path (§5.6) is
skipped, having no encoder to move between. transport-cc is still not
implemented: REMB is the only estimate.

*Shedding puts a ceiling on the ladder (6c, forge-media `v2026.09.10`).*
When the node's CPU cannot keep up, an output is held a rung down (§9.2),
and the ladder does not climb a subscriber through that ceiling however
good its link is: the limit is the node, not the link, and climbing would
undo the shed on the very next tick. Going *down* is never held back — a
link that cannot carry the ceiling gets the smaller picture the ladder
would have given it anyway.

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
- Frame budgets: the compositor must finish within one tick. When it
  overruns three ticks in a row the room first **sheds**: its largest
  output drops a rung of the bitrate ladder (§7), each later streak takes
  whichever output is then largest down another, and only once every
  output is at the bottom rung is the room's fps halved (never below a
  quarter of its target). A shed is a ceiling on that output's scope
  rather than a one-off move, so the ladder cannot climb straight back
  through it. Ten calm seconds undo one step, in the opposite order: the
  frame rate first, then the most degraded output a rung. Every downward
  step is a `conference.video_overload` whose `action` is `shed` or `fps`
  (§12). *(6c, forge-media `v2026.09.10`. Until then halving the fps was
  the only lever, so one expensive composite cost everybody their
  motion.)*

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

*As built (3c-2):* a **work unit** is one thousandth of one execution
unit's time per second — on the host, a millicore of one codec-pool
thread — so a stream costs `⌈k × W × H × fps / 10⁶⌉` units and a node with
a ten-thread pool advertises `10 000 × headroom` (`conference.
video_budget_headroom`, default 0.8): a 720p15 H.264 encode of hard
content is 61 units. `k` comes from the startup self-benchmark
(`forge_video::bench`, bounded by `conference.video_benchmark_seconds`,
default 10; 0 uses the §9.4 hard column) run before the first heartbeat,
so the very first `video_budget` a node advertises is measured. A room is
priced from what runs (`VideoRoomStatus`: a decode per source at what it
sends, an encode per flavor, a composite per output with the
`1 + 0.1 × tiles` term) every five seconds and at every heartbeat; a join
is priced conservatively (its decode plus an encode at the dearest of the
meeting's codecs, plus a composite when it starts the room's video or
asks for a canvas without itself), reserved on admission and released by
the next re-pricing. Hosting nodes keep a video room regardless of budget
(inter-node trunks are audio-only until phase 5), so the budget ranks the
choice of a *new* room's node, a known-empty node last and a node
advertising no budget in between; the region rule still comes first. The
budget, its constants and their source are in the conference server's
`/status` and on the admin cluster page.

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

*Shipped in 5a (2026-09-08).* forge-media gained `forge-webm`, a
dependency-free Matroska writer, and forge-conference gained
`VideoRoom::record` — a sink of **coded** composite frames that is not a
participant and so draws no tile — beside `ConferenceRoom::tap_mix`, the
mixed frame per clock tick. The conference server's `recording.rs` puts
the two together: it resamples the mix to 48 kHz, Opus-encodes 20 ms
frames, takes VP8 (or VP9) frames from the room, and writes one WebM with
both tracks on one timebase — time zero is whichever side produced a frame
first, so lip sync holds without a shared clock. It is behind the
`recording-webm` feature, since Opus builds libopus from source; without
it every recording is the WAV it always was, and the node says so at
startup rather than failing later.

`conference.recording_format` decides the container: `auto` (WebM when the
meeting has video and this node can encode for it, WAV otherwise), `wav`,
or `webm`, with `recording_video_codec`, `recording_resolution` (the
room's cap by default) and `recording_audio_bitrate` beside it.

A recording is a **row first**: `conference_recordings` (migration 053)
and `fcp_core::recording::RecordingStore` hold id, tenant, meeting, room,
node, path, format, trigger, consent, start, end, duration, size and the
participant count, written when the recording starts and completed when it
stops. The row is what makes a recording listable per tenant, downloadable
across nodes and deletable by policy; `POST …/recording/start|stop` have
`/internal` twins (a start is forwarded to the lowest-id host so a room is
recorded once, a stop is room-wide), the
size monitor watches the rows rather than parsing filenames, and a node
closes off rows it left running when it restarts.

Around it: the tenant's `RecordingConsentMode::Announce` prompt plays to
the room before recording begins and to each later joiner;
`recording_started` / `recording_stopped` carry `recording_id`, `format`,
`node_id` and, on stop, `duration_secs` and `size_bytes`; retention is the
holding node's, under the tenant's `recording_retention_days`
(`docs/COMPLIANCE.md`); and `/v1/recordings` lists, fetches, downloads
(streamed from whichever node holds the file) and deletes, with a
**Recordings** page and player in the admin UI.

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
these and every create/update API returns 400 with the message. *Added
in 7b:* `video_content_share` (`off`, `hosts`, `anyone`; default
`anyone`) and `video_content_layout` (`presentation`, `content_only`;
default `presentation`), migration 056.

**Conference API**: participant JSON gains `video: {codec, resolution,
fps, bitrate_kbps, keyframes_sent, nacks, plis, state}` — and, from 7b,
`video.content {…, channel}` of the same shape for a shared screen's
section and `video.presenting`; room JSON gains
`video: {layout, spotlight, fps, outputs: [...]}` — and, from 7b,
`video.content {participant_id, display_name, remote, held_for_secs,
codec, resolution, fps, live}`, `video.content_layout` and a `view` per
output; `POST …/video/content/stop` and `PUT …/video/content-layout`
with their `/internal` twins and admin proxies; new endpoints
`PUT /conferences/{room}/video/layout`, `POST …/video/pin/{pid}`,
`POST …/video/spotlight/{pid}`, `POST …/participants/{pid}/video/{on|off}`,
mirrored on the internal cross-node router. *Shipped in 3c-1*, plus
`DELETE …/video/pin` and `DELETE …/video/spotlight` to clear, and
`GET /conferences/{room}/video` (the room's video and every participant's,
located across nodes). The admin API proxies them as
`/api/v1/conferences/live/{room}/…` to the conference server in its
`[conference] server_url`; the admin UI's "Live Video" dialog per meeting
uses that.

**Events**: `conference.participant_joined` gains `has_video`;
`conference.video_state_changed` gains `stream` (`camera` \| `content`,
from 7b; absent from an older node and read as `camera`);
`conference.content_started {room_number, participant_id, node_id,
codec}` and `conference.content_stopped {…, reason}` (7b);
new `conference.video_layout_changed` (`{layout, pinned, spotlight}`, from
the API, a host, or a pinned participant leaving; one per hosting node),
`conference.video_state_changed` (participant video on/off/lost/disabled/
failed), `conference.video_overload` (the room could not keep up: `action`
`shed` when an output dropped a rung, with `output`, `from_resolution` and
`to_resolution`, or `fps` when there was nothing left to shed and the frame
rate halved; §9.2).

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
| 4 | **WebRTC** — done 2026-09-07 (§15.3): forge-webrtc video (forge-media `v2026.09.07.1`), the gateway's media path and direct room join (v0.25.107), join tokens, join page and surface (v0.25.108) | forge-media, FCP | done |
| 5 | **Recording and cascade** — done 2026-09-08 (§15.4): WebM muxing and a recording registry (v0.25.110–112), trunk video (v0.25.113/114), cross-node speaker state (v0.25.115); MP4 deferred | both | done |
| 6 | **Passthrough fast path, bitrate ladder, polish** — done 2026-09-10 (§15.5): single-source forwarding (v0.25.117/118), ladder switching (v0.25.119), overload shedding and parser fuzzing (v0.25.121; forge-media `v2026.09.10`) | both | done |
| 7 | **Screen share** — done 2026-09-11 (§15.6): the content source kind, the per-room floor and the `Composite` / `Cameras` / `Content` views (7a, forge-media `v2026.09.11`; v0.25.123), the RFC 4796 content section on SIP with its settings, API, events and UI (7b, v0.25.124), RFC 8285 header extensions and a second video mid on WebRTC with the join page's share button and the floor told by SIP `INFO` (7c, forge-media `v2026.09.11.1`; v0.25.125/126), content across trunks with a room-wide floor (7d, forge-media `v2026.09.11.2`; v0.25.127), BFCP floor control for SIP room systems in the new `forge-bfcp` (7e, forge-media `v2026.09.11.3`; v0.25.128) | both | done |
| 8 | **GPU backend** — done 2026-09-12 (§15.7): `forge-video-hw` over FFmpeg device contexts with NVDEC decoders, NVENC encoders and the L4's numbers (8a); the pipeline on the device — the filter-graph compositor, per-stage host fallback, parity within 39.5 dB (8b); the node — `[conference] video_device`, the `video-hwaccel` feature, the saturation budget, per-device budgets and placement by the first codec, the NVIDIA image (8c). NVIDIA built and measured, VAAPI compiles; the box is `deploy/gcp-gpu` | forge-media, FCP | done |
| 9 | **MP4 recording sink** — planned 2026-09-12 (§15.9): a fragmented-MP4 writer `forge-mp4` (H.264 / HEVC / AV1 with Opus), the recorder recording on the device a room runs on, `auto` picking MP4 where H.264 is encodable | forge-media, FCP | 1 week |

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

### 15.2 Phase 3c plan (2026-09-06)

3c closes phase 3: the runtime controls §8 and §12 promise, and the
capacity model of §9.3. The two halves touch disjoint code, so they are
delivered as two FCP PRs in order, each shippable alone, with one small
forge-media release in front of the second.

**What the code survey found.** `VideoRoom` already has every control
(`set_layout`, `pin`, `spotlight`, `set_participant_video_enabled`) and
emits `LayoutChanged`, which the server's event pump drops on the floor
at the phase-3c stub; a host disabling a participant's video already
surfaces as `video_state_changed` with state `disabled` through the
per-tick state machine, so no new event is needed for it. The control
endpoints have a settled pattern (`local_*` returning 404 for "not on
this node", `room_wide` broadcast or `wherever_it_is` locate through the
`/internal` mirror under the cluster secret). The admin UI has no live
conference view: its only live client is dead code that calls the
conference server on port 8090 directly with no bearer token. Cluster
membership carries `region` as the precedent for an optional field
(`serde(default)`, skipped when absent). `choose_conference_node` knows
only participant counts and, on conference nodes, `active_calls` is
never updated, so its load tiebreak is inert today. No self-benchmark
exists: the only cost-measuring code is a `#[cfg(test)]` bench in
forge-video-codecs with private synthetic sources.

| PR | Deliverable | Where |
|---|---|---|
| 3c-1 | **Controls** — implemented 2026-09-06 (FCP v0.25.102). Conference server: `PUT /conferences/{room}/video/layout` (`{"layout": …}`, room-wide), `POST …/video/pin/{pid}` and `DELETE …/video/pin`, `POST …/video/spotlight/{pid}` and `DELETE …/video/spotlight`, `POST …/participants/{pid}/video/{on|off}` (single-target), `GET …/video` (the room's video and every participant's, located across nodes), each mirrored on `/internal`; the controls live in `video_bridge` as functions on the room (`set_layout`, `pin`, `spotlight`, `set_participant_video`) with a `VideoControlError` the HTTP layer maps to 404 (not here, ask the peers), 409 (room here, no video yet) or 400 (unknown layout). The event pump turns `LayoutChanged` into `conference.video_layout_changed {room_number, layout, pinned, spotlight}`. Admin API: `ApiState::conference_server_url` + token (`[conference]` in fcp-admin's config, the events-server precedent), routes under `/api/v1/conferences/live/{room}/…` resolving the room to a meeting in the caller's tenant and passing the upstream answer through, dial-out included; `/api/v1/me/conferences/{id}/dial-out` for owners. Admin UI: a "Live Video" dialog per meeting (status polled every 3 s, layout select, pin / spotlight / video on-off per participant) and the unauthenticated direct-to-8090 client removed. *As built:* clearing the spotlight keeps the spotlight layout, as forge-conference does; a host's video-off surfaces as `video_state_changed` `disabled` from the room's own state machine, so no second event; the loopback test spotlights Alice and checks she fills Bob's canvas while the event reaches the bus | fcp-conference-server, fcp-events, fcp-api-core, fcp-admin, fcp-admin-ui |
| 3c-2 | **Capacity** — implemented 2026-09-06 (FCP v0.25.103; forge-media `v2026.09.07`: forge-video 0.2.0). forge-video's public `bench` module (the synthetic sources, `measure_codec` / `measure_all` / `measure_compose` in ns/px, `CodecCost`). Conference server: `video_capacity` — `CostTable` (self-benchmark or the §9.4 hard column, H.265 priced as AV1), `units()`, `VideoCapacity` (per device, host only until phase 7) with `room_cost` from `VideoRoomStatus`, a conservative `join_cost`, `try_reserve` admission refusing with the participant-cap 503 after `plan_video`, and `recompute` every five seconds and at every heartbeat; the self-benchmark runs before membership starts so the first heartbeat carries a measured `video_budget`; `active_calls` is finally updated on conference nodes and `video_mixing` joins the capabilities; `/status` shows the budget, constants and their source. Config: `conference.video_benchmark_seconds` (10), `conference.video_budget_headroom` (0.8). Cluster: `ClusterMember.video_budget: Vec<VideoDeviceBudget {device, codecs, left, total}>` (omitted when empty, unknown to readers), `video_budget_left(codecs)`, `VideoBudgetHandle`, `VideoNeed`, `choose_conference_node` ranking by budget for video rooms as §9.3 says; `fcp_sip::ConferenceVideoNeed::of(&meeting)` threads the meeting's codecs through `ConferenceNodeSelector` from both routing sites. Admin: `video_budget` on `ClusterNodeInfo` and a "Video Budget" column on the cluster page. Docs: §9.3 as-built, DISTRIBUTED_CONFERENCING and MULTI_REGION placement | forge-media (release), fcp-conference-server, fcp-cluster, fcp-sip, fcp-call-manager, fcp-admin-ui |

Decisions proposed with this plan, each open to reversal:

- **Two PRs, not one.** Controls and capacity share no code; the second
  waits on a forge-media release the first does not need.
- **The admin UI reaches the conference server through the admin API**,
  not directly. One configured conference node is the entry point; it
  locates or broadcasts to the hosting node through `/internal`, so the
  proxy needs no cluster knowledge. The direct client goes.
- **A work unit is one thousandth of one device execution unit's time
  per second** — on the host, a millicore of one codec-pool thread. A
  node with a ten-thread pool advertises `total = 10 000 × headroom`
  (headroom 0.8 by default); a 720p15 H.264 encode of hard content costs
  `4.4 ns/px × 921 600 px × 15 fps ≈ 61` units. The same definition fits
  a GPU media engine in phase 7.
- **The self-benchmark runs at start, before the first heartbeat**, on
  the codec pool with no rooms yet, one codec at a time at 640×360 on the
  hard synthetic source, bounded by `conference.video_benchmark_seconds`
  (default 10; 0 skips it). Constants it did not reach fall back to the
  §9.4 table (hard column). Results are logged and shown in `/status`.
- **An over-budget join is refused with 503, as §9.3 says**, not admitted
  audio-only: budget-aware placement makes the refusal rare, and a loud
  failure is a capacity signal an audio-only fallback would hide. The
  alternative (admit audio-only, emit an event) is a one-line change if
  operators want it later.
- **`video_enabled` stays off by default.** Capacity accounting now
  exists, but a meeting's video still needs native codecs compiled into
  its conference nodes; opting in per meeting keeps that a deliberate act.
- **Layout events in cascaded rooms are per hosting node**: a room-wide
  layout change makes every node's `VideoRoom` emit once, so consumers of
  `video_layout_changed` for a room spanning nodes see one event per node
  with identical content. Participant migration and a single-emitter rule
  belong with the distributed-conferencing follow-ups.
- **Unpin and unspotlight are `DELETE`s**, additive to §12's list.

### 15.3 Phase 4 plan (2026-09-07)

Phase 4 puts browsers in a video room. The design (§4.1, §12) gives it
two parts — a video section in forge-webrtc and a direct "join room" path
in the gateway — and the surveys that produced this plan found the second
part has less under it than §4.1 assumed, so the phase is three PRs and
three to four weeks rather than two.

**What the surveys found.** forge-webrtc is a complete, self-contained
audio endpoint: its own ICE (host, server-reflexive and TURN relay, one
socket, BUNDLE and rtcp-mux assumed), its own DTLS-SRTP on OpenSSL, offer
and answer in both roles, trickle, renegotiation. It has *no* RTCP
transmit path at all — no SR, RR, NACK, PLI or REMB is ever sent, and
inbound RTCP is handed up as undecoded bytes — no RTP header extensions
(RFC 8285 is not parsed or written, so no transport-cc sequence, no `mid`
extension), no RTX, no FEC, no `a=msid`, `a=extmap`, `a=rid` or
`a=simulcast`, and exactly one `m=audio` (Opus, PCMU, PCMA). forge-sdp
already answers a video section (`answer_video` mirrors `mid` and
`rtcp-mux`, keeps the offer's protocol, allow-lists `nack`, `nack pli`,
`ccm fir`, `goog-remb`); forge-rtp already parses and builds every
feedback type but transport-cc; and `VideoRoom`'s contract is
transport-free (`push_rtp` returns the feedback to send, `subscribe` hands
back a channel of packetized frames, `handle_feedback` takes the
receiver's RTCP), so the gap is inside forge-webrtc. On the FCP side the
gateway is thinner than its documentation: it builds an audio-only offer
through forge-api and lets the browser answer, and its `POST /v1/calls`
sends a G.711 INVITE through fcp-sip — but nothing joins the peer
connection to the SIP leg, so no audio has ever flowed through it (the
Opus↔G.711 transcoding its module docs describe does not exist). Browsers
reach a room only as a SIP caller through the B2BUA; the conference server
admits participants by INVITE alone (dial-out and trunks are INVITEs too),
and a meeting's PINs are collected by DTMF. The conference server mixes
PCMU, PCMA and G.722, all three of which browsers ship, so audio can be
relayed without a transcoder. `fcp_media::construct_answer` would turn a
browser offer into an unusable answer (protocol downgraded, BUNDLE, ICE
and fingerprint dropped) — but with the gateway offering, it never sees
one. There is no join link, guest token or WebRTC type anywhere in
fcp-core, fcp-events or the admin surface, and the gateway has no deploy
configuration.

| PR | Deliverable | Where |
|---|---|---|
| 4a | **forge-webrtc video** — implemented 2026-09-07 (forge-media `v2026.09.07.1`: forge-webrtc 0.6.0, forge-rtp 0.5.0; FCP v0.25.106 bump). `PeerConfig::video: Option<VideoConfig>` (codecs in preference order with offered payload types, direction, `max_kbps`) puts a second section in offers — `a=mid:1`, `a=group:BUNDLE 0 1`, `a=rtcp-fb` `nack` / `nack pli` / `ccm fir` / `goog-remb` per payload type, H.264 `packetization-mode=1` with `profile-level-id` from the config, `a=ssrc … cname` and `a=msid` on both sections — and answers a remote video section with `choose_video_codec` and `answer_video`, rejecting it with port 0 when unconfigured; re-offers add or drop the section. The transport demultiplexes inbound RTP by payload type into `TransportEvent::Rtp` (audio) and `TransportEvent::VideoRtp`, parses inbound SRTCP into `TransportEvent::Rtcp(Vec<RtcpPacket>)`, keeps per-SSRC reception statistics, and gains a transmit path: `send_rtcp` (compound, SRTCP-protected), a one-second scheduler sending SR for each sending SSRC and RR for each received one with the SDES cname, and `VideoSender::send_packet` for a room subscription's pre-built packets with the SSRC rewritten to the advertised one and the sequence number kept so the room's NACK cache still matches. G.722 (static type 9) joins the audio codec list so a browser can meet the mixer without transcoding. Tests: the Chrome fixture answered with video accepted and with it rejected; a two-peer loopback moving raw-codec video and Opus both ways with NACK, PLI and REMB crossing the same socket; SR/RR arriving on cadence. *As built:* the video section is built by forge-webrtc's own section builder rather than forge-sdp's `answer_video`, so ICE, DTLS, `msid` and candidates land on it the way they do on the audio section; H.264 entries count only in packetization mode 1 and RTX, RED and FEC formats are skipped; `forge-rtp` needed more than statistics — its `SrtpContext` kept one rollover counter and replay window for every SSRC, which two streams on one transport corrupted, so both are per SSRC now — and its SDES and BYE serialisation was a word short, cutting every compound off after them (nothing had sent one before); a peer whose answer drops a feedback type gets it left out of `NegotiatedVideo`, and a rejected or unconfigured section pins nothing | forge-webrtc, forge-rtp |
| 4b | **Gateway media path and direct join** — implemented 2026-09-07 (FCP v0.25.107). fcp-webrtc-gateway gains the relay it was documented to have: for every peer connection it owns a SIP leg towards the conference server and moves audio RTP (same codec both sides, G.722 preferred then G.711, telephone-event forwarded), video RTP (coded packets relayed, SSRC preserved per direction) and RTCP feedback (NACK, PLI/FIR, REMB carried across; SR/RR regenerated per leg by forge-webrtc on one side and the SIP socket on the other) between them. `POST /v1/rooms/{room_number}/join` `{display_name, video, join_token}` creates the peer connection with an audio + video offer and, once DTLS completes, INVITEs the conference server directly — placed through `fcp_sip::ConferenceNodeSelector` as the call manager does, `ConferenceVideoNeed` included, with a static `[conference] server_uri` fallback — carrying an SDES-SRTP offer in the browser's negotiated codecs and the join token in an `X-FCP-Join` header; `GET/DELETE …/rooms/{id}` for status and leave. `POST /v1/calls` reuses the relay, so audio flows there too (G.711 only, as its offer says). Config: `[conference] server_uri`, `[webrtc] public_ip`, `turn_servers`, `[cluster]` placement. Events: none new — the conference server's `participant_joined` (`has_video`) already reports the browser. *As built:* the survey had understated the gateway's gaps — nothing read its SIP socket, so no INVITE response or far-end BYE ever reached it; it now runs a `TransportListener` with a small inbound handler (responses to the transaction layer, BYE ends the session, OPTIONS answered, anything else 405). The crate is a library plus binary so the relay is tested in process: a real forge-webrtc peer stands in for the browser and socket pairs for the conference server, plain and SRTP. fcp-media gained the offerer's half of SDP and SDES (`construct_offer`, `SrtpOffer`), which dial-out can reuse in phase 5. The old `MediaManager` session (a forge forwarding session nothing ever fed) is gone; the media port range is split between the relay (ICE sockets included, so one firewall rule covers everything) and forge-api's own sessions. Placement calls `fcp_cluster::choose_conference_node` directly rather than through the call manager's selector, with `[conference] tenant_id` for the registry lookup; the conference server resolves the tenant from the To host (an IP literal means the default tenant) until 4c's token carries it. The gateway offers server-reflexive candidates inside the offer by waiting `webrtc.gather_timeout_ms` for STUN before re-offering, since browsers never fetch its candidates later; a static public IP for ICE is a forge-webrtc follow-up. forge-webrtc has no consent-freshness check, so the relay ends a session after `webrtc.inactivity_timeout_secs` without a packet or report from the browser. A conference node that rejects video (no native codec, no budget) leaves the browser's video section negotiated but idle; the session reports `video: null`. `POST /v1/calls` takes an optional `connection_id` for the old two-step flow and otherwise creates its own connection; a connection whose browser picked Opus is refused with a clear error rather than transcoded | fcp-webrtc-gateway, fcp-media, fcp-cluster |
| 4c | **Join tokens, browser page, surface** — implemented 2026-09-07 (FCP v0.25.108). Join tokens: `POST /v1/conferences/{id}/join-tokens` `{display_name, role: guest|host, ttl_secs}` on the admin API (and `/me/conferences/{id}/join-tokens` for owners) mints an HMAC-SHA256 token over `{tenant, room_number, display_name, role, exp, nonce}` with `[conference] join_secret`; the conference server verifies `X-FCP-Join` with the same secret, resolves the tenant and room from it, admits the participant with the token's role and skips PIN collection — a wrong or expired token is a 403, an INVITE without one takes today's DTMF path unchanged. A join page (`crates/fcp-webrtc-gateway/browser/join.html`, served by the gateway under `/join` when `[http] serve_join_page`) takes a gateway URL, a join token, a display name and a camera choice, and drives the join, answer and trickle calls; forge-media's `tests/browser` page gains `video: true`. Admin UI: a "Join in browser" action per meeting that mints a token and opens the page. Docs: `docs/WEBRTC.md` (topology, ports, TURN, tokens), the gateway's deploy configuration and compose entry, STATUS, README. *As built:* the token type lives in fcp-core (`JoinToken`, `JoinRole`, `JOIN_TOKEN_HEADER`), minted with `hmac`/`sha2` and verified constant-time, payload JSON with a bare tenant UUID; the conference server keeps `join_secret` out of the configuration store (TOML only, preserved across the DB overlay) and refuses a token when no secret is set rather than falling through to the PIN path; the browser's own signalling does not use forge-api's operator-only routes — the gateway issues a per-session token with the offer and takes the answer and candidates on `/v1/rooms/{id}/answer` and `/ice-candidate` under it, with `[http] public_join` letting the join itself in without an operator token (the join token inside is what admits); the join page is `include_str!`-served at `/join` behind `[http] serve_join_page`, and the admin API's `[conference] gateway_url` turns a minted token into the link the admin UI's "Join in browser" dialog copies or opens (on the admin meetings page and the owner's page alike); the token is used with `autojoin` and stripped from the address bar on load. forge-media's manual `tests/browser` page stays audio-only: it drives forge-api's two-step flow, which the gateway's join page supersedes for browsers | fcp-core, fcp-api-core, fcp-admin, fcp-conference-server, fcp-webrtc-gateway, fcp-admin-ui, deploy |

Decisions proposed with this plan, each open to reversal:

- **The gateway relays; the conference server stays a SIP server.** §4.1
  said the gateway joins rooms directly, and this keeps that: the
  browser-facing attack surface (ICE, DTLS, TURN, public addresses) lives
  in one edge binary, and the conference server admits browsers through
  the INVITE path it already has. The alternative — embedding forge-webrtc
  in the conference server for one hop less — would put ICE and public
  reachability on every mixing node and duplicate the join logic across
  two front doors; the passthrough fast path of phase 6 can revisit it
  with numbers.
- **Audio is relayed, not transcoded: G.722 first, then G.711.** The mixer
  takes PCMU, PCMA and G.722 and browsers offer all three, so no Opus
  transcoder is built now; Opus in the mixer is its own item.
- **The gateway offers, the browser answers**, as forge-api does today. A
  browser-offer path is not needed for a join page we write ourselves;
  JSEP glue for third-party clients can come later.
- **Feedback set for 4a: `nack`, `nack pli`, `ccm fir`, `goog-remb`**.
  transport-cc, RTX, header extensions, FEC and simulcast ingress wait for
  phase 6: without them browsers degrade to same-SSRC retransmission and
  REMB-driven rate control, which the room already speaks, and each of the
  deferred pieces needs RFC 8285 first.
- **forge-webrtc grows an RTCP transmit path**, SR and RR on a one-second
  cadence included. Browsers' loss-based rate control reads our RR and
  lipsync reads our SR, so this is not optional once video is in the
  section; the audio-only path never needed it.
- **`TransportEvent::Rtcp` changes from bytes to parsed packets** (a
  breaking change, hence forge-webrtc 0.6.0). forge-api, its only
  consumer, ignores the event today.
- **Join tokens replace DTMF PINs for browsers and are verified by the
  conference server**, not the gateway: the gateway never learns a PIN,
  the secret is shared between the admin API (mints) and the conference
  server (verifies) in the `[conference]` section each already has, and
  the gateway passes the token through in a header — the trunk headers are
  the precedent. SIP callers keep the DTMF path.
- **The gateway INVITEs the conference server directly**, bypassing the
  B2BUA, with the same placement the call manager uses (`fcp-cluster`
  selector with the meeting's video need, region preference); a static
  `server_uri` serves single-node deployments.
- **`POST /v1/calls` gets its audio path fixed in 4b** as part of the same
  relay; no separate bug-fix release.
- **Three PRs.** 4a is a forge-media release the other two build on; 4b and
  4c touch disjoint code (4c needs the header the conference server checks,
  which 4b only sends), so 4b ships browser joins gated on a PIN-less
  meeting and 4c adds the tokens.

### 15.4 Phase 5 plan (2026-09-07)

Phase 5 makes a video room durable and distributed: a recording of what
the room shows and hears, video on the trunks between the nodes hosting
a room, and one active speaker the nodes agree on. The surveys found
less under §10 and §11 than they assumed, so the phase is three blocks
(each a forge-media release and an FCP PR) and three to four weeks
rather than two to three.

**What the surveys found.** Nothing in either repository writes a video
container: forge-recorder is WAV (hound) and Ogg/Opus with a hand-written
RFC 7845 header, and no WebM, Matroska, EBML or MP4 code or dependency
exists. A room's recording is fed by the mixer's frame clock —
`ConferenceRoom::mix` writes each mixed frame to an `AudioRecorder` — at
the mixer's rate (8 kHz), and the file is a `.wav` on one node's disk with
nothing else knowing of it: no table, no store trait, no tenant-scoped
listing (the conference server's `GET /recordings` reads its directory and
filters on `.wav`), no admin API route, no admin page, and the compliance
sweep prunes the B2BUA's call-recordings directory, not the conference
server's. Recording bypasses the tenant's consent mode, which only the
call-recording API enforces; the size monitor recovers the room from the
filename. `VideoRoom` lets nothing out but packetized RTP per subscriber,
and a subscriber must be a participant — registering a recorder as one
would draw its avatar tile, since the tile loop, unlike the level loop,
does not skip `__` ids. Two Opus encoders exist behind off-by-default
features; the resampler exists; VP8/VP9 encode needs `video-vpx`. Trunks
are complete for audio — virtual participants, mutual-TLS identity,
reconcile, the "a trunk carries local callers only" anti-echo mix — and
audio-only: the INVITE is a one-section string, the answer uses the
audio-only builder, `add_virtual_participant` never registers with the
video room, and `TrunkLeg` and the trunk's session carry no video.
`OutputKey.exclude` already builds a composite without one tile. The
active speaker is computed per node with 800 ms / 2 s hysteresis and the
conference server drops the event on the floor; the trunk's energy is the
peer's whole mix, and the media plane has no side channel (RTCP APP parses
to an error, header extensions are unused, INFO is DTMF-only). The
cross-node channel that exists is the `/internal` HTTP control plane under
`cluster.secret`, with `broadcast` and `locate`; `GET …/video` is one
node's view.

| Block | Deliverable | Where |
|---|---|---|
| 5a | **Recording.** forge-media (release): `forge-webm`, a dependency-free Matroska writer — EBML header, live Segment, Info (1 ms timecode scale), Tracks (`V_VP8`/`V_VP9`, `A_OPUS` with OpusHead), Clusters of SimpleBlocks with keyframe flags, Cues written on finalise with the Segment size patched — with a reader-side test that walks it back; `VideoRoom::record(RecordRequest {codec, resolution, fps, max_kbps}) -> RecordingSink` (coded frames with room pts, from a dedicated output whose scope is everyone and which draws no tile; `stop_recording`; charged like a subscriber); `ConferenceRoom::tap_mix() -> mpsc::Receiver<MixedFrame>` (the mixed frame per clock tick, with its sample offset) beside the existing recorder path; `VideoRoom::started_at()` so pts converts to wall time. FCP: `recording.rs` in the conference server — the recorder task resamples the mix to 48 kHz, Opus-encodes 20 ms frames, takes VP8 (VP9 optional) coded frames, and writes one WebM with both tracks on one timebase; `conference.recording_format` (`auto`: WebM when the meeting has video and the node encodes VP8/VP9, else WAV as today; `wav`; `webm`), `recording_video_codec`, `recording_resolution` (room cap by default); a `recordings` table and `RecordingStore` (id, tenant, meeting, room, node, path, format, started, ended, size, duration, trigger, consent) written at start and stop; `POST …/recording/start|stop` gain `/internal` twins, start routed `wherever_it_is` and stop `room_wide`, a `recordings` map replacing the size monitor's filename parsing; consent — the tenant's `RecordingConsentMode::Announce` plays the prompt to the room before recording starts and to each later joiner; events carry `recording_id`, `format`, `node_id` and, on stop, `duration_secs` and `size_bytes`; retention — the node's cleanup deletes file and row by the tenant policy's `recording_retention_days` (falling back to `conference.recording_retention_days`) and the compliance report counts them; admin API `/v1/recordings` (list, get, delete, download proxied to the holding node) and a `RecordingsPage` with a player; `docs/COMPLIANCE.md` and `DISTRIBUTED_CONFERENCING.md` updated | forge-recorder→`forge-webm`, forge-conference, fcp-conference-server, fcp-core, fcp-storage-postgres, fcp-events, fcp-api-core, fcp-admin-ui |
| 5b | **Trunk video.** forge-media (release): a remote tile — `VideoRoom::add_remote(id, codec)` registers a source that is a tile without being a participant, drawn as one tile at the composite's aspect, and `OutputScope::LocalOnly` for a subscription that composes local participants only (the audio anti-echo rule's analogue: a composite sent to a peer must not carry any peer's tiles, or a three-node room shows a tile twice); `add_virtual_participant` gains the video registration. FCP: `dial_trunk` builds its offer with `construct_offer` and a `VideoOfferSection` when the meeting has video and the node has a codec the meeting allows, SRTP per section under `cluster.trunk_srtp` (a peer that drops the crypto on either section is hung up, as today for audio); `handle_trunk_invite` answers with `build_answer` and a `VideoAnswer` from the same meeting lookup; `TrunkLeg.video`, a trunk `VideoLeg` attached as remote source plus local-only subscription at the room's cap and fps, `ParticipantSession.video` set; capacity counts the trunk's encode and decode; the trunk's video is negotiated when the trunk is dialled (a meeting enabling video later gets it when its trunks are next re-established, which reconcile does on the hour or when a node drains); liveness stays on audio; `DISTRIBUTED_CONFERENCING.md` §trunks updated | forge-conference, fcp-conference-server, fcp-media (reuse) |
| 5c | **Cross-node speaker state.** forge-media (release): `VideoRoom::set_remote_speaker(remote_id, Option<RemoteSpeaker {participant_id, display_name, energy, speaking}>)` so a remote tile's level is the peer's claimed speaker rather than its mix, and `VideoRoomEvent::ActiveSpeaker` carries `via: Option<String>` (the remote id) with the speaker's own id. FCP: the node whose local participant becomes the room's speaker broadcasts `POST /internal/conferences/{room}/speaker {node, participant_id, display_name, energy, at}` through `ClusterControl`; every node feeds it to its video room, so all run the same rule on the same inputs and converge within one take interval (ties by node id); `conference.active_speaker_changed {room, participant_id, node_id}` emitted once, by the speaker's node; `GET …/video` becomes a `room_wide` read that merges the nodes' views (union of participants, the agreed speaker and layout) for the admin proxy and the Live Video dialog | forge-conference, fcp-conference-server, fcp-events, fcp-api-core, fcp-admin-ui |

Decisions proposed with this plan, each open to reversal:

- **WebM written by hand, MP4 deferred.** Matroska is a few hundred lines
  of EBML and this codebase already writes Ogg/Opus by hand; a crate would
  add a dependency for less. MP4 (H.264 + Opus-in-MP4) is not scheduled:
  every browser and VLC play WebM, and the design lists MP4 as an option
  for players that need it — it becomes a block of its own when one does.
- **VP8 + Opus by default.** VP8 is the codec every player and browser
  decodes and the cheapest to encode; VP9 is a setting. Recording needs the
  node built with `video-vpx`; without it, a video meeting records audio
  only, as WAV, with a warning — never silently nothing.
- **WAV stays for audio-only meetings** (`recording_format = auto`), so
  the tooling that reads today's files keeps working; WebM audio-only is
  available by setting `webm`.
- **The recorder takes coded frames, not packets**, through a sink on the
  video room rather than a subscription: packetising and depacketising the
  same frame would be waste, and a sink that is not a participant draws no
  tile and needs no `__` exception.
- **The mix is tapped at the mixer's rate and resampled to 48 kHz for
  Opus.** Encoding 8 kHz Opus is legal but unusual in WebM; resampling is
  cheap and the resampler exists.
- **One recording per room, on the lowest-id hosting node** (the trunk
  dialling rule), routed there by `wherever_it_is`; until 5b lands, a
  multi-node room's recording is that node's composite and mix — audio is
  already complete through the trunks, video shows that node's callers.
- **A recording is a row first.** The table is what makes recordings
  tenant-scoped, listable, downloadable across nodes and retainable by
  policy; the file follows the row. The node writes the row (it has
  storage); the admin API reads it and proxies downloads to the holding
  node's own `GET /recordings/{filename}`.
- **Consent is announced, not asked.** The room hears the tenant's prompt
  before recording starts and each later joiner hears it once; refusing
  to be recorded means leaving, as on the call-recording path.
- **A trunk composite is local participants only.** Exclude-self would
  suffice for two nodes, but with three a tile forwarded through a peer
  would appear twice; local-only matches the audio rule exactly.
- **The remote tile is one tile.** Per-participant tiles across nodes
  stay deferred, as §10 says.
- **Speaker state crosses the `/internal` control plane**, not the media
  plane: it changes at most every 800 ms, the channel exists with a secret
  and fan-out, and it needs no new RTCP or extension parsing. Agreement is
  by shared inputs and a deterministic tie-break, not by election.
- **`active_speaker_changed` has one emitter** — the speaker's node —
  unlike `video_layout_changed`, whose per-node emission stays (it is a
  room-wide write acknowledged per node); the merged `GET …/video` covers
  the read side.
- **Three blocks, in order.** 5a stands alone; 5b makes 5a's multi-node
  recordings complete; 5c depends on 5b's remote tile.

**5c as built (2026-09-08, forge-media `v2026.09.08.2`).** As planned, with two additions the plan did
not call for and one it did not settle:

- **The claim is refreshed on a timer, not only on change.** A claim sent
  once would go stale: the sending node's own speaker keeps the floor
  while quiet (an empty spotlight is worse than a quiet one), so no
  further change fires, and its peers would hold "still speaking, loudly"
  for good. `cluster.speaker_interval_ms` (400 ms) re-sends the current
  level, and a claim not refreshed for five intervals is dropped, so a
  node that partitions does not hold the whole room's floor.
- **Ties are settled by node id, which needed a route in.** `Level` gained
  a `node` and the candidate list is sorted before the election: two nodes
  comparing the same candidates iterate their own maps in different orders
  and would otherwise disagree on an exact tie.
- **The claim stands under the remote tile's id**, not the speaker's. That
  keeps every layout, ordering and tile rule working unchanged, and the
  person is resolved on the way out — `active_speaker()` names them,
  `active_speaker_via()` the tile they are behind.

**5b as built (2026-09-08, forge-media `v2026.09.08.1`).** As planned, with
three notes:

- **`add_virtual_participant` did not gain the video registration.** A
  trunk's audio participant is made before the answer arrives, so the
  codec is not known yet; video is registered when it is agreed, by
  `attach_trunk_video`. One call that did both would have to guess.
- **Capacity needed no new code.** `room_cost` prices the room's own
  sources and outputs, so the remote tile's decode and the local-only
  composite's encode are already counted. Admission still does not price
  a trunk in advance: a trunk is infrastructure and is never refused for
  budget.
- **The trunk's tile is left out of the participant-facing events** —
  `video_state_changed` skips `__`-prefixed ids — as a trunk is left out
  of join and leave. `GET …/video` grew a `trunks` array instead, saying
  which peers carry video and in what codec.

**5a as built (2026-09-08).** As planned, with §11 carrying the detail.
Three things the plan did not say:

- **A dropped recording sink now really stops the recording.**
  `RecordingSink`'s documentation promised it; nothing implemented it, so
  a recorder that went away without calling `stop_record` left the room
  encoding a composite nobody read. forge-media's tap reports the closed
  channel and the compose tick reaps it (forge-media PR #145, released as
  `v2026.09.08`).
- **The recorder's stop is a `notify_one`, not `notify_waiters`.** The
  task is only parked between frames; a wakeup sent while it was busy was
  dropped, and the recording never stopped — a hang, and a file never
  finalised. Found by running the WebM test under load.
- **Retention is split, not shared.** The holding node deletes file and
  row together under the tenant's policy; the hourly compliance sweep
  takes only rows still there a week past the window, so the node that can
  delete the file is not raced to the row. `RecordingStore` gained
  `node_tenants` and `recording_at` for that, and `RetentionReport` a
  `conference_recordings_deleted` count (migration 054).

### 15.5 Phase 6 plan (2026-09-08)

Phase 6 is the one that makes the mixer cheaper rather than more
capable: the **passthrough fast path** promised in §2 and §5.6, the
**bitrate ladder** promised in §7, and the hardening the first five
phases deferred. Three blocks, each a forge-media release and an FCP PR,
and about two weeks.

**What the surveys found.** Two of the three pieces are already built and
have never been called. `forge_rtp::StreamRewriter` — one continuous
SSRC, sequence and timestamp across a source change, switching only at a
keyframe — is complete, tested, exported, and referenced nowhere outside
its own module. `H264Fmtp::forwardable_to` is likewise complete and used
only by its own tests. Phase 1 laid both down for this phase.

The ladder is the opposite: the *table* in §7 exists as `default_kbps`,
but nothing switches rungs. A subscriber's flavor is chosen once, in
`VideoRoom::subscribe`, from the request and the room cap, and never
revisited; REMB only moves the shared encoder's target, and
`FlavorEncoder::set_target_kbps` takes the **minimum** across that
flavor's subscribers. So one caller on a poor link drags every other
subscriber of the same flavor down to their bitrate — the exact case §7
says should move that caller to a lower rung instead.

Overload handling is one lever: `VideoClock` halves the room's frame rate
after three consecutive overruns and restores a step after a quiet
stretch (`video_overload`). There is no shedding by output or resolution,
so a room that is overrunning because one 1080p composite is expensive
pays for it in everybody's frame rate.

forge-media has **no fuzzing at all** — no `fuzz` directory, no
`cargo-fuzz`, nothing in CI — while the video path now parses attacker-
reachable bytes in five depacketizers, the frame assembler, and
`forge-webm`'s reader. siphon-rs has had seven fuzz targets since before
this work started.

| Block | Deliverable | Where |
|---|---|---|
| 6a | **Passthrough fast path.** forge-media (release): a subscription may be served by forwarding rather than by an encoder. `VideoRoom` decides per subscriber per tick whether its output resolves to a *single* video source (spotlight on a participant with video; active-speaker or grid where exactly one tile has a frame) and whether that source is forwardable to it — same codec, `forwardable_to` profile, source resolution within the subscriber's cap — and if so feeds the source's coded frames through a `StreamRewriter` under the subscriber's own SSRC instead of the flavor's encoder. Switching sources, or switching between forwarding and composing, happens at a keyframe and asks for one with PLI; the subscriber's retransmission cache holds forwarded packets exactly as it holds encoded ones, and a PLI from the subscriber is forwarded to the source rather than re-keying an encoder that is not running. `VideoRoomSettings::passthrough` turns it off per room; `VideoSubscriberInfo::forwarding` reports what a subscriber is getting. FCP: `Meeting.video_passthrough` (with migration, API, admin UI), the room setting wired from it, `video_capacity` prices a forwarded subscriber at neither encode nor decode so the budget stops over-reserving, and `GET …/video` shows which subscribers are forwarded | forge-conference, forge-rtp (reuse), fcp-core, fcp-storage-postgres, fcp-api-core, fcp-admin-ui, fcp-conference-server |
| 6b | **Bitrate ladder.** forge-media (release): rungs (§7: 1080p 2.5 Mb/s, 720p 1.2 Mb/s, 360p 500 kb/s, 180p 200 kb/s, capped by the room) as a `Ladder` in forge-video, and a `VideoRoom` that *moves* a subscriber between rungs instead of dragging its flavor's encoder down — down after a sustained stretch below its rung, up after a longer stretch comfortably above the next, both with hysteresis so a flapping link does not flap the picture. A move is internal: same SSRC, same payload type, a keyframe from the new flavor, nothing on the wire but a resolution change, so no re-INVITE and no SIP visibility. `FlavorEncoder::set_target_kbps` keeps taking the minimum, which after the move is a minimum over subscribers that belong together. Metrics per rung and per move. FCP: the rung and its reason in participant JSON and the Live Video dialog; `conference.video_ladder_changed` is **not** added (see the decisions) | forge-video, forge-conference, fcp-conference-server, fcp-admin-ui |
| 6c | **Overload shedding and fuzzing.** forge-media (release): shedding in a stated order before the room's frame rate is touched — the largest output drops a rung, then the next, then the clock halves as it does today — so a room overrunning on one expensive composite degrades that composite rather than everyone's motion; `VideoRoomEvent::Shed { output, from, to }` beside `FpsChanged`. A `fuzz` directory with `cargo-fuzz` targets for the five depacketizers, the frame assembler, the RTCP parser and `forge_webm::read_summary`, seeded from the corpora the round-trip tests already generate, and a CI job that runs each for 60 seconds on a PR — a smoke run, not a campaign, because the Actions budget is tight (2,000 minutes a month, 1,808 used in the first week of September). FCP: `video_overload` gains the shed detail; `docs/VIDEO_CONFERENCING.md` §7 and §9 updated with what the ladder and shedding actually do | forge-video, forge-conference, forge-rtp, forge-webm, fcp-conference-server, fcp-events |

Decisions proposed with this plan, each open to reversal:

- **Passthrough gives up the tile's name banner and speaking border, and
  that decides where it is on by default.** A forwarded frame is the
  source's own picture: no label, no border, no avatar for a muted
  participant, because nothing composites it. That is right for a
  presenter at full canvas and wrong for a two-person grid where the
  names are the only way to tell a stranger apart. So: **on by default
  for the `spotlight` layout only**, off elsewhere, with a per-meeting
  `video_passthrough` of `auto` (that rule), `on` (wherever a single
  source allows it, labels be damned) or `off`. The alternative — drawing
  the label into the forwarded stream — is not possible without decoding,
  which is the cost being avoided.
- **Forwarding is decided per subscriber, not per room.** Two subscribers
  watching the same single source may differ in codec or profile; one can
  be forwarded while the other is composed, and the composite is built
  anyway if anybody needs it.
- **A move between rungs is invisible to signaling.** Same SSRC, same
  payload type, a keyframe, a new resolution — which every decoder
  handles, and which SIP has no opinion about. The alternative, a
  re-INVITE per rung change, would put a signaling round trip on a
  congestion signal.
- **REMB is still the only bandwidth signal.** transport-cc, RTX as a
  separate stream, FEC and simulcast stay deferred: each is a phase of
  its own, and the ladder's value does not depend on a better estimator.
- **No `video_ladder_changed` event.** A rung change is an adaptation, not
  a fact about the meeting; it can happen every few seconds per
  subscriber on a bad link, and an event bus that carries it would be
  mostly that. The rung is in the participant JSON and in metrics, where
  a graph wants it.
- **Shedding degrades the expensive output before the room's motion.**
  Frame rate is what a viewer notices first; resolution on the largest
  composite is what costs most. Today's clock does the opposite, because
  it was the only lever there was.
- **Fuzzing is a 60-second-per-target smoke run on PRs**, not a nightly
  campaign, and the corpus is committed. It catches the panic a
  malformed packet causes now; a real campaign belongs on hardware that
  is not paid for by the minute.
- **Three blocks, in order, but 6b and 6c are independent.** 6a is the
  one that touches the egress path, so it goes first; 6b and 6c can be
  reviewed in either order after it.
- **Not in this phase:** screen share (§16 puts it after phase 6), the
  GPU backend (phase 7, still waiting on a machine with a GPU), Opus in
  the mixer, per-participant tiles across nodes, and MP4 recording.

**6a as built (2026-09-08).** As planned, with one deviation: the plan
said a forwarded subscriber would be priced at "neither encode nor
decode", and it is priced at neither encode nor composite — the decode
still runs, since a tile's state is read off its decoded frames, and the
capacity model charges what actually happens. The rest is as planned,
including the `auto` default and the per-meeting override. The room
setting is a `PassthroughMode` rather than a bool so `auto` can follow a
layout that changes at runtime.

**6b as built (2026-09-08).** As planned. Two things the plan left open,
settled here: the thresholds are 70% down and 120% up with five- and
fifteen-second windows (a `LadderPolicy` and two `Duration`s on the room,
so they can be changed without a release), and a link that has collapsed
skips straight to the rung it can carry rather than descending one rung
per window. As decided, there is no `video_ladder_changed` event: the
rung is in the participant JSON (`rung`, `rung_kbps`, beside the
`remb_kbps` that explains it) and in
`forge_conference_video_ladder_moves_total`.

**6c as built (2026-09-10).** Two forge-media PRs rather than one, since
the halves share no code — #151 (fuzzing) and #152 (shedding), released
together as `v2026.09.10` (forge-video 0.4.0, forge-conference 0.11.0) —
and FCP v0.25.121. Shedding is as planned, with "then the next" read as
*the next largest*: `VideoClock::done_with` asks a `LoadShedder` before it
halves, and the room answers by taking the output held at the highest
rung down one, again on each overrun streak, until every output is at the
bottom; only then does the rate halve. A shed is a ceiling on the
output's `OutputScope` rather than a one-off move, since the ladder would
otherwise climb straight back on the next tick, and it is keyed by scope
because a rung change changes the output's key. Recovery is ten calm
seconds a step in reverse: the frame rate first, then the most degraded
output a rung. `VideoRoomEvent::Shed { output, from, to }` reports both
directions (a restore has `to` above `from`); FCP publishes the downward
steps as `conference.video_overload` with `action: "shed"` and logs the
upward ones, as it always has a restored frame rate. The halving is
`action: "fps"`, and a relayed event from an older node, which carries no
`action`, reads as that.

Two deviations on fuzzing. The corpora are *derived*, not committed:
`fuzz_seeds` tests in forge-rtp and forge-webm write them from the same
packetizer and WebM-writer round-trips the unit tests assert, so a format
change moves the seeds with it and a seed that stops round-tripping fails
a test. And the run is scheduled rather than a smoke run on each PR:
#151 made it weekly at ten minutes a target, on the grounds that a
fuzzer's value is in how long it runs, and forge-media PR #153 makes it
nightly — forge-media is a public repository, whose standard runners cost
nothing, so the Actions budget that argued for a short run, FCP's, never
applied to it. The eight targets were first compiled and run on
2026-09-10, before the release was tagged: 30 seconds each, 2.5–10
million inputs a target, no findings.

### 15.6 Phase 7 plan — screen share (2026-09-10)

Screen share is the phase §16 put after phase 6: a participant sends a
**second video stream**, the content of a screen or a window, and the
room shows it to everyone, records it, and carries it across nodes. On
SIP it is a second `m=video` section marked `a=content:slides` (RFC
4796); on WebRTC it is a second video track. The mixer treats it as a
new *kind* of source, not as another camera: it is drawn large without a
name banner, it is never a "tile" the speaker election or a pin can
claim, and there is one of it per room at a time. The GPU backend, which
§15 called phase 7 until now, becomes phase 8; it still waits on a
machine with a GPU, and screen share does not.

Five blocks, each a forge-media release and an FCP PR where both are
touched, and about five weeks. Block 7c is the largest single piece of
new work; block 7e is the one that decides whether SIP room systems can
share at all.

**What the surveys found.** Every video path, in both repositories,
holds "the participant's video" as a single thing. In forge-conference
the participant id is the source key, the tile id, the subscriber key
and the speaker id, in three `DashMap<String, _>`s: `add_source` called
twice for one id silently replaces the first source, `subscribe`
unsubscribes before it inserts, `OutputKey` names a scope and a
resolution and nothing else, and `RecordRequest` has no way to ask for
a different output. `TileSource` has no field that could mark a tile as
content, and `Layout` has no arrangement with a content region. Every
encoder is camera-tuned with no knob for screen content: OpenH264 is
built with `CameraVideoRealTime` hardcoded, libvpx never sets
`VP9E_SET_TUNE_CONTENT`, SVT-AV1 never sets `screen_content_mode`, and
`EncoderSettings` has no field to carry the hint. A decoded frame over
the 1080p source cap is *dropped*, and a source that stops changing is
*lost* after `freeze_timeout` (2 s) — both right for a camera and wrong
for a static slide.

In SDP, both repositories take the *first* active `m=video` and reject
the rest: `forge_sdp::active_video`, `fcp_media::parse_video_offer`,
`remote_video_endpoint`, `parse_video_sdes_crypto`, the `negotiated.is_none()`
guard in `answer_sections`, and forge-webrtc's `parse_remote`. A phone
offering a camera and a slides section today gets the slides section
answered with port 0, correctly, and never a way to send them.
`a=content` is read or written nowhere; sip-sdp round-trips it as a
generic attribute, so parsing it is a helper, not a parser change.
`rewrite_for_anchoring` has a latent bug the second section exposes: the
audio arm is guarded by `!audio_seen`, the video arm is not, so two video
sections would both be rewritten to one server port. The gateway relays
one video SSRC per direction, sorts inbound browser packets by **payload
type alone** (`transport.rs:898`), has no RFC 8285 header extensions and
no `a=extmap`, and can neither send nor receive a re-INVITE (its inbound
handler answers one with 405). The join page uses `getUserMedia` only,
`addTrack` never `addTransceiver`, reads no mid, and renders every
inbound track into one `<video>`. `handle_info` on the conference server
recognises DTMF `Signal=` lines and answers 200 to anything else.

Floor control does not exist in any form. The only BFCP in the tree is a
test fixture proving that `m=application … UDP/BFCP` is rejected with
port 0, and sip-sdp parses `UDP/BFCP` as `Protocol::Other`. This matters
more than it looks: Cisco, Poly, Yealink and Lifesize room systems share
content over SIP **only** when the answer carries a BFCP section and a
floor server grants their request; without one they disable the feature.
Browsers need no floor protocol, so the mixer, the SIP section handling
and the browser path can be built and proven before BFCP, which is why
it is the last block rather than the first.

What is already right: the compositor letterboxes every tile
(`scale::letterbox`), so 16:10 and 4:3 captures need no aspect work;
`reject_section` and `answer_video` are per-section and mirror `a=mid`;
hold and resume already rewrite every RTP section; the retransmission
cache, keyframe gate and `StreamRewriter` are per subscription and carry
over; and SRTP keeps state per SSRC, so a third socket pair on SIP and a
second SSRC on WebRTC are both within what phase 4 built.

**The model.** Three ideas carry the whole phase:

1. A source has a **kind**: `StreamKind::Camera` or `StreamKind::Content`.
   Sources, tiles and ingress are keyed by `(participant, kind)`. A
   participant has at most one of each. The camera keeps every existing
   behaviour; the content source has no freeze timeout (the last frame
   is held), is scaled to the cap rather than dropped when it is larger,
   and is priced as one decode.
2. A room has at most **one content source live at a time** — the
   *floor*. It is granted to the first participant whose content packets
   arrive (implicit floor, for browsers and BFCP-less endpoints) or to
   the participant whose BFCP `FloorRequest` is accepted (7e), refused
   while someone else holds it or the meeting's policy says no, and
   released when the holder stops (port 0 re-INVITE, track ended, BFCP
   `FloorRelease`), leaves, goes idle for `content_idle_secs`, or is
   stopped by a host. A refused sender's packets are dropped.
3. An output has a **view**, the third field of `OutputKey` beside scope
   and resolution: `View::Composite` (the meeting's layout, and the
   *presentation* arrangement while content is live), `View::Cameras`
   (the meeting's layout, never content) and `View::Content` (the
   content alone, letterboxed, no chrome). A subscriber with one video
   section, a recording, and a trunk's main section take `Composite`; an
   endpoint with a content section takes `Cameras` on its main section
   and `Content` on its content section, which is what room systems and
   the join page expect: people in one picture, the slides in another.
   `Content` is the view the passthrough fast path was made for — there
   is no banner to lose — so a subscriber that can decode the presenter's
   stream is forwarded it verbatim, text unblurred by a transcode.

| Block | Deliverable | Where |
|---|---|---|
| 7a | **The content source, the floor and the views.** forge-media (release): `StreamKind` and `(participant, kind)` keys for sources, tiles and ingress in `VideoRoom` (`add_source` / `push_rtp` / `handle_feedback` / `remove_source` take the kind; the camera-only forms stay as they are); `ContentFloor` with `request_content(participant) -> Granted \| Refused(reason)`, `release_content`, `content_holder()`, an idle timeout on RTP not frames, and `VideoRoomEvent::Content { participant, node, state: Started \| Stopped(reason) }`; `View` on `OutputKey`, `SubscribeRequest.view` and `RecordRequest.view` (default `Composite`); `Layout::Presentation` — the content in `ActiveSpeaker`'s main region and up to five cameras in its strip, ordered by speech recency, the content region drawn without band, border or avatar — and `content_only`, a `Composite` that shows the content alone; `TileSource.kind`; a box-filter downscale for content tiles (bilinear stays for cameras and for upscaling); `EncoderSettings.content: ContentHint { Camera, Screen }` wired to OpenH264 `ScreenContentRealTime`, `VP8E_SET_SCREEN_CONTENT_MODE` / `VP9E_SET_TUNE_CONTENT`, and SVT-AV1 `screen_content_mode`, set for every `Content` view flavor; passthrough for the `Content` view under every `PassthroughMode` but `Off`; the content view kept off the resolution ladder (REMB lowers its bitrate, then its frame rate) and shed last; `add_remote` with a kind, so a trunk can bring a content source; capacity: a content source is a decode, a presentation composite is priced at `tiles + 1`. forge-sdp: `content_of(&MediaDescription) -> Option<Content>` and `set_content` for RFC 4796 (`slides`, `speaker`, `sl`, `main`, `alt`), `label` / `set_label` for RFC 4574, and `video_sections(&sdp) -> VideoSections { camera, content }` beside `active_video`. Tests: golden images for `Presentation` and `content_only`, a content source that stops sending and is not lost, floor grant / refuse / release / idle, a `Content` subscriber forwarded and a `Cameras` subscriber unchanged by a share, a recording that follows `Composite` into presentation and back | forge-conference, forge-video, forge-video-codecs, forge-sdp |
| 7b | **The content section on SIP, and the surface.** fcp-media: `parse_video_offer` keeps its meaning and gains `parse_video_sections -> VideoSections { camera, content }`; `AnswerOptions.content: Option<VideoAnswer>` and `Answer.content`; the camera is the first active video section without `a=content:slides`, the content is the first with it, any further video section is rejected as today; `OfferOptions.content` and `construct_offer` emitting `a=content:slides` and `a=label`; `remote_content_endpoint`, `parse_content_sdes_crypto`; `AnchorPorts.content` and the `!video_seen` guard, so anchoring can never alias two sections onto one port (the B2BUA still disables video when it anchors; only the bug is fixed). Conference server: a third socket pair and SRTP context, `ParticipantSession.content: Option<VideoLeg>` (the leg type gains its kind), join / re-INVITE add and remove / UPDATE / BYE for the content leg, the content bridge task pushing to the room under `StreamKind::Content`, the floor per §15.6's rule with `[conference] content_idle_secs` (30) and `try_reserve` of one decode before a grant; `Meeting.video_content_share` (`off` \| `hosts` \| `anyone`, default `anyone`) and `Meeting.video_content_layout` (`presentation` \| `content_only`, default `presentation`), migration 056, `validate_video_settings`, create/update API and the admin UI's meeting forms; `GET …/video` gains room `content {participant_id, display_name, node_id, since, codec, resolution, layout}` and per-participant `video.content {state, codec, resolution, fps, …, channel}`; `POST …/video/content/stop` and `PUT …/video/content-layout`, their `/internal` twins and the admin API's `/conferences/live/…` proxies; the Live Video dialog shows the presenter, a Stop share button and the content layout; `conference.content_started {room_number, participant_id, node_id, codec}` and `conference.content_stopped {…, reason: ended \| idle \| host \| replaced \| left}`; `video_state_changed` gains `stream` (`camera` \| `content`, default `camera` for older nodes). Recording needs no code: the recorder's `Composite` view follows a share into presentation and back, one video track. Tests: `sdp_audio_video_content()` beside `sdp_audio_video()` in fcp-sip's fixtures; fcp-media answer, reject-the-third, anchoring and endpoint tests on two video sections; a conference-server loopback in which a raw-codec content source reaches a second participant's content socket forwarded and a third participant's main socket composited; the floor refused to a second sharer and to a guest under `hosts` | fcp-media, fcp-conference-server, fcp-core, fcp-events, fcp-api-core, fcp-admin-ui, fcp-sip (fixtures) |
| 7c | **The browser: header extensions, a second video mid, the share button** — done 2026-09-11 (forge-media `v2026.09.11.1`, FCP v0.25.125/126). forge-media (release): forge-rtp parses RFC 8285 one-byte and two-byte header extensions (`RtpHeader::extensions()`); forge-webrtc offers `a=extmap` for `urn:ietf:params:rtp-hdrext:sdes:mid`, echoes what the answer accepts, and sorts inbound RTP by **mid**, learning SSRC → section from the first packet that carries one (the answer's `a=ssrc` lines as the fallback, payload type as the last resort); `LocalParams.video` becomes a list, `VideoConfig.content: bool` adds a second video section (`a=mid:2`, `a=content:slides`, BUNDLE `0 1 2`), `TransportEvent::VideoRtp` carries the section, `video_sender(StreamKind)` and a second video SSRC; `PeerConfig::video` and `video_sender()` keep their meaning for callers with one section. Gateway: both video sections offered to the browser and to the conference server from the start, so **no renegotiation on either leg**; `SipLeg` / `BrowserLeg` carry a content leg beside the camera and the relay maps each section's SSRC pair and feedback separately; `JoinRequest.content` (default true when `video` is); session JSON `content {sending, floor, presenter}`. The conference server tells a gateway session about its floor with a SIP `INFO` (`application/vnd.fcp.content+json`, `{floor: granted \| refused \| released, reason, presenter}`), sent only to participants that joined with `X-FCP-Join`; the gateway's inbound handler answers it and updates the session. Join page: a Share screen / Stop sharing button that calls `getDisplayMedia`, puts the track on the content transceiver with `replaceTrack` and clears it on stop or when the track ends; a second `<video>` for the room's content, shown large with the composite small while it flows; "N is sharing" from the session's `content`. `docs/WEBRTC.md` updated. Tests: forge-webrtc loopback with two video sections both ways, demuxed by mid and by SSRC with the extension stripped; `tests/relay.rs` with a content leg; header-extension parsing in forge-rtp's fuzz seeds | forge-rtp, forge-webrtc, fcp-webrtc-gateway |
| 7d | **Content across nodes** — done 2026-09-11 (FCP v0.25.127; forge-conference 0.12.1). Trunk offers and answers carry a content section (`a=content:slides`, payload type 98, its own port pair and SRTP under `cluster.trunk_srtp`); `TrunkLeg.content`; the node holding the floor sends its presenter's content `LocalOnly`, the peer attaches it with `add_remote(…, Content)`, so every node's `Composite` and `Content` views show it; the floor is a room-wide fact agreed the way the speaker is (§15.4 5c): `POST /internal/conferences/{room}/content` carries `{node, participant_id, display_name, since}`, the earliest `since` wins with the node-then-id tie-break, a node whose local holder loses releases it (`replaced`) and tells the endpoint, claims expire when not refreshed; `GET …/video`'s merged read names the presenter and their node; `content_started` / `content_stopped` emitted once, from the holder's node; `room_trunk_json` reports `content`. Tests: two in-process rooms trunked, a share on one visible on the other's subscribers, a second share on the other node refused, the holder leaving freeing the floor room-wide | fcp-conference-server, fcp-cluster (claim shape) |
| 7e | **BFCP for SIP room systems** — done 2026-09-11 (forge-media `v2026.09.11.3`, FCP v0.25.128). forge-media (release): a new `forge-bfcp` crate — RFC 8855 message codec (common header, the attributes a floor server needs: BENEFICIARY-ID, FLOOR-ID, FLOOR-REQUEST-ID, PRIORITY, REQUEST-STATUS, ERROR-CODE, PARTICIPANT-PROVIDED-INFO, the grouped FLOOR-REQUEST-INFORMATION / REQUESTED-BY / FLOOR-REQUEST-STATUS / OVERALL-REQUEST-STATUS), `Hello` / `HelloAck`, `FloorRequest` / `FloorRequestStatus`, `FloorRelease`, `FloorQuery` / `FloorStatus`, `Error`, the RFC 8856 UDP framing (the response flag, retransmission with the client's transaction id) and TCP framing; a transport-agnostic `FloorServer` state machine for one floor per conference (request → Pending → Granted or Denied, Released, Revoked) with the tests RFC 8855 §13 implies; a fuzz target beside the others. FCP: the conference server answers `m=application … UDP/BFCP *` (and `TCP/BFCP`) with `a=floorctrl:s-only`, `a=confid`, `a=userid`, `a=bfcpver:2` and `a=floorid:1 mstrm:<label of the content section>`, on one `[conference] bfcp_bind` socket demuxed by peer address and user id; a `FloorRequest` is the explicit floor claim into the same `ContentFloor` as 7b (a granted request opens the content leg's ingress; an implicit sender on a BFCP endpoint is refused), a host stop is `Revoked`, `FloorStatus` is pushed to every BFCP endpoint on each change; `bfcp` in participant JSON. The B2BUA and SBC keep rejecting BFCP sections when they anchor, so BFCP works for calls that reach the conference server directly or relayed. Tests: a scripted BFCP client in the conference-server tests (Hello, request, grant, release, revoke, query) over UDP and TCP; an SDP answer test against a Cisco-shaped and a Poly-shaped offer | forge-bfcp, fcp-conference-server, fcp-media |

Decisions proposed with this plan, each open to reversal:

- **Content is a source kind, not a participant.** The alternative —
  a synthetic participant `"<id>#content"` — needs no new keys and is
  what the trunk did for its remote tile, but it would draw a labelled
  tile, enter the speaker election, be pin-able, count against
  `max_tiles`, and confuse `exclude_self`. The kind costs a key change in
  three maps and buys every rule above.
- **One content source per room at a time.** Two people sharing at once
  is a product question with no good answer on a one-picture endpoint,
  and every MCU that matters makes the same call. A second sharer is
  refused, not queued.
- **The floor is implicit unless the endpoint speaks BFCP.** Whoever's
  content packets arrive first holds it. Browsers and BFCP-less phones
  need nothing more; a BFCP endpoint gets the explicit protocol it
  requires in 7e, on the same floor. The alternative — driving the floor
  through RFC 3264 directions with a re-INVITE per change — would put a
  signaling round trip to every participant on each share, and the
  gateway cannot receive a re-INVITE at all.
- **The floor is granted before the picture is decoded.** A grant costs
  `try_reserve` of one decode; over budget, the floor is refused rather
  than the room slowed, which keeps 6c's shedding order honest.
- **Endpoints with a content channel get people and slides as two
  pictures; endpoints without one get the presentation composite.** That
  is the view rule, and it is why the composite needs a `Presentation`
  arrangement at all. `content_only` exists for the meeting that wants
  the slides full-frame on every phone regardless.
- **The content channel is capped at 1080p, not at the meeting's
  `video_max_resolution`.** Text at 360p is useless, and a forwarded
  stream costs no encode. A larger capture is scaled to 1080p at decode
  rather than dropped as a camera would be; 4K content waits for the GPU
  backend. Where the content is transcoded (the subscriber cannot decode
  the presenter's codec, or the meeting is `content_only` under a
  composite), its flavor is bounded by what the subscriber negotiated.
- **The content view is off the resolution ladder and is shed last.**
  Under REMB its bitrate falls, then its frame rate; resolution only
  moves when every other output is at the bottom rung. A frame rate of
  two on sharp slides is the right degradation; a blurred slide at
  fifteen is not.
- **Screen-content encoder modes are set per flavor, not per room.**
  `EncoderSettings.content` is `Screen` for every `Content` view flavor
  and `Camera` elsewhere; the presentation composite is camera-tuned,
  since most of its pixels are still faces. The recording is a
  `Composite` and follows the same rule.
- **Sharing policy is one setting.** `video_content_share` is `off`,
  `hosts` (the host PIN or a `host` join token) or `anyone`, default
  `anyone` when video is on. No presenter role is added to join tokens;
  `host` already is one.
- **No renegotiation on the browser path.** Both video sections are
  offered from the start on both legs and the browser swaps its content
  track in with `replaceTrack`. Offering a section that may never carry
  packets is cheap; adding re-INVITE to the gateway and a second
  `createOffer` round to a live page is not, and every deferred WebRTC
  item (simulcast, RTX) would want the same mid-based demux 7c builds.
- **The conference server tells gateway sessions about the floor with
  SIP INFO**, and only them. A phone with BFCP has its own channel for
  this; a phone without one has no way to show it anyway; a browser
  needs it to stop a capture the room refused. Gateway sessions are the
  ones that arrived with `X-FCP-Join`, which is the mark the server
  already reads.
- **Recording keeps one video track.** The presentation composite is
  the record of the meeting as its single-picture participants saw it.
  A separate content track (WebM can carry two) is deferred with MP4,
  and would also want per-track seeking in the admin player.
- **BFCP lives in forge-media as `forge-bfcp`.** It is conference
  control rather than SIP, it belongs beside forge-sdp and
  forge-conference, and forge-media's nightly fuzzing is free where
  FCP's minutes are not. The floor server is transport-agnostic; FCP
  owns the sockets and the SDP.
- **Five blocks, in order; 7d and 7e are independent of each other.**
  7a first because everything keys on it; 7b proves the model on SIP
  with a loopback before the browser path adds RFC 8285; 7c before 7d
  so a share can be demonstrated end to end early; 7e last because it
  needs 7b's floor and nothing needs it.
- **Not in this phase:** two simultaneous shares; a separate content
  track in recordings; content over the B2BUA when it anchors (video is
  disabled there today, unchanged); RTP-level `a=label`-only content
  from endpoints that omit `a=content` (rejected as a third section, as
  now); H.265 or 4K content; annotation, remote control, or any
  application-sharing protocol; the GPU backend (phase 8).

**7a as built (2026-09-11, forge-media `v2026.09.11`; FCP v0.25.123).**
As planned, with the mechanism settled where the plan left it open. The
floor is a `ContentFloor` behind the room: the first content packet takes
it (`push_rtp_kind`), `request_content` / `release_content` are the
explicit forms BFCP will use, `stop_content` is the host's, and a host's
stop also marks the presenter `content_enabled = false`, since a browser
that keeps sending would otherwise have the floor straight back — the
plan's rule that the floor is implicit needed that one exception. A
refused sender is told once per floor, not per packet, through
`VideoRoomEvent::Content { Refused(..) }`, which is what 7c's SIP INFO
will carry. Idle is judged on packets, not frames, and a content source
has no freeze timeout at all. The content channel's cap is a room setting
(`content_max_resolution`, 1080p) rather than a constant, and a source
over it is shrunk at decode with the same box filter the compositor uses
for content tiles. The content channel is not on the resolution ladder; it
moves in frame rate under the ladder's own thresholds and windows
(`Subscriber::fps_move`, floor 2 fps), and shedding takes every other
output to the bottom rung before it touches one. `Layout::Presentation`
reuses the active-speaker geometry with the strip one slot shorter, and
`content_only` is the spotlight geometry with a content tile. The
compositor draws a content tile with no border, band or avatar and paints
its bars in the background colour. A presenter's own content channel is
sent nothing at all, decided per subscriber at the tick rather than
through a scope, so content subscribers still share one encoder. FCP's
share: `VideoCapacity` implements `ContentGate` — a grant reserves one
decode at the content cap, at the dearest codec the node runs — installed
on the room when its video is enabled, and `room_cost` prices content
sources beside cameras; the events and JSON for the content section come
with 7b.

**7b as built (2026-09-11, FCP v0.25.124).** As planned. fcp-media sorts
an offer's video sections into a camera and a content section
(`parse_video_sections`; `parse_video_offer` keeps meaning the camera,
so a content-only device's slides are never taken for one), answers each
on its own port with its own key (`AnswerOptions::content`,
`Answer::content`, `NegotiatedVideo::label`), rejects a third video
section as before, offers a labelled `a=content:slides` section
(`OfferOptions::content`, `CONTENT_LABEL`), and anchors each section to
its own port — the `!video_seen` bug is fixed by classifying every
section before the line-by-line rewrite, so two camera sections with one
port disable the second rather than alias it. The conference server
gives a participant a third leg: `ParticipantSession::content`, a
`VideoLeg` with a `kind`, a third port pair and SRTP context at join,
`prepare_leg(kind)` on a re-INVITE that adds either stream and
`tear_down_leg(kind)` on one that removes it, `refresh_leg` for the
address and key of a kept one, and the bridge task pushing under
`StreamKind::Content`. The floor is the room's from 7a; FCP adds the
policy: `video_content_share` `off` leaves the section unanswered,
`hosts` answers a guest's section — a phone that offered one expects an
answer — but marks them `content_enabled = false` so the room refuses
their packets and tells them once, `anyone` answers all; the node's
`[conference] content_idle_secs` (30) feeds the room's idle timeout. A
caller whose offer carried a content section gets the cameras alone on
their main section and the slides on the other, decided by attaching the
content leg first so the camera's attach can see it; a caller with one
section gets the composite, and one who adds a content section by
re-INVITE mid-call keeps the composite on their main until they
renegotiate. The recording needed no code. Not done here: a
content-only device (slides, no camera) is answered but its content is
priced like a camera's join; the gateway and the join page are 7c.

**7c, the forge-media half, as built (2026-09-11, forge-media
`v2026.09.11.1`: forge-rtp 0.7.0, forge-webrtc 0.7.0; FCP repointed at
v0.25.125).** As planned, with the shape settled by the code rather than
the table above: the extension walk lives on `RtpExtension`
(`elements`, `element(id)`, `one_byte`, `two_byte`) and `to_bytes` now
sets the X bit to match, which it never did; there is no list of video
sections in `LocalParams` — `LocalVideo::content` and
`Negotiated::content` are a second `Option` beside the camera, so a
caller with one section reads exactly as before — and the content
section's packets surface as their own `TransportEvent::ContentRtp`
rather than a tag on `VideoRtp`, with `content_sender()` beside
`video_sender()` and a third SSRC. Sorting is by the `sdes:mid`
extension (offered as id 1 on every section, the answer mirroring the
offer's id) with the SSRC remembered from the first packet that carries
one, else the SSRC each side signalled, else the camera; `DemuxConfig`
and `Transport::set_demux` replace `set_payload_map`. A peer without
`content` rejects the section with a zero port. The gateway, the SIP
`INFO`, the join page and `docs/WEBRTC.md` are the FCP half, still to
come; until then the gateway drops a content packet it cannot yet
receive.

**7c, the FCP half, as built (2026-09-11, FCP v0.25.126).** As planned.
The gateway offers the screen on both legs from the start — the
browser's `VideoConfig::content` (on by default with `video`;
`JoinRequest.content: false` leaves it out) and a third `m=video` with
`a=content:slides` on the INVITE, on its own port pair and key — so
sharing never renegotiates; `open_video_section` answers each section
the same way, and a rejected screen section leaves the join without one.
The relay carries the screen on its own sockets and SSRC pair
(`SipLeg::content`, `BrowserLeg::content`, `content_to_sip` /
`content_to_browser`) and sends feedback about it to the screen's far
end, keeping a REMB — about the transport, not a section — on the
camera's. The floor is told by SIP `INFO`: the video bridge's event
pump hands every content event to a `FloorNotifier` as a `FloorNotice`
(`granted` / `released` / `refused`, the presenter's name, the reason),
the conference handler is the notifier — it sends
`application/vnd.fcp.content+json` with `mine` to each session in the
room that `joined_by_token`, a refusal only to the one refused, and a
late joiner a grant a second after joining — and writes the advanced
dialog back so the BYE's CSeq follows; the gateway's inbound handler
answers the INFO (200, 481 for an unknown call, 415 with `Accept` for
another type) and keeps a `FloorState` on the session, reported as
`content {codec, payload_type, srtp, sending, state, presenter, reason}`.
The join page answers the screen section `sendrecv` with no track,
shares with `getDisplayMedia` + `replaceTrack` (five frames a second,
`contentHint: detail`), stops on the button, the browser's stop bar or
a `refused` / `released` floor, shows the room's screen large while its
track flows (the receiver's `unmute` / `mute`) and says who is sharing.
Not done here: a screen from a browser with no camera (the section is
offered only beside a camera's); the conference server's INFO reaches
SIP phones nowhere, which is BFCP's job (7e).

**7d as built (2026-09-11, FCP v0.25.127; forge-conference 0.12.1).** As
planned, with one forge-media addition the plan did not list:
`VideoRoom::release_content_with(id, reason)`, the public way to take the
floor with the `Replaced` stop 7a reserved (or `Ended` for a peer's share
known to be over) without the host-stop's ban. The trunk's content
section is offered when the meeting allows sharing and only beside a
video section, answered under the same rule, on its own port pair and
key (`TrunkLeg::content`, `trunk_video_from_answer` and
`trunk_leg_from_answer` taking the stream kind); `attach_trunk_content`
adds the peer's screen as a remote content source under the trunk's tile
and subscribes the trunk to the `Content` view `LocalOnly`. The floor is
agreed on the speaker timer: `local_content_claim` (`{node,
participant_id, display_name, since_ms}`), `announce_content` to every
hosting peer, `apply_content_claim` on receipt — the earlier `since_ms`
wins, ties to the lower node then id (`claim_beats`), the local loser
released `replaced` — `expire_content_claims` after five intervals, a
claim with no participant withdrawing the peer's share. Browsers here
are told the presenter's name from the claim (`floor_changed`
substitutes it for the tile's), and told again if the packets beat the
claim. `GET …/video`'s merged read lifts the presenter's node's
`content` (with `node_id`) into the video block; `trunks[].content` says
a trunk carries the screen. Events already had one emitter, since a
trunk's tile is left out of them. Not done here: the presenter's node
does not learn a peer refused their screen (a budget-refused remote
source is simply not shown there); BFCP is 7e.

**7e as built (2026-09-11; forge-media `v2026.09.11.3`: forge-bfcp 0.1.0;
FCP v0.25.128).** As planned, with the shape settled by what a floor
server actually needs. forge-bfcp is three modules: `message` (the
common header, every attribute of RFC 8855 §5 with the grouped ones
nested, a parser that never panics and hands back what an `Error` reply
needs whenever the header was readable, a writer exact about lengths
and padding; text that is not UTF-8 is decoded lossily and cut to what
the length field can carry again, the fuzzer's first finding), `server`
(`FloorServer`, one floor per conference and transport, decisions left
to its owner: a `FloorRequest` is Pending and an `Event::Requested`
until `grant`, `deny` or later `revoke`; `FloorRelease` is Released or
Cancelled; `FloorQuery` subscribes to `FloorStatus`; every mistake draws
the RFC's error; notifications carry transaction id 0 over TCP and a
fresh one over UDP) and `transport` (`TcpFramer`; `UdpPeer` with the R
flag, a cached response replayed for a retransmitted request until T2,
one outstanding notification at a time retransmitted on T1 until
acknowledged or the peer is broken). Left out: chairs, several floors,
TLS/DTLS, fragmentation. fcp-media: `parse_bfcp_offer` (the first active
`m=application … UDP/BFCP` or `TCP/BFCP`, its `floorctrl`, `setup` and
`bfcpver`; `answerable()` when the offerer will be the client and, over
TCP, will connect) and `AnswerOptions::bfcp` / `Answer::bfcp`
(`a=floorctrl:s-only`, `a=confid`, `a=userid`, `a=floorid:1 mstrm:<the
content section's label>`, `a=bfcpver`, and `a=setup:passive` /
`a=connection:new` over TCP). Conference server: `[conference]
bfcp_bind` serves UDP and TCP on one address; the join reserves a
conference id per room and a user id per caller when the offer has an
answerable BFCP section beside a labelled content section (`bfcp.rs`,
`Floors::reserve`), binds them to the participant once admitted
(`bind`), forgets a reservation the call never bound, and repeats the
ids on a re-INVITE or UPDATE. A BFCP endpoint's floor is explicit: its
content leg attaches with implicit sending switched off whatever the
policy (`set_may_share`), a `FloorRequest` is put to the room's floor
through `request_content` and granted or denied at once (held, off,
budget, the host's stop, the policy), a `FloorRelease` or `Goodbye`
releases it, and the room taking the floor away — a host's stop, idle,
a peer node's older share, leaving — comes back as `Revoked` through the
7c notifier; a host's stop keeps the endpoint from asking again until
its content section is renegotiated. UDP datagrams are sorted by
conference and user id and replayed or retransmitted per peer; a TCP
connection is bound to the user its first message names. `bfcp` in the
participant's video JSON (`transport`, ids, `connected`, `floor`,
`host_stopped`). The B2BUA and SBC still reject BFCP sections when they
anchor, unchanged. Not done here: TLS/DTLS BFCP, chairs, a BFCP
endpoint on a trunked room hearing a peer's presenter by name.

**Phase 7 close-out (2026-09-11).** Five blocks, five forge-media
releases (`v2026.09.11` to `v2026.09.11.3`) and six FCP versions
(v0.25.123–128), all merged in one day against a plan of about five
weeks; the plan's shape held, with the deviations each block's note
records. What a screen share is now: one floor per room that a browser,
a BFCP-less phone or a trunked peer takes by sending and a BFCP room
system takes by asking; a content section on SIP and a second video mid
on WebRTC, each on its own port, key and SSRC; every output a view, the
composite becoming a presentation while a screen is shared; the floor
told to browsers by `INFO`, to room systems by BFCP, to the bus by
`content_started` / `content_stopped`, and agreed across nodes the way
the speaker is. Left open, gathered from the blocks' notes, none of them
blocking:

- A content-only device (slides, no camera) is answered but its content
  is priced like a camera's join (7b); a screen from a browser with no
  camera is not offered, since the section rides only beside a camera's
  (7c).
- A content section added by re-INVITE keeps the composite on the
  caller's main section until they renegotiate (7b).
- The presenter's node does not learn that a peer refused their screen
  for budget; the peer simply does not show it (7d). A BFCP endpoint on
  a trunked room hears a peer's presenter by tile, not by name (7e).
- BFCP over TLS/DTLS, floor chairs and BFCP-level fragmentation are not
  spoken (7e); the B2BUA and SBC still reject BFCP sections when they
  anchor, so BFCP works for calls that reach the conference server
  directly or relayed.
- Fuzzing covers the RTP extension walk and the BFCP parser; the SDP
  attribute parsing of the new sections relies on sip-sdp's.

Phase 8, the GPU backend, is planned as §15.7 (2026-09-11), with the box
`deploy/gcp-gpu` (`docs/GPU_BOX.md`), an L4 on GCP rented by the session.

### 15.7 Phase 8 plan — the GPU backend (2026-09-11)

The GPU backend is the phase §16 designed in from the first draft and
built last: the same pipeline — decode, frame slot, compose, scale,
encode — placed on a GPU end to end, so that on a GPU node nothing but
RTP crosses the bus, with the host path staying the reference and the
fallback. §9.1 made the case in density and power, not feasibility, and
§9.4 sharpened it: CPU mixing turned out three to seventeen times cheaper
than the design assumed, so a 12-core node already mixes on the order of
fifty to a hundred 720p H.264 outputs. A GPU node is for the rooms
beyond that, for AV1 and HEVC at scale, and for the power budget of a
rack, and placement is what sends such rooms to it.

The machine that was missing is now `deploy/gcp-gpu` (`docs/GPU_BOX.md`):
a Spot NVIDIA L4 on GCP, rented by the session, with driver 550, CUDA
12.4 and an FFmpeg 7.1 built with NVENC, NVDEC and the CUDA filters, all
proven on 2026-09-11. Three blocks, each a forge-media release or an FCP
PR, and three to four weeks; 8a begins with the measurements §9.4 left
open, and nothing in 8b or 8c is settled until they are in.

**What the surveys found.** The design's seams are in place and empty.
`MediaDevice::Gpu { backend, address }` parses `cuda:0` and prints it;
`VideoFrame::Device(DeviceFrame)` carries an opaque `Arc<dyn Any>` handle
and a `device` every stage checks; `VideoDecoder`, `VideoEncoder`,
`Scaler` and `Compositor` each name their device and refuse a frame from
another; `CodecRegistry` is keyed by `(codec, device)` and
`codecs_on(device)` lists what a device runs; `VideoBackend` carries one
`device` for the node's rooms, set to `Host` by the only constructor;
`bench::measure_codec` takes a device; the cluster's `VideoDeviceBudget`
names a device as `backend:address` and placement's `video_budget_left`
takes the best budget among devices that have the room's codecs. Nothing
constructs a `DeviceFrame`, nothing registers a non-host codec, no
config names a device, and the conference server's benchmark and budget
are built on `backend.device` being the host (`self_benchmark` prices
composition only when it is). The `HostCompositor` draws everything —
tiles, bands, labels from the bitmap font, avatars, speaking borders —
into an I420 canvas with scalar code, so a device compositor has to
reproduce all of it or share it. The recording sink (`VideoRoom::record`)
is a `Composite` subscriber whose flavor is VP8 or VP9, codecs no NVIDIA
encoder has. `EncoderSettings::content` (the `ContentHint` of 7a) is the
one encoder knob every backend maps.

On the box: `h264_nvenc`, `hevc_nvenc` and `av1_nvenc` each take 720p30
at about 170 fps from a synthetic source, eight at once at about 84 fps
each — the source generation on four vCPUs is the limit, not the
engines — and a CUVID decode feeds `scale_cuda` on the device. The
`nv-codec-headers` had to be pinned to the driver's NVENC SDK (12.2 for
driver 550), FFmpeg's `nvcc` flags to the L4's `sm_89`, and Google's
image had no CUDA toolkit: the setup script carries all three.

**Decisions, each with a recommendation.**

1. **The FFI.** `ffmpeg-sys-next`, raw bindings, pinned to the FFmpeg
   major the node has (7.1 on the box; the crate's 7.1.x line), with a
   thin safe layer of our own in a new crate rather than the `ffmpeg-next`
   wrapper, whose hardware-frame and filter-graph surface is partial and
   whose ownership model fights `Arc<dyn Any>` handles. `rsmpeg` is the
   alternative if `ffmpeg-sys-next` falls behind FFmpeg 8. **Recommended:
   `ffmpeg-sys-next`, a new crate `forge-video-hw`** that `forge-video-
   codecs` pulls in under its `hwaccel` feature, so the FFI and the
   FFmpeg headers are a build requirement only where the feature is on.
2. **Composition on the device.** Two ways: an FFmpeg filter graph per
   output — `scale_cuda` per tile into `overlay_cuda` chained onto a
   background, rebuilt when the layout or the tile count changes — or
   our own CUDA kernels (scale-blit, fill, overlay) loaded through the
   driver API. The graph is FFmpeg's, tested, and portable to VAAPI
   (`scale_vaapi`, `overlay_vaapi`) by swapping filter names; the kernels
   are faster to launch and free of graph rebuilds but NVIDIA-only and
   ours to test. **Recommended: the filter graph first**, with the chrome
   (bands, labels, avatars, borders) rendered by the host compositor's
   own code into small host planes, cached per tile and uploaded only
   when they change, and overlaid like any tile. Kernels only if the
   graph's per-tick cost shows up in 8a's numbers.
3. **Which vendors.** NVIDIA is the hardware we have. The design's
   VAAPI/oneVPL (Intel) and AMF (AMD) paths share the FFmpeg device and
   frame contexts, so the crate is written against `AVHWDeviceType` with
   the CUDA arm filled in and the others compiling but untested.
   **Recommended: NVIDIA built and measured in phase 8; VAAPI designed
   for, not claimed**, until an Intel box exists.
4. **The FFmpeg build that ships.** The box's FFmpeg is built with
   `--enable-nonfree` for `libnpp` and `--enable-cuda-nvcc`, which is fine
   for a dev box and not for a product. **Recommended: the product build
   is `--enable-cuda-llvm` (clang compiles the CUDA filters, no `nvcc`),
   no `libnpp`, LGPL 2.1 dynamic**, with the `nv-codec-headers` (MIT)
   the only NVIDIA-specific piece; the box script gains the same
   configure so what is measured is what ships.
5. **Codecs a GPU room cannot encode.** NVENC has no VP8 or VP9. A GPU
   room with a VP8 subscriber (a browser that offered only VP8, a WebM
   recording) needs that flavor's encoder on the host, fed by one
   canvas download per tick — the "copy the design allows but the
   scheduler avoids" of §5.5. **Recommended: allow it, price it** (the
   download is a compose-sized cost on the host device), and have
   placement prefer a GPU node only for meetings whose first codec the
   device encodes, so a VP8-first meeting stays on the host; the
   recording of a GPU room is VP8 on the host until an MP4/H.264 sink
   exists (§16 kept MP4 as an option; it is not this phase).
6. **Bitrate retargeting on NVENC.** FFmpeg's NVENC wrapper does not
   change bitrate on a live encoder. **Recommended: reopen the encoder
   on the next keyframe** when `set_bitrate` moves it by more than a
   rung's worth, which the ladder already coalesces; a GPU encoder
   opens in milliseconds.
7. **The budget of a GPU.** A GPU is several engines — NVENC, NVDEC, the
   CUDA cores — not one execution unit, and §9.3's single number per
   device would mispriced a node that is out of NVENC with decode to
   spare. **Recommended: keep one advertised budget per device, but
   measure it by saturation** — the self-benchmark runs encodes (and
   decodes, and composites) in parallel until throughput stops growing
   and prices each stage at its saturated share — and carry the three
   stages' constants in the cost table so a room is priced per engine
   even though the cluster sees one number. Splitting the budget per
   engine in membership is a follow-up if the one number misplaces.

**Blocks.**

| Block | What | Where |
|---|---|---|
| 8a | **Device contexts, codecs and the numbers.** forge-media (release): a new `forge-video-hw` crate over `ffmpeg-sys-next`: a `HwDevice` per `MediaDevice::Gpu` (an `AVHWDeviceContext` for `cuda`, with `vaapi` and `qsv` arms compiling), `HwFrames` pools, the `DeviceFrame` handle as an `AVFrame` with its `hw_frames_ctx` behind the `Arc<dyn Any>`, `upload` / `download` through `av_hwframe_transfer_data`; decoders through FFmpeg's hardware acceleration (`h264`, `hevc`, `vp8`, `vp9`, `av1` on NVDEC, output resident), encoders `h264_nvenc`, `hevc_nvenc`, `av1_nvenc` taking device frames, `ContentHint` mapped to the NVENC tuning (`ull`/`ll` plus `-tune` for screen content), keyframe on demand, bitrate retarget by reopen on the next keyframe (decision 6), `EncoderSettings` otherwise as the host codecs take them; `register_all` registering them on the device under `forge-video-codecs`' `hwaccel` feature; `bench` measuring them by saturation (decision 7) and a `GPU` column for §9.4 with the L4's numbers for every codec, decode and encode, single and saturated; `forge-video` gains `DeviceScaler` (`scale_cuda`) and the `Scaler` round-trip test per device. Tests, on the box: encode → decode round trips per codec with PSNR thresholds against the host codecs' output of the same frames, keyframe-on-demand, bitrate targeting within tolerance, upload/download identity, and the benchmark as a test that records. The box script rebuilt on the product configure (decision 4) | forge-video-hw (new), forge-video-codecs, forge-video, `deploy/gcp-gpu` |
| 8b | **The pipeline on the device.** forge-media (release): `DeviceCompositor` on an FFmpeg filter graph (decision 2), one graph per output rebuilt on layout change, the chrome as cached host planes uploaded on change; `VideoRoom` placing a room's sources, slots, compositor and flavors on `backend.device`, with a per-stage host fallback that uploads or downloads once per tick — a decoder the device lacks, an encoder it lacks (VP8/VP9, decision 5), the recording sink — and the room's status naming each stage's device; passthrough (§15.5) unchanged, since it never decodes; the content view (§15.6) composited on the device like the rest; `CodecPool` threads bound to a device context each. Tests, on the box: layout parity — every golden image of the host compositor rendered on the device within a PSNR threshold, presentation layout included; a room mixed on the device reaching a host-decoded subscriber; a VP8 subscriber of a device room fed from the downloaded canvas; overload shedding on the device clock. In CI without hardware: the room's device placement and fallback decisions unit-tested against a registry that declares a fake device | forge-video, forge-conference |
| 8c | **The node.** FCP: `[conference] video_device` (`host`, or `cuda:0`), the backend built on it and the codec registry's `hwaccel` feature behind a `video-hwaccel` cargo feature on the conference server; the self-benchmark run per device with saturation (decision 7) and the cost table carrying host and device constants; `video_budget` advertising the device beside `host` with the codecs it *encodes* (decision 5), placement by capability confirmed against it (`video_budget_left` already picks the best device with the codecs; the change is which codecs a device claims) and tested in `conference_placement`; a join priced on the device the room is on; `GET …/video` and `/status` naming the device per room and stage, the admin cluster page and Live Video dialog showing it; `conference.video_overload` unchanged; the Dockerfile and compose gaining an NVIDIA variant (`nvidia-container-toolkit`, the LGPL FFmpeg build) and `docs/GPU.md` (deployment: drivers, container runtime, the codec list per device, what falls back to the host). Tests: the conference server's loopback tests run on the box with `video_device = cuda:0` through the same `gpu-box` script (`gpu-box test`), a budget test that a saturated device refuses the next join with 503 while the host still admits one, the placement test with a GPU node and a VP8-first meeting | fcp-conference-server, fcp-cluster, fcp-call-manager, fcp-admin-ui, deploy |

**Order and size.** 8a first and alone, and its numbers decide the rest:
if the L4 encodes fewer than about forty 720p30 H.264 streams saturated
(§9.1's expectation), or the filter graph's per-tick cost is more than a
host composite's, decisions 2 and 7 are revisited before 8b. 8b needs
8a's release; 8c needs 8b's. About one and a half weeks for 8a (the
FFI layer is the bulk), one and a half for 8b, one for 8c. Every block's
tests that need the hardware run through `gpu-box`; CI keeps building
with the feature off, and the feature's build is checked on the box.

**What this phase does not do.** VAAPI and AMF beyond compiling
(decision 3); an MP4/H.264 recording sink (decision 5); a room that
moves between devices (a room stays on the device it started on, §9.3);
more than one GPU per node (`cuda:1` is a second device the config could
name later, but a node's backend is one device); GPU-side RTP
depacketization or SRTP (they stay on the host, as the design has it);
and a GPU runner in CI.

**8a as built (2026-09-12; forge-media `v2026.09.12`: forge-video-hw
0.1.0, forge-video-codecs 0.1.2; FCP v0.25.133).** As planned, and the
numbers clear the gate. `forge-video-hw` sits over `ffmpeg-sys-next` 7.1
(bindings generated against the machine's FFmpeg, `avcodec` and
`avfilter` only): `HwDevice` opens a `MediaDevice::Gpu` — `cuda:0`;
`vaapi` and `qsv` compile, untested (decision 3) — and hands out
`HwFrames` pools by size; a `DeviceFrame`'s handle is an `AVFrame` in
device memory, NV12 throughout, and `frame::upload` / `download` cross
the bus with one chroma interleave each way, bit-exact. `HwDecoder` is
FFmpeg's `h264`, `hevc`, `vp8`, `vp9` and `av1` decoders with the
device's acceleration (NVDEC), output left on the device; `NvEncoder` is
`h264_nvenc`, `hevc_nvenc` and `av1_nvenc` taking device frames (a host
frame is uploaded), zero latency, parameter sets in-band on every IDR,
`ContentHint` mapped to preset and spatial AQ, keyframe on demand, and
the bitrate retarget by reopening before the next frame, which is then
a keyframe (decision 6). `DeviceScaler` is `scale_cuda` in a kept
two-node filter graph per pair of sizes. `probe` says what a device
offers, `register` puts it in a `CodecRegistry`, and forge-video-codecs'
`hwaccel` feature registers the default device when it opens.
`bench::saturate` runs 1, 2, 4, … streams of one stage on their own
threads until the summed frame rate stops growing (decision 7). On the
L4 (driver 550, the LGPL `cuda-llvm` FFmpeg of decision 4, 720p30 at
1.2 Mb/s): H.264 encodes saturate at about 62 real-time streams, AV1 at
79, HEVC at 57; decodes at 237, 252 and 306; round trips 25–26 dB
against the source; `scale_cuda` matches the host's bilinear within
29 dB. Well past the forty-stream gate, so decisions 2 and 7 stand for
8b. Two measurements the plan should carry: a CPU-fed encode is about
16 streams on four vCPUs — that is the upload, which is why the
saturation bench feeds device frames and why 8b keeps the frames on the
device — and the filter graph's per-tick cost was not measured in 8a
beyond the scaler, so 8b measures the compositor's graph against the
host composite before committing to it. Found on the way: a temporary
`BufferRef` handed its raw pointer to a codec context and then dropped
it (a dangling `hw_frames_ctx`, a segfault in `avcodec_free_context`;
fixed with an owning `into_raw`); NVENC has no `repeat_headers` option;
Debian 12's clang 14 knows `sm_86` at most, which the L4 runs through
the driver's JIT. The box script now builds the product's FFmpeg
(decision 4), `gpu-box sync` takes one directory and goes by tar,
`gpu-box move <zone>` follows Spot capacity with the disk as a
snapshot, and CI installs the FFmpeg development headers so the crate
and the feature compile there without a GPU (the tests skip).

**8b as built (2026-09-12; forge-media `v2026.09.12.1`: forge-video
0.6.0, forge-video-hw 0.2.0, forge-video-codecs 0.1.3, forge-conference
0.13.0; FCP v0.25.134).** As planned, with one shape settled by the
parity test. forge-video's `DeviceBackend` is what a room is placed on
(its compositor and scaler, `upload` / `download`, `resident`; the host
is `HostBackend`), and the compositor's chrome is one piece of code for
every device — `tile_geometry`, `draw_chrome_under`, `draw_band`,
`label_text` — which the host compositor draws through, pixel for pixel
as before, and a device compositor paints into host planes and uploads.
`parity::scenes` is the golden set (nine scenes: mixed sizes and
aspects, avatars, speaking, muted, presentation with a 4:3 document,
content alone, sixteen tiles, a tiny canvas), the host's rendering the
reference. `CodecRegistry::decodable_on` / `encodable_on` split what
`codecs_on` asked of both; `testing::FakeBackend` is a device that is not
there (host frames behind device handles, copies counted, a compositor
that can be told to fail) with the raw codec registered on it. forge-
video-hw's `DeviceCompositor` is one filter graph per output: onto a
background uploaded once, in tile order, each tile's chrome plane (ring,
bars or avatar, cached by what it shows and uploaded when that changes),
its picture through `scale_cuda` (bilinear for cameras, Lanczos for a
shrunk screen) and its name band, each an `overlay_cuda`; the graph is
rebuilt when its shape changes and a steady scene crosses the bus not at
all. The first cut painted every ring into one underlay under all the
pictures and drew picture-in-picture at 23 dB (the main picture covered
the corner's ring); tile order, the host's order, is what fixed it.
`HwBackend` is the backend over an open `HwDevice`, with the codecs
registered on the same open device; forge-video-codecs' `open_device`
does both. forge-conference places each stage on the device when it can
and on the host otherwise, one copy per frame: a decoder the device
lacks uploads what it decodes (at decode time, once, not per output); an
encoder it lacks — VP8 and VP9 on NVENC, so every WebM recording — takes
the canvas downloaded once a tick for all such flavors; an output whose
device compositor does not build composes on the host from downloaded
tiles. Found on the way: an oversize shared screen decoded on a device
was dropped (the shrink only knew host frames); it is shrunk on the
device's scaler now. The status names the device per stage
(`VideoRoomStatus::device`, per source, output and flavor; the conference
server's JSON carries them as `device`), `video_frames_moved_total`
counts the copies, and five fake-device room tests run in CI. The
`CodecPool` binds no context per thread: FFmpeg pushes the CUDA context
per call, so the plan's item is dropped. On the L4: every scene within
39.5 dB or better of the host (the scaler is the difference; the chrome
is identical), the room test end to end (NVDEC in, the device composite,
NVENC out to one subscriber and the downloaded canvas to a VP8 one). The
plan's open check: a 720p composite costs the host 3.9 ms a tick at any
tile count (release build) and the device 0.5 / 1.5 / 3.3 / 5.7 ms for
1 / 4 / 9 / 16 tiles — the graph's cost is about 0.35 ms per filter
stage, launch and sync overhead rather than pixels, so the device is
cheaper below about nine tiles and dearer above. It fits a 30 fps tick
with room to spare and composition is not what a GPU node is bought for,
so the graph stays for 8c, priced by 8c's benchmark in the budget either
way; a single-kernel compositor (decision 2's alternative) is a later
optimisation if the number matters.

**8c as built (2026-09-12; FCP v0.25.135).** As planned, with two
things the code had to settle. `[conference] video_device` names the
device (`host`, or `cuda:0`; validated at start) and the `video-hwaccel`
feature — forge-video-codecs' `hwaccel` and forge-video-hw, not part of
`video-all` — lets the node open it through `open_device`, falling back
to the host with a warning when it does not open. First: the node's
codec list. `usable_codecs` asked for a decoder and an encoder on one
device, which on a GPU would have dropped VP8 and VP9 (NVDEC decodes
them, NVENC encodes neither) and, under `native_only`, everything else
too — a GPU node would have offered no video at all. `node_codecs` is
now a map per device: a codec is usable when the node decodes it
somewhere and encodes it somewhere, the device's own codecs always
count, and the host's count under `native_only` only with a binding.
Second: the budget. `VideoCapacity` holds one `DeviceCapacity` per
device — the GPU and the host beside it — and a `Cost` is units per
device: a join is priced on the device each stage would run on (decode
and composite on the room's device, the encode where the codec's
encoder is), reserved only when it fits everywhere it touches and
holding nothing when refused (still a 503 before the 180), and rooms
are re-priced from the device each stage reports in 8b's status. The
GPU is one execution unit (`1000 × headroom` units) priced by
saturation (decision 7, `self_benchmark_device`): each stage of each
codec runs with 1, 2, 4, 8 streams of 720p30 for a share of
`video_benchmark_seconds` until throughput stops growing, and the
constant is what makes that many real-time streams fill the device; the
composite is timed on the device's compositor. Membership advertises
every device's budget with the codecs it *encodes* (decision 5) — the
list per node was there since phase 3, only one entry was ever pushed —
and placement prefers a device only when it encodes the meeting's first
codec (`VideoNeed::first`, `video_budget_left_for`; the any-codec rule
when no device has it), so a VP8-first meeting is not sent to a GPU for
a budget it would spend on that node's host. `/status` lists every
device under `video_capacity.devices`; the Live Video dialog shows the
room's device and what fell back; the cluster page already showed one
line per device. Deploy: `Dockerfile.conference.nvidia` builds the LGPL
`cuda-llvm` FFmpeg in the image and the binary with `video-hwaccel` and
the native bindings, the container toolkit injecting the driver's
libraries (`NVIDIA_DRIVER_CAPABILITIES=compute,video,utility`);
`docker-compose.gpu.yml` and a GPU configuration sample; `gpu-box test`
runs the hardware tests; `docs/GPU.md`. On the box, the conference
server built with the feature against `cuda:0` passes every test, and
the node test opens the device (encodes H.264, AV1, HEVC; decodes all
five), prices it and advertises the two budgets (`cuda:0` with H264,
AV1, H265; `host` with all five). Two measurements to carry: the
saturation run with the test's six-second budget read 109 real-time
H.264 encodes against 8a's 62 measured with long steps — the default
ten seconds spread over four steps of every stage and codec is coarse,
and a GPU node is better given a minute; and the composite is priced
single-threaded at the compositor's per-tick cost (6.4 ns/px in the
debug test build, about 3.6 in release), which makes a 720p15
composite about five per cent of the device although the device runs
several graphs at once — the budget is conservative there, and a
per-engine split (decision 7's follow-up) would price it better. Not
done, as the plan says: VAAPI and AMF beyond compiling, an MP4 sink (a
GPU room's WebM is VP8 on the host), a room moving between devices,
more than one GPU per node, GPU-side RTP or SRTP, a GPU runner in CI.
Phase 8 is complete.
### 15.8 Phase 8 follow-ups — the budget's engines (2026-09-12)

Phase 8 shipped with three things its own measurements say the budget
gets wrong on a GPU, one small mispricing that predates it, and two
larger items it deferred. This section settles which to do now and
how, in one block.

**What the numbers say.** A GPU's budget is one number priced by
saturation (8c, decision 7), but NVENC, NVDEC and the CUDA cores
saturate separately: a node out of NVENC with decode to spare reads as
full, and the composite is priced at the compositor's per-tick cost as
if the device ran one graph at a time — a 720p15 composite is about
five per cent of an L4 by that reckoning, although the device runs many
graphs at once. The saturation run with short steps overshoots (a
six-second run read 109 real-time H.264 encodes where long steps
measure 62). And `ContentGate::admit` prices a share with the default
room settings rather than the room's own.

**Decisions, each with a recommendation.**

1. **Engines, not one number.** Three ways: keep the one number and
   only fix the composite by measuring it saturated (the smallest
   change; a node out of one engine still reads as full); three
   budgets per device on the wire, each stage charged to its engine
   and placement matching the *encode* engine (right, but a wire and a
   UI change); or three budgets kept internally with one advertised —
   the tightest engine's — which keeps admission exact and placement
   blind to why. **Recommended: three budgets on the wire, as an
   optional addition an older node simply lacks.** `VideoDeviceBudget`
   gains `engines: Option<Engines>` with `encode`, `decode` and
   `compose` each `{ left, total }`; a node without it reads as today.
   `DeviceCapacity` holds three pools for a GPU (the host keeps one:
   its threads are one pool) and charges each stage to its engine;
   `video_budget_left_for` takes the encode engine's `left` when the
   budget has engines; `left`/`total` on the wire stay the tightest
   engine's, so an older placer still sees something true. The
   composite's constant comes from its own saturation — 1, 2, 4, 8
   graphs at once until the summed tick rate stops growing — through
   a `measure_compose_saturated` beside `measure_compose_on`. `/status`
   lists the engines; the cluster page shows three bars for a GPU
   device and one for the host.
2. **The benchmark budget.** Longer default seconds, fewer steps, a
   warm-up, or a floor per step. **Recommended: a warm-up and a
   floor** — `bench::saturate` discards each step's first second
   before it counts, and the device benchmark runs each step for at
   least one second whatever `video_benchmark_seconds` says, logging
   what it took. The default stays 10: the host's benchmark keeps it,
   and a GPU node's run takes what its floor needs (about half a
   minute for three encoders, five decoders and the composite).
3. **The share's price.** **Recommended: `ContentGate::admit` takes
   the room's settings** — `admit(room, participant, &VideoRoomSettings)`
   in forge-conference, the capacity pricing the share at the room's
   content cap and rate.
4. **A single-kernel compositor** (decision 2's alternative).
   **Deferred**: the graph fits the tick with room to spare; revisit
   if the compose engine is what saturates first once 1 measures it.
5. **An MP4/H.264 recording sink**, which would let a GPU room record
   on NVENC rather than VP8 on the host. **Deferred**: a muxer of its
   own (forge-webm's sibling) and a phase of its own if wanted.
6. Multi-GPU nodes, VAAPI and AMF beyond compiling, GPU-side RTP:
   unchanged, deferred.

**Block.**

| Block | What | Where |
|---|---|---|
| 8d | **The budget's engines.** forge-media (release): `bench::saturate` with a per-step warm-up; `forge_video::bench::measure_compose_saturated` (graphs in parallel until throughput stops growing, on any backend); `ContentGate::admit` with the room's settings (forge-conference 0.14.0). FCP: `DeviceCapacity` with a pool per engine on a GPU (`Engine::{Encode, Decode, Compose}`), each stage charged to its engine and `Cost` naming engines; `self_benchmark_device` with the one-second floor and the composite's saturation; `VideoDeviceBudget.engines` (optional, `skip_serializing_if`) and `video_budget_left_for` on the encode engine; `/status` engines; the cluster page's bars; the Live Video dialog unchanged. Tests: a device out of encode but not decode refuses an encode-bearing join and admits a decode-only one (a source without a flavor); membership with and without `engines` on the wire, both ways; placement on the encode engine; the share priced at the room's cap; on the box, the composite's saturation and the warm-up's effect on the H.264 number | forge-video, forge-video-hw, forge-conference; fcp-conference-server, fcp-cluster, fcp-admin-ui |

**Size.** About a week; one forge-media release, one FCP PR after its
repoint. The 8d numbers replace the two notes 8c carried.

**8d as built (2026-09-12; forge-media `v2026.09.12.2`: forge-video
0.6.1, forge-video-hw 0.2.1, forge-conference 0.14.0; FCP v0.25.137–138).**
As planned. A GPU's `DeviceCapacity` is three pools — encode, decode,
compose — each `1000 × headroom` units; the host keeps one. A `Cost` is
units per device *and engine*: a join is reserved only when every
engine it touches has room and holds nothing when refused; rooms are
re-priced per engine from the device each stage reports; a share, a
decode alone, is admitted while the compose engine is out.
`forge_video::bench::measure_compose_saturated` runs 1, 2, 4, …
compositors at once (each warmed up uncounted) until the summed render
rate stops growing, and `ComposeSaturation::ns_per_px` is the constant
that makes that many composites fill an engine — what the device
benchmark now prices `compose` with; `forge_video_hw::bench::saturate`
warms each step up for a second before counting, and the device
benchmark runs each step for at least a second whatever
`video_benchmark_seconds` says, logging what it took. On the wire,
`VideoDeviceBudget.engines` is optional (`skip_serializing_if`), so a
node from before reads exactly as today, with `left` / `total` the
tightest engine's; placement weighs the encode engine's `left`
(`placement_left`); `/status` lists the engines per device and the
cluster page shows the three under a GPU's line. `ContentGate::admit`
takes the room's settings. Tests: the compose engine running out while
encode has plenty (an H.264 join refused, a share admitted, `left` the
tightest engine's), a smaller content cap priced lower, `engines` on
the wire both ways, placement reading the encoder's budget. On the box (L4), both hardware tests
green, and two numbers that settle 8c's notes. With the warm-up, the
saturation run reads what 8a measured with long steps — 62 real-time
720p30 H.264 encodes, 78 AV1, 57 HEVC; 255, 279 and 329 decodes — and
the node's benchmark, at the one-second floor, took 79 seconds for
three encoders, five decoders and the composite. The composite's
saturation is the surprise: nine 720p tiles render at 1,737 ticks a
second on one graph (0.58 ms a tick, 0.61 ns/px), and a second graph
adds nothing, so the engine holds about fifty-eight rooms' 30 fps
composites; the 3.1 ms a tick that 8b and 8c reported was the first
render's uploads and graph build averaged over sixty frames, not the
steady state — the device composite is about seven times cheaper than
the host's at nine tiles, not dearer. An H.264 join at 720p15 now
costs 21 units of the GPU (9 encode, 2 decode, 10 compose) where 8c
charged 110, and a VP8 join 35 on the GPU plus 162 on the host. The
single-kernel compositor (decision 4) is further off than the plan
thought.

### 15.9 Phase 9 plan — the MP4 recording sink (2026-09-12)

Recordings are WebM: VP8 or VP9 with Opus, written by `forge-webm`
(§15.4). That was the right first container — a writer of a few
hundred lines with no licence to think about — and it has one cost that
phase 8 made plain: a room on a GPU records on the host, because NVENC
has no VP8, and every WebM player short of a browser wants a
transcode. MP4 (ISO base media, ISO/IEC 14496-12) fixes both: H.264,
HEVC and AV1 go in as they come off the encoder, so a GPU room records
on NVENC, and the file plays everywhere. §16 kept MP4 as an option;
this is it.

**What the surveys found.** `RecordRequest` takes any codec — the room
does not care — and `RecordingSink` hands out `CodedFrame`s: for H.264
and HEVC an Annex B byte stream (start codes before each NAL unit,
parameter sets in-band on keyframes — 8a's NVENC repeats them on every
IDR, OpenH264 does too), for AV1 a temporal unit of OBUs. The
conference server's `recording.rs` accepts VP8 and VP9 only
(`webm_codec`), mixes the room to 48 kHz Opus through `audiopus` behind
the `recording-webm` feature, and stamps both against the first frame's
clock; `RecordingFormat` in fcp-core is `Wav | Webm`, stored as text in
`conference_recordings.format` (an unknown value reads as the default,
so a newer node's rows do not break an older one); `GET /recordings/
{file}` picks `video/webm` by extension; the admin UI plays the file in
a `<video>`. `forge-webm` has a reader for its tests and a fuzz target
on that reader, the pattern to follow.

**Decisions, each with a recommendation.**

1. **Write it ourselves or use a crate.** The `mp4` crate writes whole
   files (the `moov` after the `mdat`, nothing playable until the
   end); `mp4-atom` and `re_mp4` read. A fragmented writer is a small
   thing — `ftyp`, a `moov` with empty sample tables and an `mvex`,
   then `moof` + `mdat` per fragment — and `forge-webm` set the
   precedent. **Recommended: a new `forge-mp4` crate, no dependencies
   beyond `thiserror`, with a reader for its tests and a fuzz target.**
2. **Fragmented or whole.** A recording grows for an hour and must
   survive the node dying: a whole-file MP4 with its `moov` at the end
   is unplayable until then; a fragmented one is playable to the last
   complete fragment and streams. **Recommended: fragmented MP4**, one
   fragment per video keyframe as they come (the recording's flavor is
   a subscriber's, so the room's keyframe interval, ten seconds by
   default, is the fragment length — no extra keyframes asked for),
   `tfdt` on every fragment, and an `mfra` (fragment random access)
   appended on a clean stop for fast seeking; a file cut short seeks
   by scanning `moof`s, which every browser does.
3. **Audio.** AAC is what MP4 players expect and has no encoder this
   workspace can take (fdk-aac's licence, no pure-Rust one); Opus in
   MP4 is a standard (`Opus` sample entry with `dOps`, per the Opus in
   ISOBMFF spec) that Chrome, Firefox, Safari 17 and VLC play.
   **Recommended: Opus, the same 48 kHz mix the WebM path already
   encodes; AAC deferred** until a player that matters lacks Opus.
4. **Video codecs.** H.264 (`avc1` + `avcC`, samples as AVCC length-
   prefixed NAL units converted from Annex B, SPS/PPS lifted from the
   first keyframe), HEVC (`hvc1` + `hvcC`, VPS/SPS/PPS arrays) and AV1
   (`av01` + `av1C`, the sequence header OBU, samples as temporal
   units). VP8 and VP9 stay WebM (VP9 in MP4 is legal and pointless
   here). **Recommended: all three**, since the depacketizers already
   hand the writer what it needs and the parameter-set lifting is the
   same shape for each; H.264 is the one 9b's tests run on the box.
5. **What `auto` means.** Today `recording_format = auto` is WebM when
   the room has video. **Recommended: `auto` is MP4 when the node can
   encode H.264 (natively or on its device) and WebM otherwise**, so a
   GPU node records on NVENC and a host node with OpenH264 records the
   container every player opens, while a VP8-only build changes
   nothing; `mp4` and `webm` stay as explicit choices, and
   `recording_video_codec` grows `h264`, `hevc` and `av1`, validated
   against the format (`vp8` in `mp4` is refused at start, as is `h264`
   in `webm`).
6. **The player and the API.** `video/mp4` by extension, `Mp4` in
   `RecordingFormat` (an older node reads an `mp4` row as the default,
   as designed), the admin UI's `<video>` needs nothing. Retention,
   consent and the compliance sweep are format-blind already.

**Blocks.**

| Block | What | Where |
|---|---|---|
| 9a | **`forge-mp4`.** forge-media (release): the writer — `Mp4Config` with a video track (`avc1`/`hvc1`/`av01`, size, the codec configuration record built from the first keyframe's parameter sets) and an Opus audio track (`dOps` from the same pre-skip/sample-rate the WebM path uses), `ftyp` + `moov` (`mvhd`, `trak` × 2 with `stsd` and empty `stts`/`stsc`/`stsz`/`stco`, `mvex` with `trex`) written at open, then `moof` (`mfhd`, `traf` × 2 with `tfhd`, `tfdt`, `trun`) + `mdat` per fragment, a fragment closed at each video keyframe, `mfra`/`tfra`/`mfro` on `finish`; Annex B → AVCC conversion; 90 kHz and 48 kHz timescales; a `read` module that parses the boxes back (tracks, fragments, sample counts and durations, the codec record) for the tests and a fuzz target `mp4_read` with seeds derived by a test. Tests: round trips per codec from synthetic NAL units, a file cut mid-fragment still parsing to its last complete one, `mfra` present on a clean stop and absent otherwise, and — if `ffprobe` is on the machine — the file probing as `h264`/`opus` (skipped without it). forge-conference unchanged | forge-mp4 (new) |
| 9b | **The recorder.** FCP: `recording.rs` gains the MP4 path behind `recording-mp4` (`dep:forge-mp4` + `forge-codecs/opus`, the same Opus the WebM feature builds), `FormatChoice::Mp4`, `auto` per decision 5, `recording_video_codec` in `h264`/`hevc`/`av1`/`vp8`/`vp9` validated against the format, the recording flavor asked for at the room's resolution as today; `RecordingFormat::Mp4` in fcp-core (`mp4` on the wire and in the store, migration-free), `video/mp4` on download, `docs/COMPLIANCE.md` and `docs/GPU.md` mention. Tests: a room recorded to MP4 with the raw codec standing in for H.264 (the raw payload is not NAL units, so the FCP loopback test asserts the container's tracks and fragments through `forge_mp4::read`, and the codec record is checked in 9a's own tests); on the box, a GPU room recorded on NVENC into an MP4 that `ffprobe` reads as H.264 + Opus, through `gpu-box test` | fcp-conference-server, fcp-core, fcp-api-core, fcp-admin-ui (types only), deploy configs, docs |

**Order and size.** 9a first, released; 9b after the repoint. About a
week: four days for the writer and its reader, two for the recorder,
one on the box.

**What this phase does not do.** AAC (decision 3); MP4 for VP8/VP9;
transcoding an existing WebM to MP4 (a job, not a sink); SIPREC or
per-participant recordings (§11's scope is the composite); a
`faststart` whole-file mode; encrypted MP4 (`cenc`) — recordings are
protected at rest by the store's encryption and the file system, as
today.

## 16. Resolved questions

Raised with the first draft and decided 2026-09-04, each as recommended.

| Question | Decision |
|---|---|
| Default frame rate | **15 fps** by default; 30 fps per meeting. Half the CPU, and what phone-heavy rooms want |
| Self view | **Own tile included** by default so encoders are shared; `exclude_self` per meeting, charged in the capacity model |
| H.264 library | **OpenH264 built from source**, feature-gated. Reproducible; the C++ toolchain is only needed with the feature on |
| Recording container | **WebM** (VP8/VP9 + Opus) by default, every part permissively licensed; MP4 (H.264 + Opus) as an option |
| Screen share | Section handling designed in phase 3 so a second video section (RFC 4796 `a=content:slides`) is additive; **implemented after phase 6** — planned as phase 7 (§15.6) |
| Hardware acceleration | Revised after review: **designed in now** (device-resident frames, pipeline per device, per-device budgets, placement by capability; §5.5, §9), **built as phase 8** (phase 7 until screen share took the number) on FFmpeg hardware contexts once the CPU path is the proven reference |
| Maximum resolution | **720p** by default; 1080p per meeting |
| Dev machine memory | Phase 0 measures **relative** constants here (12 cores, 2 GB, a Microsoft VM with no GPU device); absolute capacity is re-measured once on a production-shaped node before phase 3, GPU constants on a GPU box, and every node self-benchmarks at start |

## 17. References

RFC 3550 (RTP), RFC 4585 (RTCP feedback: PLI, NACK), RFC 5104 (FIR,
TMMBR), RFC 6184 (H.264 RTP), RFC 7798 (H.265 RTP), RFC 7741 (VP8 RTP),
RFC 9628 (VP9 RTP), AV1 RTP payload specification (AOM), REMB
(draft-alvestrand-rmcat-remb), transport-cc
(draft-holmer-rmcat-transport-wide-cc-extensions), RFC 3264 (offer/
answer), RFC 4796 (content attribute), RFC 4574 (label attribute), RFC
8285 (RTP header extensions), RFC 8843 (BUNDLE), RFC 8855 (BFCP), RFC
8856 (BFCP in SDP, UDP transport), RFC 8829 (JSEP), RFC 8834 (WebRTC
media transport).
