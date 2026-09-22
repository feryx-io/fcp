# Edge Security — Design

Status: **agreed** (2026-09-21), survey done, Q1 and Q3 settled by the
user; **phase 1 done** (v0.25.222), **phase 2a done** (v0.25.223),
**phase 2b done** (v0.25.224), **phase 3 done** (v0.25.225), **phase 4
done** (v0.25.226), **phase 5a done** (v0.25.227). Chosen after the
capacity and survivability project closed.

One document for what an internet-facing FCP node exposes: **who may place
a call**, **what a peer may claim**, **what a flood costs**, and **what is
actually verified** where the code says "trusted".

Read with `docs/TENANT_TRUNKS.md` (§7 trunk admission), `docs/E911_BLOCK_2.md`
(§4, §6.2 the trusted-edge headers), `docs/CAPACITY_AND_SURVIVABILITY.md`
(§5 admission, the limits that do exist) and
`docs/DISTRIBUTED_CONFERENCING.md` (inter-node mutual TLS).

## 1. What the survey found sound

A survey that lists only faults cannot be trusted about the faults. These
were checked and hold:

- **Inter-node conference trunks are properly authenticated.** A trunk
  INVITE without a verified client certificate is refused `403`, and the
  certificate must name the node the header claims
  (`fcp-conference-server/src/conference_handler.rs:4993–5014`). The
  `X-FCP-Trunk-Node` header alone buys nothing.
- **The trusted-edge headers are gated on the peer.**
  `X-FCP-Caller-Tenant`, `X-FCP-Caller-User` and `X-FCP-Client-Address`
  are ignored — and the ignoring is logged — unless the peer is in
  `[e911] trusted_edges` (`fcp-sip/src/emergency.rs:73–110`), with
  IPv4-mapped-IPv6 peers normalised before the comparison.
- **The TLS client verifier is not a bypass.** The `.dangerous()` call in
  `fcp-sip/src/transport.rs:194` installs a deliberate verifier: a cluster
  peer under the private CA, anything else under the system roots with the
  ordinary name check.
- **REGISTER floods are bounded** per source address and per AOR, and the
  gate runs before any credential work (capacity phase 1).
- **The parser is fuzzed**: request, response, URI, headers, Via,
  multipart, CPIM and a serialise round-trip
  (`external/siphon-rs/fuzz/fuzz_targets/`).

## 2. What is wrong today

### 2.1 An INVITE is authorised by source address, never authenticated — fixed (phase 2b)

The call path admits an INVITE when the peer is local, is a trusted edge,
**matches a registration**, or matches an enabled trunk's host; otherwise
`403` (`fcp-sip/src/handler.rs:2400–2438`). There is no digest challenge
on INVITE anywhere in the workspace — the credentials exist and the
registrar uses them, but a call is authorised by where it came from.

So any host that can source packets from an address a registered device
also uses — the same NAT, the same office LAN, a compromised device beside
it, a carrier-grade NAT pool — can place calls **as that user**, at that
tenant's cost. This is the ordinary toll-fraud shape, and challenging
INVITEs (RFC 3261 §22) is the ordinary answer.

Since v0.25.224 it is challenged, under `[registrar] auth_invites`. What
the flag changes:

- **The digest decides, instead of the source address.** A carrier
  arriving on an enabled trunk and a peer in `[e911] trusted_edges` keep
  source-based admission — a carrier holds no user's password, and an edge
  vouches with proof of its own — and are never challenged. Everything
  else must carry credentials. Where they are absent or the nonce is not
  usable the answer is `401` with a challenge, never `403` (Q2): a phone
  that has lost its credentials has to be able to recover, and a phone
  whose nonce has aged out retries silently on `stale=true` (phase 2a).
  For a challenged caller the registration check of §2.2 no longer
  applies: proving the password is stronger than arriving from a
  plausible place, and a phone that has moved has not stopped being
  itself.
- **Loopback is no longer special.** `is_local` was an admission decision
  and is now only a fallback for when the flag is off. This is the strict
  reading of goal 1, and the only version the test floor can prove, since
  every phone there runs on one address. An embedded component that
  INVITEs this node over loopback therefore needs a place in
  `trusted_edges` or credentials of its own before the flag goes on.
- **Credentials are the caller's own.** A digest proves a password; it
  does not say whose `From` the request may wear. Alice's password
  placing a call as Bob is refused `403` rather than challenged, since no
  retry fixes it.
