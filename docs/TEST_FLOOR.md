# The Test Floor

End-to-end tests that drive the FCP service binaries the way phones do:
over real SIP and real RTP, on loopback, in `cargo test`. This document is
the plan (2026-09-13) and tracks what is in place.

## Why

Every binary has a start/health/SIGTERM smoke test, the REST surface has
handler tests against Postgres, and the SIP stack and media engine have
their own suites. What nothing covers is the thing the platform is for:
a phone registers, calls another phone, hears it, hangs up; a caller
joins a room and hears the others. Phase 5 of distributed conferencing
made the gap concrete — the migration round trip could only be checked
by hand, because the repo has no SIP client that talks to a running node.
Every later phase pays the same price until this exists.

## What exists to build on

- `fcp-testkit` starts a binary on free ports with a written config,
  waits on its health route, captures its logs and stops it (`Smoke`);
  makes a throwaway Postgres database (`TestDatabase`); nothing else.
- siphon-rs has the whole client side — `IntegratedUAC` (REGISTER with
  automatic digest re-auth, INVITE with headers and a `CallHandle`,
  re-INVITE, REFER, INFO, BYE), `IntegratedUAS` (answering an INVITE into
  a dialog, `UasRequestHandler` for in-dialog requests, sipfrag NOTIFY) —
  and the documented way to be both at once: a UAC and a UAS on one
  shared `DialogManager`, so a re-INVITE, REFER or BYE from the far end
  finds the dialog the UAC created. What it has no notion of is an
  endpoint: nothing binds a socket, answers, plays or hears audio.
- forge-media has the media side: `RtpSocketPair`, G.711 and G.722,
  RFC 2833 generation and detection, a sine generator. No sender that
  paces packets, no level or tone measurement outside the mixer.
- `tests/manual_test_checklist.csv` lists 170 scenarios an operator was
  meant to run by hand; the floor exists to make most of them run
  themselves.

## The phone

`fcp_testkit::phone::Phone` is a SIP endpoint with a media port, in
process, one per test participant:

- **Transport.** One UDP socket bound to `127.0.0.1:0` — bound first, so
  the port is known before the UAC and UAS are built (siphon-rs refuses a
  contact on port 0). `sip_transport::run_udp` feeds an inbound loop that
  parses, builds the `TransportContext`, hands ACKs to the transaction
  manager and every other request to `IntegratedUAS::dispatch`, responses
  to `receive_response`; sends go through `RealTransportDispatcher`. Each
  phone owns its transaction and dialog managers, so any number run in
  one test process. The process-wide UDP rate limiter is disabled once
  (`RateLimitConfig::disabled()`): several loopback phones share one
  source address.
- **Identity.** A username, a password and the server's realm and
  address; `register()` does the digest exchange and returns the 200,
  `register_expecting(code)` the failure cases.
- **Calls.** `call(target) -> Call` places an INVITE with the phone's SDP
  and reports provisionals and the final; `incoming()` yields an
  `IncomingCall` the test answers (`answer()`), rejects (`busy()`,
  `decline()`) or leaves ringing. Behaviour defaults to answer-on-command
  so a test states what the callee does. A `Call` can `hold()`,
  `resume()`, `refer(target)`, `send_info_dtmf(digit)`, `hangup()`, and
  exposes what arrived on it as events: `Bye`, `ReInvite { sdp }`,
  `Refer { refer_to }`, `Info { body }`, `Notify { body }`. `Phone::events`
  is the stream; `expect(pred, timeout)` waits for one.
- **Media.** An even UDP port pair on loopback from a small probing
  allocator (bind both ports or try the next). The SDP offers PCMU, PCMA,
  G.722 and telephone-event 101; the answer picks the codec and the far
  end. A sender task paces 20 ms packets with running sequence numbers
  and timestamps; `play_tone(hz, ms)` encodes a sine at the negotiated
  codec, `send_dtmf(digit)` sends the RFC 2833 start/continue/end
  packets. A receiver task decodes, feeds a Goertzel bin per frequency
  asked for and the RFC 2833 detector: `heard(hz, within)` resolves when
  that tone's energy dominates for a few consecutive frames,
  `heard_dtmf(within)` when a digit ends, `packets_received()` for the
  plain count. Latching accepts the answered address only.

The phone is the only new machinery. Two smaller helpers go beside it:

- **`fcp_testkit::seed`** — a fixture over `PostgresStorage` and the
  fcp-core store traits: the default tenant, users with
  `sip_ha1_hash = compute_ha1(user, realm, password)` and an extension
  each, a meeting with a PIN. Storage rather than HTTP because the call
  manager has no user-creation route. The realm is the one the config
  gives the registrar; the tenant is the nil one the binaries default to,
  which the registrar falls back to when no SIP domain matches.
