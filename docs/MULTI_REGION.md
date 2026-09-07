# Multi-Region

How FCP is meant to run across regions, and what is in place.

## Decisions (2026-09-04)

- **Postgres stays the system of record, one writer.** A primary in one
  region with streaming replicas in the others. Writes cross regions;
  configuration reads stay local.
- **CockroachDB is dropped.** The compatibility pass was started and
  stopped: applied migrations cannot change (sqlx verifies their
  checksums), so every incompatibility would need a CockroachDB-only
  rewrite of the SQL kept forever; its serializable-only isolation needs a
  transaction retry loop the backend does not have; and its schema changes
  ran the storage suite ~50× slower. If an active-active SQL layer is ever
  required, YugabyteDB (which reuses the Postgres query layer) is the
  candidate to evaluate — untested.
- **Regional state lives in Redis and memory**: registrations, cluster
  membership, the conference registry, parking, the event bus. Membership
  is region-aware and the events server bridges events between regions
  (both below).

## Read replicas (done, v0.25.86)

Every binary's storage section accepts a replica:

```toml
[storage]
database_url = "postgresql://fcp:…@primary.us-east.internal/fcp"
read_replica_url = "postgresql://fcp:…@replica.eu-west.internal/fcp"   # this region's standby
max_replica_lag_secs = 5
```

### What goes where

**Replica** (while healthy): configuration reads — tenants, users
(credentials), extensions, devices, line bindings, phone numbers, config
templates, meetings, trunks, routing rules, queue configuration, sites,
resource lists, flows, geo-routing, and the config store's `get`/`list`.
These are the lookups on the call path, and they tolerate a few seconds
of staleness the same way the in-memory caches in front of them already
do.

**Primary**, always: every write, and every read that must be current —
registrations, CDRs, billing (balances, ledger, rate plans, settlement),
encryption keys, presence, queue runtime state, webhooks and delivery
logs, voicemail, porting, API tokens, provisioning tokens, audit logs, the
config store's version and audit reads, E911, compliance, traces.

### Read-after-write

For `max_replica_lag_secs` after a backend instance writes, its own reads
go to the primary, so a process that creates a tenant and reads it back
sees it. A *different* process (another node, another region) may not see
the change until the replica has replayed it. Admin tooling that reads
back through a different node right after a write should expect that, or
be pointed at a node without a replica.

### Health and failover

The backend probes the replica every 5 s: reachability, whether it is in
recovery, and how far behind the primary it is. Lag is zero when the
replica has replayed everything the primary has written (`pg_current_wal_lsn`
vs `pg_last_wal_replay_lsn`), else the age of the last replayed
transaction. Beyond `max_replica_lag_secs`, or when the replica is
unreachable, reads go to the primary; the transition is logged both ways.
A replica that is down at startup does not stop the node (the pool
connects lazily). A "replica" that is not in recovery — a primary given by
mistake — is used as-is, with one warning.

`PostgresStorage::replica_status()` returns the probe's view for health
endpoints (host, reachable, healthy, lag, limit).

### Not covered

- Failover of the *primary* (promotion) is the database's job — Patroni,
  or the cloud provider's managed failover. Nodes reconnect through the
  pool; writes fail while the promotion runs.
- Getting a region's callers to that region's nodes in the first place:
  DNS (SRV per region, or geo-DNS), anycast, or the SBC's trunk routing.
  Once a call is on a node, placement keeps it in that node's region
  (below).

## Region-aware membership (done, v0.25.87)

Every node that joins the cluster can say which region it is in:

```toml
[cluster]
enabled = true
region = "eu-west"      # or the FCP_REGION environment variable
```

The label is free-form and only has to agree across nodes (`us-east`,
`eu-west`, `dc-frankfurt` — whatever the deployment calls its sites). It is
**not** the geographic-routing `Region` (country/state/city of a caller or
number); it names infrastructure, not people. With neither the setting nor
the variable, a node advertises no region and is *local to nothing*: it is
never preferred and never excluded, so mixed fleets keep working during a
rollout.

### What it needs

Membership is the heartbeat record each node writes to the Redis in its
`storage.redis_url` (or `[cluster] redis_url`). For nodes in different
regions to see each other, that Redis has to be shared across regions —
one global Redis, or an active-active replicated one (Redis Enterprise,
KeyDB). Nothing on the hot call path reads it: heartbeats and discovery
run every 5 s, so cross-region latency to Redis is fine. A per-region
Redis gives each region its own cluster, which is also a valid (and
simpler) deployment; regions then never place across each other.

### What changes

- **The record** (`cluster:node:<id>`) carries `region` when set; older
  readers ignore the field, older writers omit it. `system.node_joined`
  events carry it too.