- **The credentials are looked up in the caller's tenant.** The digest
  store resolved its tenant from the request's `To`, which is the
  registering user on a REGISTER but the **callee** on an INVITE. On a
  multi-tenant edge that looked the caller's credentials up in the
  callee's tenant, found nobody and challenged for ever. A non-REGISTER
  request now resolves by its `From`.

Costs, accepted: the challenge is applied where the `403` of the old gate
was, which is *before* emergency detection, so with the flag on a caller
who cannot answer a challenge cannot dial 911 either. That is what the
source-based gate already did to an unadmitted source, so it is not a
regression — but it is the reason Q2's `401` matters, and a deployment
that wants emergency calls to survive lost credentials should say so
before the default flips. `auth_invites` lives on the call manager, which
is the edge of every default deployment; the standalone registrar handles
no INVITEs and the SBC has its own admission.

### 2.2 The registration check matches the peer as a *substring* — fixed (phase 1)

```rust
regs.iter().any(|binding| binding.contact().contains(&peer_ip.to_string()))
```
— `fcp-sip/src/handler.rs:1752`.

A contact of `sip:bob@10.0.0.10:5060` contains the string `10.0.0.1`. A
host at **10.0.0.1** is therefore authorised as the device registered at
**10.0.0.10**, which is not an exotic address pair but the everyday shape
of a LAN. The same holds wherever an address appears inside a contact's
port or parameters. The check should compare the parsed host of the
binding, not search the URI text.

Since v0.25.222 it does. `registrar::registration_is_at` places a device
by the address its REGISTER arrived from where the registrar kept one,
and otherwise by the contact's **parsed** host, compared as an address
with IPv4-mapped IPv6 normalised on both sides; a contact whose host is a
name places nobody, since resolving it would let a device's DNS decide
who may call. The admission gate reads the stored registration rows
rather than the bindings built from them
(`FcpRegistrar::get_registration_rows`), because only the rows carry the
source address. An INVITE naming a user who is registered *somewhere
else* is logged before it is refused.

### 2.3 A peer names the record its call is filed under — fixed (phase 3)

`LegRecord::from_request` reads `X-FCP-Call-Id` from **any** request and
uses it as the leg's record id
(`fcp-conference-server/src/conference_handler.rs:168`). Its sibling
headers are gated on `trusted_edges`; this one is not. Records are rated
and billed, rolled up per tenant and exported for compliance, so a caller
who chooses the id of a record chooses which call — and whose — their leg
is correlated with. The header's own documentation says it "correlates
records and decides nothing else": correlation *is* the thing being
decided.

The audit phase 3 called for found three more of the same kind, and one
that is not about records at all:

| Header | Read at | What it decided |
|---|---|---|
| `X-FCP-Call-Id` | `conference_handler.rs:168` | which record a conference leg is filed under |
| `X-FCP-Transferred-From` | `handler.rs:2682` | which call a leg's record is correlated with (`correlation_id`) |
| `P-Asserted-Identity` | `conference_handler.rs:8458` | the name and number a room shows for a participant |
| `Alert-Info`, `Call-Info`, `X-FCP-Intercom-Mode` | `handler.rs:3627`, `handler.rs:7992` | how the callee's phone alerts, and the direction of its audio |

The identity one is the sharpest: `extract_caller_info` prefers
`P-Asserted-Identity` because it is "more trusted on SBC/trunk calls",
and nothing checked that the peer was an SBC or a trunk. Anyone who could
dial a room sat in it under any number they liked. Demonstrated on the
test floor before the fix: a phone dialling with
`P-Asserted-Identity: <sip:+15125550100@carrier.example>` is listed as
`+15125550100`.

The last row is narrower than it first looks and worth stating exactly.
`Alert-Info: <…>;info=alert-autoanswer` and `Call-Info: <sip:>;
answer-after=0` are what this platform's **own** intercom sends to make a
phone answer itself (`fcp-pbx/src/intercom.rs`), and
`collect_outbound_intercom_headers` copies all three from the caller's
INVITE to the callee — but only on the paths that go through
`create_outbound_call`, which is hunt groups and forward-on-no-answer,
not an ordinary call. `is_oneway_intercom_request` is read on the
ordinary path too and rewrites the callee's leg to `recvonly`. Neither
was reproduced end to end on the floor; both are plain in the code, and
neither is something a caller has any standing to say.

