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
`lease_settle_secs` 5, `lease_ttl_secs` 60 — how long a killed node's
calls go on counting, refreshed every quarter of it); the conference server adds `secret` — required
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

- the same `[registrar] realm` and `domain_aliases`, which the admin
  computes SIP password hashes in too (`[sip] realm`, else the call
  managers'; `docs/SINGLE-NODE-SETUP.md`);
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
| `fcp-events-server` | `[auth] token`, or a signed-in user's access token (with the admin's key as `[auth] session_signing_key`) | `/status`, `/ingest`, `/ingest/batch`, `/calls`, `/registrations`, `/nodes`, `/stats`, `/ws/:tenant_id` (the token as a bearer header or `?token=`); `/metrics` open |
| `fcp-jobs` | none | `/status`, `/metrics` — open, so bind it privately |
| `fcp-api-gateway` | none of its own | `/gateway/status` open; `/api/v1/*` and `/metrics` proxied to the admin with the caller's header |
| `fcp-admin` | `[api] auth_token`, a signed-in user's access token (`[api.session]`, `docs/API_TOKENS.md` §7), or an API token | `/api/v1/*`; `/metrics` for an admin; `/api/v1/health` open |

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

**Services** (`docs/SERVICES.md`). Every binary with a Redis writes a
service beacon (`fcp:service:<id>`, 5 s, 15 s TTL) whether or not it is a
cluster member: what it is, its release, `serving`/`degraded`/`draining`/
`stopping`, its checks (database, Redis, the services it calls) and its
load. `/ready` answers 503 for a failing required check (database,
Redis) as well as draining. The admin with `[cluster] redis_url` keeps a
record of every one it has seen (`service_instances`) and a site's,
carried up by its link: `GET /api/v1/system/services` (platform
administrators only), `fcp_service_up{id,kind,site}` and
`fcp_service_check_failing{id,kind,site,check}` on its `/metrics`. A
service whose beacon goes is `down` since then — `stopped` if it said so
— until it returns, is forgotten (`DELETE /api/v1/system/services/<id>`),
or has been down for `[services] forget_after_days` (7). With several
admins one sweeps, holding `fcp:services:sweep`. Alert on
`fcp_service_up == 0`.

**The admin UI's cluster page** (`/admin/`) shows every node's record:
region, load, draining, version, a banner over the nodes whose view of
the cluster is not whole, and a banner naming who is on what while the
nodes run more than one release.

**Events.** `system.node_left` (with `shutdown` or `heartbeat_expired`),
`system.cluster_view_changed`, `system.version_skew` (the cluster
started or stopped running more than one release —
`docs/BACKUP_AND_RESTORE.md` §5), `system.site_link_changed` (a tenant's
site's link came up or went quiet — `docs/HYBRID_CONTROL_PLANE.md`),
`system.service_up` and `system.service_down` (once each, by the admin
that sweeps; never said down while Redis cannot be read) and
`trunk.health_changed` reach webhooks and WebSocket clients through the
events server. The admin's `[link]` listeners, when on, are what sites
enroll to (TLS) and heartbeat at (mutual TLS); the cluster page's Sites
section shows them, with each site's sync — current, how many journal
entries behind, or the error its link last hit. At a site, `fcp-link`
with `[site] database_url` keeps the site's Postgres a copy of the
tenant's configuration (its cursor in `sync.json` beside the
certificate; delete that file to start from a fresh snapshot), and the
site's admin runs with `[api] managed_by` naming the cloud, so a write
to a synced object there is refused with the cloud's address.

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
  fixed in v0.25.233). Calls up carry on and new calls are placed. A
  lease a call could not give back while Redis was away lapses within
  `lease_ttl_secs`, so a user who takes one call at a time is busy for
  at most that long after Redis returns (`fcp-call-manager/tests/faults.rs`).
