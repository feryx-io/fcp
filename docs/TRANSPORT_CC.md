# Transport-cc for forwarding rooms (R5)

Status: agreed 2026-11-16, every recommendation. R5a done (forge-media
v2026.11.16.3: forge-bwe 0.1.0, forge-rtp 0.8.0); the decision point passed (§5.1).
R5b done (forge-media v2026.11.16.4: forge-conference 0.17.0, forge-webrtc 0.11.0).
R5c1 done (v0.25.575–576).
This is phase R5 of `docs/FORWARDING_ROOMS.md`, which
asks for this note before any code is written (§5, §8 Q7).

## 1. Where we are

A forwarding room spends each viewer's budget with its allocator
(`forge_conference::video::forward::allocate`). The budget is the viewer's
latest REMB (`Viewer::remb_kbps`, 0 meaning no limit). A composite subscriber's
bitrate ladder reads the same REMB (`egress.rs`, `set_remb`). Nothing in the
room estimates anything itself. It believes what the far end says.

Who says it today:

- **Anvil** runs a receive-side estimator shaped like Google Congestion Control
  (`anvil-core/src/video/bwe.rs`, `RembEstimator`, draft-ietf-rmcat-gcc-02).
  - It measures delay as each frame's arrival spacing against its RTP spacing.
    It counts loss on first arrival and moves the rate by AIMD.
  - It sends the result as REMB with each report, and at once when the estimate
    falls below 97% of the last one sent.
  - It is already behind a `BandwidthEstimator` trait. Its doc comment expects
    transport-cc to sit beside it.
- **Browsers** send REMB to the WebRTC gateway, which relays it to the room as the
  viewer's budget (`fcp-webrtc-gateway/src/relay.rs`).
  - forge-webrtc leaves `transport-cc` out of its answers, because it writes no
    transport-wide sequence number (`forge-webrtc/src/sdp.rs`,
    `SUPPORTED_FEEDBACK`).
- **SIP phones** send REMB or nothing. Nothing is the case where the allocator
  has no limit.
- **Trunks** (R7) send nothing. A trunk viewer is limited only by the node's
  egress budget.

forge-rtp parses RTPFB FMT 15 (transport-cc feedback) but keeps its body opaque
(`TransportFeedback::Other`). It writes one-byte header extensions
(`RtpExtension::one_byte`). The forwarding path drops each sender's extensions in
`StreamRewriter::feed` and caches the rewritten packet for NACK
(`Stream::offer`).

## 2. What REMB cannot do

A receiver's estimate has three limits that matter to a forwarding room:

1. **It cannot see above what arrives.** Anvil's estimator holds the rate within
   1.5× of the arriving rate, plus 50 kb/s. A viewer whose streams are paused, or
   held at 180p, receives little. Its REMB therefore grows slowly. An upgrade the
   allocator wants waits for that growth, and then for `STREAM_UP_AFTER` (3 s).
   The sender is the only one who could send extra packets to test the path, but
   REMB gives it no way to learn what those packets showed.
2. **It works in frames, per stream.** Delay is judged once per frame, about every
   33 ms at 30 fps. A viewer receiving nine streams interleaves their frames, so
   the receiver judges nine separate rhythms. A sender sees one clock, its own,
   across every packet it sends to that viewer.
3. **It lives on the far end.** Every client has to run a good estimator: Anvil
   on four platforms, every browser, and whatever a SIP phone does. With
   transport-cc, the receiver only reports arrival times. The estimate is made
   where the allocator runs, by one implementation that FCP tests.

Transport-cc (draft-holmer-rmcat-transport-wide-cc-extensions-01) addresses all
three:

- The sender numbers every packet on a transport with one sequence (header
  extension `http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01`).
- The receiver reports when each packet arrived, or that it did not (RTPFB FMT
  15), about every 100 ms.
- The sender pairs those reports with its own send times. It then runs the same
  delay, loss and AIMD controllers per packet group.
- The sender can probe: it sends padding in a short burst, reads the arrival
  times back, and learns whether the path has room before committing a layer to
  it.

What it costs:

