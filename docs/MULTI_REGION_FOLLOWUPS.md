# Multi-Region — the leftovers

Status: **agreed** (2026-09-22), survey done, all four questions settled
by the user. Chosen after the edge security project closed.

`docs/MULTI_REGION.md` shipped three things in v0.25.86–88 and was honest
about what it left: each of its sections ends with a **Not covered**
list. This document is the survey of those lists — what is still true,
what turned out to be worse than the list says, and what is not worth
doing.

Read with `docs/MULTI_REGION.md` (what shipped),
`docs/DISTRIBUTED_CONFERENCING.md` (placement and inter-node trunks) and
`docs/CAPACITY_AND_SURVIVABILITY.md` (§5 admission, §6 draining).

## 1. What the survey found sound

Checked, not assumed:

- **The replica split is real and correctly drawn.** Configuration reads
  go to `self.reads()` — the replica while it is healthy — and writes and
  must-be-current reads go to `&self.pool`
  (`fcp-storage-postgres/src/lib.rs:474`). The lag probe, the
  read-after-write window and the fall back to the primary all work as
  described.
- **The hot path is cached.** Credentials, extensions and devices sit
  behind DashMap caches (`fcp-core/src/cached_storage.rs`, wired by
  `apply_cached_stores`), so the common call-path config reads do not
  reach Postgres at all, let alone another region.
- **Region-aware placement works and is the right shape.** A caller's
  region is preferred even when another region already hosts the room,
  and the two nodes trunk to each other once rather than dragging the
  media across an ocean.
- **A node with no region is local to nothing** — never preferred, never
  excluded — so a rolling upgrade does not strand traffic.

## 2. What is wrong today

### 2.1 Registrations are the one hot-path store with no regional story

Every other lookup a call makes is either replica-served or cached.
Registrations are neither: they are **primary-always** by design
(`docs/MULTI_REGION.md` §What goes where lists them first under
*Primary*), and nothing caches them.

So in a two-region deployment with the primary in one of them, an
ordinary call in the *other* region makes at least two cross-region round
trips before the callee's phone rings:

| Read | Where | Why |
|---|---|---|
| the caller's bindings | `fcp-sip/src/handler.rs:1762` | admission — is this INVITE from a device registered here (edge security §2.2) |
| the callee's bindings | `handler.rs:4919`, `:5017`, `:5087`, `:5992` | where to send the call |

That is the floor. A hunt group reads once per member
(`handler.rs:13137`), call pickup and park retrieval read the parker's
and retriever's bindings (`:12385`, `:12555`), and **domain aliases
multiply every one of these**: `get_registration_rows_impl` queries each
aliased domain in turn (`fcp-sip/src/registrar.rs:1520`+), so a
three-alias deployment triples the count.

At a transatlantic round trip of ~80 ms that is ~160 ms added to a
two-party call setup before anything rings, and more than half a second
for a five-member hunt group — on every call, paid by the region that
does not hold the primary.

**The Redis alternative exists and does not fix it.** There is a
`RedisRegistrationStore` with native TTL expiry
(`fcp-storage-redis/src/registration.rs`) and a
`registration_backend = "redis"` switch that swaps it into the split
backend (`fcp-call-manager/src/storage.rs:121`). But it reads
`storage.redis_url`, and that is the *same* URL cluster membership uses —
the call manager's `[cluster]` section has no `redis_url` of its own, only
`enabled` and `region`, and its documentation says "Requires
`storage.redis_url`" (`fcp-call-manager/src/config.rs:670`). Membership
requires that Redis to be **shared across regions** for nodes to see each
other at all (`docs/MULTI_REGION.md` §What it needs). So opting into
Redis registrations moves them from one global store to another global
store. It changes the latency constant, not the topology.

**And it is one binary.** `registration_backend` is read only by
`fcp-call-manager`. `fcp-registrar` and `fcp-sbc` both build an
`FcpRegistrar` and have no such option, so a deployment that puts
registration behind a standalone registrar has no choice at all.

### 2.2 Nothing gets a region's callers to that region's nodes

Named in `docs/MULTI_REGION.md` as not covered, and still true. Placement
keeps a call in the region of the node that received it — but what
decides which node receives it is DNS, anycast or a carrier's trunk
configuration, all of them outside the platform. There is no per-region
SRV story, no health-aware regional endpoint, and nothing in the admin UI
or API that tells an operator what to point where.

This is the difference between "the platform is region-aware" and "a
deployment is regional". Everything §1 lists as sound only pays off once
traffic arrives in the right place.

### 2.3 A partitioned region does not know it is partitioned

A region cut off from the shared Redis keeps serving the calls it has and
loses the cluster view as the other regions' nodes expire after 15 s.
Nothing distinguishes that from "the other regions went away", so:

- placement will happily choose local nodes for everything, which is
  correct behaviour by accident rather than by decision;
