# Backups from the admin

Designed 2026-11-27; the questions were agreed the same day on every
recommendation except one: question 9, where tenant administrators may see
the platform's backups and put back their own files. Backup and restore exist (`docs/BACKUP_AND_RESTORE.md`,
`deploy/docker-compose/scripts/backup.sh` and `restore.sh`), but an
operator runs them by hand on the host. Nothing schedules them, nothing
takes them off the box, and the admin knows nothing of them.

## 1. What there is

- **`backup.sh`** writes the following into a directory on the host:
  - a `pg_dump` of the database, taken through the `postgres` container;
  - a tar of each stateful volume: the certs volume with the master key,
    recordings, voicemail, prompts, the CDR spool and the admin's data;
  - a manifest naming the release and the KEK id, and `SHA256SUMS`.
- **`restore.sh`** puts all of that back:
  - it checks the checksums, and checks the key's fingerprint against the
    manifest;
  - it recreates the volumes, `pg_restore`s into a fresh `postgres`
    container, and brings the stack up.

  `crates/fcp-call-manager/tests/compose_restore.rs` proves it.
- **Both need the host.** They need the Docker socket and every volume.
  No FCP process has either, so the admin cannot run them, and nothing
  runs them on a schedule.
- **Nothing goes off the box.** A backup lives on the same disk as what it
  backs up. D8 showed how that disk fills (`docs/DEEP_TESTING.md` D8).
- **The key travels with the data.** A backup directory holds the master
  key beside the dump that key unseals. Whoever has the file has every
  tenant's trunk passwords, SSO secrets and vendor keys.
