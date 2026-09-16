# Tenant Trunks — Design

Status: **proposed** (2026-09-16). One document for the item E911 block 2
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
working. `TrunkConfig` in `fcp-sbc-core` becomes `From<&Trunk>`: the TOML
shape stays what an operator writes, and the router sees only `Trunk`.

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

**Failover** is unchanged in shape: the ordered list is tried in turn, and
the record names the trunk that answered.

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
   the trunk's transport and TLS verification.

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

1. **One model.** `fcp_core::Trunk` gains the missing fields;
   `TrunkConfig` is derived from it; TOML trunks load through the same
   model with stable ids; health, CPS, registration and the router key on
   `TrunkId`. No routing change: platform trunks only, as today.
2. **Tenant trunks route.** The provider takes a tenant; the node holds a
   `TrunkSet` refreshed from the database on `ConfigChanged`; selection is
   the tenant's trunks then the platform's; `trunk_id` on the records; the
   test floor routes two tenants to two carriers.
3. **The trunk's own behaviour.** Number transform, caller ID and
   from-domain, codecs, SRTP, DTMF, session timer, header rules, outbound
   proxy — applied, and tested on the wire.
4. **Inbound.** The trunk decides the tenant; per-trunk inbound
   authentication and limits; admission rewritten on the match.
5. **Cluster.** One node registers a registering trunk; health per node;
   failover across nodes; the events.
6. **Emergency and policy.** `emergency_capable` on tenant trunks closes
   block 2 §12.1 (with the platform's trunks as the last resort); routing
   rules gain a "route via trunk / trunk group" action; per-tenant LCR
   tables; the admin UI shows what routes and what each node sees.

## 13. Open questions

Each with a recommendation; none is settled until you say so.

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
