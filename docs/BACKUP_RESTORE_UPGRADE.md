# Backup, restore and upgrade

Status: **agreed** (2026-09-24), all five questions in §3 settled by the
user, each on its recommendation.
Chosen when the production-shaped stack closed (`docs/PRODUCTION_STACK.md`):
the stack can be run, and this is what it takes to keep it — to know what
must be saved, to get it back, and to move it to the next release without
losing what it holds.

Read with `deploy/docker-compose/README.md` (the stack and its volumes),
`docs/ENCRYPTION.md` (why the key is part of the backup),
`docs/CLUSTER_OPERATIONS_RUNBOOK.md` §5 (draining and rolling restarts)
and `docs/COMPLIANCE_FOLLOWUPS.md` §9 (what erasure cannot reach, which
is backups).

## 1. What the code has, and what it lacks

### 1.1 The state, and where it lives

Every binary keeps state in one of four places. Checked against the
compose stack's configurations and the binaries' defaults:

| Where | What | Survives a restart | Re-derivable |
|---|---|---|---|
| **Postgres** | Every configuration object (tenants, users, devices, trunks, numbers, rules, flows, meetings), registrations (unless `[registrar] registration_backend = "redis"`), the CDR tables and rollups, recordings and voicemail rows (with each file's path and the node that holds it), legal holds, the deletion log, subject requests, audit log, the key ring (DEKs wrapped by the master key), billing ledgers | yes | **no** — this is the platform |
| **The certs volume** (`fcp-certs`) | `master.key`, the CA, every service certificate | yes | the certificates, yes (the script remakes them under a kept CA); **the master key, no** — without it every sealed column is unreadable, and a new key with an old database refuses to start (`docs/ENCRYPTION.md`) |
| **Files on a node** | Call recordings (call manager, media server), conference recordings (WebM/MP4), voicemail messages and greetings, prompts, the CDR spool of records not yet shipped, subject-export archives, the TTS cache | as volumes, yes | recordings, voicemail and exports, **no**; prompts and the TTS cache, yes |
| **Redis** | Cluster membership, registrations when the Redis backend is on, the event stream and its consumer positions, the CDR stream `fcp:cdrs`, the job queue (webhook deliveries, recording processing, cleanup), call parking, the registration cache | with `--appendonly yes`, yes | membership and caches, yes; registrations re-register; the CDR stream re-ships from each node's spool and the processor stores `ON CONFLICT DO NOTHING`; the job queue's pending entries, **no** — a webhook retry or a cleanup queued and not yet run is lost with Redis |

Three findings from the table:

- **The call manager's recordings are not in a volume.** Its compose
  configuration sets no `[media] recording_dir`, whose default is
  `./recordings` — `/app/recordings` in the container, on the container's
  own filesystem, gone at the next `up --build`. `prompts_dir` and
  `tts_cache_dir` default under `/tmp/fcp`. The conference, media and
  voicemail servers put theirs in volumes; the call manager is the odd
  one out.
- **A recording's row names its node and its path** (`call_recordings`,
  `conference_recordings`: `node_id`, `path`), so a restore must put each
  node's files back where that node reads them — in the compose stack,
  the same volume at the same mount. The reaper and the download route
  go by those paths.
- **`docs/STORAGE_BACKENDS.md` and `docs/SQLITE_STORAGE.md` describe
  backends that do not exist.** SQLite (called "production ready"),
  MySQL and MongoDB, with backup commands for each; SQLite was descoped
  on 2026-09-02. Neither document is indexed anywhere. They are the only
  place the word "backup" appears as an instruction.

### 1.2 Nothing backs anything up

There is no backup procedure, no tool and no test. The compose README
says the key is part of the backup and that `down -v` drops the
database; it does not say how to take one. A deployment that follows
every document in the tree has never been told to save anything.

### 1.3 Migrations run forward only, and refuse to run backward

Every binary that opens Postgres runs `sqlx::migrate!` at startup with
the defaults. Two consequences the runbook does not mention:

- A binary **older** than the schema refuses to start
  (`MigrateError::VersionMissing`: the database has applied a migration
  the binary does not carry). So once one node of a new release has
  started, no node can be rolled back to the old release by restarting
  the old binary; the only way back is a restore of the pre-upgrade
  backup. That is the right default — an old binary reading a schema it
  does not know is worse — but nobody has said so.
- A migration file that is **edited** after it shipped fails the
  checksum (`VersionMismatch`) on every node. Also unsaid.

And a rolling upgrade — old and new nodes on one schema for a while —
is only safe when a migration never removes what the previous release
still reads. Three of the seventy-seven migrations drop or rename
(`047_rate_plans`, `058_money_numeric`, `059_cdr_partitions`); they
predate the runbook, and nothing prevents the next one.

### 1.4 A node's version is invisible

Every node's heartbeat carries its version (`ClusterMember.version`),
and each binary's `/status` prints its own; nothing shows all of them
together, so a cluster half-upgraded looks like a cluster. The admin
API's node list and the admin UI's cluster page do not have the field.

### 1.5 What is sound

The pieces a procedure needs exist: `pg_dump` in the Postgres image,
the CDR spool that makes the stream re-shippable, the processor's
idempotent insert, the certs service that keeps what exists, the key
ring that names its KEK so a wrong key fails at startup with a message,
the record's `schema_version` that the processor refuses when newer
than its own, drain and `/ready` for one node at a time.

