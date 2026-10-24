# Phones on any network: a port per leg, and TLS by default

Designed 2026-10-24, from D8 session 3 (`docs/DEEP_TESTING.md` D8). A
phone at home sits behind whatever router its owner bought. Some rewrite
SIP (an ALG), most do not; several phones often share one public
address. FCP has to give two-way audio in all of these, and today it does
not:

- **ALG on.** Anvil 0.2.2 and a Poly VVX 250 behind one router, the
  router rewriting both phones' Contact and SDP to its public address.
  The VVX heard Anvil; Anvil heard nothing, though the node sent valid
  Opus to exactly the address and port Anvil sent from. Not explained
  from the server's side: Anvil sends no RTCP and keeps no log.
- **ALG off.** The same two phones, now naming their LAN addresses.
  Both legs of the call learn their peer from the first packet from the
  shared public address (v0.25.457, `fcp_sip::nat`); the VVX's packet
  came first and was taken as leg A's, Anvil's as leg B's. Relaying
  still reached the right phones, but transcoding ran backwards — the
  VVX's G.711 decoded as Opus (`Opus decoding failed`), passed through
  raw, dropped by Anvil — so neither side heard the other.

Both come from one design choice: forge-engine gives a call **one** RTP
port for both legs (the A-leg answer and the B-leg offer both said
`34.16.92.157:10896`), so it must tell a packet's leg from its source
address (`forwarding.rs` `determine_packet_sides`), and two phones on one
address defeat that. The NAT test (`calls.rs`
`phones_behind_nat_ring_hear_each_other_and_hang_up`) puts its phones on
127.0.0.2 and 127.0.0.3, so it never met the shared address.

## 1. What other systems do

Every hosted PBX and SBC anchors media and latches — sends each leg's
media back to where that leg's packets come from (symmetric RTP,
RFC 4961): FreeSWITCH's `rtp-auto-adjust`, Asterisk's
`rtp_symmetric`/`force_rport`/`rewrite_contact`, Kamailio's nathelper
with rtpengine. FCP's latch is the same idea. What they all do and FCP
does not: **a port per leg**. rtpengine, FreeSWITCH and Asterisk know a
packet's leg from the port it arrived on; the source address only says
where to answer and which strangers to refuse. A shared public address
is then nothing special.

Zoom Phone, Webex Calling and their peers latch the same way and add
**SIP over TLS** (Webex Calling requires TLS and SRTP for devices; Zoom
provisions them): an ALG cannot read, so cannot rewrite, encrypted
signalling. That is how they work whatever the router; their support
pages still say "turn off SIP ALG". Their own apps use ICE against their
media servers, with TURN on 443 for networks that block UDP.

