# Backup, restore and upgrade

How a deployment is kept: where every piece of state lives, what a
backup must hold, what a restore needs, and what an upgrade does. The
design behind it, with what the survey found, is
`docs/BACKUP_RESTORE_UPGRADE.md`; this is the document an operator
reads. Sections 4 and 5 arrive with that project's phases 3 and 4 and say
so until they do.

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
| | `[media] prompts_dir` | `/tmp/fcp/audio/prompts` | `/app/data/prompts` |
| | `[media] tts_cache_dir` | `/tmp/fcp/tts_cache` | `/app/data/tts-cache` |
| | `[cdr] spool_dir` | — | `/app/data/cdr-spool` |
| `fcp-conference` | `[conference] recordings_dir` | `/var/fcp/conference/recordings` | the same (`conference-recordings`) |
| | `[conference] announcements_dir` | `/var/fcp/conference/announcements` | mounted from `configs/announcements`, read-only |
| | `[cdr] spool_dir` | — | `/var/lib/fcp/cdr-spool` (`conference-cdr-spool`) |
| `fcp-media` | `[recording] base_dir` | `/var/lib/fcp/media/recordings` | the same (`media-recordings`) |
| | `[recording] prompts_dir` | `/var/lib/fcp/media/prompts` | the same (`media-prompts`) |
| `fcp-voicemail` | `[voicemail] recordings_dir`, `greetings_dir` | `/var/fcp/voicemail/…` | `/var/lib/fcp/voicemail/…` (`voicemail-data`) |
| `fcp-admin` | `[compliance] exports_dir` | `/var/lib/fcp/exports` | `/app/data/exports` (`admin-data`) |
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
key refuses to start. Everything the deployment did after the backup
is gone, including erasures: a restored backup can hold what a subject
request removed since, which is why `docs/COMPLIANCE_FOLLOWUPS.md` §9
counts backups as a store erasure does not reach and the report says
so.

*The tool that performs one, and the test that proves it, arrive with
phase 3.*

## 5. What an upgrade does

Every binary that opens Postgres runs the schema migrations at
startup. The first node of a new release migrates the schema for all;
a node of the previous release keeps running against it, but a binary
older than the schema **refuses to start** — so once one new node has
started, the way back is not the old binary but the backup taken before
the upgrade. Take one.

*The rules in full — what a migration may and may not do while old and
new nodes share a schema, the version on the node list, the procedure
for the compose stack and for a cluster — arrive with phase 4.*