- `/status` and the admin UI show a shrunken cluster with no indication
  that this node's *view* is the thing that is broken;
- when the partition heals, nothing reconciles — the conference registry
  and the call registry simply start agreeing again.

Detecting a partition as such is cheap (a node that can reach Postgres but
not Redis, or that loses every remote region's nodes at once while its own
stay, is almost certainly the one that is cut off) and would let the
health surface say so.

### 2.4 Everything crosses regions on the event bridge

The `EventBridge` forwards each region's whole event stream to every peer
region, including `system.node_joined` / `node_left` — which is how a
remote events server's node list fills in, so it is not simply noise. But
there is no filter, so CDR-adjacent chatter, per-call events and
registration churn all cross the link at whatever volume the busiest
region produces.

`docs/MULTI_REGION.md` calls a per-peer filter "a small addition if
cross-region volume ever matters". It has not been measured, which is the
first thing to do rather than the last.

### 2.5 Regional affinity stops at conference placement

Parking, pickup and the call registry are cluster-wide by call id and do
not consider regions. In practice this is mostly fine — they are Redis
lookups, not media — but "mostly fine" is the same reasoning that left
registrations where they are, and it deserves a measurement rather than a
shrug.

## 3. What is not worth doing

Recorded so the next survey does not re-litigate them:

- **Primary promotion.** Explicitly the database's job — Patroni or the
  cloud provider's managed failover. The platform reconnects through the
  pool; writes fail while the promotion runs. Building our own would mean
  owning split-brain, and we would own it badly.
- **An active-active SQL layer.** CockroachDB was evaluated and dropped
  with reasons (`docs/MULTI_REGION.md` §Decisions). YugabyteDB is noted as
  the candidate if it is ever required; nothing since then has changed the
  argument.
- **Regional affinity for WebSocket event clients.** A client connects to
  the events server it is pointed at, which is a deployment concern, not a
  platform one — the same answer as §2.2 and it comes free with it.

## 4. The questions, settled (2026-09-22)

**Q1. Where do registrations belong?** — *Cache first, then judge.*
Registrations get the short-TTL cache credentials, extensions and devices
already have, and the decision about making them regional is deferred
until phase 1's numbers and phase 2's effect are known. The reasoning:
the regional answer is the right *shape* — `docs/MULTI_REGION.md`'s own
Decisions section already claims registrations are regional state — but it
is a large change resting on an estimate, and the cache is a small change
that produces the measurement. If the cache does not close the gap, §5's
decision point takes it up with evidence.

**Q2. Is regional ingress a feature or a guide?** — *A guide.* Getting
traffic into a region is DNS, anycast and carrier configuration, all of
which the operator already owns and none of which the platform can do
better from inside. A runbook chapter, not a subsystem.

**Q3. Detect a partition, or only make it visible?** — *Make it visible.*
A node that loses every remote region at once while still reaching
Postgres says so; nothing changes its behaviour because of it. Acting on a
suspected partition — refusing placement across a suspect link — is a much
larger promise than saying "my view of the cluster is probably wrong", and
the second is most of the value.

**Q4. Filter the event bridge?** — *Measure first.* A counter of events
and bytes forwarded per peer region, looked at, before any filter earns
its configuration surface.

## 5. Phases

One PR each, in order.

| Phase | What |
|---|---|
| 1 | **Measure.** A per-peer counter of events and bytes on the `EventBridge` (§2.4, Q4), and a count of registration reads per call setup (§2.1), so the cost is a number rather than my arithmetic. Nothing changes behaviour. |
| 2 | **The registration cache** (Q1). Registrations get the short-TTL cache the other hot-path stores have, invalidated on the node's own writes, with the TTL as the knob. Measured against phase 1. |
| 3 | **A partition is visible** (§2.3, Q3). A node that loses every remote region at once while reaching Postgres says so on `/status`, in the admin UI, and as an event. |
| 4 | **The deployment guide** (§2.2, Q2). A regional runbook: SRV layout per region, which Redis is global and which is regional, what to point a carrier at, what a partition looks like, and the registration cost a region away from the primary still pays. |

**Decision point after phase 2**, not a phase: if the cache does not close
the gap phase 1 measures, making registrations regional (per-region Redis,
a cross-region lookup for the calls that need it, and the same backend
choice for the standalone registrar and SBC) becomes its own project with
its own design. If it does close it, §2.1 is finished and that project is
never written.

## 6. Testing

- The two-node cluster harness (`fcp_testkit::cluster`) already starts
  nodes with regions; a two-*region* variant is the same harness with two
  Redis instances and a label each.
- Phase 1's measurements want a floor test that counts registration reads
  per call setup, which is a counter assertion rather than a timing one —
  timings on a loaded CI box prove nothing.
- Phase 3's partition is testable by pointing one node's Redis at a
  closed port mid-run, which the harness can do.
