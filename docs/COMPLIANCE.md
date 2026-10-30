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
  message's row — by retention, the API, a phone's voicemail menu, or the
  mailbox it belonged to — queues its file in the
  same transaction; the reaper in `fcp-admin` removes it, only from the
  directories it mounts (`call_recordings_dir`, `voicemail_recordings_dir`,
  `voicemail_greetings_dir`, `profile_pictures_dir`, `[retention]
  extra_file_roots`) and only when no row names the path any more.
  Retention never walks a directory, so one tenant's window cannot reach
  another tenant's files. Mount the recording hosts' directories on the
  admin host at the paths they write under.
- **The deletion log** (`GET /api/v1/compliance/deletions?class=&since=&request_id=&limit=`)
  has an entry for every deleted recording and voicemail message, and one
  per tenant per statement for records, traces and audit events: what,
  when, the cause (`retention`, `manual`, `cascade`, `node_sweep`,
  `upkeep`, `erasure`, `tenant_deletion`, or `unattributed` for a delete
  nothing named), the actor (a username, an API token's id,
  `static_token`, `fcp-admin:<node>`, a conference node,
  `phone:<mailbox>`) and, for an erasure or a tenant's deletion, the
  request. It is written by the database in the transaction that deletes.
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

## Legal holds

A legal hold stops the deletion of what it covers until it is released —
by retention, the API, a phone's voicemail menu, a conference node's sweep,
a cascade from a mailbox, or the tenant's own deletion. (Since v0.25.316
a number or line binding no longer takes its mailbox with it: the mailbox
outlives its line, `docs/USERS.md` §4.) Released holds stay, with who released them, when and why.

`POST /api/v1/compliance/holds` places one:

```json
{
  "matter": "Smith v. Example",
  "reason": "litigation",
  "from": 1704067200,
  "to": 1706745600,
  "subjects": [
    { "type": "user", "username": "alice" },
    { "type": "number", "number": "+14155550100" },
    { "type": "aor", "aor": "bob@carrier.example" },
    { "type": "email", "address": "carol@example.com" }
  ],
  "call_ids": ["call-…"],
  "classes": ["cdr", "call_recording", "voicemail"]
}
```

Every part but `matter` narrows the hold, and an empty part covers
everything: no window holds all time, no subjects or calls hold everyone,
no classes hold every class (`cdr`, `call_recording`,
`conference_recording`, `voicemail`, `sip_trace`, `flow_trace`,
`audit_event`). Data is dated by a record's start, a recording's start, a
message's receipt, a trace's capture or an audit event's time, and `to` is
exclusive. An item is covered by the subjects or the calls, either one.

Subjects are matched in a canonical form, so `+1 (415) 555-0100`,
`tel:+1-415-555-0100` and `sip:4155550100@pbx.example` are the same
subject (a number also matches by its last ten digits — a hold errs
towards holding). A user is expanded when the hold is placed to their
username, extension, numbers (their own and their line bindings'),
addresses on the tenant's SIP domains, email and mailbox; a number bound
to them later is not in the hold. Records match by their `from`/`to`;
call recordings and flow traces by their call's record (one whose call has
no stored record yet is held by any hold with subjects); conference
recordings by the people they name; voicemail by the caller and the
mailbox; audit events by the actor.

| Route | |
|---|---|
| `POST /compliance/holds` | Place a hold. `201` with the hold and its expanded `subject_keys` |
| `GET /compliance/holds?all=true` | The tenant's holds; active ones unless `all` |
| `GET /compliance/holds/:id` | One hold, with `held`: how many items of each class it holds now (`null` for a class too large to count in two seconds) |
| `POST /compliance/holds/:id/release` | `{"reason": "…"}`, required. `409` when already released |

All take the `legal_hold_manage` permission (`super_admin`, `tenant_admin`).
Placing and releasing publish `compliance.hold_placed` and
`compliance.hold_released` (`docs/API_EVENTS.md`) and are in the audit log.

What a hold does to each way of deleting:

- **Retention** leaves held items where they are and says so in the run's
  report: `held`, a count per class. The records of a month the hold
  reaches keep that month's partition from being dropped.
- **API deletes** of a held item — a voicemail message, a conference
  recording, a mailbox, number or line binding with a held message beneath
  it, the tenant — answer `409 Conflict` naming the hold. A conference
  recording's hold is asked before its file is touched.
- **A phone's voicemail menu** takes a held message out of the mailbox
  (marked deleted) and keeps it; the purge of deleted messages passes it by.
- **A conference node's sweep** does not see held recordings.
- **The database refuses** any other delete of a held row, however it is
  made: every governed table has a trigger that checks the holds.