- A header extension on every forwarded packet.
- A send-time record per packet for about a second.
- A feedback packet from each receiver about ten times a second.
- Code in forge-media, the gateway and Anvil.

Whether it is faster in practice is not known yet. Phase R5a settles that by
simulation before anything is wired in (§5).

## 3. The model

### 3.1 One estimator crate: `forge-bwe`

A new forge-media crate holds the congestion controller once:

- **The GCC core.**
  - The trendline delay detector with its adaptive threshold.
  - The loss controller.
  - The AIMD rate controller.
  - These move from Anvil's `bwe.rs` with its tests, generalised from frames to
    packet groups (GCC §5.2: packets sent within 5 ms of each other form one
    group).
- **`RembEstimator`.** The receive side, as Anvil runs it today, fed per packet
  and answering with a REMB value.
- **`FeedbackWriter`.** The receive side of transport-cc.
  - It records each packet's transport-wide sequence number and arrival time.
  - It builds the FMT 15 packet: base sequence, status chunks (run-length and
    status vectors), and receive deltas in 250 µs units.
  - It sends every 100 ms, or sooner once it has 100 packets to report.
- **`SendSideEstimator`.** The send side of transport-cc.
  - It keeps a send history (sequence, send time, size, and whether the packet
    was a probe) for 1 s.
  - It reads feedback into send/arrival delta pairs and runs the GCC core.
  - It includes the acknowledged rate in its output, so an estimate never runs
    far above what actually got through.
- **`Prober`.**
  - Probes are bursts of padding-only RTP packets (RFC 3550 §5.1).
  - It sends a cluster at a target rate, at least 5 packets and 15 ms long.
  - It reads the cluster's arrival rate from the feedback.
- **`testing::Bottleneck`.** A simulated link on a virtual clock: a rate, a
  queue, a propagation delay, and loss when the queue overflows. The estimators'
  tests, and the comparison in R5a, run through it without real time or sockets.

forge-rtp gains a typed `TransportFeedback::TransportCc`, which parses and
writes FMT 15, and a helper that writes the transport-wide sequence extension
into a packet that is already built.

### 3.2 The room

Each viewer, and each composite subscriber, whose section negotiated
transport-cc gets a transport-wide counter, a `SendSideEstimator` and a `Prober`.

- **Stamping.**
  - The sequence number is written as the packet leaves for that viewer: after
    the rewrite, and after the NACK cache, which keeps the packet without it.
  - A retransmission gets a fresh number, as the draft requires.
  - Without BUNDLE, a SIP section is its own transport, so the counter belongs to
    the viewer's camera section. The content section, when it forwards, gets its
    own counter.
- **Feedback.** FMT 15 from the viewer goes to its estimator, in the same RTCP
  match that handles NACK and REMB in `room.rs`.
- **The budget.**
  - `Viewer::budget_kbps` uses the send-side estimate while feedback is fresh,
    meaning some arrived in the last second.
  - Otherwise it uses the REMB.
  - It is never the smaller of the two. A client that negotiates both can let
    its REMB go stale, as Chrome does.
  - The composite ladder reads the same budget.
- **Probing.**
  - A probe runs only when the allocator holds something back that the budget
    nearly covers: a paused stream, or a layer below what the view asks for.
    This is application-limited probing in GCC's terms.
  - It probes at the rate the next upgrade needs, at most every 2 s.
  - Padding goes on the viewer's tallest active stream, so it costs no new SSRC
    and no renegotiation.
  - When the estimate already covers the upgrade, the upgrade still waits for
    `STREAM_UP_AFTER`. A probe that proves the path can halve that wait.
- **Feedback for what the room receives.**
  - A source whose section negotiated transport-cc gets a `FeedbackWriter` in the
    room.
  - Anvil and browsers can then estimate their own uplink, the simulcast they
    send. Today nothing tells them anything about it.
  - The writer is the same code the clients use, and costs a packet every 100 ms
    per source.

### 3.3 SDP

The conference server answers transport-cc only when it is offered: the extmap
URI and `a=rtcp-fb:<pt> transport-cc` on the camera (and content) section. A SIP
phone that does not offer it goes on with REMB.

