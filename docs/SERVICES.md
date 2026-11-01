# Seeing every service

D8 finding 34 (2026-10-29): "is there only 1 node running for d8? looking at the
cluster i only see the cm-docker-01 node. I do not see a voicemail, sbc, conference
node etc." Then: "we need at least a services view so we can see all the running
nodes, the services they are running, their health, etc." — under a System menu with
the cluster and any future system pages.

*Drafted 2026-10-29. Agreed the same day: every question on its recommendation.*

## 1. What exists

The D8 server runs eighteen containers, all healthy. The admin's Cluster page shows
one: `cm-docker-01`.

- **The Cluster page lists cluster members, not services.** A member is a process
  that heartbeats a `ClusterMember` into Redis (`fcp-cluster/src/membership.rs`,
  `fcp:cluster:node:<id>`, a 15 s TTL renewed every 5 s); the admin reads them with
  `ClusterReader` (`/api/v1/cluster/nodes`, `cluster_api.rs`).
- **Membership is opt-in, and only the call manager opts in.** Ten binaries can join
  (`[cluster] enabled`): call manager, SBC, voicemail, conference, IVR, media,
  presence, TTS, WebRTC gateway, events server. In `deploy/docker-compose/configs`
  only `fcp-call-manager.toml` sets it; `fcp-sbc.toml` and `fcp-webrtc-gw.toml` say
  `false`, the rest leave it to the default, off. The site project is the same.
- **Membership is not only a listing.** Being a member changes behaviour: SIP from a
  member skips admission (`sip_peer_check`), the SBC's registering trunks are owned
  through it (`trunk_owner`), conference placement and the gateway's handover pick
  members by type (`least_loaded_near`, `handover_peer`), drain hands off to a peer.
  Turning it on everywhere to fill a page would turn all of that on too.
- **Five processes cannot be listed at all.** `fcp-jobs`, `fcp-api-gateway`,
  `fcp-registrar`, `fcp-link` and `fcp-admin` itself have no membership. The nginx
  proxy is not ours.
- **A dead process vanishes.** Its key expires 15 s after its last heartbeat, and the
  page lists what is in Redis: a crashed voicemail server is simply not there, which
  looks the same as one that was never deployed.
- **"Healthy" means "not draining".** Every binary's `/ready` is
  `fcp_sip::drain::ready_body`: ready unless draining. None checks its database,
  Redis or the services it calls. The membership loop probes the database only while
  the cluster view is not whole, for the partition banner.
- **A site's nodes** reach the cloud on the link's heartbeat as `SiteNode { node_id,
  node_type, version, healthy }` (`fcp-core/src/site.rs`), read by the link from the
  site's Redis, so they have the same gap.
- **The menu.** `Cluster` and `Sites` sit in the admin UI's "Advanced" section
  (`AdminLayout.tsx`) between Flow Traces and Compliance, beside routing, schedules,
  tokens and SSO. `/cluster/nodes` needs `SystemViewLogs`.

## 2. The model

**A service is a running process, and every one says it is there.** Separate from
cluster membership:

| | Cluster member | Service |
|---|---|---|
| Who | the ten that can join, when `[cluster] enabled` | every FCP binary with a Redis, always |
| Key | `fcp:cluster:node:<id>` | `fcp:service:<id>` |
| Used for | placement, handover, trunk ownership, admission, drain | the Services page, alerts |
| Changes behaviour | yes | no |

