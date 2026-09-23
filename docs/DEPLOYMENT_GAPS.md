# The deployment guide's gaps

Status: **agreed** (2026-09-23), survey done, all three questions in §3
settled by the user; **phases 1–5 done** (v0.25.236–240). Chosen when the multi-region leftovers closed
(`docs/MULTI_REGION_FOLLOWUPS.md` §5, the decision point).

`docs/MULTI_REGION_DEPLOYMENT.md` was written against the code and ends
with a list of nine things it found on the way (§8 there). This document
is the survey of that list — each gap checked against the code, what
turned out to be worse than the guide says, what the guide got wrong —
and the phases that fix them. The first item on the guide's list, one
Redis per call manager, is the regional-registrations project the
decision point declined; it stays on the list and is not here.

Read with `docs/MULTI_REGION_DEPLOYMENT.md` (the guide),
`docs/CAPACITY_AND_SURVIVABILITY.md` (§6 draining, whose token rules one
binary skipped), `docs/TENANT_TRUNKS.md` (the trunk model) and
`docs/DISTRIBUTED_CONFERENCING.md` (placement, which routes callers to
the address one gap gets wrong).

## 1. The survey

Eight gaps, checked. File references are to `crates/` unless they start
with `external/` or `docs/`.

### 1.1 A conference server advertises its SIP bind address as-is

**As the guide says, and worse.** The heartbeat record is built at
`fcp-conference-server/src/main.rs:607-617`: `http_address` comes from
`advertised_http_address()` and `sip_tls_address` from
`advertised_tls_address()`, both of which replace an unspecified bind IP
with `[sip] local_ip` (`config.rs:69-87`, `:319-326`, each with a test);
`sip_address` is `udp_bind` parsed as-is. `local_ip` is nearly always set
by the time that line runs — `--public-ip` / `PUBLIC_IP` fills it
(`main.rs:138-143`) and otherwise it is auto-detected when the bind is
unspecified (`main.rs:146-161`) — and the same file already derives a
routable address for its own Via (`main.rs:2000-2006`). The heartbeat is
the one place that ignores it.

Three consumers send callers there, not one:

- the call manager's placement, `fcp-call-manager/src/conference_placement.rs:82`,
  returns `pick.sip_address` as the target host (`fcp-sip/src/handler.rs:1051-1060`);
- the WebRTC gateway's placement, `fcp-webrtc-gateway/src/placement.rs:107-108`;
- **participant migration**, `fcp-conference-server/src/migration.rs:82-87`,
  builds the `Refer-To` as `sip:<room>@<target.sip_address>` — a
  participant moved off a draining node is sent to `0.0.0.0` too.

The inter-node trunks do not have the bug: they dial `sip_tls_address`
(`trunks.rs:257`), which is why the TLS address was fixed and the UDP one
was not. `docs/DISTRIBUTED_CONFERENCING.md:90-92` describes the raw bind
as the intended behaviour.