- **The trunk does not negotiate it.** Both ends are inside the cluster's
  network, and the egress budget already prices the trunk (§2.8 of the
  forwarding design).
- **The call manager** relays video packet for packet with RTCP passed through
  (`fcp_media::VideoRelay`). It re-encrypts per leg and keeps the RTP header, so
  the extension and the FMT 15 feedback cross unchanged. R5c proves this rather
  than assuming it.

### 3.4 The gateway

The gateway ends the browser's transport: one BUNDLE carrying audio and K video
transceivers. A transport-wide sequence is numbered by whoever sends on a
transport, so the room's numbers cannot pass through as they are. Instead, the
gateway **translates**: it numbers each hop itself and turns each side's feedback
into the other side's numbers. That makes the room the sender for the whole
path, as an SFU that ends a browser's connection is (Jitsi Videobridge, Zoom's
and Webex's media servers). The room estimates from the room all the way to the
browser, and its probes (§3.2) reach the browser.

This revises question 5 (§6), agreed 2026-11-16 as "end it and tell the room by
REMB". Building it showed that a gateway estimate without probing repeats §5.1's
defect: held near what reaches the browser, a browser viewer stays at 180p after
a dip. The gateway cannot probe on its own, because the browser's stream
sequence numbers are the room's. The revision was agreed the same day.

**Toward the browser (downlink):**
- The gateway's SIP section to the room negotiates transport-cc, so the room
  numbers what it sends there.
- The gateway numbers each video packet it sends the browser on the BUNDLE, and
  remembers which room number each of its own numbers carried.
- The browser's FMT 15 is rewritten into the room's numbers and sent to the room
  on that SIP section:
  - each packet keeps the arrival time the browser reported;
  - a room number that never reached the gateway is reported lost;
  - packets the gateway itself sent (none carry video today) are left out.
- The room's padding probes cross like any other packet of the stream.

**From the browser (uplink):**
- The browser numbers what it sends on the BUNDLE: audio, the camera's layers
  and the screen share in one sequence.
- The gateway numbers each video packet again on the SIP section it goes to the
  room by, so the room sees one gapless sequence per section.
- It turns the room's feedback back into the browser's numbers.
- Chrome then estimates its uplink from what the room reported, and fits its
  simulcast to it.

forge-webrtc keeps `transport-cc` in its answers and offers the extension under
a fixed id beside `sdes:mid` and the rid extension (R5b). The translator is the
gateway's (`fcp-webrtc-gateway`); whatever forge-webrtc needs so that a packet's
extension can be read and written on the way through comes with R5c2.

### 3.5 Anvil

- Anvil takes `forge-bwe` and deletes its own `bwe.rs`. Its `RembEstimator`
  behaves exactly as before, from the same code.
- It offers transport-cc beside goog-remb.
  - When the answer keeps transport-cc, Anvil writes feedback for what it
    receives.
  - For what it sends, it stamps each packet and estimates from the room's
    feedback. That estimate replaces the room's silence about its uplink.
  - When the answer drops transport-cc, everything is as today.

## 4. What this does not do

- **RFC 8888 congestion control feedback.** Browsers have it only behind a flag,
  and transport-cc is what Chrome, Firefox and Safari send. `forge-bwe` keeps the
  feedback format apart from the estimator, so RFC 8888 can be added later
  without touching the controller.
- **RTX for probes.** Chrome probes with padding on an RTX stream. FCP's
  retransmissions reuse the media SSRC, so there is no RTX stream to pad on.
  Padding on the media SSRC is valid RTP, and every decoder drops it.
- **Pacing every packet.** A forwarded frame leaves in one burst, as it does
  today. The estimator groups packets by send time and does not need a pacer.
  Probes are the only packets paced.
- **Trunk estimation.** See §3.3.

## 5. Phases