Fixed in v0.25.225 by one gate rather than four. `fcp-sip/src/claims.rs`
names the headers a peer may not assert and removes them at **ingress**,
before anything reads the request, so a reader written later is gated by
construction rather than by somebody remembering. It runs in
`handle_invite` on the call manager and the SBC, and again in the
conference server's own handler; the conference server gains
`[sip] trusted_edges`, which is how the call manager and the gateway
beside it keep naming the record a leg belongs to.

Two things deliberately kept out of the sweep. Headers with a gate of
their own keep it — `X-FCP-Caller-*` and `X-FCP-Client-Address` against
`trusted_edges` where they are read, `X-FCP-Trunk-Node` against the
peer's certificate, `X-FCP-Join` and `X-FCP-Migrate` against their HMAC —
because those gates work and are tested. And a transfer origin naming a
call this node actually has survives from an untrusted peer: a blind
transfer puts the id in the `Refer-To` and the transferred phone copies
it into its new INVITE (RFC 3261 §19.1.5), so an ordinary phone carries
one legitimately. An invented id names nothing and goes. A transfer that
lands on another node of a cluster loses its correlation rather than
trusting an id nobody there issued.

Costs, accepted: `Alert-Info` and `Call-Info` from an untrusted peer are
removed outright rather than inspected for auto-answer directives,
because a blocklist of vendor spellings is the wrong shape for a
security gate. A deployment that wants a carrier's distinctive ringing
puts that trunk's address in `trusted_edges` or re-adds the header with
a header-manipulation rule, which is the same "stripped or re-asserted,
never passed through" the identity headers get.

### 2.4 The edge of the default deployment has the fewest defences — fixed (phase 4)

The call manager is the edge in every single-binary deployment, in
`deploy/docker-compose` and in the quickstart. It has **no IP ACL** and
**no per-source INVITE limit**. What exists elsewhere:

| | per-source INVITE limit | IP ACL | malformed-SIP policy |
|---|---|---|---|
| Conference server | `max_invite_rate_per_ip` | — | — |
| Presence server | SUBSCRIBE and PUBLISH limiters | — | — |
| SBC | rate-limit policy | `IpAcl` | `MalformedSipPolicy` |
| **Call manager** | **none** | **none** | **none** |

`[admission] max_cps` is a node-wide budget, so one source can spend all
of it; and it is unset by default, so the default deployment has no bound
at all. The shared handler's `invite_rate`/`reinvite_rate_limit` are
per-dialog, which bounds one caller's mischief and nothing about the node.

Fixed in v0.25.226. `fcp-sip/src/edge.rs` is the guard, and it lives in
the **shared handler**, so every binary built on it can have the same
defences; the call manager wires it from a new `[security]` section. It
sits in two places, because the two kinds of check can do different
things:

- **Before the parser, on the bytes**: the address list, the message size
  and whether this is SIP at all. Each of these **drops**. That is a
  deliberate difference from the SBC, which answers `403`: replying to an
  address the node has been told to deny confirms the port is live and
  makes the node a reflector, and a message that is not SIP has nothing
  to answer to. The "is this SIP" check is shallow on purpose — a first
  line that ends in or begins with `SIP/2.0` — because it is not a second
  parser, it is the cheapest way to tell SIP from a port scan, a TLS
  hello at the wrong port, or an amplification probe.
- **On a parsed request, where there is a transaction to answer on**: the
  per-source call rate, refused `503` with `Retry-After`, the way every
  other refusal on the platform is (`docs/CAPACITY_AND_SURVIVABILITY.md`
  §5). Counted only where a call *starts*: a re-INVITE is bounded per
  dialog already, and counting it here would refuse a long call for being
  put on hold twice.

`max_invite_cps_per_source` defaults to **20**, settled with the user on
2026-09-22. A bound now exists out of the box, which is what this section
complains is missing, and 20 calls a second from one address is several
times what a busy office behind one NAT peaks at, so it refuses nothing a
working deployment does. The peers in `[e911] trusted_edges` are exempt,
because an edge or an inter-node peer carries other people's calls by
design and its rate is not one device's. The known cost: a carrier trunk
that legitimately exceeds 20 cps from a single address is throttled until
it is listed or the limit is raised, and what it gets is a `503` with a
`Retry-After` rather than a silent drop.