- **`fcp_testkit::bins`** — where another crate's binary is: the
  `CARGO_BIN_EXE_*` variables only name a crate's own binaries, and the
  conference tests start the call manager too. `bins::path("fcp-conference",
  "fcp-conference-server")` takes `FCP_E2E_BIN_DIR` when set, else the
  target directory the test executable itself lives under (where
  `cargo test --all-targets`, CI's invocation, has already built every
  binary), else builds it with `cargo build -p … --bin …` once, under a
  lock.
- A Redis helper matching `TestDatabase`: `FCP_TEST_REDIS_URL`, a key
  prefix per test, skipped when unset.

## Where the tests live and when they run

Next to the binary under test, as the smoke tests are:
`crates/fcp-call-manager/tests/calls.rs`,
`crates/fcp-conference-server/tests/rooms.rs`, and later the gateway's.
Each test starts its own processes on free ports and a fresh database,
so tests run in parallel and leave nothing behind. They gate on the same
variables as the suites they join — `FCP_TEST_DATABASE_URL`, and
`FCP_TEST_REDIS_URL` where the cluster is involved — and skip without
them, so `cargo test` stays green on a bare checkout and CI runs them
against its services with no workflow change. Budget: a binary starts
in a few seconds (the migrations are most of it); a scenario is a few
seconds more; a file of a dozen scenarios stays under a minute.

Asserting through the API where the API is the product: the call
manager's `/registrations`, `/calls` and `/cdrs`, the conference
server's `/conferences/{room}`; through the phone where the phone is.

## Phases

One PR each, in order.

| Phase | What | Where |
|---|---|---|
| T1 ✅ v0.25.150 | The phone, the seed, the binary locator. Call manager: register (good and bad password, listed at `/registrations`), a call answered with audio heard both ways, hangup from either side, a CDR at `/cdrs` | `fcp-testkit`, `fcp-call-manager/tests/calls.rs` |
| T2 ✅ v0.25.153 | Call features on the call manager: busy, decline, no answer within the forking timeout, an unregistered callee, hold and resume relayed, RFC 2833 DTMF through anchored media, DTMF INFO relayed, blind transfer (REFER, the new call, the old leg dropped) | `fcp-call-manager/tests/calls.rs` |
| T3 ✅ v0.25.154 | Conference server: a room joined directly, the PIN entered by DTMF (guest and host, three wrong ones hung up), two phones hearing each other through the mixer, mute by DTMF and by API and a kick, the room's participants at `/conferences/{room}`, leave and auto-destroy; the same room reached through the call manager (the room number dialed as an extension, `conference_server_uri`) | `fcp-conference-server/tests/rooms.rs` |
| T4 ✅ v0.25.156 | Two conference nodes with Redis under `cluster.secret` and mutual-TLS trunks from a test CA: placement spreads a room past `conference_node_capacity`, a caller on each node hears the other over the trunk, and phase 5 end to end — SIGTERM one node, the caller through the call manager stays in the room on the other as the same participant, a phone that dialed the node directly follows the REFER itself | `fcp-conference-server/tests/cluster.rs` |
| T5 ✅ v0.25.157 | WebRTC gateway: a forge-webrtc peer standing in for the browser joins a room through the gateway's HTTP API — the offer, its answer, trickled candidates, DTLS-SRTP — and hears a phone in it and is heard; the migration swap under the browser against two real nodes, the browser's peer connection untouched | `fcp-webrtc-gateway/tests/rooms.rs` |

*T1 shipped 2026-09-14 (v0.25.150).* As planned, with three things
learned on the way. A call to a user rings the user's **devices**, and a
device's contacts are the registrations on its SIP URI, so the seed
makes a softphone device on each user's AOR beside the user and the
extension; without it the call manager answers "User has no devices".
The UAS gives a 180 and a 200 different To tags unless the 200 is built
first, so the phone builds the answer at ring time and sends the 180
with the answer's tag. And `tokio::sync::watch` drops a `send` when
nobody is subscribed, which the phone's call phase learnt the hard way
(`send_replace`). The three scenarios — digest registration with its
two failure cases and the listing, a call answered with a tone heard
each way, the call listed, hung up from the caller's side with the CDR
checked, and hung up from the callee's side — run in about two seconds
after the build, each on its own call manager and database. The Redis
helper is deferred to T4, where its needs are known. `Service` joined
the testkit: the smoke harness's start and stop without the one-shot
shape, the process killed on drop so a panicking test leaves nothing
running.