- **R5a: `forge-bwe` (forge-media).**
  - The crate as §3.1 describes, the typed FMT 15 in forge-rtp, and the
    simulation.
  - The estimators run through `Bottleneck` against these scenarios:
    - a step from 2.5 Mb/s to 600 kb/s;
    - a step back up;
    - 2% random loss;
    - a competing flow.
  - The measures for each run are time to an estimate under capacity, queue delay
    at its peak, and time to reach 90% of a raised capacity. REMB is measured with
    Anvil's cadence, and transport-cc with a 100 ms feedback interval and
    probing.
  - The numbers go in this document. **Decision point:** if transport-cc is not
    clearly better (at least twice as fast down, or a raised capacity found in
    under half the time), R5 stops here. Anvil still takes the crate in R5d, so
    the receive side lives in one place.
- **R5b: the room (forge-media).**
  - Stamping, the send-side estimator per viewer and composite subscriber, and
    feedback read in `room.rs`.
  - The budget rule from §3.2, probing, and a `FeedbackWriter` per source.
  - forge-webrtc offering and keeping transport-cc.
  - Tests in forge-conference: a viewer behind a `Bottleneck` paused and resumed
    by feedback alone, and a probe raising the estimate.
- **R5c: FCP**, in two parts.
  - **R5c1: the conference server and the test kit.**
    - fcp-media's answer keeps transport-cc when the section offers both its
      extension and `a=rtcp-fb:<pt> transport-cc`, under the offer's id
      (`VideoAnswer::transport_cc`, `NegotiatedVideo::transport_cc`).
    - The conference server answers it on every camera and content section and
      wires the id into the room:
      - a forwarding viewer's `ForwardRequest`;
      - the composite subscriber's numbering;
      - the sender's own feedback.
    - The trunk keeps it off.
    - The video JSON's `forwarded` says `estimate` (`transport_cc`, `remb` or
      `none`), `budget_kbps` and `transport_cc_kbps`.
    - The test kit's phone offers it (`Phone::set_video_transport_cc`) and
      writes feedback.
      - Its receive side can be shaped (`VideoLeg::shape`): each arriving
        packet passes forge-bwe's `Bottleneck`, which drops it or dates its
        arrival.
      - That is all a sender's estimate reads, and it is deterministic, so it
        stands in for the UDP relay first planned here.
    - `rooms.rs`'s
      `a_viewer_s_feedback_alone_pauses_a_stream_on_a_narrow_link_and_probes_bring_it_back`:
      - A viewer with no REMB at all is forwarded two people of about
        300 kb/s each.
      - Narrowed to 400 kb/s, the second priority is paused at an estimate of
        about 345 kb/s.
      - Opened again, probes bring it back, in about 12 s in all.
  - **R5c2: the gateway, the call manager and the browser.**
    - The call manager relay proved to carry the extension and FMT 15.
    - The gateway's translator, both ways (§3.4).
    - The gateway's SIP section negotiating transport-cc with the room.
    - Playwright: Chromium negotiates transport-cc through the gateway, and the
      room's estimate for the browser follows the browser's feedback.
- **R5d: Anvil.** `forge-bwe` in place of `bwe.rs`, transport-cc offered, feedback
  written, and the uplink estimated from the room's feedback, proved against the
  stack.

R5a comes first. R5b and R5c follow only if the decision point passes. R5d
follows R5a either way.

### 5.1 What R5a found

`forge-bwe/tests/simulation.rs` models one viewer of a forwarding room:

- Three people, each at 150, 500 or 1,500 kb/s, so seven steps from 485 kb/s
  to 4.7 Mb/s on the wire.
- The allocator's rule: down at once, up only once the budget has covered the
  next step for 3 s.
- A link with a 64 KB drop-tail queue and a 25 ms one-way delay.
- REMB runs at Anvil's cadence: an estimate every 200 ms, sent each second or
  at once on a 3% fall.
- Transport-cc writes feedback every 100 ms. It probes at 1.25× the next step
  when the budget covers half of it. A proven probe halves the 3 s wait.