The address list is `Allow` by default with no rules, and the malformed
policy is log-and-drop, which is exactly what the handler did before
there was a policy — so nothing changes for a deployment that sets
nothing. A typo in a CIDR is a startup error rather than a node that
quietly refuses everyone.

The ACL types are the SBC's, moved down into the shared handler so there
is one implementation rather than two; `fcp-sbc-core` re-exports them
under the names its configuration has always used. The SBC keeps its own
richer pipeline — message size, header counts, URI length, per-method
limits — which runs before its handler and answers rather than drops.

### 2.5 The stateful surfaces are not fuzzed

The parser is fuzzed; the transaction and dialog state machines, the
registrar's binding logic and SDP offer/answer are not. Those are where a
malformed-but-parseable message does its work.

### 2.6 The rest of the survey — done (phase 5a)

These were named in the original survey so they would not be mistaken for
clean. Surveyed in phase 5a, and mostly they *were* clean; what follows
says so explicitly, because a survey that reports only faults cannot be
trusted about the faults.

**The unauthenticated HTTP surface of each binary — sound.** Every binary
that serves an API refuses to bind to a non-loopback address unless a
token is configured, so the fail-open path (`no token configured → let
everything through`) is reachable only from localhost. `/health`,
`/ready` and `/metrics` are deliberately open, as a load balancer and a
scraper read them without credentials. The WebRTC gateway is the one that
is *meant* to face browsers: it is fail-closed, with `/join` and `/ready`
the only routes that need nothing, `/drain` and `/status` behind an
operator token, and a session's own routes behind that session's token.
The events server accepts a static token, a hashed `fcp_…` API token or a
JWT session; `/ingest` is behind all three.

**Token strength — sound.** An API token is 32 bytes from the system CSPRNG,
base64url, stored only as its SHA-256; a provisioning token is two v4
UUIDs; join tokens and migration tickets are HMAC-signed. Nothing is
derived from a timestamp, a counter or a name.

**What reaches the logs — sound.** Nothing logs a secret's value; the lines
that mention tokens report *whether* one is configured or *that* one
failed. The one thing to know: the events server and the call manager's
wallboard accept a token as a `?token=` query parameter, because a
browser cannot set a header on a WebSocket, and a query string is the
part of a URL that proxies and access logs keep. That is inherent to
browser WebSockets rather than a defect here, and the tokens it applies
to are the ones a deployment can rotate.

**Constant-time comparison — one finding, fixed.** The original survey
guessed this: "present in the gateway, unchecked elsewhere". Three
binaries — `fcp-presence-server`, `fcp-ivr` and `fcp-voicemail-server` —
compared the API token with `==`, which stops at the first byte that
differs, so how long the answer takes measures how much of the token the
caller guessed. Nine *other* binaries each carried their own correct copy
of the same ten-line comparison. Both halves are fixed the same way:
`fcp_core::secret::constant_time_eq` is the one implementation, the three
`==` are gone, and the nine copies with them.

**What a carrier may inject on a trunk — sound, with a hazard to name.**
A carrier's `P-Asserted-Identity` no longer survives ingress: phase 3
strips it from any peer that is not a trusted edge, and a trunk's header
rules put back whatever that trunk is entitled to assert, which is the
"stripped or re-asserted, never passed through" this project asked for.
What the platform then signs under STIR/SHAKEN is **not** taken from what
the carrier sent: the attestation is `trunk.stir_shaken.attestation`,
static per trunk, and the numbers the `Identity` header covers are the
ones the platform itself puts on the call, after the trunk's caller ID
and digit plan. So a carrier cannot drive the platform into vouching more
strongly than it was configured to.

The hazard, named here rather than fixed (settled with the user on
2026-09-22): because the attestation is static, a call that merely
*transits* the platform — in one carrier, out another — is signed at
whatever its outbound trunk declares. If that trunk says `A`, the
platform is making a full attestation about an originator it never
authenticated. **A trunk that carries transit traffic should declare `B`
or `C`.** The platform has, since phase 2b, the information needed to
choose an attestation by provenance — whether the caller proved a
password, and whether the tenant owns the number — and doing so would
make the ceiling automatic rather than advisory. That is a feature with
regulatory weight, not a defence, so it is recorded here rather than
built.