A process with no Redis (a laptop's single binary) is not listed and says so in its
log once.

**What a service says**, every 5 s with a 15 s TTL (the membership's numbers):

- who: `id` (`[service] node_id`, else the host name and the binary), `kind`
  (`call_manager`, `sbc`, `voicemail`, `conference`, `ivr`, `media`, `presence`,
  `tts`, `webrtc_gateway`, `events`, `jobs`, `api_gateway`, `registrar`, `admin`,
  `link`), `host`, `version`, `started_at`, `region`, whether it is also a cluster
  member;
- where: its HTTP address, and its SIP and public addresses where it has them;
- state: `serving`, `draining`, or `degraded`;
- **checks**: one line per thing it depends on — `database`, `redis`, and the services
  it calls (the TTS server for voicemail, the media server for the call manager, the
  events server for anything that posts to it) — each `ok` or `failing` with the error
  and since when, measured by the process on its own timer, never on a request;
- **load**: a few numbers the kind has — calls, conference rooms and participants,
  gateway sessions, jobs queued and failed, events delivered — and the capacity it
  advertises where it has one.

`degraded` is a failing check. Readiness (`/ready`) carries the same checks, so a load
balancer and the page agree; it answers 503 only for a failing *required* check (the
database, Redis), not for a dependency the process can serve without. Compose's
health checks stay on `/health`, so a dependency's outage never restarts a container.

**The admin keeps a record of every service it has seen**, in Postgres
(`service_instances`, migration 127): id, kind, host, first seen, last seen, last
version, last state. One sweep every 5 s in the admin (only the admin holding a Redis
lock, when there are several) compares the keys with the record:

- a new key: a new row, `system.service_up`;
- a key gone: `down since <last seen>`, `system.service_down` once;
- a key back: `system.service_up` with how long it was down;
- a row down for longer than `[services] forget_after_days` (default 7) is dropped,
  and an operator can forget a decommissioned one at once.

`fcp_service_up{id,kind}` and `fcp_service_check_failing{id,kind,check}` on the
admin's `/metrics` let Prometheus alert without polling every node.

**When Redis is gone**, no service can say it is there. The page says that Redis is
unreachable and shows the record as it last was, not every service down; the sweep
sends no `service_down` while it cannot read Redis.

**A site's services** come up on the link's heartbeat: the link reads the site's
`fcp:service:*` instead of its members, and `SiteNode` grows the state, checks and load
(old fields kept). The cloud keeps them in the same record under the site's id.

**Who sees it.** The platform's services are the platform's: only a platform
administrator (and roles holding `system_view_logs`, as the Cluster page today) sees
them. A tenant administrator sees their sites' services and nothing of the cloud's.

## 3. The admin UI

A **System** section in the sidebar, after Advanced:

| Page | What it is |
|---|---|
| **Services** (new, first) | every service: kind, id, host, state, version, uptime, region, checks, load; grouped by host or kind; filtered by kind, state and site; a red row for down with "down since"; a service's own page with its checks, load, addresses and recent up/down history; Forget on a down row |
| **Cluster** | as today: members, regions, version skew, the partition view |
| **Sites** | as today, with each site's services linking to Services filtered by that site |
| **System Settings**, **Audit Log**, **Compliance** | moved from Advanced |

Advanced keeps what shapes calls: trunks, routing, schedules, classes of service,
LCR, flows and traces, tokens, webhooks, SSO. The dashboard gets one tile: services up
of services known, linking to Services filtered to what is not serving.

## 4. Phases

**S1. Every process says it is there** (FCP). `fcp_cluster::ServiceBeacon`: the record
in §2, on its own key and timer, started by every binary that has a Redis — the ten
with membership, and the admin, jobs, API gateway, registrar and link. Checks in a
small `fcp_cluster::checks` (database, Redis, an HTTP dependency's `/health`), the
binary naming what it depends on; `/ready` answering from them. Load per kind from
counters the binaries already keep. The compose configs need nothing new; the
membership flags stay as they are. Tests: `fcp-cluster/tests/services.rs` against
Redis (beacon written, refreshed, expired; a failing check makes `degraded`), and the
compose stack test reading one beacon per container.

**S2. The admin's record** (FCP). Migration 127 (`service_instances`), the sweep under
a lock, up/down events, metrics, the Redis-unreachable state;
`GET /api/v1/system/services` (the record joined with the live beacons, paged like
every list), `GET /api/v1/system/services/{id}`, `DELETE …/{id}` to forget one that is
down; the link carrying a site's beacons up; routes in `route_permissions::ROUTES`,
the OpenAPI contract and both SDKs regenerated. Tests: `api_postgres` (a service seen,
gone, back, forgotten; a tenant administrator sees only their sites'), a sweep test
with Redis stopped, `fcp-link/tests` for a site's services.

**S3. The System menu and the Services page** (FCP admin UI). The section, the moves,
Services and a service's page, the dashboard tile, live updates from
`system.service_up`/`_down` on the admin's WebSocket. Tests: the UI's Playwright suite
(a service listed, one stopped shown down, forgotten).

Then D8 by hand: stop the voicemail container, see it red within 15 s with the time
it went, start it, see it back; stop the TTS server, see voicemail `degraded` with its
`tts` check failing.

### 4.1 S1 as built (v0.25.476)

- **`fcp_core::service_health`**: `ServiceState` (`serving`, `degraded`, `draining`,
  `stopping`), `CheckResult` (name, required, ok, error, since) and `ServiceHealth`, the
  checks and load shared by the beacon and `/ready`.
- **`fcp_cluster::services`**: `ServiceIdentity`, `ServiceBeacon` (builder: `check`,
  `load`, `draining`; `start(RedisConfig)` → `BeaconHandle`), `ServiceRecord`, and
  `ServiceBeacon::read_all` for the admin. The beacon connects in its own task and
  again while Redis is away, so a process never waits on it; every five seconds it
  runs its checks (three seconds each, together), its load, and writes
  `fcp:service:<id>` with a fifteen-second TTL. The `redis` check is the write
  itself. On shutdown it writes `stopping` once and leaves it to expire.
- **Checks**: `Check::tenants` (a tenant that does not exist, read through the
  binary's store), `Check::database` over `PostgresStorage::ping` (`SELECT 1`),
  `Check::http` (a dependency's `/health`, not required), `Check::new` for anything
  else.
- **`/ready`**: `fcp_sip::drain::Drain::set_health`, and `ready_body` answering 503
  for a failing required check as well as draining, with `checks` in the body; the
  SBC's own readiness does the same.
- **Every binary**:

| Binary | Kind | Redis from | Checks | Load |
|---|---|---|---|---|
| call manager | `call_manager` | `[storage] redis_url` | database, events, tts | calls |
| SBC | `sbc` | `redis_url` | database | calls |
| voicemail | `voicemail` | `[storage] redis_url` | database, events | calls |
| conference | `conference` | `[storage] redis_url` | database, events | participants |
| IVR | `ivr` | `[storage] redis_url` | database, events | calls |
| presence | `presence` | `[storage] redis_url` | database | subscriptions |
| WebRTC gateway | `webrtc_gateway` | `[storage] redis_url` | — | sessions |
| media | `media` | `[storage] redis_url` | database | — |
| TTS | `tts` | `[cluster] redis_url` | — | — |
| events server | `events` | `[cluster]` or `[event_bus] redis_url` | database (with webhooks or notifications) | — |
| jobs | `jobs` | `[redis] url` | database (with the CDR processor) | jobs_queued, jobs_failed |
| API gateway | `api_gateway` | `[redis] url` | — | — |
| registrar | `registrar` | `[storage] redis_url` (new, optional) | database | registrations |
| admin | `admin` | `[cluster] redis_url` | database, events | — |
| link | `link` | `[site] redis_url` | cloud (not required) | — |

- **Compose**: the media server's and TTS server's configurations gained a Redis
  (`[storage] redis_url`, `[cluster] redis_url`), the registrar's `[storage]
  redis_url`; membership flags unchanged.
- **Tests**: `fcp-cluster/tests/services.rs` (written, refreshed, degraded, not ready,
  draining, stopping and expired; started without Redis), `drain::tests`
  `ready_answers_from_the_services_checks`, `fcp-registrar` smoke
  `a_registrar_says_it_is_there` (the binary's beacon, `/ready`'s checks, its last
  `stopping`), `compose_stack.rs` (a beacon from each of the twelve default services on
  this release).

### 4.2 S2 as built (v0.25.477)

- **The record**: `fcp_core::service_registry` (`ServiceInstance`,
  `ServiceInstanceStore`), migration 127 `service_instances` keyed by (`site_id`,
  `id`), `''` for the platform's own; `fcp-storage-postgres/src/service_registry.rs`.
- **`fcp-admin/src/services.rs`**: `reconcile` (one scope's known records against its
  live beacons: new or back is up, gone is down once, `stopping` is down at once and
  stopped), `ServiceRegistry::observe`, and the sweep — every 5 s, while holding
  `fcp:services:sweep` (15 s) in the admin's Redis, so one admin of several speaks;
  nothing is said down while the beacons cannot be read; hourly, records down longer
  than `[services] forget_after_days` (7) are dropped.
- **Events**: `system.service_up` (`down_secs` when it was down) and
  `system.service_down` (`stopped`, `last_seen`), on the admin's bus and posted to the
  events server for webhooks.
- **Sites**: `SiteHeartbeat.services`, the beacons in the site's Redis read by the link
  (`ServiceBeacon::read_all`); the cloud's heartbeat handler observes them under the
  site's id. A site whose link is down shows its services `unknown`. `ServiceRecord`
  moved to `fcp_core::service_health` for this.
- **API** (`/api/v1/system/services`, the routes behind `require_platform_admin` and
  `system_view_logs`; forgetting needs `system_config`): the list (`kind`, `state`,
  `site_id` or `platform`; paged), `summary` (counts, whether Redis can be read, when
  it was last read), one service, and `DELETE` for one that is down (409 while up).
  A service's state: `serving`, `degraded`, `draining`, `stopping`, `down`, `stopped`,
  or `unknown` while Redis or the site's link cannot be read. In the contract, the UI's
  types and both SDKs.
- **Metrics**: `fcp_api_core::MetricsSource`, which the admin's `/metrics` renders:
  `fcp_service_up{id,kind,site}` and `fcp_service_check_failing{id,kind,site,check}`.
- **Tests**: `services::tests` (seen, lost once, back with how long, stopped; scopes
  apart), `fcp-admin/tests/services.rs` (an admin with Redis: itself listed, a beacon
  seen with its checks and load, 409 while up, down, back with its first seen kept,
  stopped, the summary and filters, the metrics, forgotten, 403 for a tenant's
  administrator), `fcp-link/tests/services.rs` (the link's own beacon reaches the
  cloud's record under the site by name, not among the platform's; unknown or stopped
  once the link stops).
- **Found on the way**: `/api/v1/cluster/nodes` and `/cluster/stats` need only
  `system_view_logs`, which a tenant's administrator holds, so in a hosted deployment a
  tenant's administrator can read the platform's cluster. Closed in S3.

### 4.3 S3 as built (v0.25.478)

- **The System section** (`AdminLayout.tsx`): Services, Cluster, Sites, System
  Settings, Audit Log and Compliance, moved out of Advanced; Services and Cluster are
  `platformOnly` and hidden from anyone but a platform administrator
  (`role === 'super_admin'`).
- **The Services page** (`pages/ServicesPage.tsx`, `/admin/services`): four counts
  (serving, needs attention, down, unknown) that filter on a click, a banner while the
  admin cannot read Redis, filters by kind, state (or "not serving") and where (the
  platform or a site) kept in the URL, and a table of state, service, kind, site and
  host, version, up or down for how long, failing checks, and load. **A service's
  page** (`/admin/services/:id`, `?site_id=` for a site's): its state in words, checks
  with their errors and since when, load, addresses and times, and Forget once it is
  down or stopped. Both refetch every five seconds and at once on
  `system.service_up`/`_down` from the admin's event stream.
- **The dashboard** shows a platform administrator "N of M serving", leading to the
  services not serving; the command palette finds Services.
- **The gap S2 found, closed**: the whole cluster router (`/cluster/nodes`,
  `/cluster/stats`, `/cluster/sites`) is behind `require_platform_admin`; a tenant's
  administrator sees their own sites at `/sites`.
- **Tests**: `e2e/services.spec.ts` (the tile; Services in the navigation; the admin
  serving and a stopped voicemail server; "not serving" in the URL; its page and
  Forget; a tenant's administrator sees neither Services nor Cluster nor the tile),
  with `fcp-admin/tests/ui.rs` giving the admin a Redis and writing the stopped
  service; CI's browser job gained a Redis. `pages.spec.ts` opens both new pages with
  every other one.

## 5. What this does not do

- No restart button. The admin is not the process supervisor (Docker, systemd,
  Kubernetes are); a page that cannot restart reliably everywhere should not offer to.
- No logs. Finding a service's logs stays with the deployment's log store.
- The nginx proxy and Postgres, Redis, Prometheus themselves are not FCP services; the
  `database` and `redis` checks of every service stand in for the last two.

## 6. Questions (agreed 2026-10-29, every recommendation)

**Q1. Turn membership on in the shipped configs as well?** With S1 the page no longer
needs it. *Recommendation:* no; membership stays a clustering choice, documented in
the runbook as such.

**Q2. Should Cluster fold into Services?** *Recommendation:* not now; two pages, the
Cluster page linking each member to its service. Revisit once Services exists.

**Q3. Forget after how long?** *Recommendation:* 7 days, configurable, and Forget by
hand at any time.

**Q4. Alerts in FCP or only in Prometheus?** *Recommendation:* both cheap ones — the
events (so a webhook or the on-site notifier can say it) and the metrics; no alerting
rules engine in FCP.

**Q5. Show a site's services to the tenant's administrators?** *Recommendation:* yes,
their own sites' only.
