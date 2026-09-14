# The CDR Pipeline — Design

Status: **phases 1–4 done** (2026-09-14, v0.25.159 to v0.25.163);
design agreed the same day with every recommendation in §14 taken. This document is the
plan of record for the dedicated CDR pipeline that `STATUS.md` kept on
the Phase 5 roadmap on 2026-09-02 ("inline CDR creation plus `fcp-jobs`
post-processing is sufficient until carrier call volumes"). It describes
what a call detail record is on FCP, how one gets from the node that
owned the call into the system of record without being lost, and how
the records are read once there are hundreds of millions of them.
`STATUS.md` tracks progress; this document describes intent and the
decisions behind it.

Read with `docs/BILLING.md` (rating and the ledger, which the pipeline
takes over), `docs/COMPLIANCE.md` (retention and export, which it
changes) and `docs/MULTI_REGION.md` (the event bridge, whose stream
consumer it reuses). `docs/ARCHITECTURE-v0.2.md` §8 is the original
sketch — collector, normalizer, enricher, three sinks — and this design
is that sketch made concrete against the code as it is.

## 1. Decisions

Proposed 2026-09-14, after surveying the CDR path end to end (§2). Each
is a recommendation until agreed; the questions that need a ruling are
in §14.

1. **The node that owned the call writes the record, complete, once, at
   the end.** The record is not stitched from `call.*` events in a
   consumer. The events are published fire-and-forget, the durable
   stream behind them is trimmed at ten thousand entries, and stitching
   needs per-call state in the consumer that a restart loses. The node
   already knows everything the record needs at hangup.
2. **Spool, then ship.** Every record is appended to a local
   append-only spool before the hangup path moves on, and a shipper
   drains the spool into the transport. A record is lost only with the
   node's disk. This is what every carrier-grade switch does, and it is
   the property the current fire-and-forget write lacks (§2).
3. **One processor, two ways to run it.** The pipeline is a library,
   `fcp-cdr`: normalize, enrich, rate, store, announce. With Redis
   configured it runs in `fcp-jobs`, consuming a dedicated stream. With
   no Redis it runs in-process on the node, behind the same spool. One
   code path, one set of tests, and a single-node install keeps working
   with Postgres alone.
4. **Rating leaves the call path.** The processor rates the record and
   settles the ledger; the node never touches rate plans or balances at
   hangup. Admission and mid-call cut-off stay on the node, because they
   must be synchronous.
5. **Idempotent by `cdr_id`.** The node mints a UUID per record; the
   store skips a duplicate, the ledger keeps its one-charge-per-call
   index, and the announcement carries the id so consumers can skip a
   repeat. Every stage may be replayed.
6. **One record per call, and other records beside it.** The
   winning-leg-only rule of `docs/PARALLEL_FORK_CDR_ANALYSIS.md` stands
   for the call record. A queue segment, a conference leg and a gateway
   leg are records of their own kind, correlated to the call by
   `call_id`; a transfer links the two calls by `correlation_id`.
7. **Reading at volume is part of the pipeline.** Monthly partitions,
   keyset pagination, a streamed export, hourly rollups for the usage
   routes, and retention by dropping partitions. A write path that never
   loses a record is half the job; the other half is the table still
   answering in a year.
8. **No new binary.** `fcp-jobs` is the worker the platform already
   ships for exactly this; the vision's `fcp-cdr-processor` becomes a
   role inside it, the way the standalone proxy worker was folded into
   the binaries that exist.

## 2. What is wrong today

The survey behind this design, with the places to look:

- **A record that fails to write is gone.** `terminate_call` in
  `fcp-b2bua-core/src/call_manager.rs` spawns the insert and forgets
  it; on error it logs "Failed to create CDR (non-blocking)" and the
  call has already been removed from the map. Nothing retries, spools or
  dead-letters. For a billing record this is the one property that
  matters, and it is missing.
- **Rating runs in that same task**, two storage round trips per hangup
  (`settle_cdr_cost` → `BillingEngine::settle`), so a slow or absent
  Postgres delays every record and a failure loses the charge with it.
- **Two writers, neither complete.** The B2BUA builds one shape; the
  queue handler (`fcp-call-manager/src/queue_handler.rs`,
  `create_queue_cdr`) builds another with a freshly minted call id, so
  the two records for one call cannot be joined, and it is never rated.
  The conference server, the WebRTC gateway, IVR and voicemail write no
  record at all.
- **The record is thin where it matters.** `hangup_party` is always
  `None` (a Phase 2 TODO since the MVP), so are all four RTP counters
  and `recording_url`; direction is not stored and is re-derived per row
  in SQL by two correlated `EXISTS` subqueries on `phone_numbers`; the
  disposition is chosen by substring-matching the human-readable failure
  string (`map_call_state_to_disposition`, `reason.contains("486")`);
  the codec is sniffed from the SDP text; there is no `cdr_id`,
  `correlation_id`, node, region or write timestamp; `cost` became
  `DOUBLE PRECISION` in migration 047 because sqlx could not decode the
  original `DECIMAL`.
- **The table will not scale as it is.** `call_id TEXT PRIMARY KEY`;
  no composite `(tenant_id, start_time)` index for the one query every
  page runs; `LIKE '%…%'` on the URI columns; `OFFSET` pagination with
  no tiebreaker; `query_cdrs` and `count_cdrs` both scan per page; the
  usage and billing summaries aggregate the live table on every request;
  the CSV export builds the whole body in memory and stops at a hundred
  thousand rows; partitioning is a commented-out sketch in
  `001_initial_schema.sql`. A row that fails to decode is silently
  dropped from a listing (`filter_map(.. .ok())`).
- **Retention runs in every admin replica**, an hourly `tokio::spawn`
  in `fcp-admin/src/main.rs` with no lease.
- **The async path described in `IMPLEMENTATION_DOCS.md` §2 does not
  exist.** `JobType::CdrProcessing` has a consumer in `fcp-jobs` and no
  producer anywhere in the workspace. Its retry-on-storage-error is the
  only such logic on the CDR surface, and it is unreachable.
- **Nothing tells an external system a rated record exists.** Webhooks
  fire on `call.ended`, before rating, and there is no `cdr` event.

None of this shows at the volumes the platform has run at. All of it
shows at the first carrier customer.

## 3. Goals and non-goals

Goals, in delivery order (§13):

1. No record lost short of losing the node's disk; a Postgres or Redis
   outage delays records and loses none.
2. The record complete: who hung up, why, in which direction, on which
   node and trunk, with what codec and RTP quality, rated against which
   plan, and correlated to the calls and legs around it.
3. Rating and settlement off the call path, still exactly once.
4. Every leg the platform carries produces a record: call, queue
   segment, conference leg, gateway leg.
5. Listing, export, usage and billing routes that answer in constant
   time whatever the table's size, and retention that costs a partition
   drop.
6. A `cdr.written` event, so webhooks and the event stream carry the
   rated record.
7. Every stage observable: spool depth and age, stream lag, processor
   throughput, dead letters.

Non-goals, for this design: geographic enrichment, fraud scoring, MOS
estimation, RADIUS or Diameter accounting, per-tenant push delivery
(SFTP, tenant buckets), Parquet, and lawful-intercept metadata (§15).

## 4. Architecture

```
  node (call manager, conference server, gateway, SBC)
  ┌────────────────────────────────────────────────────────────┐
  │ hangup ──► RecordBuilder ──► Spool (append, local disk)     │
  │                                 │                          │
  │                              Shipper ──► Sink              │
  └─────────────────────────────────┬──────────┬───────────────┘
                                    │          │
                   with Redis       │          │  without Redis
                                    ▼          ▼
                          stream fcp:cdrs    in-process Processor
                                    │              │
                   fcp-jobs         ▼              │
                          Processor (consumer      │
                          group cdr-processor)     │
                                    │              │
                    normalize ─ enrich ─ rate ─ store ─ announce
                                    │          │          │
                                    ▼          ▼          ▼
                                Postgres    ledger    cdr.written
                                 (cdrs)              (bus, webhooks)
```

### 4.1 The node: builder and spool

`fcp_cdr::RecordBuilder` replaces `create_and_insert_cdr` and
`create_queue_cdr`. It takes what the node knows — the call, its
termination reason, the media session's final `MediaSessionStats`, the
trunk, the node id and region — and produces a `Record` (§5). Building
is pure and synchronous; it never touches storage.

`fcp_cdr::Spool` is a directory of segment files, JSON Lines, one record
per line with a schema version. `append` is a synchronous write to the
current segment from the hangup path: a write into the page cache costs
microseconds and survives a process crash. A spool task calls `fsync`
every `fsync_interval` (default 100 ms) and rolls the segment at
`segment_bytes` (default 16 MiB). The window a power loss can take is
one interval. Configuration:

```toml
[cdr]
spool_dir = "/var/lib/fcp/cdr-spool"   # required when storage is on
fsync_interval_ms = 100
segment_bytes = 16777216
ship_batch = 200
```

The `Shipper` reads sealed segments and the tail of the open one, ships
records in batches to the `Sink`, and advances a cursor file
(`<segment>.offset`) only after the sink acknowledged the batch. A
segment whose cursor reaches its end is deleted. A crash between the
acknowledgement and the cursor write ships a batch twice, which is what
`cdr_id` is for. The spool's depth (records behind the cursor) and the
age of its oldest unshipped record are metrics and are on the node's
health route.

### 4.2 The sink: stream or in-process

`fcp_cdr::Sink` has two implementations.

The stream sink (`fcp_storage_redis::cdr_stream::RedisCdrStream`, behind
that crate's `cdr` feature) does `XADD fcp:cdrs * record <json>` per
record, one `MULTI` per batch, against the Redis the node already has
for clustering (`storage.redis_url`). The stream is **never trimmed by
count**. The processor trims it by `XTRIM MINID` to the earlier of the
group's oldest unfinished entry (pending, or the last delivered) and the
replay window (`retain_hours`, default a day), so a record stays until it
is stored. Stream length is a metric; a Redis outage leaves the records
in the spool.

A node ships to the stream only with `[cdr] sink = "stream"`; `local` is
the default even when the node has Redis. Nothing reads the stream
unless an `fcp-jobs` runs with `[cdr] enabled = true`, and records a
node sends where nobody reads would sit in Redis unstored, so the choice
is explicit rather than implied by `redis_url` (as the first draft of
this section had it).

`LocalSink` runs the processor (§4.3) in the shipper's task against the
node's own Postgres. This is the single-node deployment: no Redis, the
spool still stands between the hangup and the database, and a Postgres
outage delays records and loses none.

The transport question — this stream, a Postgres inbox table, or a
broker that is not in the tree — is §14.2.

### 4.3 The processor

`fcp_cdr::Processor` is the same struct in both deployments. In
`fcp-jobs` it reads `fcp:cdrs` through `fcp_cdr::consumer::Consumer`, a
loop over the `RecordSource` trait that `RedisCdrStream` implements
(consumer group `cdr-processor`, `XREADGROUP`, `XAUTOCLAIM`, `XACK`), in
batches of up to `batch` (default 500) or `batch_wait` (default 200 ms),
whichever first. Per batch:

1. **Normalize.** Decode by schema version, validate (tenant, times in
   order, a URI each side), derive what is derived: durations, the
   disposition and hangup cause from the `TerminationReason` enum, the
   SIP status code from the same. A record that fails here goes to the
   dead-letter stream `fcp:cdrs:dead` with the reason and is
   acknowledged. It is never dropped without a trace, and the count is a
   metric.
2. **Enrich.** Direction, from the tenant's numbers (the lookup the
   aggregates do per row today, done once and stored); trunk and gateway
   names; the region. Lookups go through a per-tenant cache with a
   sixty-second TTL (§14.6).
3. **Rate.** `fcp_billing::rating` as it is, on the tenant's default
   plan from the same cache; the rate plan id, matched prefix, billable
   seconds and cost are stamped on the record. No storage write yet.
4. **Store.** One transaction per batch: `INSERT … ON CONFLICT (cdr_id)
   DO NOTHING RETURNING cdr_id`, so the processor learns which records
   were new.
5. **Settle.** For every rated record in the stored batch, new or a
   replay, the ledger transaction (`BillingEngine::settle_rated`),
   protected by the `idx_balance_transactions_call_charge` index, so a
   call is charged once however often its record is processed. Settling
   only new records, as the first draft said, would lose the charge of a
   batch stored just before the processor died: its replay finds every
   record already there.
6. **Announce.** `PlatformEvent::Cdr(CdrEvent::Written { cdr_id,
   call_id, tenant_id, kind, cost, … })` per record, replays included for
   the same reason, on the bus, so the events server streams it and the
   webhook dispatcher delivers it as `cdr.written`; consumers skip a
   `cdr_id` they have seen. The `billing.call_rated` event keeps firing
   from the settle step.
7. **Acknowledge.** `XACK` the batch. A failure in steps 4 or 5 (the
   database is away) acknowledges nothing; the batch is retried with
   backoff and a stale consumer's pending entries are claimed by another
   after `claim_idle` (default 60 s).

Throughput is one round trip to Postgres per batch, not per record. A
single processor at 500 per batch and a few batches a second covers
call rates well past what one call manager can originate; more
`fcp-jobs` replicas join the consumer group.

### 4.4 The store

Postgres remains the system of record (§8). The `CdrStore` trait keeps
`insert_cdr` for the in-process path and gains `insert_cdrs(batch)`
returning the ids inserted; the rate-plan and balance methods that live
on `CdrStore` today move to a `BillingStore` trait of their own in
`fcp-core`, which `fcp-billing`'s blanket impl then targets. The
overloaded trait is a survey finding, not a goal in itself; splitting it
falls out of phase 2.

## 5. The record

`fcp_cdr::Record` supersedes `CallDetailRecord` as the type the nodes
build; the storage type keeps its name and gains the columns. Times
become `DateTime<Utc>` (the vision's choice; `SystemTime` is what the
MVP had). New or changed fields:

| Field | Type | Set by | Notes |
|---|---|---|---|
| `cdr_id` | UUID, primary key | node | Idempotency key. Existing rows backfilled with UUIDv5 of `call_id`, so the migration is repeatable |
| `kind` | enum `call`, `queue`, `conference`, `gateway` | node | One `call` record per `call_id` (partial unique index); the others correlate to it |
| `schema_version` | smallint | node | Decoding in the normalizer |
| `direction` | enum `inbound`, `outbound`, `internal` | processor | Stored, so the aggregates stop joining `phone_numbers` per row |
| `hangup_party` | `caller`, `callee`, `system` | node | From `TerminationReason`, which replaces the free-text `reason: Option<String>` of `terminate_call` |
| `hangup_cause`, `sip_status_code`, `sip_reason` | as today | node | Derived from the enum, not from substring matches on the reason |
| `node_id`, `region` | text | node | Which node owned the call; the region it advertised |
| `correlation_id` | `CallId`, nullable | node | The call a transferred caller left (REFER), the ringing call a pickup took, the parked call a retrieve brought back. Replaces is not implemented by the B2BUA, so an attended transfer is not linked |
| `trunk_name` | text, nullable | node | Already on `CallEvent::Ended`; stored |
| `codec_a_leg`, `codec_b_leg` | as today | node | Parsed with `sip-sdp`, not sniffed |
| `rtp_packets_*`, `rtp_bytes_*`, `packets_lost`, `jitter_ms`, `rtt_ms` | as today plus two | node | From `MediaSessionStats` at `terminate_session`, which the handler already calls before the CDR |
| `recording_url` | as today | node | The recorder's path when the leg was recorded; today's TODO |
| `rate_plan_id`, `matched_prefix`, `billable_seconds`, `rated_at` | nullable | processor | What the cost came from |
| `cost`, `currency` | `NUMERIC(14,6)`, text | processor | §14.3 |
| `created_at` | timestamptz | processor | When the row was written; `end_time` is when the call ended |
| `custom_data` | JSONB, as today | node | The queue record's `queue_name`, `agent_id`, `wait_time_ms`, `handle_time_ms`, `outcome`; a conference record's `room_id` and `participant_id`; a gateway record's `session_id` |

Removed: nothing. `route_id` and `gateway_id` stay, and the SBC fills
`gateway_id` from its trunk router in phase 3.

Which record an API client sees is unchanged: `GET /cdrs/{call_id}`
returns the `call` record; `GET /cdrs?call_id=` returns every kind for
the call; a `?kind=` filter is added, and the default listing stays
`kind=call` so existing dashboards do not double-count.

## 6. Durability and exactly-once

The property, stage by stage:

| Failure | What happens | What is lost |
|---|---|---|
| Node process crash | Spool segments are on disk; the shipper resumes from the cursor on restart | Nothing; records appended but not fsynced were in the page cache and survive |
| Node power loss | As above | Up to one `fsync_interval` of records (100 ms) |
| Redis unreachable | Shipper backs off; spool grows; metric and log | Nothing; delay |
| Postgres unreachable | Processor acknowledges nothing; stream (or spool, in-process) grows | Nothing; delay |
| Processor crash mid-batch | Pending entries claimed by another consumer after `claim_idle`; `ON CONFLICT` skips stored rows; the ledger index skips charges; `cdr.written` may repeat | Nothing; a consumer may see one announcement twice |
| A record the normalizer rejects | Dead-letter stream with the reason; metric | Nothing; an operator reads `fcp:cdrs:dead` |
| Node disk lost before shipping | The unshipped tail of the spool | Those records; this is the floor, and it is the same floor every switch has |

The exactly-once claim is precisely: a record reaches the `cdrs` table
at most once, the ledger is charged at most once, and both happen at
least once as long as the node's disk survives to ship.

## 7. Rating in the pipeline

`docs/BILLING.md` stays accurate for what rating is; what changes is
where. `BillingEngine::settle` is called by the processor, not by the
hangup task, and takes the rate plan from the processor's cache rather
than from storage per call. Admission (`BillingEngine::admit`, the 402)
and the mid-call cut-off stay in `fcp-sip`'s handler on the node: they
need the balance now, and they are cheap reads.

One consequence to state: a rate plan changed at 12:00:00 rates a call
that ended at 12:00:30 on the old plan if the cache has not turned over.
Sixty seconds of that is acceptable for a billing system whose plans
change by the month; §14.6 asks whether it is.

The `billing.*` events and `BillingHook` are unchanged; they fire from
the processor. External metering that wants the rated record itself
subscribes to `cdr.written`.

## 8. Storage at volume

Phase 4 converts `cdrs` to a table partitioned by `RANGE (start_time)`,
one partition per month, three created ahead by the processor (an
`ensure_partitions` step it runs on start and daily; the in-process path
runs the same step). Postgres requires the partition key in every unique
constraint, so the primary key becomes `(cdr_id, start_time)` and the
one-call-record rule becomes a partial unique index on
`(call_id, start_time) WHERE kind = 'call'`; `get_cdr` by `call_id`
queries with no time bound and uses `idx_cdrs_call_id`, which is
non-unique.

The conversion migration renames the old table, creates the partitioned
one and its months, copies the rows, and builds the key and indexes after
the copy, all in the migration's transaction (a migration file is one
transaction, so the month-sized batches first planned here are not
available to it). Measured below.