**Replay windows — sound, by different means.** A migration ticket is
single-use: it carries a nonce and the conference server admits it once
(`migration::UsedTickets`), so a captured ticket moves nobody twice. A
join token is *not* single-use, and should not be: it is the meeting link,
and a browser that loses its network has to be able to come back. What
bounds it is its lifetime — between one second and 24 hours, chosen when
it is minted — so a leaked link is good until it expires. A deployment
that hands out links ahead of a meeting should mint them short.

### 2.7 A digest proves nothing, because the nonce is the client's — fixed (phase 2a)

Found while building phase 2, and the reason it split in two: the survey
above assumed digest authentication worked where it was used, and it did
not.

`Ha1Authenticator::verify` took the nonce **out of the client's own
`Authorization` header** and used it to compute the expected response
(`fcp-sip/src/auth.rs`). No nonce was ever recorded when a challenge was
issued, so nothing checked that the node had issued this one, that it had
not expired, or that its nonce-count had not been seen before — the
`NonceManager` siphon-rs provides for exactly this was not used.

A digest response covers the method, the request URI and (under
`qop=auth-int`) the body. It covers **no header**. So a single captured
`Authorization` — one packet off the wire, one line from a verbose log,
one compromised device — could be replayed for ever, on a request whose
headers said something else entirely. Against the registrar that is AoR
hijack: replay the victim's REGISTER with the attacker's own `Contact`
and the victim's calls arrive at the attacker. The same header was
accepted by the presence server's SUBSCRIBE and PUBLISH gate.

Fixed in v0.25.223. Challenges are issued from a `NonceManager` and a
request is authenticated in two steps: the digest is checked first,
against the nonce the client used, and only then is the nonce itself
examined — issued here, still live, nonce-count not walking backwards.
The order matters. A client that knows the password and has a stale nonce
is answered `stale=true` and retries silently (RFC 7616 §3.5); a client
that does not know it is simply refused, so a wrong password cannot loop.
Because the digest covers no header, the nonce's retransmission check is
given the request's **identity** — body, `Contact`, `CSeq`, `Call-ID`,
`From` and `To` — so a genuine UDP retransmission still passes and a
replay that changes what the request does is not a retransmission.

Costs, accepted: a nonce lives 300 s, so a device that comes back later
is challenged once more and retries — an extra round trip per
registration refresh, which every SIP client already implements. Nonces
are per node and in memory, so a request landing on a different node of a
cluster is challenged again; that is what an unshared registrar farm
does. The manager holds 10 000 nonces and evicts the oldest under
pressure, so a flood costs legitimate clients an extra challenge, not a
refusal — and REGISTER floods are already bounded per source and per AOR.

## 3. Goals

1. A call is placed by someone the node has authenticated, not by
   someone at a familiar address.
2. What a peer claims is believed only where it is verified, and every
   such claim has one gate with one test.
3. A flood from one source costs that source, not the node.
4. The edge of the default deployment is defended at least as well as the
   SBC.
5. Every finding here ends as a test that fails against the old code.

