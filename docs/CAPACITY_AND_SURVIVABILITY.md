# Capacity and Survivability — Design

Status: **complete** (2026-09-21), every phase shipped, every
recommendation in §12 taken. Chosen after the tenant trunks project
closed.

One document for three things a carrier-grade platform is expected to do
and this one did not: **know what it can carry**, **refuse what it
cannot**, and **leave service without dropping calls**. It does all three
now: every limit the API accepts is enforced on the call path, a tenant's
contract and a carrier's channels are counted across the cluster, every
routing binary drains without dropping a call, and what a node carries is
measured by a harness in this repository rather than asserted.

Read with `docs/TENANT_TRUNKS.md` (per-trunk limits, §9 observability),
`docs/DISTRIBUTED_CONFERENCING.md` (draining and participant migration, the
only place either exists today), `docs/TEST_FLOOR.md` (the phone the load
harness is built on) and `docs/VIDEO_CONFERENCING.md` §9.3 (the only measured
capacity budget in the tree).

## 1. Decisions

Proposed after surveying the call path end to end — the call manager's
configuration, the SBC's admission controller, the registrar, the cluster's
heartbeat, the shutdown path of every binary, and what reads each limit.

1. **A limit the API accepts is a limit the call path enforces.** The node's
   `[calls] max_concurrent_calls` and the tenant's `max_concurrent_calls` are
   accepted, stored, displayed — and read by nothing that places a call. Either
   they work or they go.
2. **One admission controller, in a library, for every routing binary.**
   `AdmissionController` moves from the `fcp-sbc` binary into `fcp-sbc-core`,
   the way the HMR engine did in tenant trunks 3b-2 and `StirSigner` in 3b-3.
   The call manager, the SBC, the IVR, the voicemail server and the conference
   server then admit calls by the same rules.
3. **Refusal is a 503 with `Retry-After`**, counted by reason, never a dropped
   packet or a hung INVITE. This is what the SBC already answers when it is
   over its admission limits.
4. **Emergency calls are exempt from every limit**, as they are from the
   per-trunk limits (tenant trunks 4b), and are admitted even on a draining
   node.
5. **A node that is leaving says so, then waits.** `set_draining(true)` in the
   cluster, `draining` on `/health`, a failing `/ready` so a load balancer
   stops choosing it, new calls refused, existing calls left alone until a
   deadline, and only then a BYE with a `Reason` header. The conference server
   already does this; the call manager does not.
6. **Ordinary calls do not migrate between nodes.** A B2BUA call has both legs
   anchored on this node; moving it means re-INVITEing a phone and a carrier
   through a new node. Draining waits for calls to end instead (§12 Q5).
7. **Capacity is measured, not asserted.** No number about calls per second or
   concurrent calls goes in a README until the harness in §8 has produced it on
   known hardware, and every default that caps anything is derived from one.
8. **Nothing refuses a call that is carried today.** Every limit defaults to
   off except where a deployment has already written one down, and every phase
   keeps a node that sets nothing behaving exactly as it does now.

## 2. What is wrong today

### 2.1 The limits that do nothing

- `[calls] max_concurrent_calls` (`fcp-call-manager/src/config.rs:429`,
  default 1000) is parsed, logged at startup
  (`config.rs:734`, `max_calls = config.calls.max_concurrent_calls`) and read
  by nothing else in the workspace. A call manager will accept the
  ten-thousandth call as readily as the first.
- `tenants.max_concurrent_calls` (`fcp-core/src/tenant.rs:96`) is stored
  (`fcp-storage-postgres/src/lib.rs:1363`), served and edited by
  `/v1/tenants`, shown in the admin UI — and read by nothing on the call path.
  A tenant's channel limit, the thing a UC contract is written in terms of, is
  decoration. This is the same defect that started the tenant trunks project:
  an API surface with no reader.
- The mid-dialog rate limits in `fcp-sip` (`info_rate_limit`,
  `reinvite_rate_limit`, `update_rate_limit`) are per dialog, so they bound one
  caller's mischief and nothing about the node's total load.

### 2.2 Admission exists, in one binary

`AdmissionConfig` (`fcp-sbc-core/src/admission.rs`) has `max_cps`,
`max_concurrent_calls`, `max_concurrent_registrations`, per-trunk CPS and an
`AdmissionAction`. `AdmissionController` (`fcp-sbc/src/admission.rs`)
implements it, exports `fcp_sbc_current_cps`, `fcp_sbc_peak_cps`,
`fcp_sbc_concurrent_calls`, `fcp_sbc_concurrent_registrations` and
`fcp_sbc_admission_rejections_total`, and is reachable at the SBC's admin API.

It is used by the SBC binary and nowhere else. The call manager — which is the
edge in every single-binary deployment, in `deploy/docker-compose`, and in the
quickstart — has no CPS limit, no concurrent-call limit, no registration limit
and no overload action. Its only real limits are the per-trunk ones from tenant
trunks 4b, which bound a carrier's channels, not this node's.

### 2.3 Registration floods are unprotected

siphon-rs's registrar takes an optional rate limiter
(`sip-registrar`: `with_rate_limiter`, `with_rate_limiter_key_fn`). FCP never
installs one — `with_rate_limiter` has no callers in `crates/`.
`fcp_core::transport::RateLimiter` (messages and bytes per window) has no
callers outside its own tests. A thousand phones whose registrar has just
restarted, or one misconfigured device retrying at 10 Hz, meet a registrar
with no defence but Postgres.

### 2.4 SIGTERM drops every call

The call manager's shutdown (`fcp-call-manager/src/main.rs:1773–1877`) breaks
out of the signal loop and then: removes its cluster heartbeat key
(`cm.shutdown()`), shuts the presence manager down and sends terminated
NOTIFYs, de-registers registering trunks (with a 4-second sleep), flushes the
CDR writer (5 seconds) and stops the listener.

Active calls are never mentioned. There is no draining state, nothing stops a
new INVITE arriving in those nine seconds, no BYE goes to either leg, and the
anchored RTP stops when the process does. Both far ends are left holding a
dialog until a session timer fires — or forever, for a leg that negotiated
none. The record for each call is written by the spool at hangup, which never
comes, so those calls end as dead spool entries rather than records.

For comparison, the conference server drains properly
(`fcp-conference-server/src/main.rs:1811–1860`): `set_draining(true)` to the
cluster, `start_draining()` on the handler, a `drain_wait_secs` deadline, then
hang-ups — and, since distributed conferencing phase 5, participants are moved
off by REFER before any of that (`migration.rs`). None of it is shared: it is
all conference-server code.

