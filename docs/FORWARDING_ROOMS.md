# Forwarding rooms

Designed 2026-11-13. FCP's video rooms mix (`docs/VIDEO_CONFERENCING.md`): the
server decodes every camera, composes one picture per layout and encodes it. Each
participant sees that picture in the layout the host chose. `docs/SOFTPHONE_VIDEO.md`
§6 left the next step out of its scope: rooms that **forward each participant's own
stream** to the clients that can take it, with simulcast, so each person lays out
their own view. This document designs that step for FCP, forge-media and Anvil.

Questions are in §8, each with a recommendation.

## 1. What exists

### 1.1 Why the rooms mix

`docs/VIDEO_CONFERENCING.md` §2 chose a mixer for good reasons, and they still hold:

- a SIP video phone has one `m=video` and one decoder;
- a room can mix H.264 phones with VP8 browsers;
- the composite is the recording;
- a cascade carries one composite per direction.

A forwarding room must keep all of that. **Forwarding is added beside mixing, per
subscriber.** A SIP phone, a recording and a trunk go on getting the composite. A
client that asks for streams gets streams.

### 1.2 forge-media

- **One subscription per participant and kind.** `VideoRoom` keeps one subscriber
  per (participant, `StreamKind`) (`forge-conference/src/video/room.rs`
  `subscribe_kind`, "replaces any existing main subscription"). A `Subscriber` has
  one SSRC, one retransmission cache and one optional `Forwarding`
  (`egress.rs:140-163`).
- **Forwarding is a selector.** The passthrough fast path (phase 6a) forwards a
  subscriber's whole view when it is a *single* source (`sole_source`,
  `decide_forwarding`). Its `StreamRewriter` (`forge-rtp/src/video/mod.rs:330-455`)
  switches between sources at a keyframe and rewrites SSRC, sequence and timestamp.
  It does not rewrite VP8 or VP9 picture ids or TL0PICIDX. It passes the sender's
  header extensions through untouched.
- **Feedback.** NACKs are answered per subscriber from its own cache, as plain
  resends on the same SSRC (no RFC 4588 RTX). A PLI from a forwarded subscriber goes
  to the source. REMB is stored per subscriber. The bitrate ladder moves a composite
  subscriber between flavors, and skips forwarded subscribers.
- **Ingress.** A source is one SSRC. A packet is offered to every forwarding
  subscriber (O(subscribers) per packet), then assembled and **always decoded**.
- **No simulcast or layers anywhere.** `a=simulcast`, `a=rid`, the rid header
  extensions, transport-cc, dependency descriptors and temporal layers appear
  nowhere in forge-rtp, forge-sdp, forge-webrtc, forge-video or the codecs.
  forge-rtp does read and write RFC 8285 header extensions generically. forge-webrtc
  negotiates `sdes:mid` only, and drops `transport-cc` from answers.
- **Encoders** each make one resolution. Keyframes on demand work everywhere.

### 1.3 FCP

- **One camera section and one content section, everywhere.** The conference
  server's `ParticipantSession` has `video: Option<VideoLeg>` and
  `content: Option<VideoLeg>`. In `fcp-media`, `parse_video_sections`,
  `AnswerOptions`, `OfferOptions` and `rewrite_for_anchoring` keep the first camera
  and the first `a=content:slides` section, and give any other section port 0.
- **The room never re-INVITEs.** The gateway answers an in-dialog re-INVITE with
  405 (`fcp-webrtc-gateway/src/inbound.rs`).
- **The roster carries no stream ids.** `roster.rs` sends no `src-id`, although
  `docs/SOFTPHONE_VIDEO.md` §2.4 says the roster maps a stream to a participant
  with it.
- **Controls are room-wide.** Layout, pin and spotlight are each one setting per
  room (`video_bridge.rs`). There is no per-viewer view.
- **Capacity** prices a decode per source, an encode per flavor and a compose per
  output (`video_capacity.rs` `room_cost`). Forwarding has no price of its own;
  flavors whose subscribers are all forwarded are skipped (`idle_flavors`).
- **Cascade.** A peer node's callers arrive as one composite, drawn as one remote
  tile (`attach_trunk_video`, `OutputScope::LocalOnly`).
- **Tests.** No room test sends video end to end. Video is proved by
  `video_bridge.rs`'s unit tests and the recording tests.
  `fcp_testkit::video::VideoLeg` is a single camera section used only by the call
  manager's tests.

### 1.4 Anvil

`docs/SOFTPHONE_VIDEO.md` §2.4 prepared Anvil for this, and most of it is real:

