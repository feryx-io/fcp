# Running FCP in several regions

A runbook for deploying FCP across regions: the one decision to make
first, what each region runs, which Redis is shared and which is not, how
callers and carriers reach the right region, what a region away from the
database primary still pays, and what a partition looks like when it
happens. The mechanisms behind it are in `docs/MULTI_REGION.md`; the
survey that led here is `docs/MULTI_REGION_FOLLOWUPS.md` (phase 4, Q2:
regional ingress is a guide, not a feature).

Everything below is what the code does as of v0.25.234. Where the code
constrains a choice, the constraint is named with its source.

## 1. The decision: one cluster, or one per region

**A call manager has one Redis.** `[storage] redis_url` is the only Redis
setting it reads, with the key prefix fixed at `fcp:`
(`fcp-call-manager/src/storage.rs`, `initialize_redis`), and everything
that needs Redis shares it: cluster membership, the conference registry
used for placement, parking, cluster-wide call and trunk leases, the
event bus (`[events] use_redis`), the CDR stream (`[cdr] sink = "stream"`),
presence Pub/Sub, and registrations when `registration_backend = "redis"`.
The conference server and the WebRTC gateway are the same: one
`[storage] redis_url` for membership, the conference registry and the CDR
stream.

So "membership on a global Redis, events on a regional one" — which
`docs/MULTI_REGION.md` describes piece by piece — is not something a call
manager can be configured to do. Only the events server takes two Redis
URLs (`[cluster] redis_url` for membership, `[event_bus] redis_url` for
events). A deployment chooses one of two shapes:

| | **A. One cluster** — one Redis shared by every region | **B. A cluster per region** — a Redis in each region |
|---|---|---|
| Nodes see each other | across regions | within their region only |
| A conference room joined from two regions | one room, a node per region, trunked (`docs/DISTRIBUTED_CONFERENCING.md`) | **two rooms** with the same name, one per region |
| Park in one region, pick up in another | works | does not: parking is per Redis |
| Tenant call limits and trunk channel leases (`[cluster] lease_*`) | across the deployment | **per region**: a limit of 100 is 100 in each |
| Events and webhooks | one stream for every region; no bridge (§3.1) | each region's own stream, bridged in a full mesh; a webhook fires once, in the region where the event happened |
| Redis on the call path of a region away from it | yes: a lease, a conference placement, a park or pickup each cross | no |
| When the link between regions is cut | the side without the Redis loses parking, leases fall back to per node, events published through Redis are lost, placement runs on a stale member list | each side carries on; only events stop crossing, queued up to the stream's cap |
| The partition is visible as | `cluster_view` on every node (§6.1) | the events server's bridge (§6.2) |

**Start with B** unless one of A's rows is a requirement. It is the
simpler deployment, its regions fail independently, and nothing on its
call path crosses a region except the registration reads in §5. A is for
deployments that must hold one conference across regions, park and pick
up across them, or enforce a tenant's limit globally — and that accept
the Redis round trips and the shared failure domain that come with it.

Mixing them is not supported: a call manager pointed at a regional Redis
is in that region's cluster, whatever the others do.

## 2. What every region runs

In both shapes, each region runs its own call managers, conference
servers, WebRTC gateways, an events server, and `fcp-jobs`; the SBC,
IVR, voicemail, presence, media and TTS servers where they are used. One
Postgres primary serves every region (`docs/MULTI_REGION.md`, Decisions);
each other region has a streaming replica.

**Label every node with its region.** `[cluster] region` (or
`FCP_REGION`) on the call manager, conference server, WebRTC gateway,
events server, SBC, IVR, voicemail, presence, media and TTS servers;
`[cdr] region` and `[cdr_archive] region` on `fcp-jobs`. The label is a
free string that only has to agree across nodes. It drives conference
placement and drain targets, the WebRTC handover peer, the `region` on
every CDR, the `origin_region` on bridged events, and the partition view.
`fcp-admin`, `fcp-registrar` and `fcp-api-gateway` take no region.

**Point each region's nodes at its replica** where they accept one:

```toml
[storage]
database_url = "postgresql://fcp:…@primary.us-east.internal/fcp"
read_replica_url = "postgresql://fcp:…@replica.eu-west.internal/fcp"
max_replica_lag_secs = 5
```