### 2.5 Nothing knows what a node can carry

- The cluster heartbeat already carries `active_calls` and `draining`
  (`fcp-cluster/src/membership.rs:20–40`), and a call manager publishes its
  count every 5 seconds (`main.rs:756–770`, Redis builds only). Nothing reads
  it except conference placement (`least_loaded_near`) and the admin UI's
  totals. There is no notion of headroom: 400 calls on a node means nothing
  without a capacity to compare it to.
- The only measured capacity in the tree is video's
  (`fcp-conference-server/src/video_capacity.rs`): a self-benchmark at startup
  produces a cost table and a saturation budget, published to the cluster and
  used to place rooms. Audio calls, registrations and signaling have no
  equivalent.
- There is no load generator, no benchmark and no measurement of any kind:
  no `criterion` dependency in the workspace, and `docs/TEST_FLOOR.md` puts
  "load or timing measurements" explicitly out of scope. T1–T5 prove the
  binaries are *correct* on one call at a time. Nobody has ever run two hundred.

### 2.6 What a node loss costs today

Not everything is missing. Registrations live in Postgres, so a surviving node
answers the next REGISTER and finds the same contacts; the CDR spool is on disk
and replays at restart (CDR pipeline phase 1); the call registry's keys expire
by TTL; the conference registry drops the dead node's rooms. What is lost in a
crash is exactly the calls the node was carrying, and that is irreducible for an
anchored B2BUA. What is *not* irreducible is losing them on a planned restart,
which is what happens now.

## 3. Goals and non-goals

**Goals**

1. Every limit the platform accepts is enforced on the call path, per node
   now and across the cluster where it belongs.
2. A node under more load than it can carry refuses new work clearly and keeps
   the calls it has.
3. A planned restart, upgrade or scale-down costs no calls: drain, wait, then
   end what is left with a BYE and a record.
4. The platform can be loaded on demand, by a harness in this repository, and
   what it can carry on given hardware is written down and re-measurable.
5. An operator can see load, headroom and drain state per node, in metrics and
   in the admin UI.

**Non-goals**

- Autoscaling policy, Kubernetes manifests or a scheduler (a separate project;
  this one produces the numbers such a thing would need).
- Kernel, NIC or Postgres tuning guides.
- Rescuing calls from a crashed node (§2.6) or from a network partition.
- Per-call QoS, bandwidth reservation or admission by codec cost.
- Load-testing the media stack's mixers beyond what a call path drives
  (forge-media has its own benchmarks for that).

## 4. The model

Three nouns, shared by every routing binary.

**Capacity** — what this node will carry, from configuration and, where a
self-benchmark exists, from measurement:

```toml
[admission]
max_cps = 0                        # 0 = unlimited
max_concurrent_calls = 0
max_concurrent_registrations = 0
max_register_cps_per_source = 50   # unset: 50; 0 = unlimited (#314)
overload_action = "reject"         # reject | queue (queue is future)
retry_after_secs = 5
```

The call manager's existing `[calls] max_concurrent_calls`, when a deployment
has explicitly set it, feeds `[admission] max_concurrent_calls`, with a
startup warning naming the new key (§12 Q2).

