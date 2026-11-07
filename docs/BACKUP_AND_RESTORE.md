# Backup, restore and upgrade

How a deployment is kept: where every piece of state lives, what a
backup must hold, what a restore needs, and what an upgrade does. The
design behind it, with what the survey found, is
`docs/BACKUP_RESTORE_UPGRADE.md`; this is the document an operator
reads.

## 1. Where the state is

Every binary keeps its state in one of four places.

| Where | What | Re-derivable? |
|---|---|---|
| **Postgres** (one database, `fcp` in the compose stack) | Every configuration object — tenants, users, devices, trunks, numbers, routing rules, flows, meetings, rate plans; registrations (unless the registrar's `registration_backend` is `redis`); the CDR tables, partitions and rollups; the rows for call recordings, conference recordings and voicemail messages, each naming the node that holds the file and its path; legal holds, the deletion log, subject requests, the audit log; the billing ledgers; the key ring — every data encryption key, wrapped by the master key | **No.** This is the platform. |
| **The master key** (`master.key` in the compose stack's `fcp-certs` volume; `FCP_MASTER_KEY` or `FCP_MASTER_KEY_FILE` from source) | The key every data encryption key is wrapped with | **No.** Without it every sealed column — SIP passwords, trunk passwords, PINs, webhook secrets — is unreadable, and a different key against the same database refuses to start naming the mismatch (`docs/ENCRYPTION.md`). The CA and the service certificates beside it in the volume are re-derivable: `make-certs.sh` remakes what is missing under a kept CA, and a new CA only means phones and browsers trust it again. |
| **Files on a node** | Call recordings, conference recordings, voicemail messages and greetings, subject-export archives — the files whose rows are in Postgres; prompts and announcements; the CDR spool (records hung up and not yet shipped to the stream); the TTS cache | Recordings, voicemail and exports, **no**: a row whose file is gone is a recording nobody can play. Prompts and announcements are what was uploaded. The spool empties itself. The TTS cache is a cache. |
| **Redis** | Cluster membership and each node's heartbeat; registrations when the Redis backend is on; the event stream and its consumers' positions; the CDR stream `fcp:cdrs`; the job queue — webhook deliveries, recording processing, cleanup; call parking; the registration cache | Membership, caches and positions, **yes**. Registrations re-register on the phone's next refresh. The CDR stream re-ships from each node's spool, and the processor writes `ON CONFLICT DO NOTHING`, so a record shipped twice is stored once. The job queue's pending entries, **no**: a webhook retry or a cleanup queued and not yet run is lost with Redis, and the next event redoes it. |

Two consequences. **The database and the master key are one unit**: a
dump without the key holds secrets nobody can open; a key without the
dump is a key to nothing. Back them up together and restore them
together. **A recording's row names a path on a node**, so a restore
puts each node's files back at the paths its rows name — in the compose
stack, the same volume at the same mount; from source, the same
directory on the same node.

## 2. The directories, per binary

What each binary writes to disk, the key that names the directory, and
the default when the key is unset. The compose stack's configurations
set every one under a volume; a deployment from source sets them to
wherever it backs up.

| Binary | Key | Default | Compose stack |
|---|---|---|---|
| `fcp-call-manager` | `[media] recording_dir` | `./recordings` (the working directory) | `/app/data/recordings` (`call-manager-data`) |
| | `[media] prompts_dir` (also the built-in `hold-music.wav`, re-derivable: written at startup) | `/tmp/fcp/audio/prompts` | `/app/data/prompts` |
| | `[media] tts_cache_dir` | `/tmp/fcp/tts_cache` | `/app/data/tts-cache` |
| | `[cdr] spool_dir` | — | `/app/data/cdr-spool` |
| `fcp-conference` | `[conference] recordings_dir` | `/var/fcp/conference/recordings` | the same (`conference-recordings`) |
| | `[conference] announcements_dir` | `/var/fcp/conference/announcements` | mounted from `configs/announcements`, read-only |
| | `[cdr] spool_dir` | — | `/var/lib/fcp/cdr-spool` (`conference-cdr-spool`) |
| `fcp-media` | `[recording] base_dir` | `/var/lib/fcp/media/recordings` | the same (`media-recordings`) |
| | `[recording] prompts_dir` | `/var/lib/fcp/media/prompts` | the same (`media-prompts`) |
| `fcp-voicemail` | `[voicemail] recordings_dir`, `greetings_dir` | `/var/fcp/voicemail/…` | `/var/lib/fcp/voicemail/…` (`voicemail-data`) |
| `fcp-admin` | `[compliance] exports_dir` | `/var/lib/fcp/exports` | `/app/data/exports` (`admin-data`) |
| | `[api.firmware] directory` | unset (no uploads) | `/app/data/firmware` (`admin-data`): uploaded firmware images, re-derivable by uploading them again, but the rows in `firmware_images` name them |
| | `[api.captures] directory` | unset (no captures) | `/app/data/captures` (`admin-data`): captures apps uploaded (`docs/CALL_DIAGNOSTICS.md` §3), not re-derivable; the rows in `device_captures` name them, and a file restored without its row is never served or removed |
| `fcp-tts` | `[tts] cache_dir` | `/tmp/fcp/tts_cache` | `/var/lib/fcp/tts_cache` (`tts-cache`; a cache) |
| every binary that opens Postgres | `FCP_MASTER_KEY_FILE` | unset (encryption off) | `/var/lib/fcp/certs/master.key` (`fcp-certs`) |

The IVR, presence, SBC, registrar, events, gateway, WebRTC and jobs
binaries write nothing to disk that a backup needs: their state is in
Postgres and Redis. (The jobs worker's CDR archive, when `[cdr_archive]`
is set, is a bucket or a directory of its own, versioned there.)

## 3. What a backup holds

A backup is: a dump of the database; the master key (and, to spare
phones a new CA, the CA beside it); every directory in §2 marked as
not re-derivable, from every node; and a manifest saying which release
took it. Redis is not in it, and §1 says what that costs. Point-in-time
recovery and streaming replicas are the Postgres documentation's
subject, not this one's.

### 3.1 The compose stack: `backup.sh`

```bash
cd deploy/docker-compose
scripts/backup.sh /backups/fcp-$(date +%F)
```

The directory must be new or empty. `postgres` must be running; the
rest of the stack may be up or stopped. The script needs `docker` and
`sha256sum` on the host and nothing else — `pg_dump` runs inside the
`postgres` container, so the client is always the server's own — and
it prints every step. It writes:

| File | What |
|---|---|
| `fcp.dump` | `pg_dump -Fc --no-owner --no-privileges` of the `fcp` database: one consistent snapshot of every table in §1, the key ring included |
| `fcp-certs.tar.gz` | the master key, the CA and every service certificate |
| `call-manager-data.tar.gz`, `admin-data.tar.gz`, `voicemail-data.tar.gz`, `conference-recordings.tar.gz`, `conference-cdr-spool.tar.gz`, `media-recordings.tar.gz`, `media-prompts.tar.gz` | each stateful volume of §2, a tar of its root with ownership kept; a volume that does not exist because its service never ran is skipped and named under `volumes_absent` |
| `manifest` | `taken_at`, `stack_version` (the admin image's binary asked `--version`, or the checkout's when no image is built), `project`, `kek_id` (the key ring's active KEK — what `master.key` must fingerprint to), `volumes`, `volumes_absent` |
| `SHA256SUMS` | every file's checksum; `sha256sum -c SHA256SUMS` verifies a copy |

Not in it: `postgres-data` (that is the dump), `tts-cache` and
`redis-data` (re-derivable, §1), `prometheus-data` and `grafana-data`
(the monitoring stack's own). The script refuses to finish without
`fcp-certs`: a dump without the key is secrets nobody can open.

**The dump is a snapshot; the volumes are copied after it, live.** A
recording still being written when its volume is copied is partial in
the backup, and a file finished after the dump has no row — both are
harmless to a restore and both are the price of not stopping. For a
backup with neither, stop the services that write files first and
leave the database up:

```bash
docker compose stop fcp-call-manager fcp-conference fcp-media fcp-voicemail fcp-admin
scripts/backup.sh /backups/fcp-$(date +%F)
docker compose start fcp-call-manager fcp-conference fcp-media fcp-voicemail fcp-admin
```

Keep the backup somewhere that is not the host: the directory is what
`restore.sh` (§4) takes, whole. Take one before every upgrade (§5).

### 3.2 From source: the same commands

A deployment from source runs the same four steps against its own
paths — the database URL each binary's `[storage] database_url` names,
the key file `FCP_MASTER_KEY_FILE` names, and the directories §2
lists as set on each node. On the node that holds the key and reaches
the database:

```bash
OUT=/backups/fcp-$(date +%F); mkdir -p "$OUT"
pg_dump -Fc --no-owner --no-privileges "$DATABASE_URL" > "$OUT/fcp.dump"
cp "$FCP_MASTER_KEY_FILE" "$OUT/master.key"; chmod 600 "$OUT/master.key"
psql -tA -c "SELECT kek_id FROM data_encryption_keys WHERE active" "$DATABASE_URL" > "$OUT/kek_id"
```

On every node, each directory of §2 that is not re-derivable — the
call manager's `recording_dir`, `prompts_dir` and `spool_dir`, the
conference server's `recordings_dir` and `spool_dir`, the media
server's `base_dir` and `prompts_dir`, the voicemail server's two, the
admin's `exports_dir` — as a tar, named for the node and the key:

```bash
tar -C /var/lib/fcp/media/recordings -czf "$OUT/node-2.media-recordings.tar.gz" .
```

Then the checksums beside them (`cd "$OUT" && sha256sum * > SHA256SUMS`),
and a note of the release (`fcp-admin --version`). The `pg_dump` and
`psql` on the node must be the server's major version or newer. A
restore is §4 with the same files: the key on every node first, the
dump into an empty database, each tar back at the path its node's
rows name.

## 4. What a restore needs

A fresh stack — or a stack whose data is gone — with the key restored
before any service starts, the database restored before any service
opens it, and each node's files at the paths its rows name. The first
service to start finds a key ring wrapped by the key it holds; a wrong
key refuses to start, naming the KEK the ring wanted and the one it
was given. Everything the deployment did after the backup is gone,
including erasures: a restored backup can hold what a subject request
removed since, which is why `docs/COMPLIANCE_FOLLOWUPS.md` §9 counts
backups as a store erasure does not reach and the report says so.
Redis starts empty, and §1 says what that means.

### 4.1 The compose stack: `restore.sh`

```bash
cd deploy/docker-compose
docker compose down -v                  # drops what is there, if anything
scripts/restore.sh /backups/fcp-2026-09-24
```

The directory is one `backup.sh` wrote. The script refuses at the first
thing wrong and touches nothing before it is sure, in this order:

1. **The manifest and the checksums.** `sha256sum -c SHA256SUMS`; a
   copy that does not match is refused as damaged.
2. **A fresh stack.** No volume of the stack may exist. A restore never
   merges into a running deployment; `docker compose down -v` is the
   operator's step, and it drops everything.
3. **The key is the key.** `master.key` inside `fcp-certs.tar.gz` must
   fingerprint to the KEK id the manifest's key ring names (`local:` and
   the first eight bytes of its SHA-256, as every binary derives it). A
   key and a dump not taken together are refused here, before any
   service could refuse them.
4. **The volumes**, each created as compose names and labels it and
   filled from its tar, the certs volume first.
5. **The database.** `postgres` is started alone and waited for; the
   dump is restored into its empty `fcp` database with `pg_restore
   --no-owner --no-privileges --exit-on-error`; the key ring it holds
   must name the manifest's KEK id.
6. **Up.** `docker compose up -d --build`, unless `--no-up`.

After it, every service starts under the restored key, the users
register with their sealed passwords, and the recordings are where
their rows say. The service certificates in the volume name the
address the backup's host had (`HOST_IP`); on a host with another
address, remove them from the volume after the restore and before
`up` — not `ca.*` and not `master.key` — and the `certs` service
remakes them under the kept CA, so phones that trust it keep
trusting it.

### 4.2 From source: the same commands

The key file first, on every node, at the path `FCP_MASTER_KEY_FILE`
names; then the dump into an empty database — `createdb fcp` or its
equivalent, then `pg_restore --no-owner --no-privileges --exit-on-error
-d "$DATABASE_URL" fcp.dump` — before any binary opens it; then each
tar back at the directory its node's configuration names (`tar -C
/var/lib/fcp/media/recordings -xzf node-2.media-recordings.tar.gz`);
then the binaries. Check the key before starting anything:

```bash
echo "local:$(tr -d '[:space:]' < master.key | base64 -d | sha256sum | cut -c1-16)"
psql -tA -c "SELECT kek_id FROM data_encryption_keys WHERE active" "$DATABASE_URL"
```

The two must be equal. A binary started with another key exits at once
with `active data encryption key … was wrapped by KEK local:… but this
process is configured with KEK local:…`.

### 4.3 Proved

`crates/fcp-call-manager/tests/compose_restore.rs` runs these commands
against the binaries' own directories: the stack's database services
start from the compose configurations with encryption on, a user made
through the admin API (so their SIP credentials are sealed) registers
and calls a room; the dump, the tars, the manifest and the checksums
are taken; the database is dropped and the directories deleted; the
backup is restored into a new database and directory. With a different
master key beside the restored certificates, every one of the nine
services that opens the database refuses to start, naming both KEKs.
With the right one, the same data encryption key is active, the old
user registers with the sealed password and the room takes a call.

## 5. What an upgrade does

Every binary that opens Postgres runs the schema migrations at
startup. The first node of a new release migrates the schema for all;
a node of the previous release keeps running against it, but a binary
older than the schema **refuses to start** — so once one new node has
started, the way back is not the old binary but the backup taken before
the upgrade. Take one.

### 5.1 The rules

1. **Back up first** (§3). A restore (§4) is the only rollback.
2. **The first new node migrates the schema** for every node. It runs
   the migrations it carries that the database has not, in order, at
   startup, before it serves.
3. **An old binary against a new schema refuses to start.** The
   database has applied a migration the binary does not carry
   (`VersionMissing` in its log). Restarting the old release on any
   node is not a rollback; restoring the backup is.
4. **A migration is never edited after it ships.** Every node checks
   each applied migration's checksum at startup and refuses on a
   mismatch (`VersionMismatch`). A mistake is fixed by the next
   migration.
5. **A migration may add, but not take away what the previous release
   reads.** Old and new nodes share one schema while an upgrade runs.
   A table, view or column may be created; a column made nullable or
   given a default; an index or constraint dropped. A table, view or
   column may not be dropped or renamed, and a column's type may not
   change, until no node of the previous release is left — which is the
   release after. Such a *contraction* ships one release after the code
   stopped reading what it removes, and says so on a line of its own:

   ```sql
   -- fcp: contraction: cdrs.cost_cents, read by nothing since v0.25.261
   ALTER TABLE cdrs DROP COLUMN cost_cents;
   ```

   CI refuses a new migration that contracts without that line
   (`scripts/check-migrations.sh`, on every migration above 077; the
   three older ones that contract predate the rule).
6. **One release at a time.** A rolling upgrade goes from the previous
   release to the next; skipping one means a migration written for a
   schema the running nodes never had.

### 5.2 What the cluster shows

Every node's heartbeat carries its version, and every node groups the
members by it at each discovery:

- **`/status`** on the call manager and the conference server:
  `cluster_versions` — `versions` (release → node ids), `skew` (more
  than one release), `summary`.
- **`/metrics`**: `fcp_cluster_version_skew` (1 while more than one
  release runs) and `fcp_cluster_nodes_by_version{version=…}`.
- **The admin API**: `GET /api/v1/cluster/nodes` has each node's
  `version`; `GET /api/v1/cluster/stats` has `versions` and
  `version_skew`.
- **The admin UI's cluster page**: the version column, and a banner
  naming who is on what while the releases differ.
- **`system.version_skew`**, when the set of releases changes while
  there is or was more than one: the first new node starting, the last
  old one stopping, with `versions`, `skew` and `summary`. Said by
  every node that sees it, never acted on — a rolling upgrade is a skew
  on purpose. A skew that stays is an upgrade that stopped half way.

### 5.3 The compose stack

```bash
cd deploy/docker-compose
scripts/backup.sh /backups/fcp-$(date +%F)      # §3
git pull                                        # the release to run
docker compose config --quiet                   # the compose file still resolves
docker compose up -d --build                    # rebuilds what changed, restarts it
docker compose ps                               # every service up
```

Every service restarts as its image changes; the first to open the
database migrates it, the rest wait on `depends_on` and find it done.
Calls in progress on the call manager end with the restart — the compose
stack is one node, with nowhere to drain to. To see it went well:
`docker compose logs --since 5m | grep -i migrat`, then the admin UI's
cluster page or `/status` on the call manager showing the new version.

**If it does not come up**: a service that exits at once prints the
rule it broke (`docker compose logs <service>`); a `VersionMissing`
means an old image met the new schema — `docker compose build` that
service again; anything else, `docker compose down` (not `-v`),
`git checkout` the previous release, `docker compose down -v`, restore
the backup (§4), `up -d --build`.

### 5.4 A cluster

One node at a time, the runbook's drain first
(`docs/CLUSTER_OPERATIONS_RUNBOOK.md` §5): back up, then on each node in
turn — drain, wait for its calls to end, stop, install the release,
start, wait for `/ready` to answer 200 and the node to show the new
version on the cluster page — and only then the next. The first node
started migrates the schema; from then on `system.version_skew` says
the cluster runs two releases, and says again when it runs one. The
order of nodes does not matter to the schema; upgrade the binaries
that share a database in one pass rather than leaving some behind.

What cannot be rolled forward is rolled back whole: stop the new
nodes, restore the backup (§4) — the database, the key, every node's
files — and start the previous release. A new node's calls and
recordings since the backup are lost with it; the CDR spool on each
node re-ships what it holds.

### 5.4.1 Behaviour an upgrade changes

Changes that need something of a configuration the operator keeps:

- **v0.25.397: `[registrar] auth_invites` is on by default**
  (`docs/EDGE_SECURITY.md` §2.1). A call manager with `auth_enabled`
  challenges every INVITE except a carrier's on an enabled trunk and a
  peer's in `[e911] trusted_edges`, loopback included. Before upgrading,
  list in `trusted_edges` every binary that INVITEs the call manager
  without a user's password: the SBC in front of it, the WebRTC gateway,
  and the conference, voicemail and IVR servers. The shipped compose and
  single-node configurations trust `127.0.0.0/8`, since everything there
  runs on one host. Phones answer the challenge with the credentials they
  register with. Setting `auth_invites = false` keeps the old behaviour,
  and is warned about at every start.
- **v0.25.512: a digest must be signed for the Request-URI**
  (`docs/EDGE_SECURITY.md` §2.7, #307). A REGISTER, INVITE or SUBSCRIBE
  whose digest `uri` is not the Request-URI, as RFC 3261 §19.1.4 compares
  them, is answered `400 Bad Request`. Real phones sign the Request-URI;
  SIPp, and anything on the way that rewrites the Request-URI after the
  phone signed it, do not. Before upgrading, look in a v0.25.511 call
  manager's log for "A digest signed for another Request-URI": a device
  named there is refused after the upgrade. `digest_uri_check = false`
  (`[registrar]` on the call manager and the standalone registrar, `[sip]`
  on the presence server) logs and accepts it as before.
- **v0.25.525: REGISTERs per source are limited by default**
  (`docs/CAPACITY_AND_SURVIVABILITY.md` §10, #314). A call manager or
  registrar whose `[admission]` does not set `max_register_cps_per_source`
  refuses a source's REGISTERs past 50 a second with `503` and
  `Retry-After` (a challenged registration is two). Peers in
  `[sip] trusted_sources` or `[e911] trusted_edges`, trunks and cluster
  members are exempt. Before upgrading, make sure every SBC, load balancer
  or site link that registers devices on their behalf is in one of those
  lists; otherwise all its devices share its 50. `max_register_cps_per_source
  = 0` keeps the old behaviour.
- **v0.25.453: an emergency test call (`933`) is answered by the call
  manager** (`docs/E911.md` "Test calls"), which reads back what a live
  call would carry and calls no carrier. Migration 120 gives every tenant
  `test_call_mode = 'local'`. A tenant that relied on its carrier's test
  service sets `test_call_mode` to `carrier` on `/api/v1/emergency/config`
  or the Emergency page.
- **v0.25.496: system prompts have no tenant** (`docs/TENANT_SCOPE.md`
  §3.2). Migration 133 moves the prompts the call manager and conference
  server seeded into their default tenant (`is_system`) to the platform:
  every tenant hears them, and the default tenant's edits to them are
  every tenant's. Where two tenants held a system row of one name and
  language, the newer becomes the system prompt and the other stays its
  tenant's, as an override. While an upgrade runs, a node of an earlier
  release cannot read a prompt without a tenant: it speaks those prompts
  with TTS, and an earlier admin answers 500 on its prompt list, until it
  is upgraded. New system prompt audio goes under `<prompts_dir>/system/`;
  files already under the default tenant's directory stay where they are.
- **v0.25.497: an audit row belongs to the tenant acted on**
  (`docs/TENANT_SCOPE.md` §3.3, `docs/COMPLIANCE.md`). From this release
  a platform administrator's action on a tenant is that tenant's row, and
  the platform's own actions have no tenant. Rows written before it stay
  under the actor's tenant: a platform administrator's earlier actions on
  other tenants are in the platform administrator's own tenant (the
  default tenant for the static token), and those tenants' administrators
  do not see them. Migration 134 adds `actor_tenant_id`, empty on the
  earlier rows.
- **v0.25.498: tokens without a maker are held to a tenant administrator**
  (`docs/API_TOKENS.md`, `docs/TENANT_SCOPE.md` §3.7). A token made with
  the static token or before makers were recorded — a legacy `admin` or
  `*` token among them — no longer reaches system configuration (`/config`,
  encryption, firmware images) or tenant creation. An integration that used
  one for those uses the static token instead; for anything else, replace
  the token with one made by a person and scoped to what it needs. The same
  release refuses a call another tenant's flow (a flow without a tenant is
  still anyone's until assigned on the Flows page), parks `*70` in the lot
  of the parker's location, and marks bound numbers `assigned`
  (migration 135).
- **v0.25.514: the audit log's links and reads** (`docs/COMPLIANCE.md`,
  #342). Migration 137 adds `api_request_id` to the audit log and the
  deletion log, and `via` to the deletion log, all empty on earlier rows.
  More reads are audited (exports of the audit and deletion logs, recording
  downloads, an administrator's voicemail reads, SIP messages), so the log
  grows faster for a tenant that reads them often. `GET /config/audit`
  without `service` now lists every service's changes, where it listed
  `_global`'s: a script that relied on that passes `service=_global`. A
  message deleted from the voicemail menu names the mailbox owner's
  username, not `phone:<mailbox>`.
- **v0.25.515: names and greetings** (`docs/USERS.md` §7b,
  `docs/VOICEMAIL.md`, #343).
  - **Names.** Migration 138 adds a user's `first_name` and
    `last_name`. SCIM's `name.givenName` and `name.familyName` and single
    sign-on's `given_name` and `family_name` now fill them, where they
    were folded into the display name. A display name that is only the
    two is no longer kept.
  - **Greetings.** Migration 138 adds a mailbox's `greeting_choice` and
    `greeting_name`. A mailbox that had a greeting of its own is set to
    `own`, so its callers hear what they did. Every other mailbox is on
    the default: the greeting with the owner's name when they have a
    first and last name, the standard greeting otherwise. A new mailbox
    no longer gets an unavailable greeting spoken from its display name.
  - **One greetings directory.** In the compose stack the admin now
    mounts `voicemail-data` and writes greetings to
    `/var/lib/fcp/voicemail/greetings`. Before this, the greetings it
    made were in its own `/tmp`, and callers never heard them: upload
    them again. A call manager from source sets `[media]
    voicemail_greetings_dir` to the directory the admin and the voicemail
    server use.
  - **During the upgrade.** A node of the previous release that writes a
    mailbox drops its personalized greeting row, since that release does
    not know the type. The greeting is made again the next time the
    mailbox or its owner's name changes. Until then callers hear the
    standard greeting.

### 5.5 Proved

- `crates/fcp-storage-postgres/tests/upgrade.rs`: this release opens an
  empty database; a migration from a later release is recorded as
  applied; this release refuses to start, naming it (`999999 … missing`);
  and with a shipped migration's checksum changed, refuses that too
  (`migration 1 … modified`).
- `crates/fcp-cluster/src/versions.rs` tests the grouping, the summary,
  the skew appearing, changing and resolving, and the metrics;
  `crates/fcp-cluster/tests/versions.rs`, against Redis: two nodes on
  one release say nothing, a third on another is grouped apart and
  said as `system.version_skew` with who is on what, the admin's
  reader counts the skew from the heartbeat records, and the odd node
  shutting down is said as one release again.
- `crates/fcp-call-manager/tests/compose_stack.rs`: the compose call
  manager, clustered as its configuration ships, is on the admin's node
  list with this release, the stats and its own `/status` group it
  under it, and no skew.
- The nightly's stack job (`.github/workflows/images.yml`,
  `docs/DEEP_TESTING.md` D5c) does §5.3 with images every night: the
  stack comes up on the previous release (the last good scheduled run on
  main, its images and its compose project), carries a call that makes
  the tenant, users, sealed passwords, registrations and a record, is
  backed up, and is upgraded in place to the release just built. A call
  through the upgraded stack finds all of it, the record read back by
  the new release, and the call manager on the new version.
- The CI step `The migration discipline` runs `scripts/check-migrations.sh
  --self-test` — an add passes, a drop, a rename and a type change fail
  without the marker and pass with it — before checking the migrations.