STUN tells a client its public address; a server that latches does not
need it, and it fails behind symmetric NAT. TURN relays media between
endpoints that cannot reach each other; a server that anchors media at a
public address already is that relay for SIP phones. ICE makes each leg's
packets identifiable by credentials (FCP's WebRTC gateway does it); desk
phones rarely support it. None of the three is needed for this.

## 2. What FCP has, and what it lacks

### 2.1 Media ports

- `allocate_and_bind` (`forge-engine/src/session.rs:3744`) takes one
  port pair per session; one forwarding task (`forwarding.rs:65`)
  receives RTP and RTCP on it. Every send to a leg goes through five
  calls on that one socket pair: relayed RTP (`:648`), generated RTP —
  hold music, playback, TTS, the media bridge, DTMF, the supervisor's mix,
  all through scheduled playout (`:1284`) —, generated sender reports
  (`:963`), forwarded RTCP (`:1604`) and DTLS replies (`:1790`).
- The leg is guessed from the source in `determine_packet_sides`
  (`:977`), the SRTP context for an unknown source defaults to leg A's
  (`:134`) — a first packet from an encrypted B leg is dropped and B never
  learns its address —, RTCP's leg is guessed from IP and port ± 1
  (`:188`), DTLS defaults to A (`:1758`), and report blocks assume A's
  clock (`:1830`).
- Per-leg state already exists for everything else: participants with
  their latch allowlists, SRTP and DTLS contexts, codec runtimes, playout
  queues, generated-stream state, relay suppression. The supervisor's
  third party (`party.rs`) already has a port of its own — the pattern.
- Above forge-engine, fcp-media's session holds one `(rtp_port,
  rtcp_port)` (`fcp-media/src/session.rs:38`) and fcp-sip reads it at
  about twenty places without saying which leg — the B-leg offer
  (`handler.rs:7370`), the answers to A (`:8449`, `:9304`), re-INVITEs
  both ways (`:10428`, `:10576`), local hold (`:10758`), pickup, takeover,
  inline flows, find-me, CFNA, announcements, conference migration,
  transfer (`transfer.rs:211`, `:337`) and the flow `Dial` node
  (`flow_handler.rs:1222`).
- `move_peer` (park, `backend.rs:908`) moves a phone from one leg slot to
  the other; with a port per leg the phone would still be sending to the
  old leg's port.
- XDP rules (`session.rs:1953`, `:2051`) and HA state (`to_state`,
  `from_state`) hold one pair; forge-api's `SessionResponse` has one
  `rtp_port` and forge-sip-server points both legs at it.
- Capacity: `[media] rtp_port_start`/`end`, 10000–20000 by default (5000
  pairs); compose and D8 10000–10999 (499 pairs) with
  `max_concurrent_calls = 100`. Queue legs and supervisors share the pool.

### 2.2 Signalling over TLS

- The call manager listens for SIPS on `[sip] sips_bind` (5061) when
  `tls_cert_path`/`tls_key_path` are set; on D8 with the Let's Encrypt
  certificate for d8.feryx.io (`deploy/gcp-d8/install-certs.sh`). A
  renewed certificate needs a restart, which drops every TLS phone's
  connection.
- A REGISTER over TLS gets an RFC 5626 flow; INVITEs to the device, BYE
  and re-INVITE toward a B leg and resync NOTIFYs go down it, and such a
  device is offered SDES-SRTP on anchored calls (`secure_device_offer`).
  Proved on localhost only (`srtp.rs`, `baresip.rs`, Anvil's
  `anvil_over_tls_is_called_encrypted`, which never runs in CI).
- What does not use the flow: requests FCP starts toward a TLS **caller**
  (`a_leg.flow_token` is set only for site links, `handler.rs:3753`) —
  the BYE when the callee hangs up, a hold re-INVITE, REFER's NOTIFYs —;
  MWI, presence, BLF and shared-appearance NOTIFYs (`mwi.rs:120` and the
  rest never look at flows); TCP registrations, which get no flow at all.
- Connection lifetime: siphon-rs never answers a client's CRLF ping
  (RFC 5626 §4.4.1), FCP never turns on server pings, and an idle inbound
  connection is dropped after 1800 s — a phone registering for 3600 s
  without keepalives loses its flow halfway.
- The B leg's media latch for a flow target uses the Contact's host
  (`handler.rs:9277`), a LAN address behind NAT, so a TLS phone behind NAT
  may be sent RTP at its private address.
- What a softphone is told (`/me/softphone`): `registrar = sip:<domain>`
  and an outbound proxy naming the transport only when the server is not
  the domain. On D8 they are the same, so the transport survives only in
  a `transport` string Anvil reads for nothing but building a TLS client
  config (`anvil-core/src/lib.rs:106`): **Anvil told `tls` registers over
  UDP.** `[api.provisioning] sip_transport` is global and defaults to
  UDP; D8's `setup.sh` never sets it.
- Anvil's TLS connections are swept after 300 s without an outbound
  request (siphon-rs `sip-transport/src/pool.rs:65`, only sends count),
  so its flow dies five minutes after its last REGISTER or call.
- Templates switch transport from the one global setting; only Yealink's
  sets SRTP; none pushes a CA or sets TLS keepalives. No real phone has
  registered over TLS yet; whether VVX 6.4.7 trusts ISRG Root X1 is
  unknown.
- FCP answers an SDES offer over UDP with a key of its own, in clear text.

## 3. The model

### 3.1 A port per leg

A media session holds a socket pair per leg. Leg A's peer is told leg A's
port, leg B's peer leg B's. A packet's leg is the socket it arrived on;
its source only says where that leg answers and whether the leg accepts
it (the leg's latch: pinned, learning from a named address, or learning
from anywhere). `determine_packet_sides` and its heuristics go; so do the
SRTP, RTCP and DTLS guesses. Everything sent to a leg leaves from that
leg's socket — which is also what a port-restricted or symmetric NAT
needs: a phone hears only from the address and port it sent to.

- **forge-engine.** `MediaSession` allocates two pairs (both or neither,
  with `min_free_port_pairs` applied to the pair of them); the
  forwarding task receives on four sockets; `ports()` becomes
  `ports(leg)`; teardown, `Drop`, XDP keys (A's port in, B's port out and
  back) and HA state carry two pairs. The supervisor's party is
  unchanged.
- **Moving a phone between legs** (park, takeover): the socket moves with
  the phone — `swap_legs`/`move_peer` exchange the legs' sockets along
  with their peers, keys and latch, so the parked phone keeps sending to,
  and hearing from, the port it always had. No re-INVITE.
- **Forked B legs** share leg B's socket, as they share the offer today;
  the winner's address is set when it answers.
- **forge-api** keeps `rtp_port` as leg A's and adds `rtp_port_a`,
  `rtp_port_b`; forge-sip-server uses them.
- **fcp-media** exposes `ports(Leg)`; every fcp-sip call site names the
  leg it is advertising to (the table in §2.1). `list_sessions` reports
  both.
- **The transcoder** rescales the RTP timestamp between the two codecs'
  clocks (8 kHz G.711 and G.722, 48 kHz Opus), a separate bug found in
  the same calls: today a re-encoded packet keeps its source clock.

### 3.2 TLS by default

- **Flows everywhere a TLS phone is reached.** A caller's flow is found
  from its registration (or the connection the INVITE came on) and kept
  on `a_leg.flow_token`, so a callee's BYE, a hold re-INVITE and REFER's
  NOTIFYs go down it. MWI, presence, BLF and shared-appearance NOTIFYs to
  a binding with a flow go down the flow. TCP registrations get flows
  too.
- **Connections that last.** siphon-rs answers a CRLF ping with a pong;
  the call manager sets the established idle timeout above the longest
  registration it grants and turns on server keepalives
  (`set_stream_keepalive_interval`) for registered flows; certificates
  reload without a restart (`run_tls_with_swappable_config`).
- **The media latch for a flow target** learns from the flow's peer
  address, not the Contact's host.
- **What a softphone is told.** `/me/softphone` always names the
  transport in a URI Anvil dials (`outbound_proxy =
  sip:<server>:<port>;transport=<t>`, the server being the domain when
  nothing else is set), and carries the CA (PEM) when the server's
  certificate is not publicly trusted (the compose stack's own CA).
  The default transport is `tls` when the call manager has a certificate.
- **Desk phones.** Templates set TLS, SRTP and a TLS keepalive per vendor
  when the transport is TLS, and install a CA when one is configured.
  D8's `setup.sh` sets `sip_transport = "tls"`.
- **SRTP.** Offered to every TLS device, as now; required by Anvil over
  TLS; FCP no longer answers SDES offered over UDP with a key in clear
  text — it answers plain RTP.

### 3.3 Anvil

- The account's transport applies when its URIs name none.
- The registrar's TLS connection is kept open while registered (pings,
  no idle sweep), and Anvil re-registers at once when it closes.
- Receive counters in its log and call stats: packets received, packets
  dropped for an unexpected payload type or a failed decrypt. The
  ALG-on call above is unexplained because nothing on the phone counted.

## 4. How it is proved

A call manager and two phones **behind one address** — both bound to the
same loopback address — in every shape a home router makes:

| Shape | Phones name | Codecs | Checks |
|---|---|---|---|
| no ALG | an address they cannot be reached at | Opus / G.711 | both hear each other; hold music reaches the held phone only; park, retrieve |
| ALG | the shared address and their own ports | Opus / G.711 | the same |
| TLS, no ALG | their LAN address, over TLS | Opus / G.711, SRTP | the same; the callee hangs up and the caller gets the BYE; MWI arrives |
| TLS, idle | as above | — | called after 35 minutes idle |

The Opus phone is the baresip harness (`fcp_testkit::baresip`), the G.711
one the testkit phone. Anvil's own suite adds Anvil (Opus) calling a G.711
phone through FCP and hearing it, Anvil over TLS idle past five minutes
and then called, and Anvil over TLS as the caller receiving the BYE; CI
gets `ANVIL_FCP_SIPS` and `ANVIL_FCP_CA`. forge-engine's own tests cover
two peers on one address on two ports, SRTP on a B leg first, RTCP per
leg and the timestamp rescale. Then D8 by hand: Anvil and the VVX with the
ALG on and off, and the VVX over TLS.

## 5. Phases

### Phase N1: a port per leg (forge-media)
- Two socket pairs per `MediaSession`; receive on four sockets; send each
  leg's media from its own socket; SRTP, RTCP and DTLS by arrival socket;
  latch per leg; `determine_packet_sides` removed.
- `ports(leg)`, leg swap with sockets, XDP and HA with two pairs,
  forge-api's `rtp_port_a`/`rtp_port_b`, forge-sip-server.
- The transcoder's timestamp rescale.
- Tagged release.

### Phase N2: a port per leg (fcp)
- fcp-media's `ports(Leg)`; every fcp-sip site names its leg; park and
  takeover swap sockets with peers; `list_sessions` with both ports.
- Two pairs per anchored call in the port budget: admission counts
  pairs, the docs say so, compose's range checked against
  `max_concurrent_calls`.
- The shared-address tests (§4, the first two rows).

### Phase T1: TLS that lasts (siphon-rs, fcp)
- siphon-rs: the pong; whatever the call manager needs to set idle
  timeouts and keepalives per listener.
- The caller's flow on `a_leg.flow_token`; NOTIFYs down flows; flows for
  TCP; the flow target's media latch; certificate reload.
- Tests: the TLS rows of §4.

### Phase T2: TLS by default (fcp)
- `/me/softphone`'s transport-bearing outbound proxy and CA; `tls` the
  default with a certificate; SDES over UDP answered plain.
- Templates: TLS, SRTP, keepalive and CA per vendor.
- D8 `setup.sh`; `docs/PROVISIONING.md`, `docs/SINGLE-NODE-SETUP.md` and
  the compose README.

### Phase A1: Anvil (anvil, siphon-rs)
- Transport applied to URIs; the registrar's connection kept and
  re-registered on close; SRTP required over TLS; receive counters.
- The Anvil tests of §4 and their CI wiring. Release.

### Then: D8 by hand
Anvil and the VVX, ALG on and off, UDP and TLS.

## 6. What this does not do

- ICE, STUN or TURN for SIP phones (§1). TURN over 443 for Anvil on a
  network that blocks UDP is a later project.
- A per-device or per-tenant transport: one default, templates and
  `/me/softphone` following it.
- Anchoring early media or UPDATE's SDP (both pass through today; seen
  in the survey, separate).

## 7. Questions

1. **Park and takeover.** *Recommendation:* the sockets move with the
   phone, so it keeps the port it was told; a re-INVITE to a new port is
   a request a parked phone may refuse or race.
2. **Forked B legs.** *Recommendation:* one socket for all of them, as
   one offer today; a socket per fork costs a pair per ringing device for
   nothing once one answers.
3. **Port budget.** *Recommendation:* keep the defaults (2500 calls on
   10000–20000) and compose's 10000–10999 (about 249 calls, above its
   `max_concurrent_calls = 100`); admission counts pairs so an exhausted
   pool refuses a call rather than half-anchoring it.
4. **Which transport by default.** *Recommendation:* TLS whenever the
   call manager has a certificate, for Anvil and the templates alike; an
   operator sets `udp` for a fleet that cannot do TLS.
5. **SRTP.** *Recommendation:* required by Anvil over TLS, offered to
   desk phones over TLS, never keyed over UDP. An old phone that insists
   on SRTP over UDP gets plain RTP or a 488.
6. **VVX trust of Let's Encrypt.** *Recommendation:* try it on the D8
   VVX in phase T2 before deciding anything; if 6.4.7 does not trust
   ISRG Root X1, the Polycom template installs the CA.
7. **forge-api.** *Recommendation:* `rtp_port` stays leg A's and
   `rtp_port_a`/`rtp_port_b` are added, so a client that reads one port
   keeps working for the leg it always meant.
8. **Order.** *Recommendation:* N1 and N2 first — they fix what D8 found,
   on UDP, for every phone — then T1, T2 and A1. Anvil's receive counters
   go into a 0.2.3 right away, so the ALG-on case can be diagnosed on the
   next D8 call.