- **Streams.** Each new SSRC in a section becomes a stream, with its own assembler,
  NACK and PLI state, playout and sink (`anvil-core/src/video/session.rs`).
  `Event::VideoStarted` is keyed by `StreamId`. It carries no SSRC and no
  participant.
- **Encodings.** `SendConfig.encodings` is a list, and `VideoSource::next_frame`
  names the encoding. But the core asks for one, and the send loop drops any
  other. Each platform host makes one encoder from one capture.
- **Estimation.** `BandwidthEstimator` is a trait, with REMB behind it. One REMB
  covers every incoming SSRC of a section.
- **Transport.** `MediaTransport` is a trait, with only the SDES/SIP one. There is
  no WebRTC code.
- **Rooms and the app.** The roster parser never reads `src-id`, and a tile has no
  name. The app keeps any number of camera streams but only one content stream. Its
  stage draws a grid, or a shared screen with up to four cameras.

## 2. The model

### 2.1 Two kinds of subscriber in one room

A room has **sources**: each participant's camera, and the shared screen. It has
two kinds of **subscriber**:

- **Composite subscribers**, as today: SIP phones and room systems, browsers that
  do not ask for streams, recordings and trunks. They get the room's picture.
- **Forwarding subscribers**: clients that said they take streams. A forwarding
  subscriber gets **one stream per source it asks for**. These are each
  participant's camera at the layer that fits, and the shared screen in its own
  section as today.

The meeting's new `video_mode` decides who may forward (§8 Q1):

| `video_mode` | What happens |
|---|---|
| `mix` | Everyone gets the composite. This is today's behaviour. |
| `auto` | A client that asks for streams gets them, and the rest get the composite. |

The room composes **only while someone needs the composite**. A room of forwarding
clients with no recording, no SIP phone and no trunk decodes nothing and encodes
nothing.

### 2.2 Streams over SIP, by SSRC, in the sections that exist

A forwarding subscriber's streams all come down **its one camera section**. Each
stream is its own SSRC, as RFC 8108 allows: several RTP streams in one session.
This choice needs no new section, no re-INVITE as people join, and no change to the
call manager's relay:

- Anvil already makes a stream per SSRC in a section.
- The call manager's `VideoRelay` relays a section packet for packet, whatever the
  SSRC.
- The gateway gives each SSRC a transceiver of its own toward the browser (§2.7).

**A stream's SSRC is stable.** It is fixed per (viewer, source) for as long as the
viewer asks for that source. A layer switch keeps the SSRC: the rewriter changes
the stream inside it at a keyframe, as it does today. A source the viewer stops
asking for stops sending, and its SSRC is dropped after Anvil's idle time. So an
SSRC means one person for its whole life, and nothing has to be re-mapped mid-call
(§8 Q3).

**The roster says whose an SSRC is.** RFC 4575's `<src-id>` names it in the
viewer's roster: each member's `<media id="video">` carries the SSRC that viewer
receives that person on. Each viewer's roster is its own already, because NOTIFYs
are sent per subscription. The shared screen keeps its section and its
`<label>content</label>`.

### 2.3 Asking for streams: the `fcp-view` INFO package

A client declares it can take streams with `Recv-Info: fcp-view` on its INVITE. It
already sends `Recv-Info: fcp-content` for the floor. In the call, the client sends
an INFO in the `fcp-view` package (`application/vnd.fcp.view+json`) whenever its
layout changes:

```json
{ "streams": [
    { "participant": "p-1a2b", "max_height": 720, "priority": 1 },
    { "participant": "p-3c4d", "max_height": 180 },
    { "speaker": true, "max_height": 720, "priority": 1 }
  ],
  "max_streams": 9 }
```

- **A participant entry** asks for that person at no more than that height.
- **A `speaker` entry** is a stream that follows the active speaker. The server
  switches it at a keyframe and names whose it is in the roster.
- **`priority`** says which streams are kept first when bandwidth is short.
- **A participant left out** is not sent.

