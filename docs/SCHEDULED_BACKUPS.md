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

**Restores are switched on by an override file** (agreed 2026-11-27,
replacing the `FCP_BACKUP_RESTORE` variable):
- Normally `fcp-backup` mounts every volume read-only and has no Docker
  socket.
- `docker compose -f docker-compose.yml -f docker-compose.restore.yml up -d
  fcp-backup` turns restores on. The volumes become writable, for file
  restores and a full restore, and the Docker socket is mounted, for a
  full restore.
- The service says which it has, and the page says when restores are off
  and how to turn them on.

**What a snapshot of the files holds.**
- The certs volume is backed up as every other volume is, CAs included,
  but without `master.key` unless the destination includes the key.
- Paths are named by their compose volume, so a download can be put back
  in `backup.sh`'s layout.

**A download, in `backup.sh`'s layout** (agreed 2026-11-27): `fcp.dump`, a
`<volume>.tar.gz` of each volume's root, a `manifest` with the release and
the KEK id, and `SHA256SUMS`, so `restore.sh` restores it on any new host.
- `fcp-certs.tar.gz` gets its `master.key` from the snapshot when the
  snapshot holds it.
- Otherwise it is the running system's own key, when its KEK is the one
  the snapshot names.
- Otherwise the page asks for the key file downloaded in B2, with its
  passphrase.
- `fcp-backup` builds the download in its work directory and streams it
  to the admin, which streams it on.

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
- `GET …/destinations/{id}/snapshots`: what is stored, one point per run
  (its database and files snapshots, release, KEK and volumes).
- `POST …/destinations/{id}/snapshots/{run}/download`: the point made a
  `backup.sh`-layout tar, with `{key_file, passphrase}` when the key must
  come from the file. It answers a download id; `GET
  /api/v1/system/backups/downloads/{id}` follows it, and when ready names
  a link, `/backups/downloads/{id}/{token}`, that streams the tar once.
- `POST …/snapshots/{snapshot}/restore`: `{scope: "system" | "files",
  paths?, key_file?}`. It answers a restore id, and `GET
  /api/v1/system/backups/restores/{id}` reports its steps.
- `GET /api/v1/system/backups/capabilities`: whether restores are on.
- `POST …/snapshots/{run}/restore-system`: `{confirm, key_file?,
  passphrase?}`, the whole system, followed at `GET
  /api/v1/system/backups/restores/current`.
- `POST …/snapshots/{run}/restore-files`: `{volume, paths?}`, put back
  beside what is there.
- `GET /api/v1/system/backups/key`: the master key, wrapped in a
  passphrase given in the request. This is audited.
