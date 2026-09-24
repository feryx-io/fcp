# Cluster operations runbook

How to run, watch and change a cluster of FCP nodes. Written against the
code (v0.25.248, `docs/SHIP_AND_OPERATE.md` phase 5): every route, key,
token and log line here was taken from a binary's router, its
`--check-config` output or the source, not from memory. What a single
node needs is in `docs/SINGLE-NODE-SETUP.md`; what several regions need
is in `docs/MULTI_REGION_DEPLOYMENT.md`; the design behind draining and
admission is `docs/CAPACITY_AND_SURVIVABILITY.md`.

## 1. What a cluster is

A cluster is the set of nodes that heartbeat into one Redis. Each node
writes `cluster:node:<node_id>` every `heartbeat_interval_secs` (5) with
a `heartbeat_ttl_secs` (15) expiry and discovers the others by scanning
those keys. What it carries — its addresses, region, capabilities, load,
whether it is draining, and its view of the cluster — is what the other
nodes and the admin API see. There is no gossip protocol and no leader.

Every binary has a `[cluster]` section with `enabled`,
`heartbeat_interval_secs`, `heartbeat_ttl_secs` and `region`; the Redis
it uses is `[storage] redis_url` (the SBC's is the top-level
`redis_url`; the admin's is `[cluster] redis_url`). The call manager
adds `call_ttl_secs` (7200), the Redis circuit breaker
(`redis_circuit_breaker_threshold` 5, `_cooldown_secs` 30) and the
leases (`lease_tenant_calls`, `lease_trunk_channels`,
`lease_settle_secs` 5); the conference server adds `secret` — required
for cross-node room control and participant migration — and
`migration_timeout_secs` (15) and its trunk settings.

**Every node's clock must agree** (NTP): heartbeat expiry and lease
settlement are wall-clock arithmetic across nodes.

## 2. Configuration

A call manager in a cluster:

```toml
[service]
node_id = "cm-east-1"          # unique; every record and event names it

[storage]
database_url = "postgresql://fcp:…@db:5432/fcp"
redis_url = "redis://redis:6379"

[cluster]
enabled = true
region = "us-east"             # or FCP_REGION; docs/MULTI_REGION.md

[http]
bind_address = "0.0.0.0:8084"
api_token = "…"                # or FCP_CALL_MANAGER_API_TOKEN
```

There is no cluster secret on the call manager. Its inter-node routes
(`/internal/*`: hangup, supervisor, unpark) are authenticated by
`[http] api_token`, so **every call manager in a cluster carries the
same `api_token`**. A `cluster_secret` key is refused at startup.

The conference server's `[cluster] secret` must be the same on every
conference node; its `[sip] tls_bind` and `local_ip` are what the
inter-node trunks dial (`docs/DISTRIBUTED_CONFERENCING.md`).

Companion settings, so the nodes agree:

- the same `[registrar] realm` and `domain_aliases`;
- `[events] server_url` and `auth_token` on every producer, all
  pointing at the region's events server;
- `[cdr] sink = "stream"` where `fcp-jobs` stores records for the
  cluster;
- `[registrar] cache_ttl_secs` (30) and `[events] use_redis` on the
  call manager, so a REGISTER on one node reaches the others' caches
  (`docs/MULTI_REGION_FOLLOWUPS.md`).

Check a node's file before starting it: `--check-config` prints the
effective configuration with secrets masked and exits 0 only if the
node would start.

## 3. Starting a cluster

Start the nodes in any order. Each logs `Cluster membership started`
when it heartbeats, and `Cluster member joined` for each other node it
discovers. A node that cannot reach Redis logs `Cluster heartbeat or
discovery failed` (or `Failed to publish cluster heartbeat`) and keeps
serving with the member list it last had.

See the cluster from any node:

```bash
T='Authorization: Bearer <api_token>'
curl -s -H "$T" http://node:8084/status | jq .cluster_view
```

or from the admin API, which reads the same Redis:

```bash
curl -s -H 'Authorization: Bearer <admin token>' \
  http://admin:8080/api/v1/cluster/nodes | jq .
```

## 4. The routes, per binary

`/health` and `/ready` are open on every binary. `/health` says the
process is alive; `/ready` says whether to send the node work — 200
serving, 503 draining — and is what a load balancer or a SIP SRV weight
should watch. Pointing a liveness probe at `/ready` kills a node in the
middle of a drain.

Everything else needs the binary's token, as a bearer header. The
routes below are the ones that exist; a binary has no others.

| Binary | Token | Routes (all need the token unless marked open) |
|---|---|---|
| `fcp-call-manager` | `[http] api_token` | `/status`, `/calls`, `/registrations` (`?aor=`), `/cdrs`, `/cdr/status`, `/media-sessions`, `/transcoding`, `/trunks`, `/emergency/trunks`, `/queues/*`, `/agents/*`, `/callbacks/*`, `/campaigns/*`, `/calls/:id/{monitor,whisper,barge}`, `/presence`, `/metrics`, `POST`/`DELETE /drain`, `/internal/*`, `/ws/wallboard` |
| `fcp-conference` | `[http] auth_token` (with `auth_enabled`) | `/status`, `/metrics`, `/meetings*`, `/conferences*` (rooms, lock, stats, announce, video, dial-out, ai), `/events`, `/recordings`, `/drain`, `/internal/*` (with `[cluster] secret`) |
| `fcp-registrar` | `[http] api_token` | `/status`, `/drain` |
| `fcp-ivr` | `[http] api_token` | `/status`, `/flows/routes`, `/flows/active`, `/drain` |
| `fcp-voicemail` | `[http] api_token` | `/status`, `/voicemail/routes`, `/voicemail/mailboxes*`, `/drain` |
| `fcp-presence` | `[http] api_token` | `/v1/metrics`, `/v1/presence/*`, `/v1/analytics/*`, `/v1/webhooks*`, `/drain`; `/v1/health` open; no `/status` |
| `fcp-sbc` | `--health-api-token` | `/drain`, `/metrics` (a JSON snapshot, not Prometheus text); no `/status` — `/ready` carries `trunks_healthy`, `active_calls` and `replica` |
| `fcp-webrtc-gw` | `[auth] tokens` | `/status`, `/drain`, `/v1/rooms*`, `/v1/calls*`; `/join` open when `public_join` |
| `fcp-events-server` | `[auth] token` | `/status`, `/ingest`, `/ingest/batch`, `/calls`, `/registrations`, `/nodes`, `/stats`, `/v1/webhooks*`, `/ws/:tenant_id` (the token as a bearer header or `?token=`); `/metrics` open |
| `fcp-jobs` | none | `/status`, `/metrics` — open, so bind it privately |
| `fcp-api-gateway` | none of its own | `/gateway/status` open; `/api/v1/*` and `/metrics` proxied to the admin with the caller's header |
| `fcp-admin` | `[api] auth_token`, or a user's JWT | `/api/v1/*`; `/metrics` for an admin; `/api/v1/health` open |

`/status` is a JSON of the node's own numbers: uptime, calls, `draining`,
and on the call manager and conference server `cluster_view`,
`registration_reads`, `registration_cache`, `admission` and `replica`
(`docs/MULTI_REGION_DEPLOYMENT.md` §6 says what a partition looks like
there). The metrics endpoints that need a token need it from Prometheus
too (`authorization: type: Bearer`); the compose stack shows the shape.

## 5. Draining and restarting

A drain (`docs/CAPACITY_AND_SURVIVABILITY.md` §6) is how a node leaves
service without dropping what it carries. The same sequence runs from
`POST /drain` and from `SIGTERM`: the node refuses new work, says so on
`/ready` (503) and in its heartbeat (so placement skips it and the admin
UI shows it), keeps its calls until `drain_wait_secs`, then ends what is
left with a BYE that says why, and tells the cluster. `DELETE /drain`
calls a drain off. The registrar drains by granting short registrations
so phones come back to a serving node; the conference server drains by
moving its participants to another node (a REFER with a signed ticket).

```bash
T='Authorization: Bearer <api_token>'
curl -s -H "$T" -X POST http://node:8084/drain     # {"status":"draining","active_calls":…,"wait_secs":…}
curl -s http://node:8084/ready                      # 503 while draining
# … when active_calls reaches 0, or wait_secs has passed:
kill -TERM <pid>                                    # logs "Shutdown complete"
curl -s -H "$T" -X DELETE http://node:8084/drain   # to call it off instead
```

**A rolling restart** is a drain, a stop, an upgrade and a start on
each node in turn (what an upgrade does to the schema, and why a backup
comes first, is in `docs/BACKUP_AND_RESTORE.md`), waiting for `/ready` to answer 200 and `Cluster
member joined` to appear on another node before the next. A node with
no `[cluster] region` is local to nothing — never preferred, never
excluded — so a node upgraded before its region label is set strands
no one.

**Adding a node** is starting it with the same Redis, database, realm,
tokens and `[cluster] secret` (conference); it joins on its first
heartbeat. **Removing one** is a drain and a stop; its record expires
after `heartbeat_ttl_secs`, and its clean shutdown leaves a
`cluster:left` marker so the others report `shutdown`, not a lost node.

**Reloading queues** on the call manager is `SIGHUP`; nothing else
reloads without a restart.

## 6. Watching

**Prometheus.** Scrape `/metrics` on the call manager, conference
server (both with the token), events server and jobs (open); the admin's
`/metrics` with an admin token. Alerts worth having, with their queries,
are in `docs/MULTI_REGION_DEPLOYMENT.md` §6.3: a node that doubts its
view of the cluster, a stale member list, a bridge down, a region
reading from the primary, a stuck replica probe. A node's draining
state is not a metric; it is `draining` on `/status` and `/ready` and
in the node's heartbeat, which the admin UI shows.

**Logs worth alerting on.** The call manager's Redis circuit breaker
logs `Redis circuit breaker opening` when it opens, `transitioning to
half-open` when it probes, `closing (probe succeeded)` or `reopening
(probe failed)`; the conference server's event forwarder logs `Circuit
breaker OPENED — event forwarding paused` when the events server stops
answering it.

**The admin UI's cluster page** (`/admin/`) shows every node's record:
region, load, draining, and a banner over the nodes whose view of the
cluster is not whole.

**Events.** `system.node_left` (with `shutdown` or `heartbeat_expired`),
`system.cluster_view_changed` and `trunk.health_changed` reach webhooks
and WebSocket clients through the events server.

## 7. When it goes wrong

- **Two halves of a cluster, each serving.** A node that can reach
  Postgres but not Redis reports `store_unreachable` in `cluster_view`;
  one that lost every node outside its region at once reports
  `remote_regions_lost`. Nothing acts on it: each side keeps serving
  the calls it has and placing new ones on the nodes it can see. When
  the link returns, the records simply agree again. Decide in advance
  which side should be told to drain; the platform will not choose.
- **Redis is down.** Every node keeps serving with its last member
  list; placement, parking, leases and the CDR stream pause; the call
  manager's circuit breaker opens after `redis_circuit_breaker_threshold`
  failures and probes again after the cooldown. A reconnect takes about
  a second after the link returns (the redis-rs default of a minute was
  fixed in v0.25.233).
- **A node's heartbeat is late.** Look at the node's clock first, then
  its Redis latency. `fcp_cluster_view_age_seconds` on `/metrics` is how
  old its member list is.
- **Cross-node calls fail with 401.** The call managers' `api_token`
  values differ, or the conference servers' `[cluster] secret` does.
- **A node refuses to start.** It prints the rule: a public bind
  without its token, a public SIP bind without digest authentication,
  an unknown key. `--check-config` shows what it was judged on.

## 8. Capacity

Redis holds one small hash per node, one key per active call
(`call_ttl_secs`), one per parked call, the leases and the event and
CDR streams; a hundred nodes and a few thousand concurrent calls fit
in a few hundred megabytes. Admission — calls per second, concurrent
calls and registrations per node and per tenant — is in
`docs/CAPACITY_AND_SURVIVABILITY.md` §5 with the numbers the load
harness measured; the `[admission]` section is on every routing
binary.
