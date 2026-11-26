# Logs in the admin (D8 finding 63)

Proposed 2026-11-25; every question in §7 agreed on its recommendation the same day.

> 63. There is no way to easily view logs from the admin interface without
> logging into each server. Can we implement a log viewer page where an
> admin can view the logs for all the services in real time, search,
> filter, etc? A tenant admin should only see logs for their tenant and
> not the system.

## 1. What there is

- **Fifteen binaries, fifteen ways to start logging.** Each has its own
  private `init_tracing`/`init_logging` building a `tracing_subscriber`
  from `RUST_LOG` or `[logging] level`, writing to stdout as `pretty`,
  `compact` or `json`. Unknown formats fall back differently (the admin
  to json, the call manager to pretty, the link to compact).
  `fcp_core::config::ObservabilityConfig` exists and nothing uses it.
- **Nothing collects them.** No Loki, Vector, OTLP, syslog, file
  appender or Redis stream; the compose file sets no logging driver.
  `docs/SERVICES.md` §5 says so: "finding a service's logs stays with the
  deployment's log store". On D8 that means `docker logs` on the VM.
- **Almost no line names its tenant.** `tenant_id` appears as a field in
  about 160 macros, mostly in `fcp-api-core` and `fcp-sip`. No span
  carries a tenant: the SIP handler's spans carry the call and dialog, the
  conference bridges' the room. A call's lines, by and large, do not say
  whose call it is.
