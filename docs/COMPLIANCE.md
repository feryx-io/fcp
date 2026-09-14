# Compliance Basics

Three things every regulated deployment needs and FCP now provides per
tenant: a **retention policy** that deletes call data once it is past its
window, a **recording-consent** setting, and an **admin audit log** of who
changed what. Plus a **CDR export** for the records that must leave the
platform.

## Retention policy

`GET/PUT /api/v1/compliance/policy` holds one policy per tenant. Every
window is a number of days or `null` (keep forever, the default):

| Field | What it deletes |
|---|---|
| `cdr_retention_days` | Call detail records that started before the window |
| `sip_trace_retention_days` | Raw SIP messages captured for the tenant's calls |
| `flow_trace_retention_days` | Flow execution traces of the tenant's calls |
| `recording_retention_days` | **Call recordings** by their rows (the file is removed after, and the record stops naming it) and **conference recordings**, which their own node deletes (see below) |
| `voicemail_retention_days` | Voicemail messages, with their audio. A mailbox's own `retention_days`, when set, applies too |
| `audit_retention_days` | Admin audit events |

Windows are applied independently and in an order that keeps referential
sense: flow traces, SIP traces, and recordings go before the CDRs they hang
off; audit events last.

### Records the archive must hold first

`cdr_archive_required: true` makes CDR retention wait for the CDR archive
(`docs/CDR_PIPELINE.md` §13, phase 5): a record is deleted only once its
tenant's day is written to the archive and confirmed there, so retention
never removes the only copy. The policy reports how far that is as
`cdr_archived_before` (Unix seconds, `null` while nothing is archived):
every one of the tenant's records that started before it is in the
archive. A run that the archive holds back deletes up to that point and
says so in its report (`cdrs: kept from … until the archive confirms
them`); with no archiver running, the tenant's records are kept. The
archive itself is written by `fcp-jobs` with `[cdr_archive] enabled`;
turn it on before requiring it.

### When it runs

- **Hourly** in `fcp-admin`, for every tenant that has at least one window
  set, then the platform's upkeep (below). With several admin replicas one
  sweeps at a time (a database advisory lock, `fcp:retention`). CDRs are
  deleted in batches of 10,000; when every tenant has a CDR window, a
  monthly CDR partition entirely older than the shortest of them is then
  dropped — once it is empty, never with rows left in it — which returns
  its space at once (`docs/CDR_PIPELINE.md` §8). For a tenant that
  requires the archive, the shortest window is its archive frontier when
  that is earlier.
- **On demand** with `POST /api/v1/compliance/retention/run`, which
  returns the report for that run.

Every run is recorded; `GET /api/v1/compliance/retention/runs` lists them
with per-category counts and any non-fatal errors (a file that could not be
removed, a store that is not configured). A run never stops at the first
problem.

### Every recording is a row, every deletion a log entry

Since the compliance foundations (`docs/COMPLIANCE_FOLLOWUPS.md` §4, §7):

- **Recordings are registered before they are kept.** A call recorded
  through `POST /calls/:id/recording/start`, a flow's `StartRecording` node
  or a queue with `record_calls` gets a `call_recordings` row (tenant,
  call, source, path, consent, start and end); a room or per-participant
  conference recording gets a `conference_recordings` row naming everyone
  heard in it. A recording whose row cannot be written is stopped and its
  file removed. New files go under the tenant and the UTC date
  (`<tenant>/<yyyy>/<mm>/<dd>/…`), named for the call or the recording,
  never for a caller or a queue; the API route's `path` is placed under the
  tenant's directory and may not climb out of it.
- **Rows are deleted, files follow.** Deleting a recording's or a voicemail
  message's row — by retention, the API, a phone's voicemail menu, or a
  number, line binding or mailbox it belonged to — queues its file in the
  same transaction; the reaper in `fcp-admin` removes it, only from the
  directories it mounts (`call_recordings_dir`, `voicemail_recordings_dir`,
  `voicemail_greetings_dir`, `profile_pictures_dir`, `[retention]
  extra_file_roots`) and only when no row names the path any more.
  Retention never walks a directory, so one tenant's window cannot reach
  another tenant's files. Mount the recording hosts' directories on the
  admin host at the paths they write under.
- **The deletion log** (`GET /api/v1/compliance/deletions?class=&since=&limit=`)
  has an entry for every deleted recording and voicemail message, and one
  per tenant per statement for records, traces and audit events: what,
  when, the cause (`retention`, `manual`, `cascade`, `node_sweep`,
  `upkeep`, or `unattributed` for a delete nothing named) and the actor (a
  username, an API token's id, `static_token`, `fcp-admin:<node>`, a
  conference node, `phone:<mailbox>`). It is written by the database in
  the transaction that deletes.
- **Platform upkeep**, hourly after the tenants' runs: SIP and flow traces
  are given the tenant of their call's stored record; each mailbox's own
  `retention_days` is applied; messages users deleted are purged after
  `[retention] deleted_voicemail_grace_days` (7); SIP traces that belong to
  no tenant's call are removed after `[retention] system_trace_days`, and
  files under the mounted directories that no row names after
  `[retention] orphan_files_days` — both off unless set.

### Conference recordings

A conference recording is a row in the registry and a file on one node's
disk (`docs/VIDEO_CONFERENCING.md` §11), so `recording_retention_days`
is applied where the file is:

