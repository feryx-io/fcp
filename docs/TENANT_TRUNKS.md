# Tenant Trunks — Design

Status: **agreed** (2026-09-16), every recommendation in §13 taken. One document for the item E911 block 2
deferred (`docs/E911_BLOCK_2.md` §12.1) and the gap behind it: **the trunks
the API manages do not carry calls.** Outbound routing is built only from
each node's TOML `[[trunks]]`, so a tenant cannot have a carrier of its own,
and most of what `/v1/trunks` accepts is never read by anything that places
a call.

Read with `docs/E911.md` (emergency trunks), `docs/BILLING.md` (rating is
per tenant, not per trunk) and `docs/MULTI_REGION.md` (nodes and regions).

## 1. Decisions

Proposed after surveying the trunk path end to end — the model, the router,
the call manager's and the SBC's wiring, inbound admission, health,
registration, and what reads each field.

1. **A trunk belongs to a tenant, or to the platform.** `/v1/trunks` trunks
   route calls for their tenant. The TOML `[[trunks]]` on each node stay, as
   the *platform's* trunks: what a deployment shares between tenants, and
   what carries calls for a tenant that has none of its own.
2. **One trunk model, one set of semantics.** `fcp_core::Trunk` becomes the
   model; `fcp_sbc_core::TrunkConfig` is derived from it. Today the two
   disagree — TOML has `emergency_location`, structured SRTP/DTMF/codec
   settings, STIR/SHAKEN and an egress number transform; the database has
   `strip_digits`, `codec_prefs`, `caller_id_override`, `cps_limit`,
   `trunk_group` and a health-check schedule. A field should mean the same
   thing wherever the trunk came from.
3. **A trunk is identified by its id.** Health, capacity, CPS, registration
   and the call's record key on `TrunkId`, not on the name, so two tenants
   may both call their carrier `carrier-a`. The name stays for display and
   for the platform's TOML trunks, which get a stable synthetic id.
4. **The caller's tenant chooses the trunk.** Outbound selection runs over
   the tenant's trunks first (priority, weight, health, capacity, LCR), then
   the platform's, unless the tenant is configured not to fall back.
5. **What arrives on a tenant's trunk is that tenant's call.** Inbound,
   the trunk a call matched decides the tenant, before the SIP domain does.
6. **The fields the API has always accepted are applied.** Number
   transforms, caller ID and from-domain, codec preferences, SRTP and DTMF
   modes, session timers, header rules and the outbound proxy take effect on
   calls that go out on that trunk.
7. **A registering trunk is registered once per cluster**, by one node, not
   by every node that loaded it.
8. **Nothing routes worse than it does today.** Every phase keeps the
   platform's TOML trunks working exactly as they do, including for
   emergency calls, and a deployment that never creates a tenant trunk sees
   no change.

## 2. What is wrong today

**The API's trunks decide almost nothing.**

- Outbound routing is `TrunkRouter`, built from `config.trunks` — the TOML
  `[[trunks]]` — in every binary that routes (`fcp-call-manager`,
  `fcp-sbc`, `fcp-ivr`, `fcp-voicemail-server`, `fcp-conference-server`).
  Nothing reads `TrunkStore` for outbound.
- `OutboundRouteProvider::resolve_outbound(&self, destination_uri)` has no
  tenant parameter at all, so even a provider that wanted to be
  tenant-aware could not be.
- Of the database trunk's fields, **nothing outside the API, the storage
  layer and the CLI reads** `strip_digits`, `prepend_digits`,
  `codec_prefs`, `caller_id_override`, `from_domain`, `cps_limit`,
  `srtp_mode`, `dtmf_mode`, `session_timer_secs`, `outbound_proxy`,
  `trunk_group` or `header_rules`. They are stored, shown in the admin UI,
  and never applied.
- `emergency_capable` on a database trunk is documented as deciding nothing
  (`docs/E911.md`), which block 2 had to work around: emergency routing
  reads the node's TOML trunks and the readiness report tells the tenant so.

**What database trunks *are* used for:** admission of inbound INVITEs by
source address (`handler.rs`, `list_trunks` matched against the peer IP),
and a health probe in `fcp-api-core/src/trunk_health.rs` that colours the
admin UI. That is all.

**The two models have drifted.** `fcp_core::Trunk` and
`fcp_sbc_core::TrunkConfig` overlap but neither contains the other:

| Only in the database trunk | Only in the TOML trunk |
|---|---|
| `strip_digits`, `prepend_digits` | `egress_number_transform` (a digit plan) |
| `codec_prefs` (names) | `codec` (allowed codecs + `allow_transcoding`) |
| `srtp_mode`, `dtmf_mode` (strings) | `srtp` / `dtmf` (structured, with suites) |
| `cps_limit`, `trunk_group`, `description` | `failover`, `stir_shaken`, `emergency_location` |
| `health_check_*`, `max_failures`, `recovery_interval_secs` | `header_rules` as a list of named rules |

**Identity is a name.** `TrunkHealthMonitor`, the CPS window, the
registrar and `call.trunk_name` all key on a `String`. Two tenants with a
trunk of the same name would share health, capacity and CDR attribution.

