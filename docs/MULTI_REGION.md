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
  membership, the conference registry, parking, the event bus. Region-aware
  membership and event-bus fan-out are the next step (not done).

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
- Routing a region's callers to that region's nodes: placement and SIP
  routing are single-region until region-aware membership lands.