The floor found its first bug on its first CI run, in siphon-rs's
transaction layer: about one INVITE in thirty, the caller got 480 while
the callee was ringing. A callee that answers at once sends 100, 180 and
200 within microseconds; the call manager handles each packet on its own
task and can process the 200 first, whose 2xx transition terminates the
client FSM; the 100's empty action list then reaped the entry from the
table, and the 200's `Deliver` and `GenerateAck`, which looked the entry
up again to find the transaction user, found nothing — the final and
its ACK were silently lost. Fixed in siphon-rs `v2026.09.14`
(sip-transaction 0.7.1: the transaction user and context travel with
the actions), with a regression test that fails on the old code at its
first round. Two phone-side lessons from the same chase: the call task
must await the final concurrently with provisionals, since a non-2xx
leaves the transaction and its provisional stream alive for timer D
(32 s on UDP); and after a 2xx it waits for the transaction to finish
(the ACK to go out) before the call counts as answered, or a re-INVITE
sent at once beats the ACK to the far end and draws a 481.

*T2 shipped 2026-09-14 (v0.25.153).* Six more scenarios in the same
file, and two product bugs found by the first run of them. **RFC 2833
digits never crossed a call.** The call manager anchors media on every
call by default (`anchor_for_dtmf`), and the anchored session's
`enable_relay_rfc2833` defaulted to `false`, so the engine detected the
telephone-event packets and dropped them; tones crossed, digits did not.
The default is now `true`. **A busy or declined callee reached the
caller as 480.** When every B-leg had failed and there was no voicemail,
the B2BUA answered 480 whatever the callee had said; it now answers with
the callee's own final (486, 603, …) when there was one, 480 when the
legs timed out — which is also what a caller hears when nobody picks up
within `[b2bua.forking] timeout`, the test accepting 408 as well. The
phone gained `follow_refers`, what a phone does on a blind transfer:
dial the Refer-To, NOTIFY the outcome, drop the old leg on success; the
transfer scenario watches Carol ring, Alice's new call answer, and Bob's
leg get its BYE. Timing: the nine scenarios run in about four seconds
after the build.

*T3 shipped 2026-09-14 (v0.25.154).* Four scenarios in
`crates/fcp-conference-server/tests/rooms.rs`, all passing on their
first complete run — the mixer, the PIN collector, the DTMF commands,
the control API and the call manager's room routing did what the docs
say. What the phase added to the kit: `Seed::meeting` (a room with
optional guest and host PINs), `http_post`, and a `Stack` that starts
the conference server and, for the routed scenario, a call manager on
the same database with `[b2bua] conference_server_uri` pointing at it
(the call manager first, so one process runs the migrations). Things
learned: the conference server refuses an RTP range under 100 ports;
`GET /conferences/{room}` lists participant ids and the detail (state,
host, caller id) is a second request per participant; a phone with no
display name is known by its caller id; a PIN is digits then `#`, taken
from RFC 2833 as well as INFO and in-band; no join tone plays unless a
sound file is configured, so the tones the tests listen for meet
nothing else in the mix. The four run in about nine seconds after the
build.

*T4 shipped 2026-09-14 (v0.25.156).* Two scenarios in
`crates/fcp-conference-server/tests/cluster.rs`, each starting two
conference nodes and a call manager on one database and a Redis logical
database of the test's own (the URL's `/N`, so membership and the
registry never see another test's nodes — a random base per process
and one index per test, since two tests drawing the same one at random
placed callers on each other's nodes about once in fifteen runs), with
inter-node trunks under mutual TLS from a CA the testkit mints with
rcgen (`fcp_testkit::certs::ClusterCa`). Gated on `FCP_TEST_DATABASE_URL`
and `FCP_TEST_REDIS_URL`. The two ran green only after **four product
bugs**, none of which the unit tests or the earlier by-hand checks had
found:

1. **Placement never spread a room.** A room number dialed as an
   unknown extension took the third routing branch, which read
   `conference_server_uri` directly and never asked the cluster; it now
   goes through `conference_target_host` like the other two. Then the
   rule itself: a hosting node full for the room was excluded from the
   hosting step and re-included in the least-loaded step, where every
   conference node tied on load and the tie fell to node id — the same
   node every time, so `conference_node_capacity` capped nothing. A node
   at capacity for a room is now out of the least-loaded step too (it is
   used only when every live node is full).
2. **Conference nodes advertised no load.** Nothing ever fed the node's
   caller count into membership's `active_calls`, so "least loaded"
   could not tell the nodes apart. The registry hook now carries the
   node's total on every join and leave.