- **The CDR archive** keeps writing held days (a rewrite adds records and
  removes none). With `[cdr_archive] object_lock = true` and a bucket that
  has S3 Object Lock enabled, the object of every day a hold on records
  reaches into gets an Object Lock legal hold, and loses it when no hold
  covers the day; the lock covers the whole day whatever the hold's
  subjects.

A hold cannot bring back what was deleted before it was placed; the
deletion log says when that was.

## Recording consent

`recording_consent` on the policy is `none` (default) or `announce`. With
`announce`, every recording the platform makes of a call or a room is
announced first, whoever starts it, and the announcement ends before the
first sample is written (`docs/COMPLIANCE_FOLLOWUPS.md` §8):

| Field | Meaning |
|---|---|
| `recording_consent` | `none` or `announce` |
| `recording_consent_prompt_id` | The tenant's prompt. When its audio is missing, a node with TTS says "This call is being recorded." instead |
| `recording_consent_on_failure` | `record` (default): an announcement that cannot be played does not stop the recording, which says `failed`. `skip`: nothing is recorded, and the record says `refused` |
| `recording_notice_interval_secs` | A 175 ms 1400 Hz beep to the parties every this many seconds while a call or room is recorded (at least 5), for regimes that require a continuing notice. `null`: none |

**What records a call.** All of these go through the same step on the call
manager — the prompt played to both parties and awaited, then the recorder,
its row and the call's record:

- `POST /calls/{id}/recording/start` (and `/stop`) on the call manager node's
  HTTP API, and `POST /api/v1/calls/:id/recording/start` where the API runs
  in the same process as the calls. The body is optional: `{"path": …}`
  places the file under the tenant's directory, otherwise it is
  `<tenant>/<yyyy>/<mm>/<dd>/<call-id>.wav`. The answer names the consent
  outcome; `409` when the call is not anchored, already recorded, or the
  announcement failed under `skip`.
- A flow's `StartRecording` node, which sets `recording_consent` among the
  flow's variables. The node cannot skip the step: a flow that plays its own
  notice is the tenant's reason to set the policy to `none`.
- **Automatic recording**, decided when the call arrives and started once it
  is answered, so both parties are on the line to hear the announcement:
  `record_all_calls` on the calling or the dialled user, `recording_enabled`
  on the dialled or calling number, or `[media] enable_recording = true` on
  the call manager (every call it anchors). A call wanted for several
  reasons is recorded once. A call to be recorded is anchored, and with
  media anchoring forced off (`Never`) it cannot be.
- **Queues** with `record_calls`: the caller and the agent hear the
  announcement together before they are connected, and the recording mixes
  both directions into one track.

The call's record carries `recording_consent` — `announced`, `failed`,
`not_required` or `refused` — when a recording was to start, and nothing
otherwise; so does the recording's row. `recording.started` events name the
source and the outcome, and a refusal publishes `recording.failed` with
`reason = consent_refused`. A node's `/metrics` counts
`fcp_call_recording_consent_total{source, consent}` and
`fcp_call_recordings_started_total{source}`. Call recordings go under
`[media] recording_dir` (default `./recordings`).

**Conference recordings announce too.** With `announce`, a conference
server plays the tenant's prompt to the whole room before a room or a
participant's recording starts, and to each participant who joins while a
room recording runs; `skip` refuses a recording whose announcement failed,
and the notice beeps into the room. The recording's row and
`conference.recording_started` record what happened. Refusing to be
recorded means leaving the room, as it does on a call.

Not announced: voicemail (the greeting is the notice, and the caller chooses
to leave a message), the SBC's local capture and SIPREC (the recorder's
duty), and forge's own recording routes on the WebRTC gateway and media
server, which record operator bridges that belong to no tenant.

## Subject export

A person may ask what a tenant holds about them. The tenant, having checked
who is asking, files an export request; `fcp-admin` gathers everything the
platform holds about that person into one bundle, and the tenant downloads
it (`docs/COMPLIANCE_FOLLOWUPS.md` §6).

| Route | What it does |
|---|---|
| `POST /compliance/subject-requests` | File a request: `{"kind": "export", "subjects": [...], "reference": "DSAR-17"}` (or `"kind": "erasure"`, below). Subjects are `{"type": "user", "username"}`, `{"type": "number", "number"}`, `{"type": "aor", "aor"}` or `{"type": "email", "address"}`; a user the tenant does not have is refused. `201` with the request |
| `GET /compliance/subject-requests` | The tenant's requests, newest first; `?status=received\|running\|preview\|confirmed\|completed\|failed` |
| `GET /compliance/subject-requests/{id}` | One request: its expansion, report, bundle and deadline |
| `GET /compliance/subject-requests/{id}/bundle` | The bundle, `application/gzip`, with `x-content-sha256`. `409` until it is ready, `410` once it has expired. Every download is in the audit log |