The answer is 200 once the room has taken the view; what was granted follows in a
partial NOTIFY, whose `<src-id>`s say which SSRC is whose (the grant is worked out
on the room's next tick, so the roster rather than the 200 carries it). A view
that cannot mean anything is 400; an INFO from a call that was not agreed to take
streams is 469 Bad Info Package (RFC 6086 §4.2.2). Without an INFO, a forwarding subscriber gets the speaker at 720p
and up to eight others at 180p, ordered by who spoke last. So a client that never
sends a view still sees people.

### 2.4 Simulcast in

A sender offers up to three encodings of its camera in its one camera section, as
RFC 8853 describes:

- `a=rid:h send`, `a=rid:m send`, `a=rid:l send` and `a=simulcast:send h;m;l`;
- the RTP stream id header extension (RFC 8852) on the first packets of each.

The room answers with the encodings it accepts. It learns each encoding's SSRC from
its rid. A sender that offers no simulcast sends one encoding, and that is all the
room has.

In the room, a source becomes **up to three layers**, each with its own SSRC,
assembler and keyframe state. Each layer is decoded only if a composite needs it,
and then only the smallest layer that fills the composite's tile. Simulcast
therefore also makes mixing cheaper for browsers and Anvil, so phase R1 is worth
having alone.

Recommended layers: 720p, 360p and 180p for a camera. Phones send two (360p and
180p) on battery or a weak link. Anvil decides; the room takes whatever rids
arrive (§8 Q5). The shared screen is not simulcast: it is forwarded as sent, and
composed from a decode as today.

### 2.5 The allocator

Every viewer has a budget: its REMB, which Anvil and browsers already send. Each
time a REMB arrives, or a view or the speaker changes, the room allocates the
budget across the viewer's requested streams:

1. **Sort by priority.** Priority-1 streams come first (the speaker, a pin), then
   the rest in the viewer's order.
2. **Give each stream its lowest layer.** Stop when the budget runs out; any streams
   left are paused, which the roster shows as `fcp:paused`.
3. **Upgrade in priority order.** Each stream goes to the largest layer within its
   `max_height` that still fits.
4. **Switch at a keyframe.** A change asks the source for one by PLI, through the
   existing per-source PLI gate, so ten viewers switching cost one keyframe.

A step up waits for the estimate to hold for a few seconds, and a step down acts
at once. These are the ladder's own thresholds, applied per stream.
**Transport-cc**, a send-side estimate that reacts faster, is a later phase (§8
Q7). The allocator sits behind the same seam.

### 2.6 What the rewriter has to learn

Forwarding a layer and switching between layers needs more of `StreamRewriter`:

- **VP8.** Rewrite PictureID and TL0PICIDX so they stay continuous across a switch.
  Without this a receiver sees gaps and asks for keyframes. The single-source fast
  path needs it today too.
- **H.264.** Nothing more to rewrite.
- **Header extensions.** Strip the sender's on the way out (its rid would confuse
  the receiver). Write only the ones negotiated with the viewer.
- **NACK.** Keep answering from the viewer's cache for that stream, which is a cache
  per (viewer, stream) now.
- **Ingress cost.** Offer each packet to the viewers forwarding *that layer*. The
  room keeps a map from layer SSRC to viewers, so the per-packet scan of every
  subscriber goes away.

### 2.7 Browsers through the gateway

The gateway already terminates WebRTC and relays each section to the room over
SIP. For a forwarding room:

- **Simulcast from the browser.** The join page sends simulcast
  (`sendEncodings` with three rids). forge-webrtc negotiates `a=rid`/`a=simulcast`
  and the rid extensions. The gateway passes the layers to the room, each under its
  own SSRC, with the rid in the SIP leg's SDP.
