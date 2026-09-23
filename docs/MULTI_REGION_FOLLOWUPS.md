# Multi-Region — the leftovers

Status: **agreed** (2026-09-22), survey done, all four questions settled
by the user; **phase 1 done** (v0.25.231). Chosen after the edge security
project closed.

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

**Measured in phase 1** (`fcp-call-manager/tests/calls.rs`), against the
counter this phase added:

| Deployment | Reads per two-party call setup |
|---|---|
| no domain aliases | **2 direct**, 0 alias |
| two aliases beside the AOR's own domain | **2 direct + 4 alias = 6** |

So the estimate was right, and the alias multiplication is exactly what it
looked like: each of the two lookups is repeated at every other alias, so
two aliases treble the cost of every call. The counter is
`fcp_registration_reads_total{reason=...}` on `/metrics` and
`registration_reads` on `/status`, with housekeeping (REGISTER handling,
expiry, deregistration) counted apart so the call-path number is not
inflated by it.

**Cached in phase 2** (`fcp-call-manager/tests/registrations.rs`). The
call manager keeps each lookup's answer for `[registrar] cache_ttl_secs`
(default 30; zero is the old behaviour), empty answers included:

| Deployment | First call after the phones register | Every call after, within the TTL |
|---|---|---|
| no domain aliases | 2 round trips | **0** (2 answered from memory) |
| two aliases | 6 round trips | **0** (6 answered from memory) |

The first call still pays: a REGISTER empties the cache for its address.
What keeps a cached answer honest:

- **This node's writes** — a REGISTER, a re-REGISTER from a new address, a
  deregistration — invalidate the entry as the store takes them, so the
  next call here reaches the phone where it now is.
- **Another node's writes** arrive as registration events, and the node
  forgets the address on each. Across nodes that needs `[events]
  use_redis`; across regions, the events server's bridge. An
  administrator's removal through the API now publishes the same event.
- **The TTL** bounds what neither covers: a lost event, and a standalone
  `fcp-registrar`, which publishes none — behind one, a phone that has
  just registered can go unfound on a call manager for up to the TTL.
- A lookup that raced a write never puts the old rows back: each entry has
  a generation, an invalidation leaves a marker, and a fill lands only
  over the generation it saw before its read.
- Expiry is judged at every lookup, so a binding that lapses while cached
  is not used.

`/status` gains `registration_reads.cached` and a `registration_cache`
object (TTL, entries, hits, misses, evictions); `/metrics` gains
`fcp_registration_cache_hits_total`. The round-trip series keep their
meaning — trips to the store — so hits against round trips is the
cache's hit rate, read off a live deployment.

**What this does not settle.** The zero above is for a party looked up
again within the TTL. An ordinary phone called a few times an hour is
looked up cold on most calls and still pays the crossing; the cache pays
off for the hot addresses — agents, hunt-group members, receptionists —
and for the alias multiplication, which it removes whenever it hits. How
much of a deployment's traffic that is, is what `/metrics` now answers,
and what the decision point in §5 needs.

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

A region cut off from the shared Redis keeps serving the calls it has.
What it sees of the cluster depends on which side of the cut the Redis is
— and the survey got one side wrong (phase 3, below): a node that cannot
reach the Redis does *not* lose the other regions' nodes after 15 s; its
discovery fails and its member list stays exactly as it was, however old.
Either way, nothing distinguishes the partition from "the other regions
went away", so:

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

**Made visible in phase 3.** Each node's membership loop now keeps a view
of the cluster (`fcp-cluster/src/view.rs`) in one of three states:

| State | When | What it means |
|---|---|---|
| `whole` | nothing below | — |
| `store_unreachable` | a heartbeat-and-discovery tick fails, or takes longer than the heartbeat TTL | this node cannot see the cluster; its member list is `view_age_secs` old. What the cut-off side of one global Redis sees. |
| `remote_regions_lost` | a discovery finds no node outside this node's region, where the last one found some, and at least one of them went quiet within two heartbeat TTLs rather than shutting down | this region may be the one cut off, or the others may be down; from inside a node cannot tell. What the side holding the Redis sees — and, with an active-active Redis, both sides. |

A node with a database probe (the call manager and the conference server,
which read one tenant) also says, while the view is not whole, whether it
still reaches its database: reaching it but not the cluster is the
signature of the node being the one cut off; reaching neither points at
its own network.

It is said in four places, and acted on in none (Q3):

- **`/status`** on the call manager and the conference server:
  `cluster_view` — state, since, lost regions, view age, database
  reachability and a sentence.