Accepted by the call manager, conference, voicemail, IVR, presence and
media servers, `fcp-admin` and `fcp-registrar` (`[storage]`), and the SBC
(`[core.storage]`). `fcp-jobs` and the events server always use the
primary; both mostly write, so that costs little. What the replica serves
and what always goes to the primary is in `docs/MULTI_REGION.md`, What
goes where. Failing over the primary itself is the database's job
(Patroni, or the cloud's managed failover).

## 3. Redis: what is shared and what is not

### 3.1 Shape A — one Redis for every region

Every node's Redis setting names the same Redis:

| Binary | Setting |
|---|---|
| call manager, conference server, WebRTC gateway, IVR, voicemail, media server | `[storage] redis_url` |
| presence server | `[storage] redis_url` (membership); `[redis] url` for presence Pub/Sub, a non-default feature |
| SBC | top-level `redis_url` |
| TTS server | `[cluster] redis_url` |
| events server | `[event_bus] redis_url` (and `[cluster] redis_url`, or leave it to fall back) |
| `fcp-jobs` | `[redis] url` |
| `fcp-admin` | `[cluster] redis_url` |

The Redis lives in one region; put it with the Postgres primary, so a
region cut off from one is cut off from both and the other region keeps
everything (§6.1).

**Events.** Every call manager publishes into the one `event_stream`, so
there is nothing to bridge — **configure no `[[event_bus.peer_regions]]`**.
A bridge pointed at the same Redis would copy every event back into the
stream it came from, marked relayed, and every consumer would see it
twice.

Every events server must be in the **same consumer group**
(`[event_bus] consumer_group`, default `fcp-events-server`). Each event is
then handled by exactly one of them, so a webhook fires once — but a
WebSocket client connected to one events server sees only that server's
share of the stream. Events servers in different groups each see every
event, and each fires every webhook: a tenant gets one delivery per
group. If WebSocket clients need the whole stream, run one events server
(or one group) for the deployment and point them all at it.

**CDRs.** Call managers write `fcp:cdrs` on the shared Redis; run the
`fcp-jobs` CDR processors anywhere, in one consumer group. Records are
stamped with the region they were made in.

### 3.2 Shape B — a Redis in each region

Every node in a region names its region's Redis, with the same settings
as the table above. Nothing else is shared.

**Events.** Each region's events server bridges to every other region's
Redis — a full mesh, since a chain does not forward (A's events reach B,
not C through B):

```toml
# events server in us-east
[cluster]
region = "us-east"

[event_bus]
bus_type = "redis"
redis_url = "redis://redis.us-east.internal:6379"

[[event_bus.peer_regions]]
region = "eu-west"
redis_url = "redis://redis.eu-west.internal:6379"

[[event_bus.peer_regions]]
region = "ap-south"
redis_url = "redis://redis.ap-south.internal:6379"
```

A webhook fires in the region where the event happened, once. Only
events that reach Redis cross: the call manager's with `[events]
use_redis = true`, and `fcp-jobs`'. Events a conference server, IVR or
voicemail server posts to the events server over HTTP (`/ingest`) go to
that events server's local subscribers only and **never cross regions**.

**CDRs.** Each region runs its own `fcp-jobs` against its own Redis; all
write the one Postgres.

**The admin API** reads membership from its `[cluster] redis_url`, so an
`fcp-admin` shows one region's nodes. Run one per region, or accept that
the cluster page is regional.

### 3.3 Either shape

- The API gateway's response cache (`[redis] url`) is per instance and can
  be anything local.
- `fcp-registrar` uses no Redis; the SBC uses Redis only for membership
  and keeps its registrations in memory.
- Make each region's Redis reachable from nothing outside its shape:
  `cluster:*`, `leases:*` and `park:*` keys under `fcp:` are the cluster's
  state, and a stray node pointed at the wrong Redis joins that cluster.

## 4. Getting callers to their region

FCP does not steer traffic between regions; DNS, anycast and the carrier
do (Q2). What each needs from FCP:

### 4.1 Phones

A phone finds its region through the tenant's SIP domain. Publish SRV
records per region with geo-DNS, each region answering with its own call
managers first and the others as failover:

```
; answered to clients in Europe
_sip._udp.acme.example.com. 300 IN SRV 10 50 5060 cm1.eu-west.example.com.
_sip._udp.acme.example.com. 300 IN SRV 10 50 5060 cm2.eu-west.example.com.
_sip._udp.acme.example.com. 300 IN SRV 20 50 5060 cm1.us-east.example.com.
; answered to clients in North America: the same, with the priorities swapped
```

and the same for `_sip._tcp` and `_sips._tcp` where those transports are
offered. Registrations are in Postgres, so a phone that fails over to
another region registers there without anything moving.

- **Health-check `/ready`, not `/health`.** `/ready` answers 503 while a
  node drains; `/health` stays 200 so a liveness probe does not kill it
  mid-drain (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). A DNS health
  check on `/health` keeps sending phones to a node that is leaving.
- **Each node advertises its own public address**: `[sip] local_ip` and
  `local_uri` (the `Contact` and `Record-Route` it puts on calls) and
  `[media] public_ip` (its SDP) — or `--public-ip`, which sets both.
  Behind NAT the auto-detected address is private and calls from outside
  the region will not come back.
- A phone keeps its registration's node for a call only as long as its
  NAT binding lasts; nothing pins a phone to a region beyond DNS.

### 4.2 Browsers

Each WebRTC gateway sets `[http] public_url` to the URL browsers reach it
at. Drain handover picks the least-loaded serving gateway in the same
region with a `public_url` (`docs/CAPACITY_AND_SURVIVABILITY.md` §6);
geo-DNS the join URL the same way as SIP.

### 4.3 Carriers

**Inbound.** A call's tenant is decided by the address it comes from
(`docs/TENANT_TRUNKS.md` §7): the trunk whose `inbound_sources` — or, when
those are empty, whose `hosts` — contain the source. A carrier that sends
from different addresses to different regions must have **all of them**
in the trunk's `inbound_sources`, or the other region cannot tell which
tenant the call belongs to and answers 404. Give the carrier each region's
signalling addresses (the SBC's, or the call managers' where no SBC is
used) and let its own routing choose; FCP accepts the call wherever it
lands.

**Outbound.** Trunk selection has no region in it: every node chooses a
trunk by the same priority and weight, so a European call manager uses
the same first-choice carrier as an American one. The platform's trunks
are each node's own `[[trunks]]`, so a region's nodes can give that
region's carrier the better priority. A tenant's trunks are in the
database and read the same in every region: for those there is no
per-region preference, and the carrier's own routing decides. A
trunk's target is always built with an explicit port, so RFC 3263 SRV
lookup is not used for trunks — list every carrier address in `hosts`.

## 5. What a region away from the primary pays

Configuration reads come from the region's replica. Three things still
cross to the primary:

- **Registration lookups** (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1). A
  two-party call setup reads the caller's and the callee's registrations:
  **two round trips**, and **six** with two domain aliases, each lookup
  repeated at every other alias. The call manager's registration cache
  (`[registrar] cache_ttl_secs`, default 30) answers a repeat lookup
  within the TTL from memory, so a party called again soon costs nothing;
  a phone called a few times an hour is looked up cold and pays. Read the
  split off `/metrics`:

  ```
  rate(fcp_registration_cache_hits_total[5m])
    / (rate(fcp_registration_cache_hits_total[5m])
       + rate(fcp_registration_reads_total{reason!="housekeeping"}[5m]))
  ```

  is the share of call-path lookups the cache saved. At a transatlantic
  80 ms, each cold lookup is 80 ms before anything rings.
- **Every write**: REGISTER, CDRs (written by `fcp-jobs`, off the call
  path), billing at rating, configuration changes. A REGISTER is one or
  two round trips; nobody waits on a CDR.
- **In shape A, Redis** (§1): lease checks, conference placement, parking.

Keep `[registrar] domain_aliases` to what a deployment needs: each alias
multiplies the lookups.

## 6. What a partition looks like

### 6.1 Shape A

The side **without** the Redis — every node there:

- `/status` `cluster_view.state = "store_unreachable"`, with
  `view_age_secs` growing and `database_reachable` saying whether it
  still reaches its database (`true` is the signature of being the side
  cut off); `fcp_cluster_view{state="store_unreachable"} 1`.
- It keeps serving the calls it has, and takes new ones: registrations,
  configuration and routing are in Postgres and its replica. Tenant and
  trunk limits fall back to each node's own count. Parking and pickup
  fail. Events published through Redis are lost, not queued — including
  webhooks for its calls. **Conference placement runs on the member list
  it last saw**, which does not shrink while the Redis is out of reach, so
  a room may be placed on a node in the other region that it can no
  longer coordinate with.
- It cannot write its heartbeat, so it disappears from the other side's
  cluster page.

The side **with** the Redis — every node there:

- `cluster_view.state = "remote_regions_lost"`, naming the regions that
  went quiet at once; the admin UI's cluster page shows a banner grouping
  the nodes that say so, and each has a badge.
- `system.cluster_view_changed` on every node, reaching webhooks and the
  events server over their local paths.

When the link heals, both sides return to `whole` within a heartbeat and
say how long it lasted. Nothing reconciles: rooms and parked calls that
were split stay as they are until they end.

### 6.2 Shape B

Membership never crosses regions, so no node's `cluster_view` changes.
The partition shows at the events servers:

- `/status` `region.bridge.<peer>.connected = false` and
  `fcp_event_bridge_connected{peer="…"} 0`, with `failed` growing.
- Events queue in the local stream and cross when the link returns — up
  to its cap (`MAXLEN ~ 10000`); past that the oldest are lost to the
  peer, never locally.

If the cut also separates a region from the Postgres primary, that region
serves calls from its replica and caches while it can: registration
lookups that miss the cache fail, and every write fails — REGISTER
answers 500. Call records still ship to the region's Redis stream; its
`fcp-jobs` processor cannot store them, leaves each batch unacknowledged
and retries with backoff (`docs/CDR_PIPELINE.md` §6), so they wait in the
stream — `cdr_stream_pending` grows — and land when the primary is back.

### 6.3 Alerts worth having

| Alert | Query |
|---|---|
| A node doubts its view | `max by (node) (fcp_cluster_view{state!="whole"}) == 1` for 1 m |
| A node's member list is stale | `fcp_cluster_view_age_seconds > 30` |
| A bridge is down | `fcp_event_bridge_connected == 0` for 1 m |
| The registration cache is not saving much | the ratio in §5 below the level a deployment decides |

## 7. A region, checked off

- [ ] Shape chosen (§1); every node's Redis setting agrees with it.
- [ ] `[cluster] region` on every node; `[cdr] region` and
      `[cdr_archive] region` on `fcp-jobs`.
- [ ] `read_replica_url` on every binary that takes one.
- [ ] Shape A: one events-server consumer group, no peers. Shape B: a
      full mesh of `[[event_bus.peer_regions]]`, `[events] use_redis` on
      every call manager.
- [ ] SRV records per region, health-checked on `/ready`.
- [ ] `[sip] local_ip`/`local_uri` and `[media] public_ip` are each node's
      public address; `[http] public_url` on each gateway.
- [ ] Every carrier source address in its trunk's `inbound_sources`.
- [ ] Conference servers bind SIP to a concrete address, not `0.0.0.0`
      (§8).
- [ ] The alerts in §6.3.

## 8. Known gaps

Found while writing this guide; none is fixed here.

- **One Redis per call manager** (§1). Splitting membership from the
  event bus, or making registrations regional, needs the call manager to
  take more than one Redis. This is the decision point after the
  multi-region follow-ups (`docs/MULTI_REGION_FOLLOWUPS.md` §5).
- **A conference server advertises its SIP bind address as-is.** The
  `sip_address` in its heartbeat is `[sip] udp_bind` unchanged, and the
  call manager routes placed callers there; bound to `0.0.0.0`, that is
  where callers are sent. Bind a concrete address until this uses
  `local_ip`.
- **`fcp-registrar`'s `/drain` takes no credentials**, unlike every other
  binary's. Bind its HTTP listener to a private interface.
- **The SBC's membership node id is random** (`sbc-<uuid>`), not its
  `[service] node_id`, so it changes on every restart and does not match
  the id its drain uses.
- **Events ingested over HTTP never cross regions** (§3.2): conference,
  IVR and voicemail events stay in the region that produced them.
- **No binary reports its replica's health** on `/status`, though the
  backend probes it every five seconds (`replica_status()`).
- **`fcp-api-gateway`'s default backend health path**,
  `/api/v1/system/health`, is not a route the API serves; set
  `health_check_path` explicitly.
- **Trunk SRV**: trunk targets carry an explicit port, so SRV records for
  a carrier are never consulted (§4.3).
- `[events] redis_channel` and `redis_stream_key` on the call manager are
  parsed and ignored.