- For a tenant administrator:
  - `GET /api/v1/backups`: the schedule and the last good backup.
  - `GET /api/v1/backups/missing`: the tenant's recordings, voicemail and
    prompts whose files are gone, with the snapshots that hold them.
  - `POST /api/v1/backups/restore-files`: `{point, items}` (a `held_in`
    point; items by class and row id), put back beside what is there.

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

  **Done (v0.25.670):**
  - **Core and storage:** `fcp_core::backup` holds destinations (SFTP or
    S3-compatible, a daily `time_of_day` in UTC, keep daily, weekly and
    monthly, `include_key`), runs (backup or test; requested, running,
    succeeded or failed) and `daily_due`. Migration 161 adds
    `backup_destinations` and `backup_runs`; the password, the SFTP
    private key and the S3 secret are sealed (`SEALED_COLUMNS`), and a run
    is claimed `FOR UPDATE SKIP LOCKED`.
  - **The service:** the `fcp-backup` binary and crate, with restic 0.17.3
    pinned by checksum in `Dockerfile.backup` on `postgres:16`, so its
    `pg_dump` matches the stack's server. It polls for runs and for each
    destination's due schedule.
    - A test pins an SFTP server's host key (`ssh-keyscan`; required
      after, `StrictHostKeyChecking=yes`) and makes the repository.
    - A backup streams `pg_dump --format=custom` into one snapshot
      (`/fcp.dump`, tagged `part:db`) and reads `[backup.paths]` in place
      into another (`part:files`). Both are tagged `fcp`, `run:<id>` and
      `release:<v>`. Then `forget --prune` keeps each part's daily, weekly
      and monthly.
    - Secrets reach restic only through its environment
      (`RESTIC_PASSWORD`, `AWS_*`). An SFTP key is a 0600 file in
      `work_dir`, used through `sftp.command`.
    - Each run's outcome is kept, and `system.backup_completed` or
      `system.backup_failed` goes to the events server.
  - **The API:** `/system/backups/destinations` (list, create, update,
    delete, `…/test`, `…/run`) and `/system/backups/runs[/{id}]`, for
    platform administrators. Secrets are write-only; a generated password
    is shown once.
  - **Compose:** the `fcp-backup` service mounts every stateful volume
    read-only under `/data` and keeps its cache and keys in
    `backup-data`.
  - **Proved by:** `fcp-backup/tests/backup.rs`, against an `sshd` of the
    test's own (refused before the host key is pinned; tested; backed up;
    the dump read back as a `pg_dump` archive and the recording listed,
    with restic as an operator runs it) and Garage
    (`FCP_TEST_S3_*`); and `backup_destinations_are_kept_and_runs_asked_for`.
  - **Left to later phases:** the cron expression (B4) and the
    `restore.sh`-layout download (B3).
- **B2: the page,** with the key download, and a Playwright spec.

  **Done (v0.25.671):**
  - **The key download:**
    - `GET /system/backups/key` says whether the master key can be
      downloaded (a local key can; a KMS key never leaves the KMS) and when
      its KEK last was.
    - `POST` with a passphrase of 12 or more characters answers an
      ASCII-armored age file, `fcp-master-key-<kek>.age`, wrapped in that
      passphrase (age's scrypt recipient, so `age -d` opens it without
      FCP). It holds `{format, kek_id, master_key}`, the key as
      `FCP_MASTER_KEY` takes it.
    - The download is recorded in `backup_key_exports` (migration 162; the
      key itself is kept nowhere new) and logged at WARN.
    - `ApiState.encryption` carries the settings, read from the
      environment when unset; `EncryptionSettings::exportable_master_key`.
  - **System › Backups** (`BackupsPage`):
    - The key card, amber until the current KEK has been downloaded.
    - Destinations, with their repository, schedule, retention and last
      backup, and Test, Back up now, Edit and Delete. The add and edit
      dialog covers SFTP and S3; secrets are kept when left empty, and
      "include the master key" is spelled out.
    - The password made for a destination, shown once.
    - The runs table, followed while a run waits or runs.
  - **Proved by:** `the_master_key_is_downloaded_wrapped_in_a_passphrase`
    (the file opened with the passphrase gives the key back) and
    `e2e/backups.spec.ts`.