Indexes: `(tenant_id, start_time DESC, cdr_id DESC)` for every listing
and for keyset pagination; `(call_id)`; `(tenant_id, start_time)` `WHERE
kind = 'call'` for the rollup rebuild; trigram indexes on `from_uri` and
`to_uri` (`pg_trgm`) if the `LIKE` filters stay, otherwise the filters
become prefix matches. The `sip_call_id` and `disposition` single-column
indexes stay; the GIN on `custom_data` stays.

Retention becomes two things. A partition older than every tenant's
retention and the platform's maximum is dropped, which is instant and
returns the space. Within a live partition, a tenant with shorter
retention than the rest still gets a `DELETE … WHERE tenant_id = $1 AND
start_time < $2`, batched, as today. The sweep takes a Postgres advisory
lock so admin replicas do not run it concurrently; it stays in
`fcp-admin` (§14.8).

## 9. Reading

- **Listing.** `GET /cdrs` gains `cursor` (opaque, encoding the last
  row's `(start_time, cdr_id)`) and returns `next_cursor`; `offset`
  keeps working but is capped at ten thousand. `total` becomes
  optional: exact when the filter is a tenant and a time window that
  closed hours cover (from the rollups), otherwise an estimate from the
  planner, with `total_exact: bool` saying which. A decode failure in a
  row is an error, not a skipped row.
- **Export.** `GET /cdrs/export` streams the CSV body from a keyset
  walk, no row cap, the audit entry unchanged. A `format=jsonl` variant
  for machines.
- **Usage and billing summary.** `cdr_usage_hourly (tenant_id, hour,
  direction, kind, calls, answered, duration_ms, cost)` is rebuilt for
  the last `rollup_window` hours (default 48) every ten minutes by the
  processor's host — idempotent recompute, not increments, so a replayed
  batch cannot double-count. `/usage`, `/usage/by-number` and
  `/billing/summary` read closed hours from the rollups and the open
  hour from the live table. `aggregate_usage_by_number` gets a
  per-number rollup of the same shape.
- **The call manager's own `/cdrs`** on its health server stays as it is
  for the test floor and for operators on a node; it is not the API.

## 10. The event

`PlatformEvent::Cdr` is a new variant with one event for now:

```rust
CdrEvent::Written {
    cdr_id: Uuid,
    call_id: CallId,
    tenant_id: TenantId,
    kind: CdrKind,
    disposition: CallDisposition,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    duration_ms: u64,
    cost: Option<Decimal>,
    currency: Option<String>,
    node_id: NodeId,
}
```

Named `cdr.written`; carried by the events server, the WebSocket
stream and the webhook dispatcher like every other event; forwarded
across regions by the bridge like every other event (the record is
stored in the region that owned the call; the announcement travels).
`docs/API_EVENTS.md` gets the row.

## 11. Observability

Every stage exports through the existing metrics registry and shows on
the host's health route:

| Metric | Where | Meaning |
|---|---|---|
| `cdr_spool_pending_bytes`, `cdr_spool_segments`, `cdr_spool_oldest_age_seconds` | node | Bytes not yet shipped, segment files, and how long the oldest has waited |
| `cdr_stream_length`, `cdr_stream_pending`, `cdr_stream_dead_letters` | processor | Redis stream size, the consumer group's unacknowledged count, dead letters kept |
| `cdr_processed_total{outcome}` | both | `stored`, `duplicate`, `dead_letter` |
| `cdr_batches_total`, `cdr_batch_errors_total` | both | Batches, and batches the store failed |
| `cdr_batch_seconds` | both | How long the last batch took |
| `cdr_end_to_write_seconds` | both | `created_at - end_time` of the last record, the pipeline's lag as a billing system sees it |
| `cdr_rollup_age_seconds` | processor host | How stale the rollups are (phase 4) |

The call manager serves these on `/metrics` and as JSON on `/cdr/status`
(spool and in-process processor); `fcp-jobs` on `/metrics` and under
`cdr` in `/status` (stream and processor). Every sample carries
`node="…"`.

Logs at `info` per batch, at `warn` for backoff, at `error` for a
dead letter with the record's ids. A record's journey is traceable by
`cdr_id` in every log line that touches it.

## 12. Testing

- **Unit.** The spool against a writer killed mid-append (a partial last
  line is skipped, the cursor is honoured, a segment is deleted only
  when fully shipped); the builder's table tests for every
  `TerminationReason` to disposition, cause, status, party; the
  normalizer's rejects; the rater unchanged.
- **Postgres suites** (`fcp-storage-postgres/tests`, `fcp-api-core/tests`,
  gated as today): batch insert with duplicates, the migration on a
  seeded table (backfilled ids, the swapped partitioned table, a query
  across a partition boundary), keyset pagination through a boundary,
  rollup rebuild equals the live aggregate, retention dropping a
  partition and deleting within one.
- **Redis suite** (`fcp-storage-redis/tests`): consumer group, a
  consumer dying with pending entries and another claiming them, the
  dead-letter stream, trimming by MINID.
- **The test floor.** Every call scenario in
  `fcp-call-manager/tests/calls.rs` asserts its record: busy is `busy`
  with the callee as the hangup party, no answer is `no_answer`, an
  answered call has RTP counters above zero on both legs and the codec
  named, a blind transfer produces two records linked by
  `correlation_id`; with a seeded rate plan the record carries a cost
  and the balance moved once. Conference and gateway scenarios assert
  their leg records in phase 3. One scenario stops Postgres under a
  call (the testkit's portable Postgres can be stopped and restarted),
  ends the call, restarts it, and sees the record land.
- **A chaos test in the two-node harness** (`fcp_testkit::cluster`):
  SIGKILL a call manager after hangup and before shipping, restart it,
  see the spool drain.

## 13. Phases

One PR per phase, reviewed before the next starts, each with its tests
and its numbers.

**Phase 1 — the record and the spool.** The `fcp-cdr` crate:
`TerminationReason`, `Record`, `RecordBuilder`, `Spool`, `Shipper`,
`Sink`, `LocalSink`, and a `Processor` with the normalize and store
steps (rating stays where it is until phase 2, called from the
in-process path). Migration: `cdr_id` and its backfill, `kind`,
`schema_version`, `direction`, `node_id`, `region`, `correlation_id`,
`trunk_name`, the rating columns, `created_at`, the composite index.
`terminate_call` takes the enum; the B2BUA and the queue handler build
through the builder and write through the spool; `hangup_party`, the
RTP counters, the parsed codecs, the trunk, the recording path and the
correlation id are filled. `FileBackedStorage`'s no-op `insert_cdr`
becomes a spool with no sink, so a node without a database still keeps
its records. The test floor's call scenarios assert their records.

*Phase 1 shipped 2026-09-14 (v0.25.159).* The `fcp-cdr` crate holds the
record builder, `TerminationReason`, the spool, the shipper, the
in-process sink and the processor; `fcp_cdr::start_node` wires them from
a node's `[cdr]` section (`spool_dir`, `fsync_interval_ms`,
`segment_bytes`, `ship_batch`; under `[storage.cdr]` on the SBC). The
record type stays `fcp_core::storage::CallDetailRecord`, re-exported as
`fcp_cdr::Record`, with the §5 fields added; migration 057 adds the
columns, backfills `cdr_id` from `md5(call_id)`, marks the queue
handler's old rows `queue` and copies `gateway_id` into `trunk_name`,
and moves the primary key to `cdr_id` with a partial unique index for
one `call` record per call. `insert_cdr` is `ON CONFLICT (cdr_id) DO
NOTHING`. `CallManager::terminate_call_with` takes the reason;
`terminate_call(Option<String>)` stays as the MVP's contract, reading a
leading status code and nothing else. The BYE handler records who hung
up and takes the media session's counters before the session goes, so
the record carries `hangup_party` and RTP packets and bytes each way
(the backend reports no jitter yet, and a zero is not stored as one).
Rating runs in the processor in phase 1 already, since it ran in the
spawned task before; the settlement records the rate plan's id.

What the plan said and phase 1 did not do: `FileBackedStorage` does not
get a spool, because no binary that writes records runs without
Postgres (it is a test fixture); `correlation_id` for transfers and the
recording path on B2BUA calls move to phase 3 with the other legs, and
the queue record's `call_id` stays minted, as phase 3 already said.

The test floor found one more **product bug**, the reason the record
matters: **every busy, declined or unanswered call through the forking
path was recorded as a failure with no cause.** The path ended the call
with the text "All B-legs failed", which the old substring matcher could
not read, so a 486 from the callee was `failed`/`other(0)` in the table.
The path now passes the callee's status, or no answer when the fork
timed out. Four call scenarios assert their records (answered and hung
up by the caller with counters and codec; hung up by the callee; busy
and declined with status, cause and party; no answer), and a new one
renames the `cdrs` table away under a call, ends the call, gives the
table back and sees the one record land from the spool.

**Phase 2 — the processor in `fcp-jobs`.** `StreamSink`, the consumer
group, batches, backoff, stale claim, the dead-letter stream, trimming
by MINID. Rating and settlement move into the processor with the
per-tenant cache; `cost` and the ledger go `NUMERIC` with `rust_decimal`
(§14.3); the `BillingStore` split. `PlatformEvent::Cdr` and
`cdr.written` on the bus and through webhooks. `JobType::CdrProcessing`
is removed. Metrics and the health route. `IMPLEMENTATION_DOCS.md` §2 is
rewritten to describe what exists.

*Phase 2 shipped 2026-09-14 (v0.25.160).* The processor is a role in
`fcp-jobs` (`[cdr] enabled`, `database_url`, `batch`, `batch_wait_ms`,
`claim_idle_secs`, `retain_hours`, `cache_secs`, `key_prefix`), consuming
`fcp:cdrs` through `fcp_cdr::consumer` over `RedisCdrStream`, which is
also the node's stream sink (`[cdr] sink = "stream"`, §4.2). The
processor now normalizes (refusing a record from a newer schema),
enriches the direction through `DirectionEnricher` (the tenant's numbers
under the cache: to a tenant number inbound, from one or over a trunk
outbound, else internal — the per-row SQL counted a trunk call from an
extension as neither), rates with `BillingEngine::rate` under
`with_plan_cache`, stores the batch with `CdrStore::insert_cdrs` (one
statement per thousand rows, one transaction, `RETURNING cdr_id`),
settles and announces. A record the store refuses for what it is — a
second call record for a call, a value out of range: SQLSTATE classes 23
and 22 — comes back as `CdrRejected`; the processor then stores the batch
one record at a time, keeps the refused one as a dead letter
(`fcp:cdrs:dead` with the reason, or the spool's `dead.jsonl` with the
reason on a node) and stores the rest, so one bad record cannot hold a
batch forever. An error of any other kind is the store being away: the
batch is retried with backoff and acknowledged only once it lands. The
same processor, with the enricher and the event publisher, runs behind
the spool on a node with `sink = "local"`, so a single-node install gets
directions and `cdr.written` without `fcp-jobs`.

Money is `rust_decimal::Decimal` everywhere it is held — rates, fees,
balances, credit limits, thresholds, ledger amounts, costs, usage totals,
billing events — and `NUMERIC(18,6)` in every money column (migration
058), wider than §14.3 said because rates and fees drift the same way
costs do. The `serde-float` feature keeps each one a JSON number on every
surface, and the API refuses an amount with more than six places or
twelve whole digits rather than letting the column round it. Rating gives
the same results: every billing test kept its numbers. The rate
plan and balance methods left `CdrStore` for `fcp_core::billing::BillingStore`;
`fcp-billing`'s narrow view of it is `LedgerStore`. `PlatformEvent::Cdr`
is new, and `EventType::Cdr` (`event_types=cdr` on the WebSocket).
`JobType::CdrProcessing`, which nothing ever produced, is gone. The usage
totals count a record's stored direction when it has one.

Tests: the processor, enricher, consumer and metrics in `fcp-cdr` (an
in-memory source with Redis's delivery rules: a store outage holds a
batch unacknowledged, a dead consumer's entries are claimed and stored
once, an undecodable entry and a refused record are dead-lettered);
`fcp-storage-redis/tests/cdr_stream.rs` against Redis (round trip, claim,
dead letters, trimming that keeps pending and undelivered entries);
Postgres (batch insert reporting new ids, a refused record rolling back
its batch, ten charges of 0.1 summing to exactly 1, the stored direction
in the totals, decimal balances after 058); and
`fcp-jobs/tests/cdr_pipeline.rs` against the running binaries — a call
through a call manager shipping to the stream is stored by `fcp-jobs`
rated at the plan's rate, internal, charged exactly once and announced
on the event stream; then `fcp-jobs` is killed, a second call's record
waits in the stream, and lands, charged once, when a new `fcp-jobs`
starts. The call scenarios read the call manager's `/cdr/status`.

What the plan said and phase 2 did not do: the enricher fills the
direction only (trunk and region were already stamped by the node in
phase 1); the webhook dispatcher needed nothing, since it forwards every
event by name.

**Phase 3 — every leg a record.** The conference server writes a
`conference` record per leg (direct, through the call manager, over a
trunk from another node), the WebRTC gateway a `gateway` record per
session, the SBC fills `gateway_id`; queue segments carry the real
`call_id`; transfers and Replaces set `correlation_id` on both records.
`?kind=` on the listing. The conference and gateway test-floor
scenarios assert their records.

*Phase 3 shipped 2026-09-14 (v0.25.161).* Every leg the platform carries
now has a record, correlated by call id:

- **Conference legs.** The conference server writes a `conference`
  record whenever a participant's session ends, from the one function
  every leave goes through, now given the reason: a BYE from the far end
  (the participant, or a dial-out's destination), a CANCEL, a kick, the
  room closing, the node stopping, a session timer, a trunk closing or
  being replaced, a migration. The record carries the room, the
  participant, host and dial-out and trunk flags, video and content,
  `migrated_from` and `migrated_to`, the codec, and packet counts (the
  audio bridge counts packets, not bytes). Trunk and dial-out legs are
  recorded too. `[cdr]` on the conference server works as on the call
  manager.
- **One call id across nodes.** A call manager's INVITE to a conference
  room, and its migration INVITE, carry `X-FCP-Call-Id`, and so do the
  gateway's; the conference leg's record takes that id, so a leg reached
  through a call manager sits beside the call's own record and a
  browser's legs on two nodes share the gateway session's id. A phone
  dialing a room directly gets an id of its own. The header correlates
  records and decides nothing else.
- **Gateway sessions.** The WebRTC gateway writes a `gateway` record per
  session: the browser's time, who ended it, the relay's packet counts,
  the SIP side's last Call-ID. The gateway keeps no database, so its
  records ship only through the stream (`[cdr] spool_dir`, `sink =
  "stream"`, `storage.redis_url`) and an `fcp-jobs` stores them; the
  tenant is read from the join token without verifying it
  (`JoinToken::peek_unverified`) — the conference node verified it when
  it admitted the browser — or taken from `[conference] tenant_id`.
- **Queue segments** carry the B2BUA call's id: the SIP handler passes it
  to the queue handler, which ends that call when it refuses the caller
  (with the status the caller was sent) or when the caller's session
  ends, so the call's record and the segment's are written together.
- **Transfers.** The call manager adds `X-FCP-Transferred-From: <call
  id>` to the Refer-To it relays; the transferred caller's phone copies
  it into the new INVITE (RFC 3261 §19.1.5) and the new call's record
  points at the call it left. A pickup's feature call points at the call
  it picked up, a retrieve's at the parked call.
- **Where a call went and what was kept.** On failover the record names
  the trunk that answered, not the primary route's (`gateway_id` is the
  trunk name, as the trunk statistics query already assumed). A
  recording started by a flow node or through the API is the record's
  `recording_url`.
- **Listing.** `GET /cdrs` (the API and the call manager's) lists `call`
  records by default, a kind with `?kind=`, every kind with `?kind=all`
  or when filtering by `call_id`.
- **Shutdown.** A node stopping gracefully waits up to five seconds for
  its records to leave (`CdrWriter::flush`): the direct writes stored and
  the spool shipped and synced. Without it a draining conference node's
  last migration record was lost with the process.

Two **product bugs** found on the way, both fixed. **A call routed to a
queue or a conference room was never ended**: the SIP handler created
the call before handing the INVITE over, neither handler knew its id,
and their BYEs never reached the B2BUA, so the call sat in the call map
forever with no record written — on the call manager for every queued
caller, on a conference node for every participant. The conference path
now drops the call when it hands the INVITE over (the room records the
leg), and the queue ends it. And **a queue was unreachable at a
Request-URI with a port**: the queue read the tenant from the URI's
host without removing the port, so `127.0.0.1:5070` was looked up as a
tenant domain, not found, and the queue declined the call; the host is
now parsed.

Not done, and why: a caller who never gets past a room's PIN, or is
refused before the answer (full, locked, rate-limited, draining), leaves
no conference record — a call manager's call record still covers one
that came through it; a recording started by the SBC's local capture is
kept in its recording store and not on the call; attended transfer is
not linked, since the B2BUA does not implement Replaces.

Tests: the call floor asserts the transfer link and a new scenario puts
a caller in a queue and hangs up (one call id on both records, the call
answered and the segment abandoned, the default listing showing the
call alone, no call left live); the room floor asserts each leg's record
for a direct join, a kick, and a call through a call manager (the leg
under the call's id); the gateway's drain scenario ships the session's
record through the stream into the database and finds the browser's leg
on each node under the session's id, one ended by the migration.

**Phase 4 — reading at volume.** The partition conversion and
`ensure_partitions`; keyset pagination and the streamed export; the
hourly rollups and the usage routes on them; retention by partition
drop under an advisory lock. Numbers measured on a seeded table of a
hundred million rows: listing, export throughput, rollup rebuild, the
conversion itself.

*Phase 4 shipped 2026-09-14 (v0.25.163).* Migration 059 converts `cdrs`
to monthly range partitions on `start_time` with a default partition;
the key is `(cdr_id, start_time)` and the one-call-record rule
`(call_id, start_time)` for call records, since Postgres puts the
partition key in every unique rule — two call records for one call at
different start times are no longer refused. Keys and indexes are built
after the copy. The b-tree indexes on `from_uri` and `to_uri` are gone:
the URI filters are substring matches, which they never served.
`CdrStore` gains `ensure_cdr_partitions` (last month through three ahead,
moving a new month's rows out of the default partition before attaching
it; run when storage starts and by the maintenance job every six hours),
`drop_cdr_partitions_before`, `count_cdrs_within`, `rebuild_usage_rollups`
and `try_maintenance_lock` (a Postgres advisory lock on one connection).

Reading: `GET /cdrs` pages by `cursor`, an opaque `(start_time, cdr_id)`
from the last record, and returns `next_cursor`; `offset` above 10,000 is
refused. `total` is an exact count that gives up after two seconds for
the planner's estimate, `total_exact` saying which — a count bounded by
time rather than read from the rollups as §9 said, because the rollups
cannot answer the listing's filters. A row that does not decode is an
error. The listing orders by `(start_time, cdr_id)` and reads record
times to the microsecond. `GET /cdrs/export` streams every matching
record from a keyset walk, CSV (now with `kind` and `cdr_id`) or
`format=jsonl`.

Usage: `cdr_usage_hourly` (tenant, hour, kind, direction) and
`cdr_number_usage_hourly` (tenant, hour, number), with
`cdr_rollup_state` saying which hours they cover, are recomputed for the
last 48 hours every ten minutes — the first run covers all of history —
in one transaction under a lock, by `fcp_cdr::spawn_maintenance`, which
`fcp-admin` and the processor in `fcp-jobs` both run. The usage routes
read the covered whole hours from the rollups and the edges of the range
from the records. Usage now counts `call` records only, so a conference
leg or queue segment beside a call does not count it twice. A record
stored more than the window after its hour is missed by the rollups
until a wider rebuild; a record with no stored direction is classed in
the rollups the way the enricher would (inbound first), where the
record-level count may put it in both columns.

Retention: `fcp-admin`'s hourly sweep runs under the `fcp:retention` lock,
deletes a tenant's records in batches of 10,000, and, when every tenant
has a CDR window, drops the months entirely older than the shortest of
them — only once they are empty. The rows are still deleted tenant by
tenant first, so the recordings and flow traces found through them are
removed too; dropping the emptied month returns its space without a
vacuum. That is the honest version of "retention by partition drop":
the delete is not skipped.

**Measured** with `crates/fcp-storage-postgres/tests/volume.rs` on this
project's development VM (12 vCPUs, 3.3 GB of memory, Postgres 18 with
default settings, a local disk) at **ten million records** over twelve
months and fifty tenants. The design asked for a hundred million; that
needs about 60 GB of table and indexes, doubled during the conversion,
and the VM has 48 GB free. The harness takes `FCP_VOLUME_ROWS` for a
larger box.

| What | Time |
|---|---|
| Conversion (059 and 060) of 10 M rows, 3.4 GB before and 6.3 GB after with the new indexes | 72 s |
| A tenant's first page of 100 | 43 ms |
| A page by cursor, ten deep | 6 ms |
| A page at offset 10,000 | 200 ms |
| Count, every tenant, 10 M | 1.6 s, exact |
| Count, one tenant, 200 k | 2.0 s, then the estimate (199,721 for 200,000) |
| Export walk, one tenant's 200 k records | 8.3 s, 24,000 records/s |
| First rollup rebuild, all 8,760 hours | 240 s |
| Rollup rebuild, the 48-hour window | 0.2 s |
| Usage for 30 days, from the rollups | 9 ms |
| Usage for 30 days, from the records | 797 ms |

Two readings of those numbers. The first version of the migration
created the key and the indexes before copying; seeding the same ten
million rows into an indexed table had run twenty-two minutes when it
was stopped, and the conversion would have done the same, so both now
build indexes after loading. And the one-tenant count, slower than the
platform count, is an index-only scan over a table not yet vacuumed
(the benchmark analyzes but does not vacuum): autovacuum makes it cheaper
on a live table, and the budget turns it into an estimate either way.

**Upgrading**: 059 locks `cdrs` while it copies. A node still running the
previous version cannot store records into the partitioned table (its
insert names the old key), so its records wait in its spool until it is
upgraded; a node without a spool loses them. Upgrade the nodes that write
records together, and give every one a `spool_dir`.

**Phase 5 — the archive sink.** Daily gzipped JSON Lines files per
tenant to an S3-compatible bucket from the processor's host, written
from the closed partition so the archive matches the table; a
`cdr.archived` event; retention allowed to drop a partition only after
its archive is confirmed when a tenant's policy says so. Optional and
last; the vision's data lake, without the Parquet (§15).

## 14. Open questions

Each with a recommendation. Settled 2026-09-14: the user merged the design
and asked to proceed, so every recommendation below is the decision.

1. **Where the processor runs.** In `fcp-jobs` (recommended: the worker
   exists, has Redis, a config, a smoke test and a deploy unit; the
   standalone-proxy precedent) or a new `fcp-cdr-processor` binary
   (scales independently of the other job types; one more of everything
   to maintain).
2. **The transport.** A dedicated Redis stream (recommended: the bus
   Redis is already on every node and in every region, the consumer
   group and stale claim exist and are tested by the event bridge, and
   the stream decouples the node from the primary's latency); a Postgres
   inbox table (no Redis dependency, but every node writes to the
   primary at hangup, which is the coupling the pipeline exists to
   remove, and the multi-region design routes writes to one primary); or
   NATS or Kafka (not in the tree; would be the first non-Redis broker).
3. **Money.** `NUMERIC(14,6)` in `cdrs.cost`, `tenant_balances.balance`
   and `balance_transactions.amount`, with `rust_decimal` in
   `fcp-billing` and the API serialising as a JSON number
   (`serde-float`) so clients see no change (recommended: sums of
   millions of `f64` charges drift; the columns were `DECIMAL` until
   sqlx could not decode them, which `rust_decimal`'s sqlx feature
   solves) — or leave `f64`.
4. **Spool durability.** Append into the page cache with `fsync` every
   100 ms (recommended: microseconds per record, a process crash loses
   nothing, power loss loses 100 ms) or `fsync` per record (a
   millisecond or more per hangup on the call path; no loss short of
   the disk).
5. **The primary key.** `cdr_id` (recommended: one record per call is a
   rule about `call` records, not about the table, and idempotency needs
   a key the node mints) or keep `call_id` and give the other kinds a
   table each.
6. **The rate plan cache.** Sixty seconds (recommended) or a live
   lookup per record (what happens today; a round trip per record
   through the batch).
7. **Conference and gateway records.** A record per leg (recommended:
   usage and billing for rooms and browsers is invisible today) or
   none, with rooms accounted only through the call manager's record
   when the caller came that way.
8. **Retention's home.** Stay in `fcp-admin` under an advisory lock
   (recommended: the smallest change; the lock removes the replica
   collision) or move to `fcp-jobs` beside the processor.

## 15. Not in scope

Geographic enrichment (the vision's "add geo"), fraud and spam scoring
on the record, MOS or R-factor from the RTP counters, RADIUS or Diameter
accounting, per-tenant push delivery of records (SFTP, a tenant's own
bucket), Parquet or a query engine over the archive, lawful-intercept
metadata delivery, and a second CDR format for a specific downstream
biller. Each is a sink or an enricher on the pipeline this design
builds, added when there is a customer for it.