3. **The dialing side of a trunk never heard the answer.** The
   conference server wired the inbound channel into its TCP pool only;
   the 200 OK to a trunk INVITE arrives on the outbound TLS connection
   that carried the INVITE, and was dropped, so the dialer never started
   its media bridge and audio crossed the trunk in neither direction.
   `RealTransportDispatcher::set_inbound_tx` wires both pools; the
   conference server and the call manager now call it.
4. **A caller admitted as the drain began was hung up.** The drain took
   one snapshot of its callers, REFERed them, and hung up whatever was
   left — including a caller whose INVITE was answered just before the
   SIGTERM and whose session landed after the snapshot. The drain now
   takes a second pass for callers admitted since the first.

The phone learnt what a phone does with a `Refer-To`'s URI headers: copy
them into the INVITE as request headers (RFC 3261 §19.1.5), which is how
Carol's migration ticket reaches the new node. The two scenarios take
about six seconds together after the build.

*T5 shipped 2026-09-14 (v0.25.157).* Two scenarios in
`crates/fcp-webrtc-gateway/tests/rooms.rs`. The browser is a forge-webrtc
`PeerConnection` answering with PCMU only, driven the way the join page
drives a real one: `POST /v1/rooms/{room}/join` with a join token
minted against the conference server's `join_secret` (past the room's
PIN), the offer answered on `/answer`, host candidates trickled on
`/ice-candidate`, `GET /v1/rooms/{id}` polled to `connected`; audio in
is `PeerEvent::Rtp` decoded with the testkit's codec and fed to a
`ToneDetector`, audio out is the testkit's `tone_frame` encoded and sent
through `AudioSender`. The first scenario puts the browser in a room
with a phone on a single conference server and hears each from the
other. The second starts two conference nodes and a cluster-aware
gateway with `node_capacity = 1`, joins the browser, dials the other
node from a phone, SIGTERMs the browser's node, and checks that the
gateway's leg is referred, that the participant reappears on the other
node with its `sip_target` changed and its state still `connected`,
that the peer connection never left `Connected`, and that the browser
still hears the phone. The two-node harness moved from T4's test into
`fcp_testkit::cluster` (`ConferenceCluster`, `Node`, `private_redis_url`)
so both crates start the same nodes; the conference server's cluster
test keeps only the call manager and its scenarios. The gateway wants a
media range of 400 ports, so its band sits below the phones' at
20000–28000. One **product bug**, the worst the floor has found: **every
join through the running gateway panicked.** The gateway took the
peer's event channel before creating the offer, and since forge-media
v2026.08.24 (in FCP since v0.25.57, 2026-09-01) the peer brings its
transport, and the channel with it, up on the offer, so `take_events`
returned nothing and the join handler hit an `expect`; the browser saw
an empty reply. No test drove the binary's HTTP join, and the in-process
relay test builds its peers the other way round. The events are now
taken after the offer, as a missing channel is an error rather than a
panic. The two run in about nine seconds after the build.

After each phase the manual checklist's rows that the phase covers get
`Automated` in their Status column and the test name in Notes, so the
checklist shrinks to what still needs a human (TLS phones, NAT, real
softphones, performance).

## Decisions

1. **Our own phone, not SIPp.** siphon-rs ships SIPp scenarios, but SIPp
   is an external binary CI would have to install, its scenarios cannot
   hear audio, and every assertion would be a log grep. A phone written
   on siphon-rs and forge-media tests the stack with the stack, runs
   anywhere `cargo test` does, and is the thing phase 5 was missing.
2. **Loopback and ephemeral ports everywhere.** No fixed port numbers
   (`relay.rs` has some; they are why two of its runs collide). SIP
   ports from `free_port`, RTP pairs from the probing allocator, HTTP
   from `free_port`.
3. **Processes, not in-process handlers.** The binaries are what ship;
   the in-process `fcp-sip` harness stays for what it is good at
   (response compliance, routing decisions) and is not extended.
4. **Skip, never fail, without the services.** Same rule as the rest of
   the suite; the floor is only ever as red as the platform.
5. **Media assertions are about hearing, not bytes.** A tone detected at
   the far end says the whole path works — SDP, ports, codec, direction —
   in one assertion; packet counts are the fallback for cases where the
   path is by design one-way.

## Not in scope

TLS and WebSocket transports for the phone (UDP is what the binaries
negotiate among themselves; TLS is covered by the trunk tests' mutual
TLS), video on the phone (the gateway's relay test covers video
forwarding; a SIP phone with video is a later addition once the audio
floor is in), load or timing measurements, and the admin UI.