- **What the admin already has to build on.** Every binary writes a
  service beacon (`fcp:service:<id>` in the cluster's Redis, every 5 s);
  the admin reads them all for the Services page. The admin's
  `/api/v1/events` WebSocket authenticates by a first message and clamps
  a tenant's identity to its tenant. A site sends its services' beacons
  up on its heartbeat and its events on `/link/v1/events`.
- **Lines that should never be shown.** D8 found the voicemail server
  logging the events token in a URL (#340, fixed); forge-rtp once wrote
  44 million lines in a day. No redaction exists beyond `--check-config`'s
  masking.

## 2. The model

**One logging setup for every binary** (`fcp_core::logging::init`): the
stdout output each has today, unchanged by default, plus a layer that
ships each record (at or above a level) to the cluster's Redis.

**A record** is JSON: time, the service's id and kind (as on its beacon),
level, target, message, the event's fields, and two lifted out —
`tenant_id` and `call_id` — from the event's own fields or the nearest
enclosing span that has them.

**Kept in Redis, bounded.** One stream per service,
`fcp:logs:<service_id>`, trimmed by `XADD MAXLEN ~` (§7 Q2). The cluster
Redis is already there for every node; a stream gives the tail
(`XREAD BLOCK`) and the window (`XRANGE` by time) for free, and losing it
loses only recent lines. This is a viewer for the last day, not an
archive: an operator who wants months keeps their own log store, fed from
stdout as today (or by OTLP, §6 L6).

**Tenants by span.** The API's request middleware opens a span with the
request's tenant; the SIP handler records the tenant on the call's span
once it is known; flows, the IVR, voicemail, the conference server and
queues do the same. Inner lines inherit it. A line without a tenant is
the platform's.

**Who sees what.**

| Viewer | Sees |
|---|---|
| Platform administrator | Every service's lines, any tenant or none |
| Tenant administrator (`log_read`) | Only lines tagged with their tenant, from any service. Never an untagged line, never another tenant's |

A tenant's lines can still name a platform address or a trunk's; that is
their call's data, as a SIP trace is.

**Redaction at the source.** The shipping layer replaces the values of
fields named like secrets (`token`, `password`, `secret`, `key`,
`authorization`, `api_key`) and anything that looks sealed (`enc:v1:`) or
like a bearer token in a message, before the line leaves the process.
Stdout gets the same treatment, so the fix is everywhere.

**Volume.** Shipped at `info` by default; a platform administrator can
raise or lower a service's level from the page (applied through a reload
handle, kept in Redis, reverting after an hour). Each service ships at
most N lines a second; beyond that it counts what it dropped and ships
one line saying so.

## 3. The API

- `GET /api/v1/logs` — `service`, `kind`, `level` (at least),
  `tenant` (platform only), `call_id`, `q` (text in message and fields),
  `since`/`until`, `limit`, `cursor`; newest first, the usual page.
- `WS /api/v1/logs/tail` — the same filters, first message
  authenticates (as `/api/v1/events`).
- `GET /api/v1/logs/levels`, `PUT /api/v1/logs/levels/{service}` —
  platform only.
- `GET /api/v1/logs/export` — the filtered window as JSON Lines,
  audited.

Permissions: `log_read` (tenant administrators, clamped to their tenant)
and platform-only for levels and untagged lines.

## 4. The page

**System › Logs** for the platform, **Logs** in a tenant's menu: a live
tail (pause, resume, follow), filters by node type, service, level,
tenant, call id and text, a time range, each line expandable to its
fields, a call id linking to the call's SIP trace and record, and export
of what is shown.

## 5. Sites

A site's services ship to the site's own Redis, so the site's admin has
the same page. Lines at `warn` and above also go up the link, batched
like its events, into `fcp:logs:site:<site>:<service>` in the cloud, so
the platform sees a site's trouble without reaching into it.

## 6. Phases

- **L1 — one logging setup and the shipper.** `fcp_core::logging` used
  by all fifteen binaries (the formats and environment variables kept),
  the Redis layer, records, redaction, the rate limit, the stream trim.
  Proved by a binary's lines arriving in its stream, redacted.
  **Done (v0.25.642):** `fcp_cluster::logging::init` in every binary
  (each one's `[logging] level`/`format` and `*_LOG_FORMAT` as before;
  `RUST_LOG` still wins), stdout and shipped lines redacted
  (`logging::redact`), records with `tenant_id`/`call_id` from the line
  or its spans, shipping started by `ServiceBeacon::start` to
  `fcp:logs:<service id>`. Tuned by the environment for now:
  `FCP_LOG_SHIP` (`info`; a level, or `off`), `FCP_LOG_SHIP_LINES`
  (100000), `FCP_LOG_SHIP_HOURS` (24), `FCP_LOG_SHIP_RATE` (500 a
  second). Proved by `fcp-cluster/tests/logs.rs`.
- **L2 — tenants on lines.** The API middleware's span, the SIP
  handler's call span recording the tenant, the flow, IVR, voicemail,
  conference and queue paths; measured by a test placing a call and
  counting how many of its lines carry the tenant.
  **L2a done (v0.25.646):** every SIP request the call manager handles
  runs in a `sip` span (method, SIP Call-ID, and `tenant_id`/`call_id`
  recorded once known: `Call::note_on_span` at the call's creation and
  wherever an in-dialog request finds its call, the registrar on
  resolving a REGISTER's tenant, ACKs too); work the handlers spawn keeps
  the span (`fcp_sip::spawn_here`), so the forking race, media and flows
  name the call; the mid-dialog spans carry the tenant; the standalone
  registrar's requests get the same span; the API's request span takes
  the tenant `TenantContext` resolves (its path left out, since some
  carry a key). Measured by `fcp-call-manager/tests/calls.rs`
  `a_calls_log_lines_name_its_tenant`: of 113 lines shipped for a call
  between two phones, all 27 naming the call name its tenant. A SIP
  library's `call_id` field is the SIP Call-ID, not the node's call.
  **L2b:** the conference server, IVR, voicemail server and queues.
- **L3 — the API.** Query, tail, levels, export, the permissions and the
  tenant clamp; proved across two tenants.
  **L3a done (v0.25.647):** `log_read` (tenant administrators and
  read-only monitors; a platform administrator holds everything) and,
  in fcp-admin (`logs.rs`, the admin's `[cluster] redis_url`):
  `GET /logs` newest first across every service's stream, filtered by
  `service`, `kind`, `level` (and worse), `tenant` (a platform
  administrator's; `none` for the platform's own), `call_id`, `q` (any
  case, message, target and fields), `since`/`until`, paged by the last
  stream id; `GET /logs/export` as JSON Lines (audited, as every
  `/export`); `GET /logs/levels` and `PUT /logs/levels/{service}`
  (platform only): `fcp:logs:level:<id>` for an hour, which the service's
  beacon takes up within a tick (`logging::set_level`, a reloaded filter;
  shipping follows it, never quieter than configured) and gives back
  when it expires or is deleted. A tenant's caller is held to lines
  naming its tenant whatever it asks. Proved by `fcp-admin/tests/logs.rs`
  and `fcp-cluster/tests/logs.rs`. **L3b:** the live tail
  (`WS /logs/tail`), with the page.
- **L4 — the page,** with a Playwright spec.
  **L4 done (v0.25.650):** System › Logs (`LogsPage`): the newest 200
  lines followed every two seconds (Pause/Follow) or paged back (Older
  lines), filtered by level and worse, time (15 minutes to everything
  kept), text, service, call and — for a platform administrator — tenant;
  a line opened shows its target, tenant, call (a click filters by it)
  and fields; Export downloads the view as JSON Lines; a platform
  administrator's Levels card raises a service's level for an hour and
  takes it back. The live tail is the API polled; `WS /logs/tail` (L3b)
  is not needed for it. Proved by `e2e/logs.spec.ts`.
- **L5 — sites,** `warn` and above up the link.
- **L6 — OTLP export** (optional): the same layer sending to an
  operator's collector.

## 7. Questions

1. **Where lines are kept.** Redis streams in the cluster's Redis
   (recommended: there already, a tail for free, bounded), a Postgres
   table (searchable for longer, but writes on every line), or require an
   external store (Loki) and only read it.
2. **How much.** Per service, the last 100,000 lines or 24 hours,
   whichever is less (recommended); adjustable in `[logging]`.
3. **Level shipped.** `info` (recommended) or `warn`.
4. **Tenant administrators** see only lines tagged with their tenant
   (recommended), or nothing at all.
5. **Changing a service's level from the page:** yes, platform only,
   reverting after an hour (recommended), or not at all.
6. **Sites:** their own page, and `warn`+ up to the cloud (recommended),
   or nothing up.
7. **OTLP export:** a later phase (recommended), or now.
8. **Export of a view** as JSON Lines, audited (recommended), or none.