- **`ClusterManager`** knows its own region (`region()`) and answers
  `local_members()`, `members_in_region(r)` and `least_loaded_near(type)`
  — the least-loaded node of a type in this region, else anywhere.
- **Conference placement** (`choose_conference_node`) tries, in order: a
  node in the caller's region already hosting the room and under capacity
  (the fullest); the least-loaded conference node in the caller's region,
  *even when another region hosts the room with room to spare*; a hosting
  node under capacity anywhere; the least-loaded node anywhere. The
  caller's region is the region of the call manager placing the call. The
  second step is the point: a US caller into a room the EU already hosts
  lands on a US conference node, the two nodes trunk to each other once
  (`docs/DISTRIBUTED_CONFERENCING.md`), and the caller's media never
  crosses the ocean. Draining nodes are skipped as before. For a video
  meeting the "least-loaded" choice prefers the node with the most video
  budget left for the meeting's codecs (`docs/VIDEO_CONFERENCING.md`
  §9.3); region still comes first.
- **Admin API**: `GET /api/v1/cluster/nodes` lists each node's `region`
  and `draining` flag and takes `?region=eu-west`; `GET
  /api/v1/cluster/stats` adds `nodes_by_region` and
  `nodes_without_region`. The admin UI's cluster page shows both.

### Not covered

- Regional affinity for anything other than conference placement. Call
  parking, pickup and the call registry are cluster-wide by call id and
  do not care about regions. Queue and hunt-group ring targets are
  registrations, which are per Redis anyway.
- Fencing: a region cut off from the shared Redis keeps serving what it
  has and loses the cluster view (nodes expire after 15 s). Nothing
  detects a partition as such.

## Cross-region event fan-out (done, v0.25.88)

Each region's nodes publish to their own Redis (`events` Pub/Sub channel
plus the `event_stream` stream), so without help a region's events stay in
it: WebSocket clients, the events server's call/registration/node views,
and webhooks in the EU never hear about a US call. The events server now
bridges them.

```toml
[cluster]
region = "us-east"

[event_bus]
bus_type = "redis"
redis_url = "redis://redis.us-east.internal:6379"

[[event_bus.peer_regions]]
region = "eu-west"
redis_url = "redis://redis.eu-west.internal:6379"
# key_prefix = "fcp:"        # defaults to this server's redis_key_prefix

[[event_bus.peer_regions]]
region = "ap-south"
redis_url = "redis://redis.ap-south.internal:6379"
```

### How it works

`fcp_storage_redis::event_bus::EventBridge` runs one task per peer inside
the events server. Each task reads the local `event_stream` through its
own consumer group (`bridge:<peer region>`) and writes every entry that
**originated here** into the peer's Redis with the same `PUBLISH` +
`XADD` a local publisher would do — so the peer's nodes see it on Pub/Sub
and the peer's events server reads it from its stream, nothing special
on the receiving side.

The envelope around each event carries `origin_region` (what the
publishing node advertised; a bridge fills it in with its own region for
an unlabeled publisher) and `relayed`. A bridge only forwards entries
that are not `relayed`, and marks what it forwards. That is the whole
loop guard: **peer every region with every other region** (a full mesh)
and each event reaches each region exactly once. A chain (A → B → C) does
not forward A's events on to C.

Delivery is at-least-once, in order per region:

- A batch is forwarded, then acknowledged. If the peer is unreachable the
  same batch is retried with backoff (1 s → 60 s) until it goes through;
  reading pauses meanwhile, and the local stream keeps the backlog
  (`MAXLEN ~ 10000` — a peer down for longer than that many events loses
  the oldest).
- Entries a dead bridge instance read but never acknowledged are claimed
  (`XAUTOCLAIM`, idle > 60 s) at start and once a minute by whichever
  instance is alive, so a restart or a second events server in the region
  picks them up. Several events servers in one region share the per-peer
  groups: each entry is forwarded once.
- Nothing is deduplicated on the receiving side; a batch acknowledged
  after a crash can arrive twice, the same as for any stream consumer.

### What the receiving region does with them

The events server routes every stream entry by its `relayed` flag:

- **Local** entries go to the bus WebSocket clients, state tracking
  **and webhooks** subscribe to.
- **Relayed** entries go to a second bus that only WebSocket clients and
  state tracking read. Webhooks fire once, in the region the event
  happened in; a tenant's webhook endpoint does not get one delivery per
  region.

`GET /status` on the events server reports `region.local` and
`region.bridge` (per peer: connected, forwarded, failed, last error).

### Not covered

- Selecting which events cross regions: everything does, including
  `system.node_joined` / `node_left` of the origin region's nodes (which
  is how a remote events server's node list fills in). A per-peer event
  filter is a small addition if cross-region volume ever matters.
- Regional affinity for the WebSocket clients themselves: a client
  connects to whichever events server it is pointed at.