## 2. Phases

One PR each, in order. Each is checked by CI: a configuration through
`--check-config`, a script through `bash -n` and the stack test, a
procedure through a test that performs it.

| Phase | What |
|---|---|
| 1 | ✅ v0.25.257 — **Every stateful path in a volume, and the state documented** (§1.1). The compose call manager gets `recording_dir`, `prompts_dir` and `tts_cache_dir` under `call-manager-data`; the operator document `docs/BACKUP_AND_RESTORE.md` is written from the table above — what lives where, what a backup must hold, what Redis loses — and `docs/STORAGE_BACKENDS.md` and `docs/SQLITE_STORAGE.md` go (Q4); the compose README and the guide point at it. |
| 2 | ✅ v0.25.258 — **Backup** (§1.2, Q1). `deploy/docker-compose/scripts/backup.sh`: `pg_dump` in custom format through the `postgres` container, a tar per stateful volume (`fcp-certs`, `call-manager-data`, `admin-data`, `voicemail-data`, `conference-recordings`, `conference-cdr-spool`, `media-recordings`, `media-prompts`), and a manifest naming the stack version, the KEK id, the time and every file's SHA-256, into a directory the operator names; the from-source equivalent, the same commands against the binaries' directories, in the document. Redis is not in it (Q2); what that loses is stated. |
| 3 | ✅ v0.25.259 — **Restore, proved** (§1.2). `scripts/restore.sh` into a fresh stack: the key and CA first, the database from the dump, each volume from its tar, checksums verified against the manifest, then `up`; the script refuses a backup whose KEK id is not the one the restored key ring names. A test (`crates/fcp-call-manager/tests/compose_restore.rs`) seeds a stack, carries a call, takes a backup with the same commands, drops everything, restores into a new database and directory, starts the stack and registers the old user with the sealed password — and, without the key, watches every database service refuse to start. |
| 4 | ✅ v0.25.261 — **Upgrade** (§1.3, §1.4, Q3, Q5). The version on the admin API's node list, `/status` and the admin UI's cluster page, with a `system.version_skew` note when nodes differ; the rules written down — back up first, the first new node migrates the schema, an old binary against a new schema refuses to start and rollback is the backup, a migration is never edited after it ships; the migration discipline for a rolling upgrade — a migration may add but not drop or rename what the previous release reads, the contraction shipping one release later — with a CI check on new migration files (Q5); the compose upgrade (`backup.sh`, `git pull`, `up -d --build`) and the cluster upgrade (the runbook's §5, one node at a time) as procedures in the document. |
| 5 | **The skew, proved.** A unit test that a binary meeting a schema newer than its own refuses with the migration's version in the message; the stack test asserting the version it started shows on the node list; a floor test that a node reporting a different version is flagged. |

Folded into phase 1, not a phase: the two stale storage documents.

## 3. Questions

Five, settled by the user on 2026-09-24, each on its recommendation.

**Q1. The tooling: shell scripts in the compose directory, or subcommands
of a binary?** — *Scripts.* `pg_dump`, `pg_restore` and
`tar` are the right tools and the ones an operator already trusts; the
scripts show every command they run, and the from-source procedure is
the same commands written out. A `fcp-cli backup` would wrap them and
hide them, and would need Postgres client tools in an image that has
none.

**Q2. Is Redis backed up?** — *No.* Its durable contents
are re-derivable except the job queue's pending entries — a webhook
retry or a cleanup not yet run — and those are minutes of work the next
event redoes. Saving the AOF alongside would buy that back at the cost
of a backup that must catch Redis and Postgres at one instant, which a
dump and a file copy cannot. The document says what is lost.

**Q3. An old binary against a new schema: keep refusing, or run with
`ignore_missing`?** — *Keep refusing.* Rollback is the
pre-upgrade backup, which phase 2 makes real; a binary that ignores
migrations it does not know is running against tables it cannot see.

**Q4. The two stale storage documents: delete, or rewrite in place?** —
*Delete*, with `docs/BACKUP_AND_RESTORE.md` taking the subject. There is one backend; a document listing four is wrong in its
first line.

**Q5. The migration discipline: a CI check, or documentation only?** —
*A CI check*: a job that fails on a new migration file
containing `DROP`, `RENAME` or `ALTER … TYPE` unless the file says
`-- fcp: contraction` and names the release whose nodes no longer read
what it removes. The three existing ones are grandfathered by number.

## 4. Testing

- Phase 1: the call manager's compose configuration through
  `--check-config`; the stack test unchanged.
- Phase 2: `bash -n` on the script; the stack test unchanged.
- Phase 3: the restore test — a call before, a backup, a fresh stack, the
  old user registers and calls; and the refusal without the key.
- Phase 4: `docker compose config`; the CI check on migrations against
  a file that drops a column (must fail) and one that adds (must pass).
- Phase 5: the unit test for the refusal message, the version on the
  node list in the stack test, the skew flag in the floor.

## 5. Not in scope

Point-in-time recovery (WAL archiving) and streaming replicas as a
backup — `docs/MULTI_REGION.md` has replicas for reads, not for
recovery; a deployment that needs PITR runs Postgres the way its
Postgres documentation says. Backups of the CDR archive bucket (it is
its own store, versioned by its bucket). Encrypting the backup itself
(the dump holds sealed columns; the key beside it is the operator's to
protect).
