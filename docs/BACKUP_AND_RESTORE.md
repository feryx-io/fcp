# Backup, restore and upgrade

How a deployment is kept: where every piece of state lives, what a
backup must hold, what a restore needs, and what an upgrade does. The
design behind it, with what the survey found, is
`docs/BACKUP_RESTORE_UPGRADE.md`; this is the document an operator
reads. Sections 3 to 5 arrive with that project's phases 2 to 4 and say
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

*The tool that takes one, and the from-source procedure, arrive with
phase 2 of `docs/BACKUP_RESTORE_UPGRADE.md`.*

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