- **`/metrics`**: `fcp_cluster_view{state=…}` and
  `fcp_cluster_view_age_seconds`.
- **The heartbeat record** carries the view while it is not whole, so
  `GET /api/v1/cluster/nodes` shows it per node, `/cluster/stats` counts
  `nodes_with_partial_view`, and the admin UI's cluster page badges the
  node and puts a banner over the page grouping the doubting nodes by what
  they lost. A node that cannot reach the store cannot write its record,
  so that state is on its own `/status` only.
- **`system.cluster_view_changed`**, on every change of state, with how
  long the previous state lasted. It goes on the node's local bus, so it
  reaches webhooks and the events server over HTTP even when the Redis is
  what is gone.

To tell a shutdown from a node going quiet, a node that stops cleanly now
leaves a `cluster:left:<id>` marker beside deleting its heartbeat, and
`system.node_left` says `shutdown` or `heartbeat_expired` accordingly. A
region taken down cleanly is therefore not reported as a partition. A
suspected partition is cleared when any node outside the region
reappears — or by a restart, which is the answer when a region has been
decommissioned without shutting its nodes down.

**Found on the way: a lost Redis connection took minutes to come back.**
The partition test cut the link to Redis and restored it, and the node
never recovered. `RedisStorage` built its connections with redis-rs 0.27's
defaults, which hand a `factor` of 100 to the backoff as its *multiplier*
from a one-second floor: after one reset the client retried after 1 s,
then every 60 s, and every command on the connection — membership, the
event bus, parking, everything sharing it — waited on the retry. A
two-second blip stalled a node for minutes. Every connection now retries
at most a second apart and gives up a connection attempt after two
seconds (`fcp-storage-redis/src/lib.rs`); the regression test
(`fcp-storage-redis/tests/reconnect.rs`) measures the recovery at about a
second after the link returns. A tick that hangs rather than fails — a
link that drops packets instead of refusing them — is now bounded by the
membership loop itself.

**Not changed, deliberately.** A node that cannot reach the store keeps
placing conferences on the member list it last saw. That is how it has
always behaved, and Q3 settled that a partition is said, not acted on;
`view_age_secs` is how old that list is, and the deployment guide (phase
4) is where what to do about it belongs.

### 2.4 Everything crosses regions on the event bridge

The `EventBridge` forwards each region's whole event stream to every peer
region, including `system.node_joined` / `node_left` — which is how a
remote events server's node list fills in, so it is not simply noise. But
there is no filter, so CDR-adjacent chatter, per-call events and
registration churn all cross the link at whatever volume the busiest
region produces.

`docs/MULTI_REGION.md` calls a per-peer filter "a small addition if
cross-region volume ever matters". It had not been measured, which is the
first thing to do rather than the last.

**Phase 1 measures it.** Envelopes were already counted per peer and shown
on the events server's `/status`; bytes were not counted at all and
neither was on `/metrics`. Both are now, as
`fcp_event_bridge_forwarded_total`, `fcp_event_bridge_forwarded_bytes_total`,
`fcp_event_bridge_failed_total` and `fcp_event_bridge_connected`, labelled
by peer region. Bytes are counted when a batch *lands*, not when it is
queued, so a retried batch does not make the number bigger the worse the
link is.

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
| 1 ✅ | **Measure.** A per-peer counter of events and bytes on the `EventBridge` (§2.4, Q4), and a count of registration reads per call setup (§2.1), so the cost is a number rather than arithmetic. Nothing changes behaviour. Result: **2 reads per call, 6 with two aliases** (§2.1). |
| 2 ✅ | **The registration cache** (Q1). Registrations get the short-TTL cache the other hot-path stores have, invalidated on the node's own writes and on other nodes' registration events, with the TTL as the knob. Measured against phase 1. Result: **a repeat call within the TTL costs 0 round trips, where it cost 2 — and 6 with two aliases** (§2.1). |
| 3 ✅ | **A partition is visible** (§2.3, Q3). A node that loses every remote region at once while reaching Postgres says so on `/status`, in the admin UI, and as an event. Result: three states — whole, store unreachable, remote regions lost — on `/status`, `/metrics`, the heartbeat record (admin API and UI) and `system.cluster_view_changed`; shutdowns told apart from silence; and a Redis reconnect that took minutes now takes about a second (§2.3). |
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
- Phase 3's partition is tested by putting a TCP proxy between one node
  and its Redis and cutting it mid-run (`fcp-cluster/tests/partition.rs`),
  and a region going quiet by stopping a node's heartbeat without a
  shutdown.