**Every other SIP binary does the same** — call manager `main.rs:189-193`,
gateway `:233-237`, voicemail `:289-293`, IVR `:283-287`, presence
`:228-232`, and the SBC's `bind_addr` (`fcp-sbc/src/main.rs:726`) — and
`http_address` is the raw bind everywhere but the conference server. Only
the conference server's `sip_address` is read by anything today, so the
others are cosmetic (the admin UI's node list shows `0.0.0.0`), but there
is no shared helper: `derive_local_ip` and `auto_detect_local_ip` are
copied into seven binaries.

### 1.2 The SBC's membership node id is random

**As the guide says, and there are four ids, not two.** Membership uses
`format!("sbc-{}", Uuid::new_v4())` (`fcp-sbc/src/main.rs:719-720`) and
ignores `[service] node_id`, whose own doc says it is "this node's id in
events and in cluster membership" (`config.rs:35-39`). The drain computes
its id separately (`main.rs:770-774`, the configured id or *another*
fresh UUID), so even with `node_id` unset the two never match; the drain
sets the flag on the random membership record while its events name the
other id. The B2BUA handler gets `String::new()` as its node id
(`main.rs:582`) and the trunk-registration context the literal `"sbc"`
(`main.rs:856`). No other binary has the mismatch: each passes the same
`config.service.node_id` to membership and drain. The SBC's field is an
`Option<String>` where every other binary has a `String` with a default,
which is why the fallback is inline.

(The survey first read the presence server's `[redis] node_id` as dead;
it is not — `main.rs:671` gives it to the presence Pub/Sub as that
node's identity, falling back to `[service] node_id`. Left alone.)

### 1.3 `fcp-registrar`'s `/drain` takes no credentials

**As the guide says, and `/status` is open too.** The registrar's router
(`fcp-registrar/src/main.rs:377-386`) has `/health`, `/ready`, `/status`
and `/drain` with no layer; its `[http]` section has only `bind_address`,
defaulting to `0.0.0.0:8083` (`config.rs:245-252`, and the compose config
binds there), and `validate` checks the SIP binds but never the HTTP one.
There is nothing to reuse: `AppState` holds a start time, a counter and
the drain; the config has no `[cluster]` and so no cluster secret.

Every other binary with a `/drain` protects it — the call manager, IVR,
voicemail and presence with `[http] api_token` and a refusal to bind
publicly without one; the SBC with `--health-api-token`; the conference
server with `[http] auth_token` under `auth_enabled`; the gateway with
`[auth] tokens` — and none shares the extractor: `fcp-sip/src/drain.rs`
provides the response bodies only, and `require_api_token` is copied per
binary. Six binaries have no `/drain` at all (media, events, TTS, gateway
API, jobs, admin).

Two documents say the wrong thing: `docs/CAPACITY_AND_SURVIVABILITY.md:566-568`
says the call manager's `/drain` is open (it is not, since edge security),
and `docs/CLUSTER_OPERATIONS_RUNBOOK.md:143-145` lists which binaries need
the operator token and leaves the call manager out.

### 1.4 The API gateway's default health path

**As the guide says, and the corrected path fails too.** The default
`/api/v1/system/health` (`fcp-api-gateway/src/config.rs:152-153`) is used
for all three backend kinds. What exists: `fcp-api-core` serves `/health`
inside its v1 router (`v1/mod.rs:103`), nested at `/api/v1`, so the admin
backend's path is `/api/v1/health`; the events server and `fcp-jobs`
serve `/health` at the root. But `/api/v1/health` sits inside the bearer
layer (`fcp-api-core/src/lib.rs:124-165`, no exemption), `fcp-admin`
requires auth on a public bind, and the probe sends no token
(`main.rs:101-105`) — so an admin backend answers 401 and shows unhealthy
whichever path is configured. The probe runs only on demand from
`GET /gateway/status`; `health_check_interval` is parsed and never read.
The wrong path is copied into the example config, the compose config,
the gateway README and `docs/SINGLE-NODE-SETUP.md`.

### 1.5 `[events] redis_channel` and `redis_stream_key` are ignored

**As the guide says.** The two fields (`fcp-call-manager/src/config.rs:673-679`)
appear nowhere else in the repository; the bus is built with
`RedisEventBus::new` (`main.rs:504-512`), whose keys are
`{prefix}events` and `{prefix}event_stream` with the prefix hard-coded to
`fcp:` (`storage.rs:93`). `RedisEventBus::with_keys` exists and has no
caller. The events server derives the stream key from its own
`[event_bus] redis_key_prefix` (`fcp-events-server/src/main.rs:460-474`),
as does `fcp-jobs` from `key_prefix`; there is no stream-key setting on
the consuming side, so if the call manager honoured a custom key the
events server would silently stop seeing its events. A mismatch is
already possible the other way: set the events server's prefix to
anything but `fcp:` and call-manager events vanish without a warning.
The section round-trips through the database config store, dead fields
included.

### 1.6 Events ingested over HTTP never cross regions

**As the guide says, and they do not reach the region's other events
servers either.** `POST /ingest` and `/ingest/batch`
(`fcp-events-server/src/main.rs:1144-1181`) update this process's state
and publish to its in-memory bus; nothing is written to Redis, so the
bridge — which reads the Redis stream with one consumer group per peer
(`fcp-storage-redis/src/event_bus.rs:640-724`) — never sees them, and
neither does a second events server in the same region. The payload is a
bare `PlatformEvent` with no envelope, so there is no node id or region
to relay; the handler's own comment notes an `X-Node-ID` header "if
needed". Webhooks and the notifier do fire for ingested events, once, in
the ingesting process.

Who posts over HTTP: the conference server, IVR and voicemail server
(HTTP only, no `use_redis`; each already holds a `RedisStorage` but their
`fcp-storage-redis` dependency lacks the `event-bus` feature), and
`fcp-api-core`'s `publish_event` from the admin. The call manager and
`fcp-jobs` publish to Redis directly. The gateway, presence server and
SBC publish to nothing beyond their process.

### 1.7 No binary reports its replica's health

**As the guide says, and no binary can.** `PostgresStorage::replica_status()`
(`fcp-storage-postgres/src/lib.rs:507-516`) returns a `ReplicaStatus`
— host, reachable, healthy, lag, limit — fed by a probe every five
seconds; its only callers are the backend's own tests. Every binary that
configures a replica drops the concrete backend into `DynStorage` at
construction (`fcp-call-manager/src/storage.rs:17-38` and the same shape
in conference, IVR, voicemail, registrar, admin, SBC, presence, media
server) and keeps no way back to it. The two binaries that keep a
concrete handle cannot have a replica: the events server builds its
config without one, `fcp-jobs` hard-codes `read_replica_url: None`.
`ReplicaStatus` carries no probe time, so a stalled probe reads as
healthy for ever. `/metrics` has no replica series anywhere. Each
binary's `/status` is its own `json!`; the SBC, presence and media
servers have no `/status`.

### 1.8 Trunk SRV

**Narrower than the guide says in one place, wider in two.** A trunk's
outbound route is `sip:<user>@<host>:<port>` with the port always filled
in (`fcp-sbc-core/src/trunk_router.rs:739-756`), and RFC 3263 §4 makes
an explicit port skip NAPTR and SRV (`external/siphon-rs/crates/sip-dns/src/lib.rs:452-467`),
so the guide is right that INVITEs never consult SRV. But:

- **The route uses `hosts.first()` only.** The guide's advice — "list
  every carrier address in `hosts`" — gives no INVITE failover: one
  trunk is one route, and failover is across trunks
  (`resolve_outbound_ordered`, `:881-890`), never across a trunk's hosts.
  The node-side OPTIONS probe (`health_probes.rs:78-111`) loops over
  *every* host and calls the trunk healthy if any answers, so a trunk
  whose first host is dead is healthy while every call to it fails.
- **REGISTER does consult SRV.** `build_registrar_uri`
  (`trunk_registration.rs:373-388`) leaves the port out when it equals
  the transport default, so a trunk registration resolves through NAPTR
  and SRV — and, with no `transport=` parameter, in TCP-first order, so
  a UDP trunk's REGISTER can leave over TCP.
- **Even with SRV answers, one target is kept.** `IntegratedUAC::resolve_target`
  (`external/siphon-rs/crates/sip-uac/src/integrated.rs:1290-1318`) takes
  the first resolved target. The transaction layer's ordered multi-target
  failover (`sip-transaction/src/manager.rs:914`,
  `start_client_transaction_with_targets`) is used only for the RFC 5626
  flow fallback, and fires only on a transport error, not a timeout or a
  503.
- **A TLS trunk is stored on 5060.** `transport_port` is
  `NOT NULL DEFAULT 5060` (migration 032) and the write path binds
  `port.unwrap_or(5060)` (`fcp-storage-postgres/src/lib.rs:4884`), so a
  TLS trunk created without a port comes back as 5060 where the node-side
  default is 5061; the API response and the admin-side probe default to
  5060 too.
- **`host:port` in `hosts` is read three ways.** The router does not
  split it (`sip:u@host:5080:5060`), `trunk_set.rs:30-40` strips it for
  inbound matching, `fcp-api-core/src/trunk_health.rs:311` parses it.
- **Two OPTIONS probes disagree.** The node-side probe goes through the
  UAC with the trunk's transport; the admin-side one
  (`trunk_health.rs:225-300`, also behind the API's test endpoint) is a
  hand-built OPTIONS on a blocking UDP socket whatever the transport.
- **Inbound matching is A/AAAA only.** `trunk_set.rs:147-166` resolves
  `hosts` names with `lookup_host`; SRV targets would not be indexed. With
  `inbound_sources` set (`docs/TENANT_TRUNKS.md` §7) none of this matters.
- **The flow engine's emergency dial bypasses the resolver**
  (`fcp-sip/src/flow_handler.rs:1409-1452`): `lookup_host` and UDP,
  hard-coded, with a "Phase 5: integrate sip-dns" comment.

## 2. What is not worth doing

- **Consolidating `derive_local_ip` / `auto_detect_local_ip`** across the
  seven copies. The heartbeat helper in phase 1 is where the advertised
  address is decided; the detection copies are identical and harmless.
- **A `/drain` on the six binaries without one.** None routes calls
  (the media server's sessions are owned by a routing binary's drain).
- **The admin-side OPTIONS probe as a real UAC.** `fcp-admin` has no SIP
  stack; the probe is a reachability check and phase 6 makes it honest
  about that (the right default port, and its transport named in the
  result) rather than complete.
- **Regional preference in trunk selection.** The guide's §4.3 answer —
  a region's `[[trunks]]` carry that region's carrier — stands.

## 3. Questions

Three, settled by the user on 2026-09-23.

**Q1. How far does trunk resolution go?** — *Two phases, hosts
as a failover list first, then SRV.* Phase 6 makes a trunk's `hosts` an
ordered failover list that INVITEs actually walk (which is what the guide
already tells operators they have), and phase 7 makes an unset port mean
RFC 3263 — with siphon-rs keeping every resolved target and handing them
to the transaction layer's failover. The alternative is phase 6 alone,
correcting the guide to say SRV is not used. SRV is what a carrier's
documentation gives an operator, and the pieces exist on both sides of
the submodule boundary; the cost is one siphon-rs release.

**Q2. Where do ingested events join Redis?** — *At `/ingest`.*
The events server writes an ingested event to its Redis stream when Redis
is its bus, in an envelope with its own region and the node id from an
`X-Node-ID` header, and stops publishing it to its in-memory bus
directly — the consumer loop delivers it, once, like every other event.
That fixes peers and same-region replicas in one place and covers the
admin's events too. The alternative — a `use_redis` on the conference
server, IVR and voicemail server — is three binaries, three features
flags and the duplicate-delivery guard the call manager needed, and
leaves the admin's events behind.

**Q3. Does the API's `/health` need a token?** — *No.* It
answers `ok` and a version, and nothing else; every other binary's
`/health` is open. Exempting it lets the gateway's probe work against an
authenticated admin backend. The alternative is a token on the probe,
which puts an admin credential in the gateway's config for a liveness
check.

Decided without asking: the call manager's two dead `[events]` fields
are removed rather than honoured (§1.5 — the consuming side has no
matching setting, and the prefix is the one knob everything else has); a
value in a config file is refused at startup with the reason, so a
deployment that set them finds out. Replica health goes on `/status`
where a binary has one and on `/ready` where it does not (SBC, presence,
media server); it never fails readiness, since reads fall back to the
primary.

## 4. Phases

One PR each, in order. Each phase strikes its item from the guide's §8.

| Phase | What |
|---|---|
| 1 ✅ | **Identities and advertised addresses** (§1.1, §1.2). The SBC has one node id — `[service] node_id` a `String` with a default like every other binary — used by membership, the drain, the B2BUA handler and trunk registration. A `ClusterMember` is built from an *advertised* SIP address: a helper that replaces an unspecified bind IP with the node's `local_ip`, used by the conference server (where placement and migration read it) and by every other SIP binary's heartbeat. `docs/DISTRIBUTED_CONFERENCING.md` stops describing the raw bind as intended. Result: `fcp_core::transport::advertised_address` (re-exported by `fcp-cluster`), applied to the SIP **and** HTTP address of every heartbeat — call manager, conference server, IVR, voicemail, presence, gateway, SBC; the conference server's `SipSection::advertised_address()` beside its TLS twin; the SBC's four ids are one. The presence server's `[redis] node_id` turned out to be read (its Pub/Sub identity) and stays. Proved by two binary tests: a conference server bound to `0.0.0.0` heartbeats `127.0.0.1`, and an SBC's drain marks the record its configured id names. |
| 2 ✅ | **The registrar's HTTP surface** (§1.3). `[http] api_token` on `fcp-registrar`; `/status` and `/drain` behind it; a public bind refused without it, like the others; the two documents corrected. Result: done as written, the same constant-time bearer check the IVR has; the HTTP bind's default moves from `0.0.0.0:8083` to `127.0.0.1:8083` to match every other binary, so a deployment that relied on the default must now bind explicitly and set the token. The compose config for the registrar, which also bound its SIP publicly with `auth_enabled = false` — refused since edge security — is corrected too. |
| 3 ✅ | **Configuration that does not do what it says** (§1.4, §1.5). The gateway's health path defaults per backend kind (`/api/v1/health` for admin, `/health` for events and jobs), the API's `/health` is exempt from its bearer layer (Q3), `health_check_interval` drives a background probe whose last result `/gateway/status` reports, and the wrong path leaves the example configs and docs. The call manager's `redis_channel` and `redis_stream_key` are gone; setting them is a startup error naming the prefix as the knob. Result: done as written. `health_check_path` is now optional; each probe's result carries the path, the HTTP status or error, and its age, and a backend not yet probed reads `unknown` and leaves the gateway `degraded`. Proved by a binary test with two fake backends that answer only on their own routes: the gateway reports both healthy, at the right paths, and a status request adds no probe of its own. |
| 4 ✅ | **Ingested events reach Redis** (§1.6, Q2). `/ingest` and `/ingest/batch` write to the stream in an envelope — region local, node id from `X-Node-ID` or `ingest` — when the bus is Redis, and no longer publish locally; the conference server, IVR, voicemail server and admin send the header. Measured by the phase-1 bridge counters: a conference event raised in one region is counted out of its bridge and delivered to a WebSocket client in the other. Result: done as written (`RedisEventBus::publish_many_from`, one pipeline per batch); a stream that cannot be written answers `503`, so the poster's retry carries the event. The call manager's HTTP forwarder sends the header too, and the consumer now files every stream entry under its envelope's node. Proved by two events servers in two regions on two Redis databases: an event posted to one is tracked by that server once (through its own consumer), tracked by the other region's server (through the bridge), and counted out of the bridge. |
| 5 ✅ | **Replica health said** (§1.7). `ReplicaStatus` gains the probe's age; each binary's storage construction keeps a way to ask for it; it is on `/status` (call manager, conference, IVR, voicemail, registrar) and `/ready` (SBC, presence, media server) as `replica`, `null` when none is configured, and on `/metrics` as `fcp_replica_healthy`, `fcp_replica_reachable`, `fcp_replica_lag_seconds` and `fcp_replica_probe_age_seconds` on the binaries with a metrics handler. The guide's §6.3 gets its alert rows. Result: `fcp_storage_postgres::ReplicaHealth`, a handle each binary's storage construction keeps beside the trait objects, on `/status` and `/metrics` of the call manager and conference server, `/status` of the IVR, voicemail server and registrar, `/ready` and `/metrics` of the SBC, `/ready` of the presence server. Not the media server: its HTTP surface is forge-media's, and its database is a config store read at startup. Proved on the call manager with the primary standing in as its own replica, and by the backend's own replica test. |
| 6 | **A trunk's hosts are a failover list** (§1.8, Q1). `build_outbound_route` yields one route per host in order, so the ordered resolver walks a trunk's hosts before the next trunk; the node-side probe keeps health per host and a host known down is tried last; `host:port` in `hosts` is parsed in one place in `fcp-core` and validated by the API; a TLS trunk's default port is 5061 in the database (a nullable `transport_port`, unset meaning the transport's default), the API response and both probes; the admin-side probe says it is UDP. `docs/MULTI_REGION_DEPLOYMENT.md` §4.3 and `docs/TENANT_TRUNKS.md` say what `hosts` now does. |
| 7 | **SRV for trunks** (§1.8, Q1). siphon-rs first: `resolve_target` keeps every RFC 3263 target and the new-INVITE path hands them to the transaction layer's ordered failover, on a transport error, a timeout or a 503; then a release tag and the bump. In FCP: an unset port leaves the port out of the route URI (and the `transport=` parameter is always present, so REGISTER stops leaving over TCP), the node-side probe resolves through `SipResolver` rather than `RequestTarget::Resolved`, SRV targets are resolved into the inbound index, and the flow engine's emergency dial goes through the same path. The guide's §4.3 says SRV is used when the port is unset. |

## 5. Testing

- Phase 1: a conference server bound to `0.0.0.0` advertises `local_ip`
  in the two-node cluster harness, and a placed call and a migration
  `Refer-To` carry it; an SBC restarted with a configured id keeps it,
  and its drain flag lands on the record its events name.
- Phase 2: the registrar's `/drain` answers 401 without the token and
  works with it; a public bind without a token is refused at startup.
- Phase 3: the gateway's `/gateway/status` shows an authenticated admin
  backend healthy; a config with `redis_stream_key` set fails to start
  with the message.
- Phase 4: two events servers on one Redis both deliver a `/ingest`
  event to their WebSocket clients, once each; with a peer bridge, the
  event is counted out and delivered in the other region.
- Phase 5: the Postgres-backed suite already exercises `replica_status`;
  the binary smoke tests read `replica` on `/status` with and without a
  replica configured.
- Phase 6: the floor's trunk tests get a trunk with two hosts, the first
  unreachable, and the call lands on the second; the probe reports the
  first down and the second up.
- Phase 7: siphon-rs's own tests for multi-target INVITE failover; in
  FCP, a trunk with an SRV name and no port resolves through a test
  resolver to two targets and fails over between them; the flow engine's
  emergency dial reaches a TCP-only target.