**Load** — what this node is carrying now: concurrent calls, calls started in
the last second (a sliding window, as the SBC's controller already keeps),
concurrent registrations, registrations per second per source. Published in the
heartbeat beside `active_calls`, exported on `/metrics`, shown on `/status`.

**State** — `serving` or `draining`. `draining` is set by `POST /drain`, by
SIGTERM, or by the cluster (a future scale-down controller); cleared by
`DELETE /drain`. It is published in the heartbeat (the field exists), returned
by `/health`, and makes `/ready` answer 503.

Per-tenant and per-trunk counts are not node state: they belong to the tenant
and the trunk, are counted per node in phase 1 and across the cluster in phase
5 (§7).

## 5. Admission: what a node accepts

On every INVITE that starts a call, after emergency detection and before any
routing work:

1. **Emergency?** Admit. No limit applies (decision 4).
2. **Draining?** Refuse: `503`, `Retry-After`, `Reason: SIP;cause=503;text="node draining"`.
3. **Node CPS or concurrent calls over the limit?** Refuse: `503` + `Retry-After`.
4. **The tenant over `max_concurrent_calls`?** Refuse: `503` +
   `Retry-After`, and a `tenant.capacity_exceeded` event so the tenant's
   operator can see it. (Billing's balance refusal answers `402` at admission
   already; capacity is not a payment problem.)
5. **The trunk over its channel or CPS limit?** Unchanged from tenant trunks
   4b, which already answers `503`.

On REGISTER, before any credential work — so a flood cannot drive Postgres:

1. Over `max_register_cps_per_source` for this source address, or over the
   per-AOR bucket? Refuse: `503` + `Retry-After` (§12 Q10).
2. Over `max_concurrent_registrations`? Refuse: `503` + `Retry-After`.
3. Draining? Admit, but answer with a shortened expiry (§6).

Every refusal increments a counter labelled by reason (`node_cps`,
`node_calls`, `tenant_calls`, `trunk`, `draining`, `register_rate`,
`registrations`), and is logged once per second per reason, not once per call.

## 6. Leaving: drain, wait, end

`POST /drain` on the health server, or SIGTERM, starts the same sequence:

1. `set_draining(true)` on the cluster manager — the heartbeat says so within
   one interval, and every placement rule that already skips draining nodes
   (conference placement) skips this one.
2. `/ready` answers `503`; `/health` keeps answering `200` with
   `"draining": true` so a liveness probe does not kill the process mid-drain.
   (`/ready` does not exist yet; this design adds it.)
3. New calls are refused with `503` + `Retry-After` (§5), except emergency
   calls, which are admitted and carried.
4. REGISTER is answered with `min(requested, drain_register_expiry_secs)`
   (default 60) so devices come back soon and DNS or the load balancer places
   them on a serving node. A draining registrar that refuses REGISTER loses
   the device until its own retry timer fires; shortening the expiry moves it
   politely.
5. Existing calls run untouched. Every second the node logs how many are left.
6. At `drain_wait_secs` (default 0 — today's behaviour; the compose and
   systemd examples set 300), each remaining call gets a BYE on both legs with
   `Reason: SIP;cause=200;text="node draining"`, its record completed with the
   termination reason `node_draining`, and its trunk and tenant counts
   released.
7. Then the existing shutdown runs: trunk de-registration, presence NOTIFYs,
   CDR flush, listener stop.

`DELETE /drain` before the deadline returns the node to `serving` — the
heartbeat, `/ready` and admission all follow. This is what makes a drain safe
to start by hand.

The same sequence, with the same keys, is wired into the SBC, the registrar,
the IVR, the voicemail server, the presence server and the WebRTC gateway —
from one implementation they all hold, not a copy each (§13, 2b-1). What
differs per binary is only the last step: what "end what is left" means to
it. The conference server keeps its own richer version (it migrates
participants first) but takes the shared `draining` state and `/ready` so an
operator sees one thing everywhere.

## 7. The cluster

**What a node advertises.** The heartbeat gains `capacity` (the configured
limits) and `load` (the current counts) beside `active_calls` and `draining`,
so headroom is computable by anything that reads membership: the admin UI's
cluster page, conference and room placement, and a future load balancer.

**Cluster-wide limits.** A tenant's `max_concurrent_calls` is a contract
across the platform, not per node, and a trunk's channel limit is the
carrier's, not this node's (tenant trunks 4b counts per node and says so).
Phase 5 counts both in Redis: a lease per call with a TTL longer than the
longest call, taken at admission and released at hangup, reconciled from the
node's own map on a tick, and falling back to the per-node count when Redis is
unreachable (the circuit breaker in `fcp-storage-redis` already models this).
The lease is one round trip on the INVITE path, which is why it comes after
the harness exists to measure what that costs (§12 Q3).

**Placement.** Where this platform already chooses a node — conference rooms,
queue dial-out, gateway sessions — the choice prefers the node with the most
headroom among serving nodes in the caller's region, rather than the fewest
absolute calls, which is what `least_loaded_near` does today.

## 8. The load harness

A new workspace member, `fcp-loadgen` (`publish = false`), a binary built on
`fcp-testkit`'s `Phone`, `Seed` and `Service` — the same scripted SIP phone
with real RTP that the test floor drives, run many at a time.

**Profiles** (`--profile`):

- `register-storm` — N devices register at once, then re-REGISTER on their
  expiry; measures answer latency and failures while the registrar is hit.
- `call-ramp` — offered CPS climbs a ladder until a stop condition is met
  (rejection rate, or p99 INVITE→180 past a bound); reports the last rung that
  held.
- `soak` — N concurrent calls with a tone each way for D minutes; reports
  RTP loss and jitter per leg, memory and CPU over time, and any call that
  ended by itself.
- `mixed` — a stated ratio of the three, which is what a real PBX sees.

**What it measures.** Offered vs admitted calls per second; INVITE→100,
INVITE→180 and INVITE→200 latency percentiles; failures by status code;
per-leg RTP loss, jitter and out-of-order counts from the testkit's media;
the node's RSS and CPU sampled from `/proc` when the harness started it; and
the node's own `/metrics` scraped each interval, so the node's view and the
caller's view are in one report.

**Output.** A JSON report and a one-page summary, plus `--budget` assertions
(`--max-p99-invite-ms`, `--min-admitted-cps`, `--max-rtp-loss`) so a profile
can be a pass/fail gate.

**Where it runs.** Real runs are manual, on the dev VM and — for video — the
GPU box, with the numbers recorded in §13's phase 4 notes and in
`docs/SINGLE-NODE-SETUP.md` §12. CI runs one small profile for behaviour, not throughput:
CI runners are too variable to assert a rate (`docs/TEST_FLOOR.md` and the
media flakes say as much).

## 9. Observability

Metrics, per node, from the shared controller — one family for every binary,
rather than the SBC's `fcp_sbc_*` names:

- `fcp_admission_calls_admitted_total{tenant}` / `…_refused_total{reason,tenant}`
- `fcp_admission_current_cps`, `fcp_admission_peak_cps`
- `fcp_admission_concurrent_calls`, `…_concurrent_registrations`
- `fcp_admission_capacity{limit}` — the configured limits, so headroom is a
  query, not a lookup in someone's TOML
- `fcp_node_draining` (0/1), `fcp_node_drain_calls_remaining`,
  `fcp_node_drain_byes_sent_total`
- `fcp_tenant_concurrent_calls{tenant}`

The SBC's `fcp_sbc_*` names stay as aliases for one release so existing
dashboards keep working.

Events: `node.draining`, `node.drained`, `tenant.capacity_exceeded`.

Admin UI: the cluster page gains load, headroom and a `draining` badge per
node; the tenant page shows concurrent calls against the tenant's limit.

## 10. Compatibility and defaults

- Every new `[admission]` key defaults to `0` — unlimited — so a node that is
  not reconfigured admits exactly what it admits today.
  *Amended in v0.25.525 (#314):* `max_register_cps_per_source` is the
  exception, 50 when unset. It bounds a flood, not a node's load: a refused
  device is told to come back in `retry_after_secs`, a 50-phone office
  booting behind one NAT registers within a few seconds, and the peers that
  carry many devices (`[sip] trusted_sources`, `[e911] trusted_edges`,
  trunks, cluster members) are exempt. A node whose only limit is this one
  publishes no capacity (§7). `0` turns it off.
- `[calls] max_concurrent_calls` set explicitly becomes a real limit, with a
  startup warning naming `[admission] max_concurrent_calls`. Left at its
  default, it caps nothing (§12 Q2).
- A tenant with `max_concurrent_calls = null` is unlimited, as now.
- `drain_wait_secs` defaults to 0: SIGTERM behaves as it does today unless a
  deployment asks for a drain. The compose files, the systemd units and the
  deployment docs set it, and `docs/CLUSTER_OPERATIONS_RUNBOOK.md` gains the
  rolling-restart procedure that depends on it.
- The SBC's `[admission]` section keeps its current keys and meanings; it is
  the section the others adopt.

## 11. Testing

- Unit: the controller's windows, limits and refusal reasons; the drain state
  machine; lease take/release including the Redis-down path.
- Database-backed: the tenant limit read through storage, and its refusal.
- Test floor (a new **T6**, `fcp-call-manager/tests/capacity.rs`): a node with
  `max_concurrent_calls = 2` carries two calls and answers the third `503`
  with `Retry-After`; a tenant at its limit is refused while another tenant is
  not; an emergency call is admitted past both; `POST /drain` refuses a new
  call, leaves an established call up, shortens a REGISTER's expiry, and the
  deadline sends a BYE that the phone sees, with a `node_draining` record at
  `/cdrs`; `DELETE /drain` restores service.
- Harness in CI: the `mixed` profile at a small size, asserting behaviour (no
  call fails that should not, no leak in the call map afterwards), not rate.

## 12. Open questions

Each with a recommendation; every recommendation was taken on 2026-09-20.
What that settles, in one place: admission is one controller in
`fcp-sbc-core` that every routing binary calls; a node limit applies only
where a deployment wrote one down, and the defaults cap nothing; a tenant's
limit is counted per node until phase 5 leases it across the cluster; a
drained node BYEs what is left at its deadline with a `Reason` and a
`node_draining` record; ordinary calls do not migrate between nodes, though
browser legs may later; a draining registrar answers `200` with a 60-second
expiry rather than refusing; the harness is a `publish = false` workspace
member and CI asserts behaviour from it, never a rate; emergency calls are
exempt from every limit and admitted while draining; and registration rate
limiting keys on the source address and the AOR at once.

**Q1. Where does admission live?** *Settled:* move `AdmissionController`
into `fcp-sbc-core` beside the HMR engine and `StirSigner`, which moved there
for the same reason in tenant trunks 3b. The SBC keeps its admin API over it.

**Q2. Does enforcing `[calls] max_concurrent_calls` break a deployment that
set it high and exceeded it?** *Settled:* enforce it only when explicitly
set, warn at startup for one release naming `[admission]`, and keep the
default unlimited. A deployment that wrote a number asked for it; a
deployment that never touched it is not newly capped.

**Q3. Is the tenant limit cluster-wide from the start?** *Settled:* no —
per node in phase 1 (documented as such), cluster-wide in phase 5, once the
harness can measure what a Redis lease costs on the INVITE path.

**Q4. What happens to calls still up at the drain deadline?** *Settled:*
BYE both legs with a `Reason` header, complete the record with `node_draining`,
release counts. Silence would leave the far ends with zombie dialogs, which is
the defect being fixed.

**Q5. Do ordinary calls migrate off a draining node, as conference
participants do?** *Settled:* no. A conference participant has one leg on
the node and a room that exists elsewhere; a B2BUA call has two anchored legs,
one of them often a carrier, and moving it means a re-INVITE through a new
node that carriers answer inconsistently. Wait for the call instead. Revisit
for gateway (browser) legs, where phase 5c already proved the swap.

**Q6. What does a draining registrar answer?** *Settled:* a `200` with a
shortened expiry (default 60 s), not a refusal.

**Q7. Where does the harness live?** *Settled:* a workspace member
`fcp-loadgen`, `publish = false`, depending on `fcp-testkit`; not part of any
release image.

**Q8. Does CI gate on capacity numbers?** *Settled:* no. CI runs a small
profile for behaviour; absolute rates are measured on known hardware and
recorded in this document.

**Q9. Are emergency calls exempt from admission and admitted while
draining?** *Settled:* yes to both, matching tenant trunks 4b.

**Q10. What key does registration rate limiting use?** *Settled:* both — a
fast token bucket per source address (catching one looping device or a flood
from one NAT) and a slower one per AOR (catching one account across
addresses). siphon-rs's registrar takes a key function, so both are one
limiter with two keys.

## 13. Phases

One PR each, in order.

| Phase | What |
|---|---|
| 1 ✅ v0.25.208 | **The limits that exist, enforced.** `AdmissionController` moves into `fcp-sbc-core`; the call manager (then the IVR, voicemail and presence servers) admits calls and registrations through it; `[admission]` section; `[calls] max_concurrent_calls` honoured as an alias with a warning; the tenant's `max_concurrent_calls` enforced per node; the registrar's rate limiter installed with both keys; 503 + `Retry-After` with a `Reason`; refusal counters by reason; load on `/status` |
| 2a ✅ v0.25.209 | **Leaving on purpose, on the call manager.** Drain state on the admission controller; `POST`/`DELETE /drain` and a `/ready` route; SIGTERM starts a drain and a second signal cuts it short; new calls refused, emergency admitted, REGISTER expiry shortened; `drain_wait_secs`, then a BYE with a `Reason` and a `node_draining` record; `system.node_draining`/`system.node_drained` events; test floor T6's drain half |
| 2b-1 ✅ v0.25.213 | **The shared drain, and the two nearest binaries.** The call manager's drain lifted out of `fcp-call-manager/src/{drain,health}.rs` into one place every routing binary can hold — `fcp-sip`, where the admission trait already lives — with the "end what is left" step supplied by the binary; then the SBC and the registrar on it: `[admission]` on the registrar, `POST`/`DELETE /drain`, `/ready` answering `503` while draining, the SIGTERM sequence with the second signal, and the shortened REGISTER expiry |
| 2b-2 ✅ v0.25.214 | **The application servers.** `[admission]` and the drain on the IVR, the voicemail server and the presence server — the three phase 1 left with no limits at all — each ending what it is carrying in its own terms: a caller inside a flow, a message being recorded, a subscription to expire |
| 2b-3 ✅ v0.25.215 | **The WebRTC gateway.** It has no health server at all, so: one, with `/health`, `/ready` and `/drain`; the `[admission]` section; and a drain that hands sessions to another node the way distributed conferencing's phase 5 migrates participants, rather than only refusing new ones |
| 2b-4 ✅ v0.25.216 | **The conference server, the badge and the runbook.** Its own drain — SIGTERM, `set_draining`, `start_draining()`, `drain_wait_secs`, REFER migration — aligned on the shared `draining` state and given `/ready` and `/drain`, without losing the migration; the drain badge and node state on the admin UI's cluster page; the rolling-restart procedure in `docs/CLUSTER_OPERATIONS_RUNBOOK.md` |
| 3 ✅ v0.25.217 | **The harness.** `fcp-loadgen` on `fcp-testkit`: the four profiles, the measurements, the JSON report, the budget flags; the small CI profile |
| 4 ✅ v0.25.218 | **What the numbers say.** Run it on the dev VM: calls per second and concurrent calls to first refusal, latency percentiles, memory per call, registration storm behaviour. Fix what it finds — this is where the defects are — and record the numbers here, in `docs/SINGLE-NODE-SETUP.md` §12 and in the heartbeat as a published capacity budget, the shape video's already has |
| 5a ✅ v0.25.219 | **A tenant's contract, across the cluster.** `fcp_cluster::CallLeases`: a lease per call in Redis, scored by expiry so a dead node's calls fade; `NodeAdmission::admit` becomes async and the controller takes the lease on the way in, falling back to this node's own count when Redis cannot be read; a settling tick gives back what has ended and refreshes what is still up |
| 5b ✅ v0.25.220 | **The rest of it.** Trunk channels leased the same way; the heartbeat's `capacity` and `load`; placement by headroom among serving nodes in region; load, headroom and limits on the admin UI's cluster and tenant pages |
| 5 (as written) | **Across the cluster.** Tenant concurrency and trunk channels leased in Redis with a per-node fallback; placement by headroom among serving nodes in region; load, headroom and limits on the admin UI's cluster and tenant pages |

Phase 1 stands alone: it is worth shipping even if nothing after it is built,
because it turns two advertised limits into real ones. Phase 2 is what makes
an upgrade free. Phases 3–4 are what let anyone say a number out loud.

*2b split into four on 2026-09-21*, after 2a showed what the work is. 2a
wrote the drain once, for one binary; six more copies of it would be the
wrong shape, and two of the six are not copies at all. The order puts the
extraction first, so nothing is written twice: 2b-1 decides where the shared
drain lives and what a binary supplies to it (the call manager supplies a BYE
on both legs and a `node_draining` record; the registrar supplies nothing but
the shortened expiry), and 2b-2 is then mechanical. 2b-3 and 2b-4 are the two
that are not: the gateway has no health server to add routes to, and the
conference server already has a drain of its own that migrates participants,
which the shared state has to accommodate rather than replace. `fcp-media-server`
stays out — it has no HTTP server, no signalling and nothing to drain.

*Phase 1 shipped 2026-09-20 (v0.25.208).* As designed, with three things
learned on the way.

**A section is not a field.** `[admission]` was first read straight into
`fcp_sbc_core::AdmissionConfig`, whose serde default is the SBC's — 100 CPS,
1000 calls, 5000 registrations. A call manager that wrote the section for one
limit therefore acquired the other four silently, against decision 8. The
node now has its own `AdmissionSection` of optional fields that resolve into
the controller's configuration, unset meaning unlimited; the SBC's own
defaults are untouched, since they are the SBC's.

**A challenged registration is two REGISTERs.** With `auth_enabled` on, a
device sends one REGISTER, is challenged, and sends another — so
`max_register_cps_per_source = 1` refuses every device on its second leg, and
nothing can ever register. Both legs are still counted, because an attacker
writes an `Authorization` header as easily as a phone does and the credential
lookup is what the limit protects; what changed is that the arithmetic is now
documented where the key is set, and the smallest useful limit is two.

**Zero now means unlimited** in `AdmissionConfig`, where it used to mean
"refuse everything" (`current >= 0` is always true). No deployment could have
wanted the old reading — a node configured to carry no calls — and the call
manager's defaults are zeros throughout.

What the phase did not do: the IVR, voicemail and presence servers keep no
limits yet. They are wired in phase 2, which touches every binary's health
server for the drain routes anyway, and none of them is an edge on its own.

What the phase added, beyond the design: `TrunkSet` learnt the tenants'
`max_concurrent_calls` on the refresh tick it already spends reading every
tenant, so no new task and no per-INVITE query; `Seed::tenant_call_limit` in
`fcp-testkit`; and `fcp-call-manager/tests/capacity.rs`, which is T6's first
half — the drain half arrives with phase 2.

*Phase 2a shipped 2026-09-20 (v0.25.209).* The call manager leaves service
without dropping calls: `POST /drain` or SIGTERM tells the cluster, refuses
new calls with a `503`, keeps the calls already up, and at
`[service] drain_wait_secs` ends what is left with
`Reason: SIP;cause=200;text="node draining"` and a record whose cause is
`node_draining`. `/ready` answers `503` while draining so a load balancer
stops choosing the node, while `/health` keeps answering `200` so a liveness
probe does not kill the process mid-drain. `DELETE /drain` returns it to
service with every call it was carrying untouched. A device that registers
while the node drains gets at most `drain_register_expiry_secs` (60), so it
comes back soon and is placed somewhere that is serving.

Four things learned:

**A BYE could not say why.** `sip-uac` had no way to put a header on a BYE,
so the far end could not tell the platform's hangup from a party's.
siphon-rs v2026.09.20 (sip-uac 0.7.5) adds `bye_with_headers` and
`bye_via_flow_with_headers`, which refuse to overwrite a header the dialog
owns.

**A second signal has to mean something.** A node with a long
`drain_wait_secs` and a call still up ignores SIGTERM for as long as the wait
lasts — which is right until an operator wants it gone, or a container
runtime's grace period runs out and SIGKILL takes the calls with no BYE and
no record. A second SIGTERM or SIGINT now ends what is left immediately, with
the same BYE and the same record.

**The `Expires` header is not the only place a device asks.** Shortening a
registration means the header *and* each Contact's `expires` parameter, and
a contact that names its own expiry does not need the header at all
(RFC 3261 §10.2.4) — the first cut of this rewrote registrations that were
already short. A deregistration (`Expires: 0`, `expires=0`, or `Contact: *`)
is answered exactly as it arrived: a device leaving must not be handed a
registration.

**The drain state belongs where the refusals are.** The design put it in
`fcp-cluster`; it went on the admission controller instead, which every
routing binary already holds from phase 1 and which is where a refused call
is counted. The cluster advertisement is still `ClusterManager::set_draining`,
which the conference server has used since distributed conferencing phase 4.

*Phase 2b-1 shipped 2026-09-21 (v0.25.213).* The drain is one
implementation — `fcp_sip::drain` — that every routing binary holds, and the
SBC and the registrar are on it.

What a binary supplies is a `DrainTarget`: what it is carrying, how to end
what is left, and how to tell the cluster. The call manager carries calls and
ends them with a BYE on both legs; the SBC carries the calls it is relaying
and ends them the way its admin API's `terminate_call` does, record first;
the registrar carries the devices registered with it and ends nothing at all
— a registration that is not renewed lapses, and by then the device has
registered on a node that is serving. `/ready`, `POST /drain` and
`DELETE /drain` answer the same JSON everywhere, from the same functions,
while each binary's router keeps its own shape. Every binary's `/drain` sits
behind its operator token — the call manager's, IVR's, voicemail server's,
presence server's and registrar's `[http] api_token`, the SBC's
`--health-api-token`, the conference server's `[http] auth_token`, the
gateway's `[auth] tokens` — and a non-loopback bind is refused without one;
`/ready` and `/health` never ask for it. (The registrar's was open until
v0.25.237, `docs/DEPLOYMENT_GAPS.md` §1.3.)

Three things learned:

**A drain over HTTP never told the cluster.** In 2a the advertisement was a
line in the SIGTERM path rather than part of the sequence, so `POST /drain`
refused new calls locally while the heartbeat still said this node was
serving — and conference placement went on choosing it. It is now the
target's `advertise`, which the sequence itself calls, so both ways in say
the same thing. A node that stops without draining says it too.

**One REGISTER gate, not two.** The refusal and the shortened expiry lived in
the call manager's handler, so the standalone registrar had neither. Both now
run `fcp_sip::admission::register_gate`, which refuses a flood with `503` and
`Retry-After` and shortens what a draining node grants. The registrar picked
up two things with it: it passes the source address to the binding, as the
call manager has since E911 block 2 phase 3, and `[admission]` is a section
it reads at last.

**`AdmissionSection` was the call manager's.** Phase 1 gave the call manager
its own optional-field section so that writing `[admission]` for one limit
did not silently import the SBC's defaults for the rest. The registrar needed
the same section, and 2b-2 needs it three more times, so it moved into
`fcp-sbc-core` beside the controller it configures.

What this phase did not do: the SBC's `/ready` still reports trunk health as
advisory rather than refusing readiness on it, since nothing asked for that
and a node with no healthy trunk can still answer in-dialog requests.

*Phase 2b-2 shipped 2026-09-21 (v0.25.214).* The IVR, the voicemail server
and the presence server read `[admission]`, answer `/ready` and `/drain`,
and drain on SIGTERM — and each ends what it is carrying in its own terms.

**The B2BUA target was written three times over, so it moved.** The IVR and
the voicemail server carry calls exactly as the call manager does: a
`SipMessageHandler`, a `CallManager`, a BYE on both legs with the drain
`Reason` and a `node_draining` record. That target is now
`fcp_sip::drain::NodeCalls`, which all three hold, and
`fcp-call-manager/src/drain.rs` is gone. What `fcp-sip` cannot hold is the
heartbeat — `fcp-cluster` is not a dependency of it — so the advertisement
is a `DrainAnnouncer`, implemented once as `fcp_sbc_core::ClusterAnnouncer`
beside `ClusterOwnership`, which is already how a binary hands its cluster
manager to a shared library.

**A message outlives the call it belongs to.** A voicemail recording ends
when the recorder notices the caller is gone — it looks once a second — and
only then is the file closed, the message row written and the MWI sent. A
node that hung up its callers and stopped would lose every message already
spoken. So `VoicemailService` counts what it is recording
(`messages_being_recorded`), and the voicemail server's target hangs up and
then waits, up to ten seconds, for that count to reach zero. It is the one
place in 2b-2 where "end what is left" is not over when the BYE is sent.

**A watcher is a binding, like a registration.** The presence server drains
the way the registrar does rather than the way the call manager does: it
grants short subscriptions while draining — the same
`[admission] drain_register_expiry_secs`, so the trait method is now
`drain_expiry_cap` rather than `register_expiry_cap` — and the watchers
leave on their own as they refresh onto a serving node. Whatever is still
subscribed at the deadline gets the `terminated;reason=probation` NOTIFY
this binary has always sent at shutdown, which is now `end_remaining` rather
than a line in the shutdown path, so `POST /drain` sends it too.

Found on the way: **an unsubscribe was being granted a subscription.** A
SUBSCRIBE with `Expires: 0` had its expiry clamped into the configured range
before anything looked at it, so `min_subscription_expiry_secs` (60 by
default) turned "I am leaving" into a minute more, and neither unsubscribe
path could ever run. Zero now stays zero — the same rule the register gate
keeps for a deregistration.

*Phase 2b-3 shipped 2026-09-21 (v0.25.215).* The gateway refuses,
advertises and ends like every other node — and, alone among them, hands
its browsers to a peer.

**A browser cannot be moved by the platform.** A conference participant
has one leg here and a room that exists elsewhere, so the node can REFER
it away (distributed conferencing phase 5), and a gateway's *SIP* leg can
be swapped under a live peer connection (phase 5c). A gateway's browser is
the other way round: the peer connection, the SIP leg and the relay
between them are all in this process, and nothing outside it can take them
over. So the browser has to join again somewhere else, and all a draining
gateway can do is tell it where — which it does through the session status
the browser is already polling every two seconds:

```json
{ "id": "sess-…", "state": "connected",
  "handover": { "reason": "node_draining", "url": "https://gw2.example.com" } }
```

The built-in join page acts on it: it joins that gateway with the same
room, name and join token, answers the new offer with the same camera and
microphone (a share in progress follows by `replaceTrack`), and only then
leaves the old session — so the room is never without it. A move that
fails, because the join token has expired or the peer will not take it,
leaves the old session exactly as it was: it plays on until the deadline.

**Where a browser goes is not in the heartbeat's addresses.**
`ClusterMember.http_address` is a bind address — usually `0.0.0.0` — and
useless to a browser, so the heartbeat gained `public_url`
(`[http] public_url`, `#[serde(default)]`, absent everywhere else).
`fcp_cluster::choose_handover_node` picks among peers of the same node
type that are serving, are not this node, and advertise one: in this
node's region if any is there, least loaded first — and "least loaded" now
means something for a gateway, which advertises its live session count in
the heartbeat as the call manager advertises its calls. With no cluster, or
no peer that qualifies, the `url` is simply absent and the browser re-joins
where it is — which behind a load balancer whose health check reads
`/ready` is a serving node, and without one is the node that is leaving.
That is the honest limit of a handover, and it is why `public_url` is
worth setting.

**Rooms move; calls to a phone do not.** A browser on `/v1/calls` has a
SIP phone at the far end and nothing that holds it but this node's dialog;
re-dialling from another gateway would be a second call to a person who is
already on one. Those sessions are waited for and hung up at the deadline,
like a B2BUA call (§12 Q5).

Two smaller things: the gateway has no health server of its own, so
`/ready`, `/status` and `/drain` joined its own router — `/ready` open,
since a load balancer reads it without a credential, the other two behind
the operator token — while forge-api's `/health` stays exactly as it was,
answering `200` through a drain. And a session that ends lingers in the
list for a minute so a browser can read why; `carrying()` counts only live
ones, or a drain would wait out that minute with nothing left to carry.

*Phase 2b-4 shipped 2026-09-21 (v0.25.216).* The node that had a drain
before the platform did is on the platform's, and phase 2 is done: every
routing binary answers `/ready` and `/drain`, and leaves service the same
way.

**What the conference server kept is the part that is its own.** Before
anyone is waited for, it asks the holder of each caller's leg to move that
caller to another node (`docs/DISTRIBUTED_CONFERENCING.md` phase 5). That
needed one addition to the shared sequence: `DrainTarget::on_drain_start`,
run once new work is refused and before the clock starts, so the wait is
for whoever could not be moved. A drain cut short — `end_now`, a second
signal — skips it, because an operator who will not wait will not wait for
a migration either.

**What it gave up is having a drain of its own.** Its `draining` flag is
gone; `is_draining()` is the node's admission's answer, so `POST /drain`
refuses INVITEs exactly as SIGTERM did, `DELETE /drain` lifts the refusal
(which the old flag could not do, having no way back), and a refusal now
carries `Retry-After` like every other one on the platform. `[admission]`
is a section it reads at last, and `end_remaining` hangs the remaining
callers up with a `Reason` rather than calling the handler's `shutdown()`,
which tears the rooms down — right for a process that is stopping, wrong
for a node that has just been told to serve again.

**The operator's two views.** The admin UI's cluster page shows a
`draining` badge beside each node's status and a count of draining nodes,
from the `draining` field the heartbeat has carried since distributed
conferencing phase 4 and nothing had ever displayed. And
`docs/CLUSTER_OPERATIONS_RUNBOOK.md`, which until now said FCP had no
drain mode and that a rolling restart drops calls, has the procedure: set
`drain_wait_secs`, `POST /drain`, watch `/status`, stop, restart, check
`/ready` — with a table of what each kind of node does with what it is
carrying, and the load balancer pointed at `/ready` rather than `/health`.

*Phase 3 shipped 2026-09-21 (v0.25.217).* `fcp-loadgen` is a workspace
member (`publish = false`) that drives `fcp-testkit`'s scripted phones —
real SIP, real RTP — many at a time, and reports what the callers saw
beside what the node said about itself over the same seconds.

The four profiles are as designed, with one correction from writing them:
**a soak holds a population, not a rate.** `--users` callers each keep one
call up, hang it up after `--call-seconds` and place another, so the node
is asked to carry N calls rather than to accept N per second — which is
what memory per call and media quality over time are measured against. The
ramp is the profile with a rate, and it stops at the first rung that
refuses too many, answers too slowly or drops a call; the last rung that
held is the node's number.

Two things the harness needed from the testkit, both worth having anyway:
a leg now reports **what the stream sent to it looked like on arrival** —
RFC 3550 jitter, loss from the sequence numbers, reordering and duplicates
(`MediaLeg::stream_stats`) — and a call records **when its milestones
happened** (`Call::timings`), so INVITE→100, →18x and →200 are measured
from the caller's own socket rather than inferred.

Two defects the harness found in its own first run against a real node,
which is the point of building it:

- **Registrations were being counted as answered calls.** One outcome map
  held both, so a storm's `200`s inflated the answered rate. They are
  counted apart now, and the summary prints them apart.
- **Nobody was speaking back.** The answering phones auto-answered and
  played nothing, so every caller's leg reported 100 % loss and the
  harness would have reported a node with perfect media as broken. The
  answering side now plays a tone for as long as the caller holds the
  call, and both legs are measured: the same run then read 0 % loss and
  0.3 ms of jitter through the B2BUA's relay.

What CI runs is `tests/ci_profile.rs`: a six-second soak and a five-second
storm against a real call manager, asserting behaviour only — every call
answered, both legs of every call hearing audio, every REGISTER answered,
and the node sampled for memory and metrics — never a rate (§12 Q8).

*Phase 4 ran 2026-09-21 (v0.25.218).* The harness was pointed at a
release-built call manager and the numbers written down. What it found
first was four defects in itself, which is the ordinary way of these
things, and then a clear account of what this hardware can say.

**The machine.** A Microsoft Azure VM, Intel Core Ultra 9 275HX, 12
cores, **2 GB of RAM**, running Postgres, the node *and* the harness at
once. That is a development box, not a capacity lab, and the numbers
below are bounded by it in ways the notes say plainly.

### What the node did

| Measured | Result |
|---|---|
| Call ramp, 2 → 20 calls/s, 60 pairs, 10-second calls | **every rung held**: 2208 of 2210 answered, refusals 0.1 % |
| INVITE → `200 OK` at 20 calls/s | p50 **4 ms**, p90 4 ms, p99 **7 ms**, max 504 ms |
| INVITE → first provisional | p50 **0 ms**, p99 0 ms |
| Media through the B2BUA relay, 4419 legs | loss **0.00 %**, jitter mean **0.37 ms**, max 0.64 ms, 3 silent legs |
| Node memory, 100 calls up | idle **34 MiB**, peak **64 MiB** → **≈0.3 MiB a call** |
| Node CPU at 20 calls/s | **3 %**, peak 28 % |
| Register storm, 300 devices, four rounds | 1200 registrations, **every one `200`**, none refused |

The ramp **did not find the node's ceiling**: it ran out of its own
five-minute budget at 20 calls/s with a p99 of 7 ms and the node at 3 %
CPU. Twenty calls a second is a floor, not a limit, and no number here
should be quoted as this platform's capacity.

### What the harness's own limit is

The register storm read p50 1525 ms, p99 3536 ms at 300 phones — and the
node was at **1 % CPU** while answering every one. Running the same storm
at three sizes settles where that time goes:

| devices (phones in one harness process) | REGISTER p50 | p99 | node CPU |
|---|---|---|---|
| 15 (30) | 10 ms | 43 ms | 1 % |
| 50 (100) | 18 ms | 37 ms | 1 % |
| 150 (300) | 1525 ms | 3536 ms | 1 % |

An 85-fold jump in p50 between 100 and 300 phones, with the server idle
throughout and refusing nothing, is the client saturating: one process
driving 300 SIP endpoints with their sockets and timers on a 2 GB VM. So:
**keep a run under about 100 phones per harness process on hardware like
this, and put the harness on a different machine from the node before
quoting any number.** The soak's anomalies at 100 pairs — 10 % of calls
answered `480`, 196 silent legs, an answer p99 of 3.5 s — sit past that
line and are consistent with the same saturation; the ramp, at 60 pairs,
was clean.

### The four defects it found in itself

- **Registrations lapsed mid-run.** Phones registered once, for 60
  seconds, and a ramp outlives that: from the third rung every call came
  back `480`, which read exactly like a capacity cliff at six calls a
  second. A refresher now re-registers every phone at half its lifetime,
  as a device does, and the ramp climbed to 20 calls/s unbroken.
- **A rung's rate counted the drain after it.** `offer_calls` waits for
  the last call it placed to end, so a 20-second rung at 2 calls/s
  reported 1.4. A rung is now measured over the window it offered in.
- **A run's rate counted its own setup.** Starting and registering the
  phones was in the denominator; the load has its own clock now.
- **A refused caller spun.** A soak places another call as each one ends;
  when they are refused instantly, "as each one ends" is immediately, and
  100 held calls became 796 offered. A refused caller now waits a second,
  as a person would.

### Left open

Two calls in 2210 were answered `480` at 8 and 16 calls/s — 0.09 %, both
while their callee's registration was being refreshed. The binding write
is an upsert (`ON CONFLICT … DO UPDATE`), so there is no delete-and-insert
window to blame, and no mechanism has been shown; it is recorded here with
its evidence rather than diagnosed from a guess. Likewise three silent
legs in 4419 (0.07 %).

**What phase 4 did not do:** publish a capacity budget in the heartbeat.
§7 wants `capacity` and `load` there so headroom is computable, but the
readers — placement by headroom, the admin UI's headroom column — arrive
in phase 5, and this platform's own history (the limits that did nothing,
§2.1) argues against shipping a field with no reader. It goes with its
reader.

*Phase 5a shipped 2026-09-21 (v0.25.219).* A tenant's
`max_concurrent_calls` is counted across the cluster, so the contract is
with the platform rather than with whichever node the call reached.

**The lease.** One sorted set per subject in Redis — `leases:tenant:<id>`,
and `leases:trunk:<id>` ready for 5b — holding call ids scored by when
the lease expires. A call takes its lease as it is admitted and gets back
the cluster's count with itself included, so two nodes admitting at once
cannot both believe they were under the limit. Taking the same call twice
counts once, which makes a retried INVITE safe. The score is what keeps
it honest without a caretaker: a node that dies cannot release anything,
so every read drops what has expired before counting, and a node refreshes
what it still holds on a tick — a long call keeps its place, a dead node's
calls fade.

**`admit` is async now.** The lease is a round trip on the INVITE path,
which is why §12 Q3 put this phase after the harness existed to measure
it. What that cost the trait was one `#[async_trait]`; what it cost the
call path is measured below.

**What it costs, measured** (§12 Q3's precondition). The same ramp, twice,
against the same node — once counting per node, once leasing across the
cluster, with Redis on loopback:

| | INVITE → `200 OK`, 302 calls |
|---|---|
| per-node count | p50 3.8 ms, p90 6.0 ms, p99 22.1 ms |
| cluster-leased | p50 3.8 ms, p90 5.4 ms, p99 8.4 ms |

The round trip is below what this measurement can see: the medians are
identical and the p99 difference runs the wrong way, which makes it noise
rather than a cost. What it does not measure is Redis across a network —
there the lease costs whatever that Redis costs, and a deployment whose
Redis is far away should watch that latency rather than trust this table.

**Redis being away must never fail a call.** The count then falls back to
this node's own — which is exactly what phase 1 enforced, so a cluster
that loses Redis degrades to per-node limits rather than to none, and
says so once rather than per call.

**A call ends in a callback that cannot await** (the call manager's
`set_on_ended`), so a released lease is queued and given back by the
settling tick, and also released at once where there is a runtime to
spawn on. Releasing twice is harmless; never releasing is covered by the
lease's expiry. The queue is bounded: what it drops lapses on its own.

`[cluster] lease_tenant_calls` is on wherever a cluster is configured —
the limit becomes correct rather than new, which decision 8 allows — and
`lease_settle_secs` (5 s) paces the tick.

*Since v0.25.407* a lease stands for `[cluster] lease_ttl_secs` (60 s)
rather than three hours, refreshed every quarter of it, and the settling
tick runs at least that often. Three hours was "longer than any call",
but it was also how long a node killed mid-call went on taking its
tenants' places and keeping its users busy (`docs/DEEP_TESTING.md` D5).
A user's call (call waiting, `max_calls`) and a shared line's appearance
take the same time.

*Phase 5b shipped 2026-09-21 (v0.25.220), and with it the project.*

**A carrier's channels are the platform's to fill.** `TrunkCallCounter`
is async now and a trunk's channels take leases like a tenant's calls —
`leases:trunk:<key>`, the subject 5a left ready. An inbound call takes
one at admission; an outbound leg the trunk answered takes one when it is
answered, since it fills a channel just the same. Redis unreadable falls
back to the per-node count, which is what tenant trunks 4b did and said
it did.

**The heartbeat says what a node will carry and what it is carrying**, at
last with the reader that justifies it: `capacity` on `ClusterMember`
(the `[admission]` limits, plus registrations and calls per second as
they stand) beside the `active_calls` that was already there.

**Placement prefers room, not idleness.** A node carrying ten calls of a
hundred has more to give than one carrying two of four, and that is now
the order — for conference rooms, for the least-loaded queries the queue
and gateway use, and for the gateway's handover. Three rules keep it
honest:

- a node with no room left is chosen last;
- a node that advertises no limit has **unknown** room, not infinite: it
  is ranked on its calls, exactly as every node was before capacity was
  published, so a cluster where nothing advertises behaves as it did;
- ties fall to the fewest calls and then to the node id, so two choosers
  pick the same node.

**The operator sees it.** The admin API's cluster view carries `capacity`
and `calls_left`, and the cluster page has a Headroom column: what a node
will still take against what it allows, with its calls per second and
registrations under it, and "no limit" where a node advertises none.

## 14. Not in scope

Autoscaling and Kubernetes (the numbers from phase 4 are its input, not its
implementation); admission by media cost or codec; queueing refused calls
(`AdmissionAction::Queue` stays unimplemented); rescuing calls from a crash or
a partition; tuning guides for the kernel, the NIC or Postgres; and load
testing forge-media's mixers, which have their own benchmarks.