- **What FCP already speaks.** It can write to S3-compatible storage
  (`fcp-admin`'s `S3Target`, used by the CDR archive and by firmware).
  It speaks no SFTP.
- **Sites have the same scripts** (`COMPOSE_DIR`), with the same gaps.

## 2. The model

**A backup service, `fcp-backup`.** It is a new binary and a new compose
service. Whatever it backs up, it mounts read-only, and it reaches the
database as a client. The admin never touches the host: it writes the
schedule and the destinations, and it asks the service for a run. The
service reports each run back. A from-source deployment runs the same
binary with its paths in a file, as every binary has. Only the restore
needs more (below).

**restic as the engine.** The service writes a restic repository rather
than a format of its own:
- **Encrypted:** AES-256 with an authenticated MAC, under a repository
  password.
- **Deduplicated:** a run sends only the chunks the destination does not
  have yet. A recording backed up once is never sent again, and a nightly
  backup of a large system is small.
- **Retention:** restic's own `forget --keep-daily 7 --keep-weekly 4
  --keep-monthly 12`.
- **Checked:** `check --read-data-subset` verifies what is stored.
- **Many destinations:**
  - SFTP;
  - S3-compatible: AWS, Backblaze B2, Wasabi, Cloudflare R2, MinIO, Garage;
  - Azure and Google Cloud Storage;
  - restic's own REST server, which can be append-only;
  - more than 70 others through rclone.
- **Safe to depend on:** restic is BSD-2 licensed, widely used, and a
  single static binary in the image.

Writing chunking, encryption and pruning ourselves would be months of work
to reach where restic already is.

**What a snapshot holds.**
- **The database:** a `pg_dump` in custom format, streamed into the
  repository and never written to the service's disk.
- **The files, read in place from their volumes:**
  - recordings, voicemail, prompts and greetings;
  - the CDR spool;
  - firmware;
  - captures;
  - the admin's data.
- **Tags:** the release, the KEK id, the node and the tables' row counts,
  for the restore to check.
- **Not kept:** Redis, the TTS cache and monitoring data. They are
  rebuilt or are not ours (`docs/BACKUP_AND_RESTORE.md` §1).

**The master key, kept apart.**
- Off-box snapshots do not hold the key ring by default. A stolen snapshot
  is then the sealed secrets without the key that opens them.
- The first time backups are set up, the page shows the key once, wrapped
  in the backup password, to download and keep offline.
- A restore asks for that key file when the target does not have the key.
- An operator who wants one file to restore from can tick "Include the
  key in each snapshot". The key is then inside restic's encryption, and
  the page says what that means.

**Destinations.** Configured on the page; there may be several, each
with its own schedule:
- **SFTP:** host, port, user, a private key (sealed), and the host key
  pinned on first connect.
- **S3-compatible:** endpoint, bucket, region, access key (sealed), and
  an optional prefix.
- **A directory:** a path the service has mounted, such as an NFS or SMB
  share, or a USB disk.
- **A REST server:** URL and credentials, for an append-only server.
- **rclone:** a remote by name, for a provider none of the above covers.

Each destination has its own restic password (sealed), so a stolen SFTP
login alone opens nothing.

**Schedule and retention.** For each destination: a daily time in the
platform's timezone (or a cron expression), and how many daily, weekly and
monthly snapshots to keep. After each run, `forget --prune` keeps only
those. Once a week a check reads back a share of the data. A "Back up now"
button runs it at once.

**Watched.**
- **Each run is a row:** when, how long, the bytes added, the snapshot id
  and the outcome.
- **Events:** `system.backup_completed`, `system.backup_failed`, and
  `system.backup_overdue` (no success within 26 hours of when one was due).
- **Metrics:** `fcp_backup_last_success_timestamp` and
  `fcp_backup_bytes_added`.
- **The page:** an overdue destination is red.

**Restoring.** A snapshot can be restored three ways.
- **Download:**
  - The page offers any snapshot as a `.tar.gz` in `backup.sh`'s layout,
    so `restore.sh` takes it.
  - This is for restoring onto a new host, by hand.
- **Restore here, the whole system:** the service, with more power than
  it needs for backing up.
  1. The admin puts the platform into maintenance:
     - every service refuses new calls and API writes, and `/ready` fails;
     - calls up are let finish, up to a deadline.
  2. The service stops the other services (below).
  3. It restores the database: into a new database, swapped in by rename.
     If the restore fails halfway, the old database is still whole.
  4. It restores the volumes.
  5. It starts the services again. They run their migrations forward if
     the snapshot is older.
  6. It brings the platform out of maintenance.

  The page shows each step. If the admin itself is replaced underneath
  it, the page reconnects when the admin is back.
- **Restore a part:** files, a tenant's recordings or a day's voicemail,
  put back beside what is there. The database is untouched. This is for
  "we deleted the wrong thing".

**Stopping and starting the services.** A full restore needs this, and it
is the one dangerous power in the design.
- **In compose:** the service needs the Docker socket. That is root on the
  host, so it is mounted only when the operator turns restores on
  (`FCP_BACKUP_RESTORE=1` in `.env`). Without it, the page offers the
  download and the CLI.
- **From source or on Kubernetes:** the service runs the operator's own
  stop and start commands, named in its file.

**Sites.**
- A site's stack runs its own `fcp-backup`, configured on the site's own
  admin, because the site's data is its own.
- Its runs and overdue state go up the link, so the cloud's page lists
  every site's last backup. A site whose backups stopped is visible without
  logging into it.

**Who may.**
- **A platform administrator** may do everything, on new `backup_read`
  and `backup_manage` permissions. A backup is every tenant's data.
- **A tenant administrator** gets two things (Q9):
  - `backup_read` held to their tenant: the platform's schedule and when
    the last good backup was taken (never its destinations or secrets);
  - `backup_restore_own`: putting back their own tenant's lost files
    (recordings, voicemail and prompts) from a snapshot.

  The files are picked by the tenant's own rows that name them, so a
  tenant can reach no other tenant's file. A tenant never downloads a
  snapshot or restores the system.
- **Audited:** restores, key downloads and changes to destinations.

## 3. The API

- `GET/POST /api/v1/system/backups/destinations`, `PUT/DELETE …/{id}`, and
  `POST …/{id}/test` (connect, write and remove a probe).
- `POST …/{id}/run`: back up now.
- `GET /api/v1/system/backups/runs`: runs, filterable by destination.
- `GET …/destinations/{id}/snapshots`: what is stored, with its tags.
- `GET …/snapshots/{snapshot}/download`: the `backup.sh`-layout tar,
  streamed.
- `POST …/snapshots/{snapshot}/restore`: `{scope: "system" | "files",
  paths?, key_file?}`. It answers a restore id, and `GET
  /api/v1/system/backups/restores/{id}` reports its steps.
- `GET /api/v1/system/backups/key`: the master key, wrapped in a
  passphrase given in the request. This is audited.
- For a tenant administrator:
  - `GET /api/v1/backups`: the schedule and the last good backup.
  - `GET /api/v1/backups/missing`: the tenant's recordings, voicemail and
    prompts whose files are gone, with the snapshots that hold them.
  - `POST /api/v1/backups/restore-files`: `{snapshot, items}`, put back
    beside what is there.

## 4. The page

**System › Backups**, for platform administrators.
- **Destinations:** each with its type, schedule and retention, a last-run
  badge (green, amber or red), and Test, Back up now and Edit.
- **Runs:** a table of runs.
- **Snapshots:** per destination, with Download, Restore files and Restore
  the system. A full restore asks for the release and the time to be typed
  back, and lists what it will replace.
- **Setup:** a first-run panel, "Download your master key", until a key
  file has been downloaded once.
- **Sites:** a card listing each site's last backup.
- **A tenant's Backups page:** the last good backup, and the tenant's
  missing files with a Restore button for each or for all.

## 5. Phases

- **B1: the service and one destination.**
  - `fcp-backup` with restic in the image.
  - Destinations and runs stored, with secrets sealed.
  - SFTP and S3-compatible destinations, a schedule, retention, Back up
    now, runs and events.
  - Proved against a stand-in SFTP server (OpenSSH in a container) and
    Garage, plus a snapshot `restore.sh` takes.
- **B2: the page,** with the key download, and a Playwright spec.
- **B3: restoring from the admin.**
  - Download in `backup.sh` layout; partial restores, the tenant's own
    included.
  - Maintenance mode on every service; the full restore, through the
    Docker socket or the operator's commands, with the database swapped by
    rename.
  - Proved by restoring a populated stack and placing a call.
- **B4: more destinations and the watch.** A directory, a REST server and
  rclone; the weekly check; overdue; metrics.
- **B5: sites.** The site's own service, and runs up the link.

## 6. Questions

1. **The engine:** restic, which brings encryption, dedup, retention and
   checks, and every destination below (recommended); or a format of our
   own, `backup.sh`'s tars sent whole each time.
2. **Where it runs:** a new `fcp-backup` service with the volumes mounted
   read-only (recommended); the admin itself (its image would mount every
   volume); or a cron script on the host that the admin only reads.
3. **Destinations first:** SFTP and S3-compatible (recommended), then a
   directory, a REST server and rclone; or SFTP alone first.
4. **The master key:** kept out of off-box snapshots by default and
   downloaded once, with a box to include it (recommended); always
   included; or never.
5. **Schedule:** a daily time and keep-daily/weekly/monthly per destination,
   with an optional cron expression (recommended); or cron only.
6. **A full restore from the admin:** yes, with maintenance mode, the
   database swapped by rename, and the Docker socket mounted only when the
   operator turns restores on (recommended); or download and the CLI only.
7. **Partial restores** (files, a tenant's recordings): now, in B3
   (recommended); or later.
8. **Sites:** each runs its own, its status visible in the cloud
   (recommended); or the cloud backs sites up through the link.
9. **Who:** platform administrators only, on new permissions, restores
   audited (recommended). **Agreed instead:** tenant administrators too.
   They see the platform's schedule and last good backup, and restore their
   own files picked by their own rows. Everything else stays with the
   platform.