| Scenario | Measure | REMB | Transport-cc |
|---|---|---|---|
| 2.5 → 0.6 Mb/s | sending under capacity | 1.1 s | 0.6 s |
| | peak queue delay | 873 ms | 666 ms |
| 0.6 → 2.5 Mb/s | estimate at 90% | never | 11.9 s |
| | sending 1.5 Mb/s | never | 9.2 s |
| 2.5 Mb/s, 2% loss | mean sending | 1,567 kb/s | 1,568 kb/s |
| 2.5 Mb/s, steady (10–30 s) | step changes | 0 | 0 |
| a 1 Mb/s flow joins | sending under 1.5 Mb/s | 1.5 s | 0.6 s |
| | mean sending after | 1,258 kb/s | 1,218 kb/s |

**After a dip, a REMB viewer never recovers.**
- Its estimate is held to 1.5× what arrives plus 50 kb/s. That is 755 kb/s
  while three 180p streams arrive, and the next step needs 800 kb/s.
- So a REMB viewer that falls to 180p stays there for the rest of the
  meeting, however much the path frees up.
- With one person it is worse still: 150 kb/s to 500 kb/s is more than 3×.
- This is §2's first limit, and it is a defect of forwarding rooms today, not
  only a slower recovery.

**Transport-cc's probes find the raised capacity in about 9 s.** Going down, it
is 1.8× as fast with a shallower queue. That misses the first margin (2×) and
passes the second, so **R5 goes on.**

Three things in the core came out of the measurement, and are in R5a:

- **Repeated decreases.** While overuse lasts, the rate comes down again every
  300 ms rather than once per episode. Without this, both estimators waited
  for loss on a falling link.
- **Probes stay out of the delay trend.** A probe is meant to fill the path
  for a moment, so its own queue is its result and not overuse. Before this
  change, each probe cut the rate it was testing, and the room stepped up and
  down every few seconds.
- **Backoff.** A probe that falls short of its target waits twice as long
  before trying that target again, up to 30 s.

Steps are priced at their wire rate, headers included. That is what an
estimator measures and what the room's measured layer rates are. A step
priced without headers fills a link exactly at its nominal rate.

## 6. Questions

Each has a recommendation; all nine were agreed on 2026-11-16.

1. **Transport-cc or RFC 8888 feedback?** *Recommendation:* transport-cc. Every
   browser sends it, and RFC 8888 is behind a flag in Chrome. The crate keeps the
   format separate, so RFC 8888 can follow.
2. **Where the estimator lives.** *Recommendation:* one crate, `forge-bwe`, in
   forge-media, used by the room, the gateway, Anvil and the test kit. Anvil's
   estimator moves there with its tests.
3. **Which estimate the allocator spends.** *Recommendation:* the send-side
   estimate while its feedback is fresh (some within 1 s), the REMB otherwise.
   Not the minimum, because a client that sends both may let one go stale.
4. **Probing.** *Recommendation:* yes, but only when something is held back that
   the budget nearly covers. Use padding-only packets on the viewer's tallest
   active stream, at most every 2 s.
5. **The gateway: end the browser's transport-cc or pass it through?**
   *Recommendation:* end it. The gateway numbers the browser's BUNDLE, so it must
   estimate that hop itself. It tells the room by REMB, as it does today.
   **Revised 2026-11-16: translate.** The gateway numbers each hop and rewrites
   each side's feedback into the other's numbers, so the room estimates and
   probes the whole path to a browser (§3.4). A gateway estimate without probes
   would repeat §5.1's defect for every browser.
6. **The uplink: should the room and the gateway write feedback for what they
   receive?** *Recommendation:* yes. It is the same writer at the cost of a
   packet every 100 ms per source, and it is the first estimate a client gets of
   its own simulcast's path.
7. **Trunks.** *Recommendation:* no transport-cc on the trunk. The egress budget
   prices it, and both ends share the cluster's network.
8. **A decision point after R5a.** *Recommendation:* yes, with the margins in §5.
   If simulation does not show transport-cc clearly faster, R5 ends with
   `forge-bwe` and Anvil using it, and the room keeps REMB.
9. **Feedback interval.** *Recommendation:* 100 ms, or sooner at 100 packets. That
   is Chrome's default and about 1% of a 1 Mb/s downlink.