**Inbound tenancy is by SIP domain.** `resolve_runtime_tenant` reads the
domain of the request URI (then falls back to the node's default tenant). A
call arriving from a carrier carries the carrier's domain, so a tenant
trunk's inbound calls land in whatever tenant the domain resolves to —
usually the default.

**Routing rules cannot choose a trunk.** `fcp-policy`'s models say
"Routing models (trunks, routes) [FUTURE]"; a rule can allow, block or
rewrite, but not say which carrier to use.

**Registration is per node.** `TrunkRegistrar` registers every registering
TOML trunk from every node that loads it. With tenant trunks loaded on every
call manager, that becomes N registrations per trunk.

## 3. Goals and non-goals

Goals:

1. A tenant's trunk, created through `/v1/trunks`, carries that tenant's
   calls: chosen by priority, weight, health, capacity and LCR, with
   failover to the next.
2. Everything the trunk model says about a call — the digits, the caller
   ID, the codecs, SRTP, DTMF, headers — is true of the calls it carries.
3. A call arriving on a tenant's trunk is that tenant's call.
4. `emergency_capable` on a tenant trunk routes emergency calls, closing
   block 2 §12.1.
5. The platform's TOML trunks keep working, and a deployment that uses only
   them sees no behaviour change.

Non-goals (for now):

- Cluster-wide CPS and concurrency counters (per node, as today; §13).
- Carrier billing per trunk — rating stays per tenant (`docs/BILLING.md`).
- The SBC becoming multi-tenant in its enforcement pipeline; it gains
  tenant identification from a trunk, not per-tenant policy.
- Number porting, LOA or carrier provisioning workflows.

## 4. The model

`fcp_core::Trunk` becomes the one model, with `tenant_id: Option<TenantId>`
— `None` is a platform trunk. It gains what the call path needs and the
database model lacks:

```rust
pub struct Trunk {
    pub id: TrunkId,
    /// None for a platform trunk from a node's TOML.
    pub tenant_id: Option<TenantId>,
    …
    /// How an emergency call's location travels (docs/E911.md §5).
    pub emergency_location: Vec<LocationDelivery>,
    /// The digit plan applied to the number as it leaves.
    pub egress_number_transform: Option<NumberTransform>,
    /// STIR/SHAKEN signing for calls on this trunk.
    pub stir_shaken: TrunkStirShaken,
    /// Allowed codecs and whether this trunk may be transcoded for.
    pub codecs: TrunkCodecs,
    pub srtp: TrunkSrtp,
    pub dtmf: TrunkDtmf,
}
```

`strip_digits` / `prepend_digits` stay as the simple form of
`egress_number_transform` and are folded into it on read, so the API keeps
working.

**One type, not two models and a conversion** (built in phase 1). The call
path holds `fcp_core::Trunk` itself: `TrunkConfig` in `fcp-sbc-core` is an
alias of it, and the settings that crate owned — transport, credentials,
failover, SRTP, DTMF, codecs, STIR/SHAKEN, the egress digit plan — live in
`fcp_core::trunk`, re-exported from their old paths. `Trunk`'s serde shape
**is** the `[[trunks]]` TOML a node has always read, so an operator's
configuration parses into the same type the API writes, unchanged.

What is left where the two worlds meet is not a model conversion but two
mappings that cannot hide a field:

- **Storage** maps the trunk to the database's flat columns, explicitly, as
  every other table is mapped.
- **The API's DTO** keeps the flat JSON `/v1/trunks` has always answered
  with (`transport_port`, `srtp_mode`, `strip_digits`), and one helper
  applies those flat fields to the grouped model for create and update
  alike.

A field added to the trunk is therefore a field the call path can read;
what is left to do is read it (phase 3).

**Where trunks come from.** Each routing node holds a `TrunkSet`:

- the platform's trunks, from its own TOML, with ids derived from their
  names so they are stable across restarts;
- every tenant's trunks, read from `TrunkStore` at startup and refreshed
  when they change, with a periodic re-read as a backstop. The API
  publishes `system.config_changed` for users, devices, extensions and
  locations but **not** for trunks today; phase 2 adds it, and the node
  invalidates its `TrunkSet` on it — the same shape the device and location
  caches already use.

A node that has no database (the SBC in a standalone deployment) has
platform trunks only, exactly as today.

## 5. Choosing a trunk

`OutboundRouteProvider` gains the caller's tenant:

```rust
async fn resolve_outbound_ordered(&self, tenant: Option<TenantId>, destination: &str)
    -> Vec<OutboundRoute>;
```

Selection, for a call whose tenant is `T`:

1. **`T`'s trunks** that are enabled, upstream (or bidirectional), healthy,
   under capacity and within CPS, ordered by LCR when the tenant has a rate
   table, else by priority then weight (the existing algorithm, unchanged).
2. **The platform's trunks**, same rules, unless the tenant is marked
   `platform_fallback = false` — a tenant whose calls must never leave on a
   shared carrier.
3. Nothing: `RouteResolution::Unavailable` when trunks exist but none can
   take the call (503), `NoRoute` when none is configured (404). The
   distinction the reachability stage already draws is kept.

A call with no tenant (a platform service placing a call) uses platform
trunks only.

**Emergency calls** run the same selection over `emergency_capable` trunks
first — the tenant's, then the platform's — which is what `docs/E911.md`
promises and block 2 could not deliver. A tenant with no emergency-capable
trunk of its own still reaches the platform's: nobody's 911 call fails
because their tenant's carrier list is short.

**Failover**: the ordered list is tried in turn, and the record names the
trunk that answered. (It was not, until phase 6c-1: an ordinary call with
more than one trunk route rang every carrier at once. See §12, phase 6.)

## 6. What the trunk does to the call

Applied on the leg that goes out on a trunk, in this order:

1. **Number transform** — `egress_number_transform` (or `strip_digits` /
   `prepend_digits`), as the SBC's transform stage already does for its
   own trunks.
2. **Identity** — `caller_id_override` and `from_domain` on the From and
   P-Asserted-Identity, then STIR/SHAKEN signing where the trunk asks for
   it. An emergency call's callback number outranks both (`docs/E911.md`).
3. **Media** — `codecs` filters the offer; `srtp` decides what is offered
   and required; `dtmf` decides RFC 2833 or INFO; `session_timer_secs` sets
   the Session-Expires.
4. **Headers** — `header_rules` names rules from the HMR set the SBC
   already has.
5. **Transport** — `outbound_proxy` when set, else the trunk's hosts, with
   the trunk's transport and TLS verification. The INVITE is addressed to
   the proxy with the dialled number as its user part, which is what a
   proxy routes by and what `[sip] outbound_proxy` already means elsewhere
   on this platform. The hosts are a failover list (v0.25.241,
   `docs/DEPLOYMENT_GAPS.md` §1.8): tried in the order written, each on
   its own port (`host:port`) or the trunk's — unset, the transport's
   default, 5061 for TLS — with the hosts this node's last probe found
   down tried after the rest; a host that fails gives way to the next
   host before the next trunk is consulted.

Inbound, the same trunk decides the tenant, the admission limits (CPS,
concurrency) and which HMR rules run.

## 7. Inbound

A call arriving from a source that matches a tenant trunk's host is that
tenant's call, before `resolve_runtime_tenant` looks at the domain. The
match is by source address (as admission does today) or, when the trunk
authenticates, by the digest credentials it presented.

That makes the existing admission check tenant-correct rather than
tenant-shaped: today it looks up the trunks of the tenant it already
guessed; with this it finds the trunk first and takes the tenant from it.

**Settled when phase 4 was built (2026-09-18).** The paragraph above left
four questions open; each was put to the user with a recommendation, and
every recommendation was taken.

1. **Where a carrier's calls come from.** A trunk gains
   `inbound_sources`: addresses and CIDR ranges, since a carrier often
   sends from addresses it does not receive on. Empty means the trunk's
   `hosts`, with names resolved when the node refreshes its trunks, off
   the call path.
2. **Two tenants on one carrier address.** Among the tenants whose trunks
   match the source, the call is the one that owns the dialled number. When
   only one tenant's trunks match, it is that tenant's call whatever the
   number. When several match and none owns the number, the call is
   refused (`404`).
3. **Direction.** Inbound honours it: an `upstream` trunk takes no calls.
   Nothing honoured direction before, and the API's default for a new
   trunk was `upstream`. So migration 074 moves every stored `upstream`
   trunk to `bidirectional`, and the API's default becomes
   `bidirectional`, as a `[[trunks]]` entry's already was.
4. **Authentication.** A trunk's credentials are the ones this node
   presents to the carrier, and carriers rarely answer a challenge, so
   there is no inbound digest. A registering trunk is recognised by the
   account the call is addressed to: the Request-URI user is its
   `credentials.username`. That is what separates two tenants on one
   carrier.

What follows from §§5 and 13 without a question:

- **The order.** A trusted edge naming its caller comes first. Next, a
  device registered at the address the call came from: it is that
  device, whoever else shares the address (phones behind an SBC that is
  also a platform trunk, every test phone on the loopback). Then the
  trunk. Then the domain, as before.
- **A platform trunk** carries any tenant's numbers, so a call on one is
  the call of whoever owns the dialled number. When nobody owns it, the
  request's own addressing decides, as before.
- **A carrier's call is not sent back out to a carrier.** A number owned
  here with nothing bound to it and nobody registered gets `404` when the
  call arrived on a trunk, rather than going out on a trunk (§5). A
  carrier that routes the number here would send it straight back.
- **Limits** are the trunk's own `max_concurrent_calls` and `cps_limit`,
  shared with its outbound calls (carriers sell channels, not
  directions), per node (§13.3).
- **Header rules** the trunk names run on its inbound requests too (§6).

## 8. Cluster

- **Registration.** A registering trunk is owned by one node: the lowest
  live node id in the cluster that loaded it (`fcp-cluster` membership
  already gives every node the list). The owner registers and refreshes; a
  node that stops being the owner stops registering. With no cluster
  (a single node, no Redis) every node is the owner of what it loaded, as
  today.
- **Health.** Each node probes what it routes and keeps its own view: a
  trunk unreachable from one node may be reachable from another, and the
  call should be placed by a node that can reach it.
- **Capacity and CPS** stay per node (§13.3).

## 9. Observability

- `GET /v1/trunks` gains what actually happens: the health each node sees,
  the calls in flight, and whether the trunk routes on this deployment.
- `GET /trunks` on each call manager (the shape `GET /emergency/trunks` has
  today) lists what that node routes by, tenant trunks included.
- Metrics per trunk id and tenant: calls attempted, answered, failed over,
  rejected for capacity or CPS.
- `trunk_id` joins `trunk_name` on the CDR and the emergency record.
- Events: `trunk.registered` / `trunk.registration_failed` /
  `trunk.health_changed` carry the tenant.

## 10. Migration

Nothing moves on its own.

- A deployment with only TOML trunks behaves exactly as before; its trunks
  are the platform's.
- A tenant that creates a trunk starts using it for its own calls at once.
  The admin UI says what a trunk is for (this tenant) and what the platform
  falls back to.
- `docs/DEPLOYMENT_BINARIES.md` and the compose configs keep their
  `[[trunks]]`; the documentation says when to use which.

## 11. Testing

- **Unit**: selection with a tenant's trunks, the platform's, both, none;
  `platform_fallback = false`; emergency ordering; the transform, identity
  and media rules applied to an outbound leg.
- **API**: a trunk created through `/v1/trunks` appears in what the node
  routes by; a disabled or unhealthy one does not.
- **Test floor** (`docs/TEST_FLOOR.md`): two tenants, two scripted
  carriers, one call each — each lands on its own tenant's carrier with
  that trunk's digits and caller ID; a tenant with no trunk falls back to
  the platform's carrier; a tenant trunk marked emergency-capable takes a
  911 call; an inbound call from a tenant's carrier is that tenant's.
- **Cluster**: two call managers, one registering trunk, one REGISTER.

## 12. Phases

One PR per phase, reviewed before the next starts.

1. **One model.** `fcp_core::Trunk` gains the missing fields and becomes
   the type the call path holds (`TrunkConfig` an alias of it); the
   `[[trunks]]` TOML parses into it unchanged; health, capacity and CPS key
   on the trunk rather than its name; `OutboundRoute` carries the trunk id.
   No routing change: platform trunks only, as today.

   *Done (v0.25.185).* What shipped:

   - **One type.** The settings `fcp-sbc-core` owned moved into
     `fcp_core::trunk` and are re-exported from their old paths, so the
     router, health monitor, registrar, HMR and STIR/SHAKEN kept their
     imports. Migration 072 gives the stored trunk the columns it lacked
     (`strip_prefixes`, `srtp_suites`, `allow_transcoding`, the four
     STIR/SHAKEN columns, `emergency_location`).
   - **Identity.** A trunk with no stored id — one a node read from its own
     TOML — is `TrunkId::platform(name)`, the same on every node and across
     restarts. Two tenants may both have a `carrier-a` without sharing
     health or capacity; the name is still what the admin API, the logs and
     the CDR read.
   - **The API is unchanged**, field for field, and asserted so by a
     round-trip test.
   - Two defaults nearly changed on the way and are now pinned by tests: a
     `[[trunks]]` entry keeps `max_failures = 5` and `direction =
     bidirectional` when it says nothing, though the API's own default for
     a new trunk is `upstream`.

   Left for later phases: registration keys on the name until phase 5 gives
   the registrar trunks from more than one tenant, and the call events and
   CDR gain `trunk_id` in phase 2 with the selection that needs it.
2. **Tenant trunks route.** The provider takes a tenant; the node holds a
   `TrunkSet` refreshed from the database on `ConfigChanged`; selection is
   the tenant's trunks then the platform's; `trunk_id` on the records; the
   test floor routes two tenants to two carriers.

   *Done (v0.25.186).* What shipped:

   - **The node holds a `TrunkSet`** — its own `[[trunks]]` and every
     tenant's, read from `TrunkStore::list_all_trunks` at startup, again
     on every `system.config_changed { entity_type: "trunk" }` the trunks
     API now publishes (create, update, delete, clone, and a change to a
     tenant's `platform_trunk_fallback`), and on a backstop tick
     (`[trunk_routing] refresh_secs`, a minute by default) for a node that
     missed the event. Health probes run over the set, so a trunk created
     while the node runs is probed as soon as it is known. The call
     manager, the IVR, the voicemail and the conference server all route
     this way; the SBC stays platform-only (§13.8).
   - **A node with no `[[trunks]]` still routes**: the outbound provider is
     always there, since the tenants' trunks arrive after startup.
   - **Selection takes the tenant** — `OutboundRouteProvider`'s four
     methods and every call site — and is the tenant's trunks, then the
     platform's unless the tenant refuses them
     (`tenants.platform_trunk_fallback`, on `/v1/tenants`).
   - **The record names which trunk carried the call**: `cdrs.trunk_id`
     beside `trunk_name`, on `/v1/cdrs`, and the failover route that
     answered names its own. `call.initiated`, `call.ended` and
     `call.failed` carry the id too; the emergency events wait for phase 6,
     where a tenant's `emergency_capable` starts to mean something.
   - **On the floor**: two tenants, two carriers, one node with no trunks
     of its own — each tenant's call leaves on its own tenant's trunk, and
     a tenant with none of its own falls back to the platform's until it
     says not to (`fcp-call-manager/tests/trunks.rs`).

   Two things the phase found, both fixed in it:

   - **An outbound call never reached a trunk on a node with a database.**
     A dialled E.164 that is nobody's DID here went phone-number lookup →
     registration lookup → `480`; only the no-storage path asked the route
     provider. Every deployment with a database — which is every real one —
     could route out only through a routing rule or the emergency path.
     The two dead ends now ask for a trunk first (503 when trunks exist but
     none can take the call, `480` when none is configured), which is what
     made the floor test above possible.
   - **`tenants.platform_trunk_fallback` read back as `true` whatever was
     stored**, because the tenant SELECT list did not name the new column
     and the row mapping defaulted quietly. The column is listed and the
     mapping now reads it like every other field — the same shape of bug as
     phase 1's trunk columns, and the second reason not to write a mapping
     that can default.

   Left for later phases: what the trunk does to the call is still the
   SBC's (phase 3), the registrar keys on the name (phase 5), and
   `emergency_capable` on a tenant trunk still decides nothing (phase 6,
   done in 6a), so the emergency-capable list a node offers is still its
   own trunks'.
3. **The trunk's own behaviour.** Number transform, caller ID and
   from-domain, codecs, SRTP, DTMF, session timer, header rules, outbound
   proxy — applied, and tested on the wire. Split in two when it was built,
   because the first half is per-trunk data the B-leg can read and the
   second needs infrastructure the call manager does not have yet.

   **3a — what the trunk presents.** *Done (v0.25.187).*

   - **The route carries the trunk itself.** `OutboundRoute.trunk` is an
     `Arc<Trunk>`; the name, the id, the codecs and the emergency delivery
     are read from it rather than copied beside it, so a field added to the
     trunk is a field the leg can read. `Call.route_trunks` carries it too,
     which is how the route that answered names its own trunk.
   - **Identity.** `caller_id_override` replaces the From user (a whole URI
     is taken whole), `from_domain` replaces the host, and the trunk's
     identity is asserted in `P-Asserted-Identity` — a carrier that bills by
     it sees the trunk's number, not the extension behind it. An emergency
     call's callback number outranks both (`docs/E911.md`).
   - **Session timer.** `session_timer_secs` sends `Session-Expires:
     N;refresher=uac`, `Min-SE: 90` and `Supported: timer`, with RFC 4028's
     90-second floor applied to anything shorter.
   - **DTMF.** A trunk whose `dtmf` is not RFC 2833 is not offered
     `telephone-event`: offering it and then not sending it is how DTMF goes
     missing on a carrier that believes the offer.
   - **Outbound proxy.** `outbound_proxy` is where the INVITE goes — a host,
     `host:port` or a SIP URI — with the dialled number still the user part,
     which is what the proxy routes by, and the trunk's own transport and
     port when the proxy names none. Health probes still probe the trunk's
     hosts: the proxy is not the trunk.
   - **Number transform and codecs** were already applied when the route was
     built; both now have their behaviour asserted on the wire.

   **3b — what the trunk requires.** Three pieces, each needing
   infrastructure of its own, so each is its own PR.

   **3b-1 — SRTP.** *Done (v0.25.188).*

   - A trunk whose `srtp` is `preferred` or `required` is **offered** SRTP:
     the B-leg's offer is `RTP/SAVP` with an `a=crypto` line carrying the
     key this node will send with.
   - A carrier that answers a `required` trunk in the clear has not
     answered it: the leg takes a BYE and the caller a `488`, rather than
     media going out unencrypted to a carrier expecting it encrypted.
     `preferred` takes either answer.
   - This node terminates the encryption, so it can only offer SRTP on a leg
     whose media it anchors. A `required` trunk on a call this node does not
     anchor is refused (`488`) instead of being sent plain RTP; a
     `preferred` one is offered plain RTP and says so in the log.
   - **A latent bug fixed on the way:** the media backend armed the B-leg
     with a *freshly generated* local key whenever an answer carried
     `a=crypto`. In SDES each side states the key it sends with (RFC 4568
     §5.1.2), so that key is the one this node advertised — a new one would
     encrypt to a key the far end never saw, and the carrier would hear
     noise. The backend now takes the offer it made
     (`update_session_b_leg_offered`) and keeps a fresh key only for a peer
     that sent crypto unasked, which is all it could do before.

   **3b-2 — header rules.** *Done (v0.25.189).*

   - **The engine is shared.** The compiled HMR engine moved from the SBC
     *binary* into `fcp-sbc-core`, beside the configuration types it was
     already reading, and the SBC now uses it from there. A call manager
     reads the same `[header_manipulation]` section the SBC does: the rules
     are the node's, and which of them run on a call is the trunk's to say.
   - **A trunk names its rules.** The engine gained
     `apply_request_egress_named`, which takes rule names rather than a
     trunk name, because a tenant's trunk arrives after the engine is built
     and was never in its trunk map. Names it does not know are logged and
     skipped.
   - **The rules reach a request this node builds.** They run on the
     finished INVITE — after the Via, Contact and Content-Length, before
     the transaction starts — through `InviteOptions::filter`, a
     `RequestFilter` added to siphon-rs for this (`sip-uac` 0.7.3,
     `v2026.09.17`). A rule can therefore remove or rewrite a header this
     node itself wrote, which is the whole point of HMR and was impossible
     while the request was built and sent in one call.
   - Emergency INVITEs are not rewritten by an operator's rules.
   - On the wire: a trunk that names one rule gets that rule (a header
     added, a header removed) and not the other one configured.

   **3b-3 — STIR/SHAKEN signing.** *Done (v0.25.190).*

   - **The signer is shared.** `StirSigner` moved from the SBC *binary*
     into `fcp-sbc-core`, beside the PASSporT primitives it was already
     calling; the SBC uses it from there and a call manager reads the same
     `[stir_shaken]` section. The key and the certificate it names are the
     node's; which calls are signed, and at what attestation, is each
     trunk's `stir_shaken` to say.
   - **Signed where the numbers are final.** The `Identity` header is
     built for the leg, over the numbers the carrier will see: after the
     trunk's `caller_id_override` and after its digit plan, because those
     are the numbers the carrier verifies. A call with a name rather than
     a number at either end, or a node with no signing key, goes out
     unsigned — an `Identity` that does not match the call is worse than
     none.
   - **A PASSporT a strict verifier accepts.** `orig.tn` was written as an
     array; RFC 8225 §5.2.1 says one number, as a string — only `dest.tn`
     is an array. It is a string now, and read from either shape, since
     implementations that copied `dest` are out there. Nothing on this
     platform signed a tenant's call before this phase, so the wrong shape
     had never left it in anger.
   - On the wire: the carrier receives an `Identity` header whose PASSporT
     names this node's certificate, attestation A, and the trunk's own
     caller ID as `orig` — not the extension behind it.

   **3b-4 — DTMF converted.** *Done (v0.25.191).* 3a stopped offering
   RFC 2833 to a trunk that does not take it; this turns what one leg
   sends into what the other takes.

   - **Settled when the trunk answers.** Each leg's way of taking DTMF is
     worked out from the caller's offer and the trunk's answer and kept
     on the call (`DtmfCarriage`): a leg takes RFC 2833 only when its SDP
     has `telephone-event`, so a trunk set to `rfc2833` that answers
     without it takes digits in the audio, and one set to `sip_info`
     takes INFO whatever it answered.
   - **Telephone-event goes only where it was negotiated.** Relay was one
     switch for both directions; it is now per leg, so a trunk (or a
     caller) that did not agree to `telephone-event` is not sent packets
     at a payload type it never saw.
   - **RFC 2833 → INFO.** A digit the caller sends as telephone-event to
     a trunk that takes SIP INFO is sent on the trunk's dialog as an
     `application/dtmf-relay` INFO, serialised with the dialog's other
     requests so its CSeq is the next one. A digit heard in the audio is
     not: the trunk hears it already.
   - **INFO → RFC 2833.** An INFO digit for a leg that takes
     telephone-event — the caller's to an `rfc2833` trunk, or the trunk's
     to a caller that offered it — is played to that leg as
     telephone-event at its own payload type and answered `200` here,
     rather than forwarded as a request it does not take. Any other INFO
     is forwarded as before.
   - **The media engine grew what this needed** (forge-engine 0.6.1,
     forge-media `v2026.09.17`): the digits each leg sends, tagged with
     that leg and how they arrived; relay per direction; and a digit
     played to one leg. The media backend's `send_dtmf`, a no-op until
     now with no caller, takes a leg and plays the digit.
   - **Needs the media anchored.** Only a node that carries the call's
     media can hear a digit or play one; an unanchored call's DTMF goes
     as each end sends it. A trunk that takes DTMF in-band is not sent
     generated tones.
   - On the wire: a caller's RFC 2833 `5` reaches a `sip_info` trunk as an
     INFO and not as telephone-event, the trunk's INFO `9` reaches the
     caller as telephone-event, and a caller's INFO `7` reaches an
     `rfc2833` trunk as telephone-event and not as an INFO.
   - The test phone counted every repeated end packet of an RFC 4733
     event as a digit of its own; it counts one per event now, which is
     what let a test hear two digits in a row.

   **Phase 3 is complete.**
4. **Inbound.** The trunk decides the tenant; per-trunk inbound
   authentication and limits; admission rewritten on the match. Its open
   questions were settled when it was built (§7), and it was split like
   phase 3.

   **4a — whose call it is.** *Done (v0.25.192).*

   - **The trunk decides.** Each node's `TrunkSet` keeps an index of where
     every trunk that takes calls takes them from: its `inbound_sources`,
     else its hosts with names resolved at each refresh. The index is
     sorted most specific range first. A carrier's INVITE is matched
     against it after the trusted edge and the registered-device check,
     and before the domain. `choose_inbound` (`fcp_sip::trunk_ingress`)
     picks the trunk and the tenant as §7 says. An `upstream` or disabled
     trunk is never a candidate.
   - **Admission is the match.** A source that matches a trunk is
     admitted; one that matches none is refused (`403`), as before. A node
     with no trunk set (the IVR, voicemail and conference servers) still
     admits by its tenants' stored trunks. The old check looked only at
     the trunks of the tenant the domain had guessed, compared hosts as
     strings, never resolved a name and ignored direction.
   - **The record** names the trunk an inbound call arrived on
     (`trunk_id`, `trunk_name`). The call remembers it too
     (`arrived_on_trunk`): a carrier's call to a number owned here that
     nothing answers gets `404` rather than going back out to a carrier.
   - **The model:** `inbound_sources` on the trunk, `/v1/trunks` and the
     admin UI (addresses and CIDR ranges, refused otherwise); migration
     074 adds the column, moves stored `upstream` trunks to
     `bidirectional` and makes that the default.
   - On the floor: one carrier on one address serves two tenants. Each
     tenant's number is answered by that tenant's flow, and each record
     names that tenant's trunk, though the Request-URI names only this
     node's address. A number neither tenant owns is refused.
   - **Not yet:** a registering trunk is matched by the account a call is
     addressed to, but every trunk still registers under this node's own
     identity. The match works for a carrier that addresses the account,
     and fully once phase 5 registers each trunk as itself.

   **4b — the trunk's limits and rules on the way in.** *Done
   (v0.25.193).*

   - **One count per trunk, both directions.** A trunk's
     `max_concurrent_calls` and `cps_limit` are its carrier's, and a
     carrier sells channels, not directions. A carrier's call counts
     against its trunk when it is admitted; past either limit it gets
     `503` with `Retry-After: 1`, unless it is an emergency call. An
     outbound call counts when the trunk answers it. The handler never
     learns when a failover attempt that did not answer has gone, so
     counting at the INVITE would hold that attempt's channel for the
     whole call. A call ringing on a trunk is not counted yet; the router
     still skips a trunk that is full.
   - **Released wherever the call ends.** `TrunkCalls` (`fcp-sbc-core`)
     keeps a ledger of what each call was counted against. The
     `CallManager` tells it of every call it ends or discards
     (`set_on_ended`), so every path that ends a call releases what it
     held.
   - **A latent bug fixed on the way:** the router counted a call on its
     primary route's trunk each time it resolved routes, and only the SBC
     ever released that count. On a call manager (and the IVR, voicemail
     and conference servers) a trunk with `max_concurrent_calls` set
     stopped being chosen once that many calls had ever left on it. Those
     nodes now leave counting to the handler, or count nothing
     (`TrunkRouter::without_call_counting`). The call manager's
     `cps_limit`, which `NoCpsLimit` had ignored, is enforced by
     `TrunkCps`: the calls started on a trunk in the current second, in
     either direction.
   - **Ingress rules.** The header rules a trunk names run on the INVITE
     it brings in, once the trunk is known and before anything else reads
     the request. The call sees the request as they leave it. A rule may
     not change what ties the request to its transaction: Via, Call-ID and
     CSeq are put back as they arrived, with a warning, so the responses
     still match what the carrier sent. The trunk itself is identified from
     the request as it arrived. Only the INVITE is rewritten, not the
     requests later in its dialog.
   - On the wire: a trunk with one channel carries Alice's call out, and
     refuses the carrier's call in with `503` while that call lasts. Once
     it ends, the carrier's next call takes the channel and a second is
     refused. A carrier's national caller number reaches the phone as
     E.164, rewritten by the ingress rule its trunk names.
5. **Cluster.** One node registers a registering trunk; health per node;
   failover across nodes; the events. Split in two: what a node registers
   is one piece of work, what every node sees of a trunk is another.

   **5a — one node registers, as the trunk itself.** *Done (v0.25.198).*

   - **Every registering trunk, not just the node's own.** Registration was
     built once at startup from the node's `[[trunks]]`, keyed by name. It
     is driven from the `TrunkSet` now, keyed by trunk id, and re-read on
     every tick: a tenant's trunk registers as soon as the node has it, an
     edit reaches the next refresh without dropping the binding, and a
     trunk the node stops routing by is given up. Two tenants' "carrier-a"
     are two registrations.
   - **The trunk registers as itself.** `From` and `To` name the carrier
     account — the trunk's credentials at the realm the carrier challenges
     with, else its `from_domain`, else the host — and the challenge is
     answered with that account's own credentials. `Contact` is this node,
     which is where the carrier's calls for the account arrive, so the
     trunk a call comes in on says whose it is (§7, and what phase 4a left
     open). A trunk with no credentials has no account to be and registers
     under the node's own address of record as it always did.
   - **One node holds it.** `ClusterMember` carries the registering trunks
     each node has loaded, because a `[[trunks]]` entry may be on one node
     and no other: the owner is the lowest live node id that loaded it,
     preferring a node that is not draining, and a node that stops being
     the owner de-registers rather than leave the carrier with two bindings
     for one account. Without a cluster every node owns what it loaded,
     which is what a single node did before. Failover is the membership
     TTL: the owner's heartbeat expires and the next-lowest node takes the
     binding over.
   - **A node does not claim what it has not yet advertised.** Ownership is
     read from what every node has published about itself, so a node that
     has just loaded a trunk has said nothing about it yet — and a node
     that acts on that silence registers a second binding beside one
     another node already holds. A trunk becomes claimable a couple of
     heartbeats after this node first advertised it, and the advertisement
     is published at once rather than on the next beat. Two nodes that load
     a trunk seconds apart still converge rather than settle: the one that
     had it first holds it until the lower node id has it too, then gives
     it up.
   - **The events**: `trunk.registered` and `trunk.registration_failed`,
     each naming the trunk, its tenant, the account bound and the node that
     holds it — two nodes reporting one trunk is how a split is seen.
   - **siphon-rs** gained the REGISTER an account other than the agent's
     own needs (`sip-uac` 0.7.4): `create_register_as` / `register_as`,
     with the refresh series (Call-ID, `From` tag, CSeq) kept per account
     and registrar rather than per registrar — two trunks at one carrier
     were otherwise sharing one series, where each refresh would have
     invalidated the other's binding.
   - On the floor: a trunk binds its own account and answers the carrier's
     digest challenge with its own credentials; and two call managers on
     one database hold one binding between them, which moves to the
     survivor when the node holding it is killed.

   **5b — what every node sees.** *Done (v0.25.200).*

   - **A node answers for itself.** `GET /trunks` on each node lists every
     trunk it routes by — the tenants' beside the platform's, which is what
     it adds over `/emergency/trunks` — with this node's own health, its
     own calls in flight, whether it would route a call there now, and what
     its REGISTER binding is doing.
   - **And says so to the cluster.** Each node publishes that view under
     its own key every ten seconds, and it expires thirty seconds after the
     last one: the answer is what the live nodes currently see and never a
     node that has gone.
   - **`/v1/trunks` shows every node's at once.** A trunk carries the views
     of the nodes that route by it, the calls in flight across them, and
     whether any node could place a call — so a carrier one node cannot
     reach and another can is visible as the disagreement it is, rather
     than as one server's opinion. A deployment with no Redis answers as it
     did before, from the API server's own probes.
   - **`trunk.health_changed`** when a node's probes change their mind,
     naming the node, what it is now and what it was: health is per node,
     so an event that did not say which node would be unreadable. Only a
     change is news.
   - **Metrics per trunk and tenant** (`fcp_trunk_calls_total` on each
     node's `/metrics`): attempted and answered, and refused for capacity
     or for calls per second — counted where the router passes a trunk over
     for its limits and where a carrier's call is admitted, which are the
     two places a limit is enforced. `failed_over` is counted where a call
     gives up on a route and takes the next, which today is the emergency
     path; the ordinary B-leg's walk through its failover bindings is not
     yet counted.

   **Phase 5 is complete.**
6. **Emergency and policy.** `emergency_capable` on tenant trunks closes
   block 2 §12.1 (with the platform's trunks as the last resort); routing
   rules gain a "route via trunk / trunk group" action; per-tenant LCR
   tables; the admin UI shows what routes and what each node sees. Split
   when it was built: 6a is the emergency half, the policy and the UI
   follow.

   **6a — a tenant's emergency trunks.** *Done (v0.25.201).*

   - **Whose trunk first.** An emergency call tries its own tenant's
     `emergency_capable` trunks (priority, then weight, among themselves),
     then the platform's, then whatever would carry an ordinary call — so a
     deployment that flagged nothing still places it. A tenant's trunk
     comes ahead of the platform's whatever its priority: priority orders
     trunks within an owner, not across them.
   - **The platform's are the last resort for anyone**, including a tenant
     that set `platform_trunk_fallback = false`. That setting is a billing
     and routing preference for ordinary calls; the emergency path asks for
     the platform's trunks directly (§13 question 6). Another tenant's
     trunk is never a candidate.
   - **The record says which trunk.** `emergency_calls.trunk_id` (migration
     075) beside `trunk_name`, on `GET /v1/emergency/calls` and on
     `call.emergency_completed`: a trunk's name is unique only within its
     tenant, and a tenant's own trunk now carries these calls.
   - **`GET /emergency/trunks`** on each call manager lists every
     emergency-capable trunk the node routes by, tenants' included, each
     with its `trunk_id` and `tenant_id`, and counts all of them in
     `configured_trunks`.
   - **On the floor** (`fcp-call-manager/tests/trunks.rs`): a tenant's 911
     call leaves on its own emergency trunk though the platform's has the
     better priority, and a tenant that refused the platform's trunks and
     has no emergency trunk of its own still reaches the platform's for
     911 while its ordinary calls keep to its own carrier.

   **The rest of phase 6, decided 2026-09-18.** A survey found the rules
   `/v1/routing/rules` manages — `allow`, `deny` and a `route_to_trunk`
   naming a trunk or a trunk group, with an API and an admin UI page —
   stored and read by nothing on the call path: the only policy a call met
   was the node's YAML file (`[routing] rules_file`), which can only allow
   or deny, and which ran *before* emergency detection, so a deny rule
   could refuse a 911 call. LCR was one TOML table per node, matched to
   trunks by name. Six questions were put, and every recommendation taken:

   1. A node holds every tenant's rules as it holds their trunks: read at
      startup, again on `system.config_changed { entity_type:
      "routing_rule" }` (which the API now publishes), and on the backstop
      tick.
   2. The node's YAML policy, the platform's, runs first; then the
      tenant's rules by priority, first match deciding: `deny` refuses
      (403), `allow` goes on to ordinary routing, `route_to_trunk` confines
      the call to what it names.
   3. A rule may name one of the tenant's own trunks by id (the API checks)
      or a trunk group: the tenant's members, then the platform's unless
      the tenant refused them. When nothing it names can take the call,
      the call is refused (503) rather than sent elsewhere; a group is how
      a rule gets failover.
   4. No rule, YAML or tenant, applies to an emergency call, and detection
      moves ahead of both.
   5. Per-tenant LCR is a new tenant-owned table (`lcr_routes`: prefix,
      trunk id, cost, priority, enabled) at `/v1/lcr-routes`, cached like
      rules; a tenant's table orders its own trunks, the node's `[lcr]`
      stays for the platform's, and LCR orders what a rule has left. Not
      the billing rate plans, which are what a tenant is charged, not what
      a carrier costs.
   6. Three PRs: 6b the rules route, 6c per-tenant LCR, 6d the admin UI.

   **6b — a tenant's rules decide its calls.** *Done (v0.25.202).*

   - **The rules are held and evaluated on each call manager**
     (`fcp_policy::TenantRuleSet`, kept current by
     `fcp_sbc_core::spawn_routing_rule_refresh`, which shares its loop with
     the trunks' refresh). A rule matches a regex over what was dialled —
     the Request-URI's user part or the whole URI, so `^\+44` works as
     written — and over the caller the same way, source CIDR ranges, and a
     time window in the rule's own timezone (days alone mean whole days,
     times alone every day, an end before the start spans midnight). Every
     condition a rule sets must hold. Disabled rules are not in force; a
     stored rule that does not compile is logged and skipped, and a row
     whose action does not read is an error rather than an `allow`.
   - **`route_to_trunk` confines the call.** The call carries the rule's
     target (`Call::trunk_target`); every place an ordinary call looks for
     a trunk asks for only that target
     (`OutboundRouteProvider::resolve_targeted_classified`), ordered the
     tenant's own first, then priority and weight. Nothing outside it is
     tried: `503` when the target exists but cannot take the call, `404`
     when it names nothing this node knows. A call the node serves itself
     — an extension, a DID of ours — is unaffected; the rule decides only
     which carrier a call leaves on.
   - **Emergency calls first.** Detection moved ahead of the YAML policy,
     and neither the node's policy nor a tenant's rules apply to an
     emergency call.
   - **The API.** Reading rules takes `trunk_read` and writing them
     `trunk_update`: the permissions a tenant's trunks already take, since
     a rule now decides where its calls go (they took no permission at
     all, so any user of the tenant could write one). A rule is checked as
     a node compiles it (patterns, CIDRs, days, `HH:MM` times, IANA
     timezones, a target for `route_to_trunk`), a trunk it names must be
     the tenant's own, and every create, update and delete publishes the
     change.
   - **On the floor** (`fcp-call-manager/tests/trunks.rs`): a rule sends
     `+44` calls to its trunk group while other calls take the ordinary
     choice; a rule naming a trunk that is switched off refuses the call
     with `503` and rings nothing else; a deny-everything rule refuses an
     ordinary call with `403` and not a 911 call; and a YAML policy that
     knows no one refuses an ordinary call but not a 911 call.
   - **Found on the way: a flow ran within 50 KB of overflowing its stack.**
     A flow a call reaches runs until it waits for the caller — a pause,
     a hangup and the SIP work behind it — and it ran on the INVITE's own
     task, under `handle_invite`'s frame. In a debug build that needed
     2.1 MB of a worker's 2 MB, measured with `RUST_MIN_STACK`, so the
     little this phase added to `handle_invite` aborted the node when a
     flow hung up (`a_flow_that_hangs_up_ends_the_call_it_answered`).
     `start_flow` now runs the flow on a task of its own and awaits it;
     the same flow fits in 1.5 MB.

   **Carriers one at a time, decided 2026-09-19.** Building 6c found that
   an ordinary call with more than one trunk route rang every carrier at
   once: the B2BUA sent its routes down the parallel-forking path built
   for a user's ring group, so a call priced on the SBC's `[lcr]`
   (`max_failover_attempts = 2` by default) reached the callee up to three
   times, and so did a 6b rule naming a trunk group. Only emergency calls
   went one route at a time. The user chose to fix it first, in its own
   PR, with per-tenant LCR after it: 6c-1, then 6c-2.

   **6c-1 — one carrier at a time.** *Done (v0.25.203).*

   - **A call whose targets are all trunk routes sends its INVITE to the
     first only**, keeping the rest per call (`TrunkFallback`, beside the
     emergency path's `EmergencyFallback`); a user's registered devices
     still ring together. The per-target INVITE is one method
     (`send_b_leg_invite`) for both.
   - **The next route is tried when the carrier failed**: a refusal of
     ours (401, 403, 407), 408, 488, any 5xx, no final response at all, or
     silence until the forking timeout. A carrier that rang has delivered
     the call, and a busy, a decline, a number that does not exist, a
     callee who is away or a cancel is the callee's answer, which another
     carrier would only deliver again (`carrier_failed`). The record names
     the trunk the call is on as it moves, and the one that answered.
   - **On the floor**: a rule naming a two-carrier group — the second
     never rings while the first does, takes the call when the first
     answers 503, and is not tried when the first answers 486, which the
     caller hears. With the split disabled the test fails at the first of
     those.

   **6c-2 — a tenant's least-cost routes.** *Done (v0.25.204).*

   - **The table.** `lcr_routes` (migration 076): a tenant's prefix, one of
     its own trunks by id, a cost per minute (`NUMERIC(18,6)`, ordering
     only — it is what the carrier charges, not what the tenant is
     charged), a priority for ties and an enabled flag; a route goes with
     its trunk (`ON DELETE CASCADE`), and a prefix is priced once per
     trunk. `/v1/lcr-routes` (list with `?trunk_id=`, create, get, patch,
     delete) takes the trunk permissions, checks the prefix (digits, an
     optional leading `+`), the cost and the priority, refuses a trunk that
     is not the tenant's own, and announces each change as
     `system.config_changed { entity_type: "lcr_route" }`.
   - **On each node** the tenant's routes are read with the tenants'
     trunks (`TrunkSet::set_tenant_lcr`, refreshed on a trunk or route
     change and on the backstop); a route naming a trunk that is not its
     own tenant's is not in force, and a table that cannot be read leaves
     the last in force.
   - **Selection, in two parts** (`TrunkRouter::select_trunks_ordered`):
     the tenant's own trunks — by its routes when one matches the dialled
     number (longest prefix, then cost, then priority, cut at
     `max_failover_attempts + 1`), else its ordinary choice by priority and
     weight — then the platform's, by the node's `[lcr]` when a route
     matches, else the ordinary choice, unless the tenant refused them.
     Tried one at a time, as 6c-1 made them.
   - **Two defects fixed on the way.** Ordinary selection put the tenant's
     trunks and the platform's in one pool, so a platform trunk with a
     better priority outranked the tenant's own (§5 says the tenant's come
     first); and the node's `[lcr]` matched routes to trunks by name across
     everything a tenant could use, so a tenant trunk sharing a platform
     trunk's name was priced by the platform's table. The node's table now
     prices only the platform's trunks.
   - **What a rule left** (6b) is ordered the same way: the priced members
     of the group first, the rest behind them by priority.
   - The call manager builds no node `[lcr]` (only the SBC does, which
     loads no tenant trunks), so on a call manager a tenant's table is the
     only least-cost routing there is.
   - **On the floor**: a UK number leaves on the tenant's cheapest carrier
     though the others outrank it, the next by cost takes it when the
     cheapest answers 503, and an unpriced number takes the ordinary
     choice; and a tenant's trunk carries its calls ahead of a platform
     trunk with a better priority.

   **6d — the admin UI, decided 2026-09-19.** Three questions, every
   recommendation taken: "what routes" is a **live route preview** asked of
   a call manager (its own policy, rules, least-cost routes, trunks and
   health — exactly what it would do), not a copy of its logic in the API;
   routing rules get **their own permission**, held by tenant admins in
   every mode; and one PR.

   **6d — what routes, and what each node sees.** *Done (v0.25.205).*

   - **Route preview.** `GET /routes/preview?tenant=…&to=…[&from=…]
     [&source_ip=…]` on each call manager (behind its `[http] api_token`)
     runs an INVITE's decisions in order without placing anything:
     emergency detection (an emergency number shows the emergency path's
     trunks and that no rule applies), the node's YAML policy, the tenant's
     rules, then the trunks in the order they would be tried, each with
     its owner, health and — when a least-cost route placed it — the
     prefix and cost and whose table; the tenant's other trunks are listed
     as `not_offered`, so an empty answer explains itself. fcp-admin's
     `GET /v1/routing/preview` (`routing_rule_read`) asks the node
     `[call_manager] url` names, else a healthy, non-draining call manager
     from cluster membership, and answers 503 with the reason when it
     cannot (`RoutePreviewer`).
   - **Rules permission.** `routing_rule_read` / `routing_rule_manage`,
     held by tenant and telecom admins in every mode (read-only may read):
     an enterprise tenant admin bars premium numbers without trunk rights.
     A rule that routes to a trunk — written, turned into one, turned from
     one, or deleted — also takes `trunk_update`.
   - **The admin UI.** The Routing Rules page says how rules apply and
     tests a number (the preview: outcome, the decisions, the trunks in
     order with prices, what was not offered); its edits now clear a field
     emptied (they omitted it, and the API keeps what it is not sent — a
     pattern or a trunk target, once set, could not be removed); API errors
     are shown; a trunk group is free text, since a group can name the
     platform's trunks. A **Least-Cost Routes** page manages
     `/v1/lcr-routes`. The Trunks page shows each node's view of a trunk
     (health, whether it routes to it, its calls, its registration), the
     calls across nodes and the trunk's least-cost routes; `/v1/trunks` now
     names the node on each view, which it dropped.
   - **Found on the way:** with `[api] auth_enabled = false` no caller had
     an identity, so every endpoint that checks a permission — trunks,
     compliance, emergency and more — answered 500 while the rest of the
     API was open. A caller is now the platform when authentication is
     off, as the endpoints without a check already treated it.
   - **Tested**: the preview on a running node (a least-cost route, a deny
     rule, an unpriced number, 911); the admin's proxy against a stand-in
     node (what it is asked, its token, its refusal passed on); enterprise
     permissions; the per-node naming.

   **Phase 6 is complete, and with it the tenant trunks project.**

## 13. Open questions

Each with a recommendation; every recommendation was taken on 2026-09-16.
What that settles, in one place: the platform's TOML trunks stay and tenant
trunks are additive; a node reads tenant trunks from the store and
refreshes them on a new config-changed event for trunks; CPS and
concurrency stay per node; the lowest live node id that loaded a
registering trunk registers it; a tenant with no trunks falls back to the
platform's unless it opts out; an emergency call from a tenant with no
emergency-capable trunk uses the platform's; the record gains `trunk_id`
beside `trunk_name`; the SBC keeps platform trunks only and gains tenant
identification from the trunk a call arrived on; and `trunk_group` becomes
what a routing rule names in phase 6.

1. **Do platform trunks stay?** (a) Yes — TOML trunks remain the
   platform's, tenant trunks are additive (recommended: it is the only
   migration story that cannot break a running deployment); or (b) move
   them into the database under a platform tenant and drop `[[trunks]]`.
2. **Where does a node get tenant trunks?** (a) `TrunkStore` at startup,
   refreshed on a new `system.config_changed` for trunks with a periodic
   backstop (recommended — it is the pattern the device and location caches
   already use, and the call path stays off the database); or (b) read
   through a cache on every call.
3. **CPS and concurrency across a cluster.** (a) Per node, as today, with
   the limit documented as per node (recommended — shared counters on the
   call path are a Redis round trip); or (b) Redis counters per trunk.
4. **Which node registers a registering trunk?** (a) The lowest live node
   id that loaded it (recommended); (b) every node (today's behaviour for
   TOML trunks, wrong once trunks are shared); (c) a node named in the
   trunk.
5. **A tenant with no trunks.** (a) Falls back to the platform's, unless
   it sets `platform_fallback = false` (recommended); or (b) never falls
   back, and a tenant without trunks cannot call out.
6. **Emergency fallback.** An emergency call from a tenant with no
   emergency-capable trunk (a) uses the platform's emergency trunks
   (recommended — never fail a 911 call for configuration); or (b) fails,
   loudly.
7. **The record.** (a) Add `trunk_id` beside `trunk_name` (recommended —
   nothing that reads CDRs breaks); or (b) replace the name.
8. **The SBC.** (a) Keeps platform trunks only, and gains tenant
   identification from the trunk a call arrived on (recommended — the SBC
   is the platform's edge); or (b) loads tenant trunks too and routes by
   them.
9. **`trunk_group`.** (a) Becomes the unit a routing rule names in phase 6
   (recommended); or (b) stays decorative and rules name trunks.

## 14. Not in scope

Carrier contracts and rate import, per-trunk billing and margin, number
inventory per carrier, SIP options ping intervals per carrier's preference,
and anything that needs a second carrier relationship to test against.