The routes take `subject_request_manage`. A request is due
`subject_request_days` after it is received (30 by default, set on the
policy); `fcp_subject_requests_open{kind}` and
`fcp_subject_requests_overdue{kind}` are on `/metrics`, and
`compliance.subject_request_completed` fires when one finishes.

**Who the subject is.** When the request runs, a user is expanded to their
username, extension, addresses on the tenant's SIP domains, numbers (their
own and their lines'), email and mailbox; the expansion is kept on the
request. Data is the subject's when one of those identifiers is in it,
matched narrowly: a number by its digits or its national form, an address
by user and host (so the tenant's `alice` is not `alice@elsewhere.example`),
a name by the same name.

**What the bundle holds.** A gzipped tar with `manifest.json` (the request,
the expansion, counts per class, every file's SHA-256, what is not exported
and why), `data/<class>.jsonl` and `media/<class>/<id>.<ext>`:

- the profile without secrets, API tokens (name and prefix), devices, lines,
  extensions, queue memberships and meetings owned;
- mailboxes, greetings, messages in them and messages the subject left
  elsewhere, with their audio;
- call records naming the subject, the recordings, SIP traces and flow traces
  of those calls, and the conference recordings they were in;
- registrations, presence and its history, subscriptions, authorisation
  policies and resource lists;
- emergency calls, the subject's actions in the admin and configuration audit
  logs, port requests naming them as the contact, webhook deliveries whose
  payload names them, and the allow and block list entries naming them.

A recording or message in which other people are heard is listed under
`review` in the report and the manifest: redact or withhold before handing
the bundle over. Items under a legal hold are exported and counted, with the
hold's matter. A file this host cannot read is listed with why. Call
recordings and voicemail audio are read from the directories `fcp-admin`
mounts, conference recordings through the conference server.

**Operating it.** `[compliance] exports_dir` in `fcp-admin` is where bundles
are written and served from, so every admin replica mounts the same
directory; `export_ttl_days` (7) is how long one can be downloaded, after
which it is removed. The worker runs on one replica at a time under the lock
`fcp:subject-requests`, and puts back in the queue any request a stopped
replica left running.

## Subject erasure

A person may ask to be forgotten. The tenant files an erasure request like
an export (`"kind": "erasure"`); it runs in two steps, because it cannot be
undone and a wrong identifier would erase someone else
(`docs/COMPLIANCE_FOLLOWUPS.md` §6.3–6.6):

1. **The preview.** `fcp-admin` finds everything that is the subject's, as an
   export does, and reports what it would do with each item — without
   changing anything. The request's status becomes `preview`, and
   `compliance.subject_request_completed` fires with that status.
2. **The confirmation.** A person reads the preview and calls
   `POST /compliance/subject-requests/{id}/confirm` (`409` unless a preview is
   waiting). The erasure then runs against the data as it is at that moment
   and reports what it did under `erasure`.

| What | Done |
|---|---|
| The user, their API tokens, profile picture, extensions, queue memberships | Deleted |
| Their mailboxes, greetings, messages in them and messages they left elsewhere, with the audio | Deleted |
| Call and conference recordings of their calls and rooms, SIP and flow traces of their calls | Deleted (conference recording files through their nodes) |
| Registrations, presence and its history, subscriptions, authorisation policies, resource lists | Deleted |
| Webhook deliveries naming them, allow and block list entries naming them | Deleted, entries removed |
| Devices, lines and meetings they owned | Kept, the owner cleared |
| Call records | Pseudonymised: their side's URI keeps its form and host (`sip:erased-3f…@pbx.example`), its display name and the record's `sip_call_id` are cleared, `custom_data` fields naming them are removed. With `cdr_erasure = "delete"` on the policy, deleted |
| Emergency calls, the admin and configuration audit logs | Kept, pseudonymised (actor, and their name in the path) |
| Port requests naming them as the contact | Kept (a carrier record), reported |
| Dead letters in the CDR stream naming them | Deleted, when `fcp-admin` has `[cluster] redis_url` |
| Items under a legal hold | Left as they are, reported with the hold's matter |

**The pseudonym** is `erased-` and twenty hex digits of an HMAC-SHA-256 over
the subject's canonical identifier, keyed by a secret of the tenant's own,
sealed with encryption at rest (`subject_pseudonym_keys`). The same person
gets the same token in every record and in every later request, so counts
and bills still add up, and it cannot be reversed without the key.

**The archive** writes the days of pseudonymised or deleted records again on
its next run; a day left with no records gets an empty object. A bucket
with versioning keeps the old version until its lifecycle rule expires it:
set a non-current-version expiry on the archive bucket.

**What it cannot reach**, and the report says so: database backups and
bucket versions (the operator's retention), dead letters in each node's
spool (removed after `[cdr] dead_letter_days`, 30 by default), records and
events in transit on the Redis streams (trimmed after `fcp-jobs`' `[cdr]
retain_hours`), the API gateway's response cache (its TTL) and service logs.

Every deletion an erasure makes is in the deletion log with cause
`erasure`, the person who confirmed it as the actor, and the request
(`?request_id=`).

## Deleting a tenant

`DELETE /api/v1/tenants/{id}` (platform admins) is a request too: refused
with `409` while any legal hold on the tenant is active; otherwise the
tenant is closed and `202` returns the tenant deletion request, which
`fcp-admin` runs. It removes the tenant's conference recording files through
their nodes and its CDR stream dead letters, notes its archived days to be
written empty, deletes every row of every table that has a tenant — the
traces of its calls kept under `_system` included — and the tenant last.
The request, the deletion log (cause `tenant_deletion`), released legal holds
and the archive's bookkeeping stay. `GET /api/v1/tenants/{id}/deletion`
reports it, including once the tenant is gone: the rows removed, by table.

## Who may do this

The compliance routes take their own permissions, held by `super_admin`
and `tenant_admin` (and the static token, and API tokens that carry them):
`compliance_manage` for the policy and retention runs, `legal_hold_manage`
for holds, `subject_request_manage` for subject requests, `audit_view` for
the audit and deletion logs. Other admin roles,
`read_only` included, are refused.

## In the admin UI

The **Compliance** page (`/admin/compliance`, under Advanced) does all of
this without the API:

- **Policy:** the retention days per class (blank keeps forever),
  `cdr_archive_required` with how far the archive is confirmed, recording
  consent with its prompt, what happens when the announcement fails and
  the notice tone, the days to answer a subject request and what an
  erasure does to call records.
- **Retention runs:** the daily sweep's reports, and a run now.
- **Legal holds:** active (or every) hold with its scope; placing one by
  matter, dates, people, call ids and classes; what one holds now; a
  release with its reason.
- **Subject requests:** filing an export or an erasure for a person named
  every way they are known; a request's progress, report and deadline; an
  export's bundle downloaded (audited) with its checksum and the files to
  review; an erasure's preview and its confirmation, then what it did.

Each tab asks for its own permission (above); a person without it sees the
API's refusal on that tab. The deletion log is on the **Audit Log** page.

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

`GET /api/v1/audit-log?actor=&action=&since=&until=&limit=` lists events,
newest first (`since` and `until` in Unix seconds, `until` exclusive). Tenant
admins see their tenant; the platform token sees everything unless it
selected a tenant with the override header. The older `GET /config/audit`
(configuration-store change history) is unchanged.

`GET /api/v1/audit-log/export` takes the same filters and returns every
event they match, newest first, streamed as it is read: `text/csv` with a
header line (`id,occurred_at,tenant_id,actor,actor_kind,method,path,status,action,source_ip,detail`,
`occurred_at` in RFC 3339, `detail` as JSON), or JSON Lines with
`format=jsonl`, one listing entry per line. It needs `audit_view`, as the
listing does, and is itself audited. The Audit Log page's **CSV** and
**JSON Lines** buttons export what its filters match, not just the page
shown.

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
   `recording_consent` to `announce` with its id; set
   `recording_consent_on_failure = skip` if a recording without the
   announcement must not exist, and `recording_notice_interval_secs` if
   the notice must continue.
3. If records must outlive the table, have the operator enable the CDR
   archive, then set `cdr_archive_required` so retention waits for it.
4. Run `POST /compliance/retention/run` (or **Run now** on the Compliance
   page) once and read the report.
5. Give your compliance officer `GET /audit-log`, `GET /audit-log/export` and `GET /cdrs/export`, and
   `subject_request_manage` for the requests people make; set
   `subject_request_days` to your regime's deadline, and `cdr_erasure` to
   `delete` if your records need not outlive an erasure.
6. For conferences, decide `recording_format` on each node (`auto` records
   video meetings to MP4 where the node encodes H.264 and to WebM where it
   encodes VP8 or VP9, audio-only ones to WAV) and let
   `recording_retention_days` do the rest.
