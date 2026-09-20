# Capacity and Survivability — Design

Status: **proposed** (2026-09-20). Chosen after the tenant trunks project
closed. The open questions in §12 have recommendations; nothing is built
until they are settled.

One document for three things a carrier-grade platform is expected to do and
this one does not: **know what it can carry**, **refuse what it cannot**, and
**leave service without dropping calls**.

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
max_register_cps_per_source = 0
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

The same sequence, with the same keys, is wired into the SBC, the IVR, the
voicemail server, the presence server and the WebRTC gateway. The conference
server keeps its own richer version (it migrates participants first) but takes
the shared `draining` state and `/ready` so an operator sees one thing
everywhere.

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
`docs/DEPLOYMENT.md`. CI runs one small profile for behaviour, not throughput:
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

Each has a recommendation; the design is settled when they are answered.

**Q1. Where does admission live?** *Recommend:* move `AdmissionController`
into `fcp-sbc-core` beside the HMR engine and `StirSigner`, which moved there
for the same reason in tenant trunks 3b. The SBC keeps its admin API over it.

**Q2. Does enforcing `[calls] max_concurrent_calls` break a deployment that
set it high and exceeded it?** *Recommend:* enforce it only when explicitly
set, warn at startup for one release naming `[admission]`, and keep the
default unlimited. A deployment that wrote a number asked for it; a
deployment that never touched it is not newly capped.

**Q3. Is the tenant limit cluster-wide from the start?** *Recommend:* no —
per node in phase 1 (documented as such), cluster-wide in phase 5, once the
harness can measure what a Redis lease costs on the INVITE path.

**Q4. What happens to calls still up at the drain deadline?** *Recommend:*
BYE both legs with a `Reason` header, complete the record with `node_draining`,
release counts. Silence would leave the far ends with zombie dialogs, which is
the defect being fixed.

**Q5. Do ordinary calls migrate off a draining node, as conference
participants do?** *Recommend:* no. A conference participant has one leg on
the node and a room that exists elsewhere; a B2BUA call has two anchored legs,
one of them often a carrier, and moving it means a re-INVITE through a new
node that carriers answer inconsistently. Wait for the call instead. Revisit
for gateway (browser) legs, where phase 5c already proved the swap.

**Q6. What does a draining registrar answer?** *Recommend:* a `200` with a
shortened expiry (default 60 s), not a refusal.

**Q7. Where does the harness live?** *Recommend:* a workspace member
`fcp-loadgen`, `publish = false`, depending on `fcp-testkit`; not part of any
release image.

**Q8. Does CI gate on capacity numbers?** *Recommend:* no. CI runs a small
profile for behaviour; absolute rates are measured on known hardware and
recorded in this document.

**Q9. Are emergency calls exempt from admission and admitted while
draining?** *Recommend:* yes to both, matching tenant trunks 4b.

**Q10. What key does registration rate limiting use?** *Recommend:* both — a
fast token bucket per source address (catching one looping device or a flood
from one NAT) and a slower one per AOR (catching one account across
addresses). siphon-rs's registrar takes a key function, so both are one
limiter with two keys.

## 13. Phases

One PR each, in order.

| Phase | What |
|---|---|
| 1 | **The limits that exist, enforced.** `AdmissionController` moves into `fcp-sbc-core`; the call manager (then the IVR, voicemail and presence servers) admits calls and registrations through it; `[admission]` section; `[calls] max_concurrent_calls` honoured as an alias with a warning; the tenant's `max_concurrent_calls` enforced per node; the registrar's rate limiter installed with both keys; 503 + `Retry-After` with a `Reason`; refusal counters by reason; load on `/status` |
| 2 | **Leaving on purpose.** Shared drain state in `fcp-cluster`; `POST`/`DELETE /drain` and a `/ready` route on every binary's health server; SIGTERM starts a drain; new calls refused, emergency admitted, REGISTER expiry shortened; `drain_wait_secs`, then BYE with `Reason` and a `node_draining` record; `node.draining`/`node.drained` events; drain badge in the admin UI; the rolling-restart procedure in the cluster runbook; test floor T6 |
| 3 | **The harness.** `fcp-loadgen` on `fcp-testkit`: the four profiles, the measurements, the JSON report, the budget flags; the small CI profile |
| 4 | **What the numbers say.** Run it on the dev VM: calls per second and concurrent calls to first refusal, latency percentiles, memory per call, registration storm behaviour. Fix what it finds — this is where the defects are — and record the numbers here, in `docs/DEPLOYMENT.md` and in the heartbeat as a published capacity budget, the shape video's already has |
| 5 | **Across the cluster.** Tenant concurrency and trunk channels leased in Redis with a per-node fallback; placement by headroom among serving nodes in region; load, headroom and limits on the admin UI's cluster and tenant pages |

Phase 1 stands alone: it is worth shipping even if nothing after it is built,
because it turns two advertised limits into real ones. Phase 2 is what makes
an upgrade free. Phases 3–4 are what let anyone say a number out loud.

## 14. Not in scope

Autoscaling and Kubernetes (the numbers from phase 4 are its input, not its
implementation); admission by media cost or codec; queueing refused calls
(`AdmissionAction::Queue` stays unimplemented); rescuing calls from a crash or
a partition; tuning guides for the kernel, the NIC or Postgres; and load
testing forge-media's mixers, which have their own benchmarks.
