# Edge Security — Design

Status: **agreed** (2026-09-21), survey done, Q1 and Q3 settled by the
user; **phase 1 done** (v0.25.222), **phase 2a done** (v0.25.223),
**phase 2b done** (v0.25.224). Chosen after the capacity and
survivability project closed.

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

### 2.3 A peer names the record its call is filed under

`LegRecord::from_request` reads `X-FCP-Call-Id` from **any** request and
uses it as the leg's record id
(`fcp-conference-server/src/conference_handler.rs:168`). Its sibling
headers are gated on `trusted_edges`; this one is not. Records are rated
and billed, rolled up per tenant and exported for compliance, so a caller
who chooses the id of a record chooses which call — and whose — their leg
is correlated with. The header's own documentation says it "correlates
records and decides nothing else": correlation *is* the thing being
decided.

### 2.4 The edge of the default deployment has the fewest defences

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

### 2.5 The stateful surfaces are not fuzzed

The parser is fuzzed; the transaction and dialog state machines, the
registrar's binding logic and SDP offer/answer are not. Those are where a
malformed-but-parseable message does its work.

### 2.6 Not yet surveyed

Named so they are not mistaken for clean: the unauthenticated HTTP
surface of each binary (`/health`, `/ready`, `/join`, `public_join` and
whatever else); token handling (strength, constant-time comparison —
present in the gateway, unchecked elsewhere, and what reaches the logs);
what a *carrier* may inject on a trunk (`P-Asserted-Identity` and the
caller identity the platform then asserts under STIR/SHAKEN); and replay
windows on the join token and the migration ticket.

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
| 3 | **One gate for what a peer claims.** `X-FCP-Call-Id` believed only from a trusted edge or an authenticated inter-node peer, the same way its siblings are; an audit of every `X-FCP-*` reader with a test each; the carrier-injected identity headers stripped or re-asserted, never passed through. |
| 4 | **The edge defended.** Per-source INVITE limits and an IP ACL in the shared handler, so the call manager gets what the SBC has; a malformed-SIP policy; sensible defaults that refuse nothing a working deployment does today. |
| 5 | **The rest of the survey, then fuzzing.** Finish §2.6 and fix what it finds; fuzz targets for the transaction and dialog machines, the registrar and offer/answer, run in CI as the parser's are. |

## 5. Testing

- Unit, per finding, written to fail against the current code: the
  substring authorisation, a nonce the node never issued, a captured
  header replayed with another `Contact`, a call id from an untrusted
  peer, an INVITE without credentials.
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