- **The holding node sweeps it**, on `conference.recording_cleanup_interval_secs`.
  For each tenant it holds recordings for, it takes that tenant's
  `recording_retention_days` — falling back to the node's own
  `conference.recording_retention_days` when the tenant has set none —
  and deletes file and row together for anything older. Files with no row
  (recordings from before the registry, or from a node with no database
  behind it) are pruned by age under the node's own setting alone; a file
  the registry knows is always left to its row's policy.
- **The hourly sweep is the backstop.** A row still there a week past its
  window belonged to a node that is gone, so `run_retention` removes the
  row and counts it as `conference_recordings_deleted`. Until then it
  leaves conference recordings alone, so the node that can delete the file
  gets to do both.

`DELETE /api/v1/recordings/{id}` deletes one on demand: the file goes
first, through the node holding it, then the row. The response's
`file_deleted` is `false` when that node could not be reached — the row is
gone and the file waits for the node's own sweep.

## Recording consent

`recording_consent` on the policy is `none` (default) or `announce`. With
`announce` and a `recording_consent_prompt_id`, starting a recording
through `POST /api/v1/calls/:id/recording/start` first plays that prompt to
both parties; the response reports `consent_announced`. The prompt must
belong to the tenant and have audio.

**Conference recordings announce too.** With `announce`, a conference
server plays the tenant's prompt to the whole room before the first sample
is written, and to each participant who joins while the recording runs.
The recording's row records what happened: `announced`, `failed` (the
prompt could not be played and the recording went ahead anyway) or
`not_required`. Refusing to be recorded means leaving the room, as it does
on a call.

Not yet: calls the B2BUA records automatically (`media.enable_recording`,
per-user `record_all_calls`, queue `record_calls`) do not play the prompt
— announce consent from the IVR or flow that answers them until the
B2BUA-level announcement lands.

## Who may do this

The compliance routes take their own permissions, held by `super_admin`
and `tenant_admin` (and the static token, and API tokens that carry them):
`compliance_manage` for the policy and retention runs, `audit_view` for the
audit and deletion logs. Other admin roles, `read_only` included, are
refused.

## Admin audit log

A middleware inside the API's auth layer records every request that is not
a `GET`/`HEAD`/`OPTIONS`, plus every CDR export, as an audit event:

| Field | Meaning |
|---|---|
| `actor`, `actor_kind` | The username of a JWT session, an API token's id (`token-…`), `static_token`, or `anonymous` (pre-auth routes such as login) |
| `tenant_id` | The tenant the request acted on: the caller's own, the `x-fcp-tenant-id` override, or the default tenant for the platform token |
| `action` | Method plus route template, e.g. `PUT /api/v1/users/:username` — stable across ids, use it to filter |
| `path`, `method`, `status` | The concrete request and how it ended (a 401 on `/auth/login` is a failed login) |
| `source_ip` | From `X-Forwarded-For` / `X-Real-IP` when a proxy sets them |
| `detail` | For a `DELETE`, the route's parameters: `{"params": {"id": "…"}}` |

Writes are asynchronous and best-effort; the response is never delayed or
failed by the audit log.

`GET /api/v1/audit-log?actor=&action=&since=&limit=` lists events. Tenant
admins see their tenant; the platform token sees everything unless it
selected a tenant with the override header. The older `GET /config/audit`
(configuration-store change history) is unchanged.

## CDR export

`GET /api/v1/cdrs/export` returns `text/csv` with the same filters as
`GET /cdrs` (`call_id`, `from_uri`, `to_uri`, `disposition`, `kind`,
`start_time_from`, `start_time_to`, and `tenant_id` for platform admins),
every matching record, streamed as it is read rather than built in memory
(`docs/CDR_PIPELINE.md` §9). `format=jsonl` returns JSON Lines instead,
one record a line in the listing's shape. Times are Unix seconds. Every
export is itself an audit event.

For records that must be kept beyond the table, the CDR archive writes
each tenant's day to an S3-compatible bucket or a directory as gzipped
JSON Lines, confirmed by size and SHA-256, and announces each object as
`cdr.archived` (`docs/CDR_PIPELINE.md` §13, phase 5).

## Conference recordings API

`/api/v1/recordings` is the registry, scoped to the caller's tenant:

| Route | What it does |
|---|---|
| `GET /recordings` | Newest first. Filter by `room_number`, `meeting_id`, `since`/`until` (Unix seconds), `running`; page with `limit`/`offset` |
| `GET /recordings/{id}` | One recording |
| `GET /recordings/{id}/download` | The file, streamed from whichever node holds it, as `audio/wav`, `video/mp4` or `video/webm` |
| `DELETE /recordings/{id}` | File first, then row. 409 while it is still being written |

Downloads and deletes go through the configured conference server
(`[conference] server_url`), which serves the file itself or fetches it
from the holding node over `/internal` — so one URL works whichever node
recorded. The admin UI's **Recordings** page lists them with a player.

## Checklist for a tenant

1. Decide the windows your regime requires (many US rules: CDRs 18 months
   or more, recordings per industry) and set them on the policy.
2. If you must announce recording, upload a prompt and set
   `recording_consent` to `announce` with its id.
3. If records must outlive the table, have the operator enable the CDR
   archive, then set `cdr_archive_required` so retention waits for it.
4. Run `POST /compliance/retention/run` once and read the report.
5. Give your compliance officer `GET /audit-log` and `GET /cdrs/export`.
6. For conferences, decide `recording_format` on each node (`auto` records
   video meetings to MP4 where the node encodes H.264 and to WebM where it
   encodes VP8 or VP9, audio-only ones to WAV) and let
   `recording_retention_days` do the rest.