- **B3: restoring from the admin,** in three parts:
  - **B3a:** snapshots listed, and a download in `backup.sh`'s layout,
    through `fcp-backup`'s own API.

    **Done (v0.25.672):**
    - **What a backup holds now:**
      - The dump is taken `--no-owner --no-privileges`, as `backup.sh`
        takes it.
      - Both snapshots are tagged `kek:<id>`, the key ring's active KEK
        (`BackupStore::active_kek_id`).
      - `[backup.paths]` is keyed by compose volume name, and the files
        snapshot tags each volume `vol:<name>:<path>`. The certs volume is
        among them, with `master.key` excluded unless the destination
        includes the key.
    - **`fcp-backup`'s API,** under `[http] token`
      (`FCP_BACKUP_API_TOKEN`; unset, it answers nothing):
      - `GET /v1/destinations/{id}/snapshots` groups restic's snapshots
        into points by run.
      - `POST /v1/downloads` builds one in the background in `work_dir`.
        It dumps `/fcp.dump`, restores the files and tars each volume from
        its root as `<volume>.tar.gz`. It writes the `manifest`
        (`fcp_backup = 1`, `taken_at`, `stack_version`, `kek_id`,
        `volumes`) and `SHA256SUMS`, then tars the directory.
      - `GET /v1/downloads/{id}` follows the build. `…/file?token=` streams
        the tar once and removes it; an unfetched download goes after an
        hour.
      - **Where `master.key` comes from:** the snapshot if it holds the
        key. Otherwise the key given (from the key file), or else this
        system's own when its KEK is the snapshot's. Otherwise the build
        fails with "needs the master key".
    - **The admin:**
      - `[backup] url` and `token` (`FCP_ADMIN_BACKUP_TOKEN`), as
        `ApiState.backup_service`.
      - The three routes above, platform only. A key file is opened with
        its passphrase in the admin, so only the key travels on; a wrong
        passphrase is a 400.
      - `/backups/downloads/{id}/{token}` proxies the stream. The link is
        the credential, because a browser saving a file sends no bearer
        token.
    - **The page:** Snapshots per destination. Choose a point, give the
      key file if needed, and Make the download; a link appears that works
      once.
    - **Compose:** the token in `.env`, the paths by volume name, and the
      certs volume added.
    - **Proved by:**
      - `fcp-backup/tests/backup.rs` against SFTP: the certs snapshot
        without its key; the point listed; a download refused without a
        key, then built with one. Its checksums pass, the manifest reads
        as `restore.sh` reads it, the dump is a `pg_dump` archive, the
        recording and the key are in their tars, and a second fetch is a
        404.
      - `snapshots_are_listed_and_downloaded_through_the_backup_service`,
        against a stand-in service.
      - `e2e/backups.spec.ts`.
  - **B3b:** the override file, and file restores (a platform
    administrator's and a tenant's own), never overwriting.

    **Done (v0.25.673):**
    - **Switching restores on:**
      - `deploy/docker-compose/docker-compose.restore.yml` mounts
        `fcp-backup`'s volumes writable and adds the Docker socket.
      - `GET /v1/capabilities` reports what the service may do: it probes
        each volume with a file it removes again, and looks for
        `[backup] docker_socket`.
      - `GET /system/backups/capabilities` passes that on, and the page's
        Restores card says how to switch restores on.
    - **Paths:**
      - `[backup.service_paths]` maps each path a service writes into a
        row to where this service mounts it (`/app/data/recordings` to
        `/data/call-manager/recordings`).
      - The longest prefix wins, by path components. A path that climbs
        out, is relative, or ends up outside a backed-up volume is held
        by nothing.
    - **`fcp-backup`:**
      - `POST /v1/files/check` says whether each path is held here and
        present.
      - `POST /v1/files/find` finds the snapshots that hold each path
        (`restic find`), newest first, with their times.
      - `POST /v1/restores/files` puts back service paths, or a volume and
        paths in it (none is the whole volume), with `restic restore
        --target / --overwrite never`. It is refused (409) while restores
        are off.
    - **A platform administrator:** `POST
      /system/backups/destinations/{id}/snapshots/{run}/restore-files`
      with `{volume}` puts back the files the volume is missing. The
      Snapshots dialog has a button per volume; files removed on purpose
      come back too, and the dialog says so.
    - **A tenant administrator** (`backup_read` for the tenant
      administrator and read-only monitor; `backup_restore_own` for the
      tenant administrator alone):
      - `GET /backups` gives the daily times, the last good backup and
        whether restores are available, and no destination.
      - `GET /backups/missing` lists the tenant's rows whose files are gone:
        call and conference recordings, voicemail and greetings, and
        prompts (`BackupStore::tenant_files`; a site's own rows are the
        site's). Each comes with the backups holding it.
      - `POST /backups/restore-files` takes `{point, items}`. Each item is
        found again among the tenant's own rows by class and id, so another
        tenant's file cannot be named (404).
      - The page shows tenant administrators the schedule and their lost
        files, with Restore and Restore all.
    - **Proved by:**
      - `fcp-backup/tests/backup.rs`: a recording deleted, found missing,
        found in its snapshot, put back by its service path, and then by
        its volume, while a file changed since is not overwritten; a path
        outside every volume is refused.
      - `lost_files_are_put_back_and_a_tenant_reaches_only_its_own`: two
        tenants, the platform's volume restore, the tenant's own and
        another's refused, a read-only monitor refused.
      - `e2e/backups.spec.ts`, with a second test for a tenant
        administrator.
    - **Found:** in compose, the admin keeps uploaded prompts in
      `/tmp/fcp/audio/prompts` (no `prompts_dir` is set), which no volume
      holds, so those files are neither backed up nor restorable. Left for
      a fix of its own.
  - **B3c:** the full restore.

    **Done (v0.25.674):**
    - **Maintenance is the drain.** Each service is stopped with SIGTERM,
      which drains every routing binary: new calls refused, `/ready`
      failing, calls up let finish for up to `[restore] drain_secs`
      (default 60). No mode needs adding to every binary.
    - **`fcp-backup`'s `POST /v1/restores/system`:**
      `{destination_id, run, confirm, master_key?}`, with the point's time
      typed back as `confirm`. Everything is checked before anything
      stops:
      - restores are on (writable volumes, and the Docker socket or
        `[restore] stop_commands`);
      - no other restore is running;
      - the key: the snapshot's own `master.key` if it holds one (`restic
        ls`), else the key given, else this system's own when its KEK is
        the point's. Otherwise it is refused with "needs the master key".
    - **The job,** in the background (`restore_system.rs`), followed at
      `GET /v1/restores/system/current` and kept in
      `work_dir/restore.json`, so it can be read after the admin is back:
      1. **check.**
      2. **stop.** Through the Docker socket, the compose project's
         containers (this one's own by its labels, or
         `[restore] compose_project`) other than `keep_running`
         (`postgres`, `redis`, `fcp-backup`, `certs`), stopped all at once
         with `t=drain_secs`. Otherwise the operator's `stop_commands`.
      3. **database.** `restic dump | pg_restore --no-owner
         --no-privileges --exit-on-error` into `<db>_restore_<stamp>`.
         Then the swap: the old database closed to new connections, its
         backends ended, renamed `<db>_before_<stamp>`, and the restored
         one renamed in. A failure up to this point drops the new
         database, starts the services and reports that nothing was
         changed.
      4. **files.** Each volume as the point holds it: `restic restore
         --overwrite always --delete`. The certs volume is restored
         without `--delete`, so a key the snapshot left out stays, and a
         key given is written to the key ring.
      5. **start.** The containers stopped in step 2, or the
         `start_commands`. They migrate forward.
      - The old database is kept, for going back by hand.
    - **The admin:**
      - `POST /system/backups/destinations/{id}/snapshots/{run}/restore-system`
        takes `{confirm, key_file?, passphrase?}`; the key file is opened
        here.
      - `GET /system/backups/restores/current` follows the job. Ids are
        `bks-`.
      - The Snapshots dialog's "Restore the whole system" says what it
        replaces and needs the time typed back.
      - The page's restore card follows the steps, and says the admin is
        restarting while it is away.
    - **Compose:** `[restore]` in `configs/fcp-backup.toml`. The Docker
      socket comes from `docker-compose.restore.yml`.
    - **Proved by:**
      - `fcp-backup/tests/backup.rs` (SFTP, real Postgres): after the
        backup, a table is made, a file added and another changed. A wrong
        time is refused, and so is a restore with no key to give. Then the
        restore runs:
        - the stop and start commands are each run once;
        - the table is gone and the point's runs are there;
        - the added file is gone and the changed one is as it was;
        - the CAs are kept and the given key is written;
        - the service goes on against the restored database.
      - `the_compose_services_are_stopped_and_started_through_the_socket`
        (`FCP_DOCKER_TESTS`, in CI): a compose project of two containers,
        the kept one left running and the other stopped and started again
        through the socket.
      - The admin's routes in
        `lost_files_are_put_back_and_a_tenant_reaches_only_its_own`, and
        `e2e/backups.spec.ts`.
    - **Not proved:** the stack restored end to end in containers (a
      populated compose stack, a destination it can reach, a call placed
      after). That belongs with the nightly stack test, which runs the
      images.
    - **Needs:** the database user may create, rename and drop databases
      (compose's `fcp` is the superuser).

  What each part covers:
  - Download in `backup.sh` layout; partial restores, the tenant's own
    included.
  - Maintenance mode on every service; the full restore, through the
    Docker socket or the operator's commands, with the database swapped by
    rename.
  - Proved by restoring a populated stack and placing a call.
- **B4: more destinations and the watch.** A directory, a REST server and
  rclone; the weekly check; overdue; metrics.

  **Done (v0.25.675):**
  - **Destinations** (`BackupTarget::{Directory, Rest, Rclone}`):
    - a directory mounted into `fcp-backup` (absolute, no `..`);
    - a REST server (`rest:<url>`; the user and password reach restic as
      `RESTIC_REST_USERNAME`/`RESTIC_REST_PASSWORD`, and a URL carrying
      credentials is refused);
    - an rclone remote (`rclone:<remote>`; its `rclone.conf` text is
      written 0600 to `work_dir/rclone/<id>.conf` for `RCLONE_CONFIG`, and
      `rclone.program` is `[backup] rclone`).
  - **Secrets:** the REST password or the rclone configuration is sealed
    in `target_secret` (migration 163, `SEALED_COLUMNS`). Like every other
    secret, it is write-only and kept when an edit leaves it out.
  - **The check:**
    - A `check` run runs `restic check --read-data-subset=5%`.
    - A scheduled destination with a good backup gets one each week
      (`CHECK_EVERY`), and there is a Check button beside Test and Back up
      now (`POST …/destinations/{id}/check`).
    - A failed check is a `system.backup_failed` with `kind = "check"`.
  - **Overdue:** a scheduled, enabled destination with no good backup for
    26 hours, counted from when it was made if it never had one
    (`fcp_core::backup::overdue`).
    - The service notices it once per lapse (`backup_overdue_notices`,
      keyed by the last success) and sends `system.backup_overdue`.
    - Destinations show `overdue`, `last_success_at` and `last_check`, and
      the page marks an overdue one in red.
  - **Metrics** on the admin's `/metrics` (`BackupMetrics`):
    `fcp_backup_last_success_timestamp`, `fcp_backup_bytes_added` (the
    last good backup's) and `fcp_backup_overdue`, per destination with its
    name and type.
  - **The image:** rclone 1.68.2 pinned by checksum. CI installs restic
    0.17.3 (apt's 0.16 lacks `restore --overwrite`/`--delete` and the REST
    credentials), `rest-server` 0.13.0 and rclone.
  - **Proved by:**
    - `a_directory_a_rest_server_and_an_rclone_remote_take_backups`: each
      tested and backed up; a REST server with the wrong password refused;
      the directory's check run; the rclone configuration private.
    - `more_destinations_a_check_and_the_overdue_watch`: bad targets
      refused, secrets write-only and kept, Check now, overdue, metrics.
    - `a_destination_is_overdue_a_day_and_two_hours_on`.
    - `e2e/backups.spec.ts`: a directory made and checked.
  - **Left:** the cron expression (`schedule.cron` is still read by
    nothing).
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