- **Postgres is down.** Calls up carry on, and their BYEs cross. A
  REGISTER or a new INVITE is answered at once with `503 Service
  Unavailable - store unreachable` and `Retry-After: 5`, so a phone
  tries again or tries another node instead of waiting 32 s for
  nothing. The node finds out by asking the primary
  `SELECT pg_is_in_recovery()` every second (two missed answers make it
  down); until then a REGISTER is
  cut off at five seconds and a request waits at most
  `[storage] acquire_timeout_secs` (5) for a connection. `/ready` is
  503 with `database.up: false` (send new traffic elsewhere), `/status`
  shows `database`, `/metrics` has `fcp_database_up` and
  `fcp_database_outages_total`, and the log says `The database does
  not answer` and `The database answers again`. The records of calls
  that end meanwhile wait in the spool and are written when it is back.
  Nothing needs a restart. An emergency call is refused like any other
  call while the store is away (#326).
- **The primary fails over to a standby.** Promotion is the database's
  job (Patroni, a managed database); give the nodes an address that
  moves with it (a VIP, a DNS name, a pooler). From the primary's death
  to the promotion, the node behaves as with Postgres down. Once the
  address leads to the promoted standby, the pool reconnects and
  registrations, calls and records carry on, with no restart. What the
  standby had not replayed is lost with an asynchronous standby.
  Calls up live through it (`fcp-call-manager/tests/faults.rs`).
- **The primary's address leads to a standby**: moved before the
  promotion, or to an old primary come back as a standby while a node
  still resolves the old address. The standby answers reads and refuses
  writes. The node sees it in recovery: REGISTERs are answered `503
  Service Unavailable - store read-only` with `Retry-After: 5`, and new
  calls are still placed (they only read; their records wait in the
  spool). `/ready` is 503 with `database.writable: false`, `/metrics`
  has `fcp_database_writable` 0, and the log says `The database is a
  standby (in recovery) and refuses writes` and, once promoted, `The
  database takes writes again`.
- **A call manager was killed** (`SIGKILL`, a crash, its host gone).
  Its calls are gone; nothing tells their phones. Its leases lapse
  within `lease_ttl_secs`, so their users and tenants are free again,
  and it leaves the member list within `heartbeat_ttl_secs`. Started
  again on the same `[cdr] spool_dir`, it ships the records it had
  spooled, and writes one for each call it was carrying, answered and
  ended by `node_failed` when it last looked (at most ten seconds before
  it died). Started somewhere else, those records stay in the old
  spool: move its `open/` directory and segments to the new node's
  spool to have them written.
- **A conference node was killed.** The other nodes hang up their
  trunks to it once it leaves the member list (`heartbeat_ttl_secs`),
  and its rooms carry on there. Each call manager moves the callers it
  had on the dead node into the same room on a live one within a second
  of seeing it gone; the log says `A caller's conference node has gone;
  moving them to another`. They join as new participants, so a PIN room
  asks again and a host must take the host PIN again. When no node will
  take the room, the call is ended with `Reason: SIP;cause=503`.
- **The events server was killed.** Events wait in the stream. With
  `bus_type = "redis"` an event is acknowledged only once its webhooks
  are in the delivery log and the emergency notifier has dealt with it;
  a server that starts again, or another in the same consumer group,
  takes over what the dead one held after `[event_bus]
  claim_idle_secs` (60) and delivers it. The log says `Claimed events a
  consumer was handed and never acknowledged`. A webhook sent but not
  recorded is sent again under the same `event_id`; an emergency notice
  sent but not recorded is sent again too, since telling someone twice
  beats not telling them.
- **`fcp-jobs` was killed.** CDRs it held unstored are claimed by the
  next processor after `[cdr] claim_idle_secs` (60) and stored, charged
  and announced once each.
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

## 9. Sites

A site is a tenant's own stack on the tenant's network, enrolled to
this cloud and run by the customer (`docs/SITE_SETUP.md` is their
document; `docs/HYBRID_CONTROL_PLANE.md` the design). It is not a node
of this cluster: it heartbeats into nothing here. Its one process that
talks to the cloud, `fcp-link`, connects outward to the admin's link
listeners and the site's call manager registers a link trunk to the
SBC; nothing here connects to the site.

**What the cloud runs for sites.** The admin's `[link]`: `enabled`,
`enroll_bind` (TLS, the one-time enrollment) and `bind` (mutual TLS,
everything after), `cert`/`key` (the listeners' own certificate),
`ca_dir` (the link CA the admin makes on first start and signs every
site's certificate with; `Link CA made`, then `Link CA loaded` — keep
the directory, it is in `admin-data`), `heartbeat_interval_secs` (what
the admin tells every link to send at), `heartbeat_stale_secs` (a site
quiet for longer is marked down: `A site's link is down: no heartbeat`),
`sip_host` (the SBC's TLS listener as `host:port`; without it,
`No [link] sip_host: the site gets no link trunk`) and `sip_realm` (the
SBC's `external_domain`). The compose cloud publishes 8446 and 8447 and
sets `sip_host` to `${HOST_IP}:5079`. Behind a load balancer, the link
listener must pass the client certificate through (TLS passthrough,
not termination): the admin verifies it against its own CA.

**A site's life.** A tenant administrator makes the site (`POST
/v1/sites` in the tenant; the UI's Sites page) and gets a token shown
once. The site's link enrolls with it (`A site enrolled`; the cloud
makes the site's link trunk, `The site's link trunk is provisioned`,
its password rotated at every enrollment), takes the tenant's snapshot
(`A site took a snapshot`) and follows the journal from there. Every
heartbeat carries the link's release, the site's nodes and their
releases, the sync cursor, the registrations at the site; the site
record (`GET /v1/sites/:id`) shows them with `link_up`, `last_seen_at`,
`sync_behind` (the journal's head minus the site's cursor) and
`sync_error`. `GET /v1/cluster/sites` is every tenant's sites for the
cluster page, and `system.site_link_changed` fires on every up and
down (`A site's link is up`).

**Watching.** The Sites page, or `GET /v1/sites` in the tenant. The
SBC's `GET /admin/sites` lists the sites bound to it (the link trunk's
registration, with its flow). A site whose link is up but whose
`sync_behind` grows has a sync error: the record names the object, the
site's link log the cause; the link retries from its cursor.

**Disabling, deleting.** `POST /v1/sites/:id/disable` refuses the
site's certificate and secret from the next request; its heartbeats
fail and its trunk registration is refused. `enable` takes it back.
`DELETE /v1/sites/:id` removes the site and its link trunk; the site's
own stack is the customer's to stop.

**Upgrading.** The site's releases are on its record and the Sites
page. The site runs the cloud's images (`ghcr.io/thevoiceguy/fcp-<service>`,
tagged `nightly` and by commit — the compose README's "Pull instead of
build"); upgrade the cloud first, then
the sites, and keep a site within one release of the cloud. The link's
protocol is versioned (`protocol` on the heartbeat); a site the cloud
refuses says so in its log.

**Draining and partitions.** Draining a cloud node does not touch a
site. A site whose link is down (`link_up` false) is not a partition
of this cluster: it runs alone on its copy and catches up when the
link returns (`docs/HYBRID_CONTROL_PLANE.md` §3.8); the records it
writes meanwhile reach the cloud afterwards, naming the site.