- **Streams to the browser.** The browser opens K receive-only video transceivers
  at the start (K is the view's `max_streams`, nine by default). The gateway maps
  each SSRC from the room onto a free transceiver, and tells the page which person
  is on which transceiver through the session's events (`/v1/rooms/{id}`). Because
  the K transceivers are fixed, there is no renegotiation with the browser.
- **The view.** The page posts its view to the gateway, which sends it as the
  `fcp-view` INFO.
- **The page.** It draws its own grid with names, speaking rings and a pin.

### 2.8 Cascade

A peer node's callers reach a forwarding viewer as what the trunk carries:

- **Until R7, one composite tile.** The roster names it as the peer node's group.
- **R7.** The trunk itself becomes a forwarding subscriber of the other node: it asks
  for each of that node's local sources at the layers its own viewers want, and
  re-forwards them. Every node then forwards everyone. The cost is a stream per
  remote source per trunk, priced by the budget like any stream (§3).

### 2.9 Controls

- **Pin and the grid are the viewer's own.** Anvil and the join page build their
  layout from the roster and send a view.
- **A host's spotlight is still room-wide.** The roster marks the spotlighted person
  `fcp:spotlight`, and clients honour it the way the composite does.
- **A host's layout applies to composite subscribers only.**
- **Mute, video off, remove and stop share** are unchanged.

### 2.10 Recording

A recording is the composite, as today (§8 Q9). A room that records composes even
when every viewer forwards. That cost is already priced (`idle_flavors` never idles
a recorded flavor).

## 3. Capacity

- **A forwarded stream** costs packet work and bandwidth: one SRTP protect per
  packet per viewer, and the layer's bitrate leaving the node. It costs no codec
  time.
- **A new egress budget.** The budget gains an egress line in megabits per second,
  set by `[conference] video_egress_mbps` (§8 Q11). `room_cost` prices a forwarding
  viewer at its expected allocation, its requested streams at their layers' nominal
  bitrates. A join is admitted only where both the work units and the egress fit.
- **Measured, not guessed.** Phase R3 measures the packet cost per stream per core
  with `fcp-loadgen`, as earlier phases did for codecs, and writes the numbers here.
- **Meeting size.** `video_max_participants` stays at 16 until the measurements
  say what a forwarding room carries. Raising it is a follow-up with numbers (§8
  Q10).

## 4. How it is proved

- **forge-media.** Unit tests of the rid parsing, the layers, the allocator under a
  scripted REMB, PictureID continuity across a switch, and NACK per stream.
- **FCP.** `fcp_testkit::video::VideoLeg` learns to:
  - send three rids;
  - receive several SSRCs;
  - send an `fcp-view` INFO.

  New `rooms.rs` tests send video end to end, which no room test does today:
  - three phones forwarding, each receiving the other two by the SSRCs its roster
    names;
  - a view change moving a stream between layers on one SSRC;
  - a REMB cut pausing the lowest-priority stream;
  - a SIP phone without `fcp-view` beside them getting the composite;
  - a room with a recording still composing;
  - through the call manager;
  - across two nodes (R7).
- **Gateway.** `fcp-webrtc-gateway/tests/rooms.rs` gains a forge-webrtc peer with
  video that sends simulcast and receives K streams. The Playwright suite joins two
  browsers and checks each draws the other.
- **Anvil.** Three Anvils with `SyntheticVideoHost` in a room on the nightly images
  check:
  - each sends three encodings;
  - each shows the other two, named from the roster;
  - a pin moves a stream to 720p;
  - the CLI's `view` command sends a view.

## 5. Phases

Each phase is one PR in its repository.

- **R1: simulcast in** (forge-media, then FCP).
  - forge-sdp: `a=rid` and `a=simulcast`.
  - forge-rtp: the rid header extensions.
  - forge-conference: a source with layers, each decoded only when a composite
    needs it.
  - forge-webrtc: rid and simulcast negotiation.
  - FCP: the conference server answers simulcast, and the gateway passes a
    browser's layers through.
  - Useful alone: browsers and Anvil cost less to mix.

  In two parts:
  - **R1a, done (v0.25.555, forge-media v2026.11.13).** SIP senders.
    - forge-sdp: `simulcast_layers` and `write_simulcast`.
    - forge-rtp: `RtpPacket::stream_id`, and `keyframe_dimensions` reading
      H.264's SPS and VP8's key frame header.
    - forge-conference: `add_source_layered`. Each layer is bound to its SSRC by
      the rid on its first packets, and only the smallest layer that fills the
      tallest tile is decoded. A move up is at once, a move down after two
      seconds, and a stopped layer is left at once. Heights come from
      keyframes; where none can be read, the layer with the highest bitrate
      stands for the tallest.
    - The conference server takes up to three layers on a participant's camera
      (`SIMULCAST_LAYERS`), and reports `layers` in its video JSON.
    - The call manager's anchoring already carries the lines across the node.
  - **R1b, done (v0.25.556, forge-media v2026.11.13.1).** Browsers.
    - forge-webrtc 0.9.0 asks for simulcast on the camera's section
      (`VideoConfig::simulcast`, the rid extension as id 2). It also takes
      simulcast from a remote that offers it.
    - The gateway asks a browser for `h`, `m` and `l`. It offers the room
      what the browser agreed, under the browser's extension id
      (`VideoOfferSection::simulcast`), so the packets cross unchanged.
      Feedback naming one of the browser's streams is sent to that stream.
    - The join page sizes the encodings at 1, 2 and 4 times down.
    - Chromium agrees as the answerer. It sends two layers for a 640×480
      camera, because it limits layers by input size. The room bound each
      layer and read its size from H.264 keyframes in
      `a_browser_s_camera_reaches_the_room_as_simulcast`, which runs where
      Playwright is installed. Firefox and Safari have not been tried.
- **R2: forwarding in the room** (forge-media).
  - Forwarding subscriptions: a stable SSRC per (viewer, source), and a cache per
    stream.
  - The `speaker` stream.
  - The allocator over REMB.
  - Layer switching at a keyframe, with VP8 PictureID/TL0PICIDX rewriting.
  - Stripping the sender's header extensions.
  - The SSRC-to-viewers map at ingress.
  - Composition only while needed.

  **Done (forge-media v2026.11.13.2, forge-conference 0.16.0, forge-rtp 0.7.3).**
  - `VideoRoom::forward` makes a participant's camera section a viewer;
    `set_view` takes its `ViewRequest`; `VideoRoomEvent::Streams` and
    `forwarded_streams` say what it is sent.
  - The allocator is `video::forward::allocate`, a pure function. A step up
    waits three seconds; a step down or a pause is at once.
  - Ingress offers a packet to the streams carrying its *source*, not a map by
    layer SSRC: a stream is on both people while a speaker switch waits for its
    keyframe, and a sender that changes its SSRC needs no re-mapping.
  - Rooms decode nothing while there is no composite subscriber and no
    recording; a camera is `on` while its packets arrive.
- **R3: forwarding rooms on SIP** (FCP). In two parts.
  - **R3a, done (v0.25.558).** The conference server.
    - `video_mode` on meetings (`mix`, the default, or `auto`; migration 140,
      the API, the conference server's own meeting routes, the admin UI).
    - A camera leg forwards when its INVITE said `Recv-Info: fcp-view` and the
      meeting is `auto`; the 200 OK then says `Recv-Info: fcp-view`. A camera
      section added by a later re-INVITE is sent what the call agreed.
    - The `fcp-view` INFO (`fcp_core::video_view::View`) goes to the room's
      `set_view`: 200, 400 for a view that means nothing, 469 from a call that
      did not ask.
    - Each subscriber's roster is the room's with its own streams laid on: a
      member's `<media id="video">` carries `<src-id>` and `fcp:paused`; the
      member the speaker stream shows has a `<media id="speaker">` with that
      stream's `<src-id>`; a host's spotlight is `fcp:spotlight`.
    - The test kit's video leg sends VP8 or H.264 frames that read as keyframes
      or not, and counts what arrives by SSRC.
      `a_caller_that_asks_is_forwarded_each_persons_stream_and_told_whose_it_is`
      runs the conference server with three forwarding phones and one that is
      not.
  - **R3b.** The call manager carrying `fcp-view` (the `Recv-Info` and the
    INFO) in the call as it carries `fcp-content`, proved for several SSRCs and
    the rid extension; the egress budget and placement; the measurements.

  As designed:
  - `video_mode` on meetings (migration, API, admin UI).
  - `Recv-Info: fcp-view` and the INFO; the roster's `<src-id>`, `fcp:paused` and
    `fcp:spotlight`.
  - The egress budget and placement.
  - `VideoLeg`'s rids, SSRCs and INFO; the `rooms.rs` tests.
  - The measurements.
  - The call manager carrying `fcp-view` INFOs in the call, as it carries
    `fcp-content`, with the relay proved for several SSRCs and the rid extension.
- **A6: Anvil forwards** (anvil).
  - Simulcast send: one capture feeding two or three encoders in each platform host,
    and the send loop sending every encoding under its rid.
  - `src-id` in the roster parser.
  - Names on tiles.
  - The view: the app's grid, a pin, the speaker, sent as `fcp-view`.
  - More than one content stream kept.
  - The three-Anvil test.
- **R4: browsers forward** (FCP).
  - The gateway's K transceivers and their mapping.
  - The view relayed.
  - The join page's own grid.
  - The gateway tests and Playwright.
- **R5: transport-cc** (forge-media, FCP, anvil). A send-side estimator in the room
  and the extension negotiated. Anvil and browsers send its feedback. The allocator
  takes the faster estimate. Its own design note comes first, after R3's numbers.
- **R7: forwarding across nodes** (FCP). Trunks as forwarding subscribers, with
  sources named across nodes in the roster.

The order is R1, R2, R3, then A6 and R4 in either order, then R7 and R5.

## 6. What this does not do

- **Per-participant recording tracks.** A recording stays the composite.
- **SVC.** Temporal and spatial layers inside one encoding (VP9 SVC, AV1 with a
  dependency descriptor) give the same result as simulcast with one encoder. No
  platform host in Anvil makes them today. Simulcast works with every encoder FCP
  and Anvil have.
- **A WebRTC transport in Anvil.** Streams by SSRC over SIP carry everything
  forwarding needs (§8 Q2). ICE for strict networks remains the separate question
  `docs/SOFTPHONE.md` §5 names.
- **End-to-end encryption.** It becomes possible once nothing is mixed, but it
  would end the composite for SIP phones and recordings. It is a project of its
  own.
- **Rooms larger than 16 with video.** Raising the limit follows R3's measurements.

## 7. Risks

- **Mid-switch SSRC/roster race.** A `speaker` stream's owner changes at a keyframe.
  The roster's partial NOTIFY can arrive a little before or after it. The client
  shows the name from the roster as it arrives. A wrong name for a fraction of a
  second is acceptable. A participant stream's SSRC never changes owner.
- **Browsers and several SSRCs in one m-line.** Browsers cannot receive more than
  one SSRC per transceiver. That is why the gateway maps SSRCs onto K transceivers
  (§2.7) rather than passing them through.
- **Hardware encoder sessions.** Simulcast takes two or three encoder sessions per
  camera. Some phones and older GPUs limit sessions. Anvil falls back to fewer
  encodings when a host refuses one, and the room takes what arrives.

## 8. Questions

Each has a recommendation; all twelve were agreed on 2026-11-13.

1. **Forward beside mixing, per subscriber, or a room that only forwards?**
   *Recommendation:* beside mixing. Meetings get `video_mode`: `mix` or `auto`. The
   default is `auto` for new meetings and `mix` for existing ones. Under `auto` a
   SIP phone sees exactly what it sees today. Nobody is forwarded until their
   client asks.
2. **Transport: SIP with streams by SSRC, or WebRTC straight to the conference
   server?** *Recommendation:* SIP, as §2.2 describes. This revisits what
   `docs/SOFTPHONE_VIDEO.md` §7 Q2 expected (agreed 2026-11-08), so here is the
   comparison.
   - **What WebRTC was expected to bring:** a stream per source, rids, mids, and
     renegotiation as people join.
   - **What SIP covers with streams by SSRC in the existing sections:**
     - RFC 8853 rids work in SIP's SDP;
     - SSRCs need no new sections;
     - a fixed SSRC per person needs no renegotiation.
   - **What SIP keeps:**
     - Opus;
     - the call manager's record, policy and recording consent;
     - one media stack in Anvil;
     - every relay already in place.
   - **What WebRTC would still add:** ICE. That is a question for one-to-one calls
     too, and is answered separately.
   - **The alternative.** forge-webrtc inside the conference server, with an HTTP
     join for Anvil and the join page. It is fewer hops for browsers, but it means
     a second signalling path into the room and a WebRTC transport in Anvil.
3. **A stable SSRC per person, or a fixed set of slots that change owner?**
   *Recommendation:* a stable SSRC per (viewer, source). Only the `speaker` stream
   changes owner. Slots exist only at the gateway's edge, where browsers need them.
4. **The forwarding codec.** Forwarding means everyone who forwards shares a codec.
   *Recommendation:* H.264 constrained baseline. It is the one codec every Anvil
   platform encodes in hardware, and every browser has it. A forwarding client that
   cannot send it is a composite subscriber. VP8 forwarding can follow if a room
   asks for it.
5. **How many simulcast layers.** *Recommendation:* up to three for a camera (720p,
   360p, 180p). The client chooses how many; phones send two on battery or a weak
   link. Screens are not simulcast.
6. **Who chooses quality.** *Recommendation:* both. The viewer asks per stream
   (`max_height`, `priority`), and the room's allocator fits that within the
   viewer's REMB.
7. **Bandwidth estimation.** *Recommendation:* REMB first, because every client
   already sends it. Transport-cc is phase R5, with its own design note.
8. **Cascade.** *Recommendation:* a peer node's callers stay one composite tile
   until R7, which forwards across trunks.
9. **Recording.** *Recommendation:* unchanged, the composite. A recorded forwarding
   room pays for one composition.
10. **Room size.** *Recommendation:* keep 16 with video until R3's measurements, then
    decide.
11. **An egress budget.** *Recommendation:* yes, `[conference] video_egress_mbps`.
    It defaults to 1,000 Mb/s, and an operator sets it to the node's link. Placement
    and admission count it beside the work units.
12. **The default view.** *Recommendation:* the speaker at 720p and up to eight
    others at 180p, by who spoke last, for a client that sends no view.