**Non-goals.** A WAF or IDS; DDoS absorption above the platform (that is
the network's); rewriting siphon-rs's TLS stack; certification against a
standard (PCI, SOC 2) — this is the engineering underneath such a claim,
not the paperwork.

## 4. Phases

One PR each, in order.

| Phase | What |
|---|---|
| 1 ✅ | **The registration check, and a test that fails today.** Compare the binding contact's parsed host, and prefer the stored `source_address` where there is one (Q3); a contact's port or parameters can no longer authorise a stranger. Cheap, contained, and the clearest defect found. |
| 2a ✅ | **The nonce is the node's.** Challenges issued from a tracked `NonceManager`, verified with the nonce-count and the request's identity, `stale=true` where the credentials are right and only the nonce is not — so that the digest phase 2b adds to INVITE is worth adding (§2.7). |
| 2b ✅ | **Authenticate INVITE.** Digest challenge on INVITE, with the credentials the registrar already uses; `[registrar] auth_invites`, off by default for one release and warned about, then on (Q1). An unauthenticated INVITE is answered `401` with a challenge, not `403` (Q2), so a phone that has lost its credentials can recover. Source-based admission remains for trunks and trusted edges, which have their own proof; loopback loses its bypass, and the digest replaces the registration check for everyone challenged (§2.1). |
| 3 ✅ | **One gate for what a peer claims.** `fcp-sip/src/claims.rs` strips at ingress what an untrusted peer has no standing to assert, so every reader is gated by construction (§2.3): the two record ids, the carrier identity headers, and the auto-answer headers the platform's own intercom uses. Trusted edges and mutual-TLS trunks keep what they send; the conference server gains `[sip] trusted_edges` for the call manager and the gateway beside it. |
| 4 ✅ | **The edge defended.** `fcp-sip/src/edge.rs` in the shared handler: an address list, a message-size bound and a "is this SIP at all" check before the parser, each dropping rather than answering; a per-source call rate refused `503` on a parsed request. `[security]` on the call manager, `max_invite_cps_per_source = 20` by default, trusted edges exempt (§2.4). |
| 5a ✅ | **The rest of the survey.** §2.6 finished: the HTTP surfaces, token strength and what reaches the logs are sound; the carrier's identity headers are already stripped and re-asserted, and the attestation hazard is named. One finding fixed — three binaries compared the API token with `==` — and the nine duplicate copies of the comparison consolidated into `fcp_core::secret`. |
| 5b | **Fuzzing the stateful surfaces.** Fuzz targets for the transaction and dialog machines, the registrar's bindings and offer/answer — all in the siphon-rs submodule, so a release-tag round trip — with a nightly workflow like the one forge-media already has. |

## 5. Testing

- Unit, per finding, written to fail against the current code: the
  substring authorisation, a nonce the node never issued, a captured
  header replayed with another `Contact`, a call id from an untrusted
  peer, an INVITE without credentials.
- Phase 5a: the shared comparison's own tests, and a smoke test on the
  presence server that a missing token, a wrong token of the same length
  and a token right but for its last byte are each refused while the
  right one is let through.
- Test floor, phase 4: eight calls in one second from one address are
  not all carried — the ones over the limit are refused `503` — and a
  phone at an address the node denies is answered by nothing at all
  while the node stays healthy. Both fail with the limit at zero and the
  rule flipped to `Allow`.
- Test floor, phase 3: a caller that names a record gets none — the
  call manager's `correlation_id` is null rather than the id it invented
  — and a caller that asserts an identity is shown in the room as the
  `From` it actually dialled from, not the number it claimed. Both fail
  with the gate off.
- Test floor, phase 2b: with `auth_invites` on, a phone that cannot
  answer the challenge cannot place a call as the user it names, while
  that user's own phone — at the same loopback address — can. The test
  fails with the flag off, which is the defect of §2.1 in one assertion.
- Test floor: a phone that is *not* registered cannot place a call; a
  phone at a neighbouring address cannot place one as its neighbour; a
  registered phone still can, with digest. Noted in phase 1: the floor
  runs every phone on one host, and the gate admits **any loopback peer**
  as local before it looks at registrations
  (`handler.rs`, `is_local`), so source-based refusal cannot be shown
  there — those two scenarios wait on phase 2, where a digest challenge
  refuses a caller the floor can actually be. Phase 1's proof is unit.
- `fcp-loadgen` as the flood generator — the register-storm profile is
  most of one already — asserting that one source's flood does not refuse
  another source's calls.
- Fuzzing in CI for the new targets, as `sip-parse` has.

## 6. Open questions

**Q1. Does INVITE authentication default on or off?** *Settled:* off for
one release, with a startup warning naming the key on any node that has
registered users and authorises their calls by address alone; on by
default in the release after. The shape decision 8 of the capacity
project used, so no working deployment breaks on upgrade — at the cost of
one release in which the defect is documented and still live.

**Q2. What does an unauthenticated INVITE get?** `401` with a challenge
is correct; `403` leaks less. Proposed: `401`, because a phone that has
lost its credentials must be able to recover, and the registrar already
challenges the same way.

**Q3. Is the substring fix enough for 2.2, or should a binding carry the
address it registered from?** *Settled:* both. Compare the binding
contact's **parsed host**, and prefer the `source_address` the registrar
has stored since E911 block 2 phase 3 where it is set — which also closes
a device that registers a contact naming an address it does not own.

**Q4. How far does phase 3 go with carrier identity?** Stripping inbound
`P-Asserted-Identity` is safe; re-asserting it under the platform's own
STIR/SHAKEN attestation is a bigger change. Proposed: strip in phase 3,
and note re-assertion as its own piece of work.
