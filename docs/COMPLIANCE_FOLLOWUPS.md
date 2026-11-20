# Compliance Follow-ups — Design

Status: **all five phases done** (2026-09-15, v0.25.166, v0.25.170, v0.25.171, v0.25.172 and v0.25.173); design agreed the
same day with every recommendation in §12 taken. This document is the
plan for the three compliance items `STATUS.md` left unscheduled when the
basics shipped on 2026-09-02: **legal hold**, **per-subject export and
erasure**, and the **recording-consent announcement on calls recorded
without an API request**. `docs/COMPLIANCE.md` describes what exists;
this document describes what comes next and the decisions behind it.
`STATUS.md` tracks progress.

Read with `docs/COMPLIANCE.md` (the retention policy, consent, the audit
log, the CDR export) and `docs/CDR_PIPELINE.md` (the record table, its
partitions and the archive, which a hold and an erasure both reach).

## 1. Decisions

Proposed 2026-09-14, after surveying every recording path, every
deletion path and every store of personal data (§2). Each is a
recommendation until agreed; the questions that need a ruling are in §12.

1. **Nothing can be governed that is not registered.** A hold, an
   erasure and a retention window all work from rows that say which
   tenant a piece of data belongs to. A file with no row is found only by
   walking a directory by age, which is how one tenant's recording window
   deletes another tenant's recordings today (§2). Every recording gets a
   row before it gets a hold, and the directory walk stops deciding on
   tenants' behalf.
2. **A hold is a filter every deletion consults, not a flag on data.**
   Holds live in one table; retention, the manual deletes, the node
   sweeps, the cascades and erasure all ask the same question — *is this
   item covered by an active hold?* — through one function in
   `fcp-core` and one SQL predicate in the Postgres backend. Nothing is
   copied aside, and releasing a hold changes no data.
3. **A hold is never deleted.** It is released, with who, when and why,
   and both events are in the audit log. Matters get reopened.
4. **Deletions are audited as data, not as requests.** The audit log
   records that `DELETE /recordings/:id` returned 204; it does not record
   what was deleted, and automatic deletions are not in it at all. A
   deletion log records what each deletion removed — per item for
   recordings, messages, erasures and holds released; per batch for the
   record table's retention.
5. **Consent is a step of starting a recording, not of each caller.**
   One `start_call_recording` in the B2BUA resolves the call's media
   session, applies the tenant's consent policy, waits for the prompt to
   finish, starts the recorder and records the outcome on the call and its
   CDR. The REST route, the flow node, per-user and per-number automatic
   recording and the queue all go through it or its queue equivalent.
6. **Automatic call recording becomes real.** `record_all_calls` on a
   user, `recording_enabled` on a number and `media.enable_recording`
   are stored, shown in the admin UI and described in `COMPLIANCE.md`,
   and today start nothing (§2). The consent item cannot be done for
   recordings that do not happen; this design makes them happen.
7. **A subject request is a record with a deadline, run by a worker.**
   Export and erasure are asynchronous: a request row (who asked, for
   whom, which identifiers, the legal clock), a job that resolves the
   identifiers to the subject's data, and a report. An export is a bundle
   the requester downloads once; an erasure is a report of what was
   removed, what was pseudonymised and what was kept and why.
8. **Records a tenant must keep are pseudonymised, not deleted.** A call
   detail record is a billing and often a regulatory record. Erasing a
   subject replaces their identifiers in the records with a stable
   per-tenant token and deletes what has no such obligation — recordings,
   voicemail, traces, profile data. The tenant's policy can choose
   deletion instead (§12).
9. **The archive follows the table.** An erasure rewrites the archived
   days it touched; a hold on archived data asks the bucket to lock the
   objects where the store supports it (S3 Object Lock legal hold).

## 2. What is wrong today

The survey found the three items bigger than their one-line entries, and
found defects in the basics they build on.

**Recording and consent.**

- Eight things start a recording: the REST route, the flow
  `StartRecording` node, queue `record_calls`, room recording on the
  conference server (automatic or by API), per-participant recording on
  the conference server, voicemail, the SBC's local capture, and forge's
  own recording routes on the WebRTC gateway and media server. The
  consent policy is applied on two: the REST route and room recording.
- `record_all_calls` (per user), `recording_enabled` (per number) and
  `media.enable_recording` only anchor media; nothing starts a recorder.
  `COMPLIANCE.md` calls them automatic recording.
- The REST route looks the media session up under the call's own id,
  but the B2BUA creates sessions under a fresh id kept in
  `media_session_id`. On an ordinarily anchored call both the consent
  prompt and the recording fail to find the session. No test calls the
  route.
- Where the REST route does play the prompt, it starts the recorder as
  soon as playback starts, not when it ends, and records the call's
  consent outcome nowhere (the conference path stores it on the
  recording's row).
- The queue writes both directions of the call one after the other into
  one mono file, not mixed, and plays no prompt.
- Per-participant conference recordings get no row, no event and no
  consent, so retention, holds and erasure cannot see them.
- There is no pause and resume (card numbers read over a recorded call),
  no per-party consent, no periodic tone and no DTMF opt-out.

**Deletion.**

- Retention's recordings-directory prune deletes every file under
  `call_recordings_dir` older than *the running tenant's* window,
  whatever tenant it belongs to. With two tenants and two windows, the
  shorter window wins for both.
- Deleting a tenant deletes one row: nothing references `tenants`, so
  every record, recording and trace is left behind, unreachable by any
  sweep (the sweep walks the tenant list).
- Deleting a voicemail message — through the API, self-service or the
  admin UI — removes the row and leaves the audio file; deleting a
  number or a line binding cascades through mailboxes to message rows
  and leaves every file. The mailbox's `retention_days` is stored and
  editable and nothing applies it; a message marked deleted is never
  purged.
- Flow-trace retention finds traces through CDRs that still exist, so a
  trace outliving its CDR is never deleted; SIP traces captured before a
  tenant is known are stored under `_system` and no sweep reaches them.
- The audit log's `detail` is never filled, API tokens are recorded
  without their id, automatic deletions and the conference, voicemail
  and TTS servers' deletes are not audited, and `retention_runs` records
  counts without saying what was deleted or whether a person asked.
- The compliance routes check only `require_admin`, which is true for
  every session role but `end_user` — a `read_only` session can change
  the retention policy or run it.

**Subject data.**

- Nothing finds data by subject. The only subject lookups are CDR
  `from_uri`/`to_uri` substring filters (with no index since migration
  059), registrations and presence by AOR, and users by username; there
  is no shared canonical form, and the same person appears as
  `sip:1001@pbx.example`, `1001`, `+14155550100`, `tel:+1-415-555-0100`
  and a display name. Four copies of an AOR normaliser and three
  digit-extractors exist in different crates.
- Deleting a user deletes the `users` row. Their devices, line bindings,
  mailboxes and messages, API tokens, meetings, queue memberships,
  emergency calls and avatar file all stay, still pointing at the id.
- Stores no deletion path reaches: `emergency_calls`,
  `presence_history`, `webhook_delivery_logs` (each holds the full event
  payload and up to 64 KB of the receiver's response), the spool's
  `dead.jsonl` on each node, the Redis dead-letter stream (capped by
  count, not age), presence keys in Redis written without a TTL, and the
  CDR archive's objects (the target has no delete).
- Conference recordings record how many took part, not who, so a
  subject's voice in a room recording cannot be found.
- A campaign call's recording is named after the dialled number, and
  queue recordings after the queue: file names carry identifiers.
- The API gateway's response cache keys on the path and caches
  `/users/:username` and filtered `/cdrs` responses for its TTL.
- Encryption at rest covers secrets (digest passwords, trunk passwords,
  PINs, webhook secrets); email, phone and address columns are plain.

## 3. Goals and non-goals

Goals:

1. A legal hold that stops every deletion of what it covers — automatic,
   manual, cascaded, erasure — on every host, until released.
2. Export of everything the platform holds about a subject, in a bundle a
   person can read and a machine can parse, within the statutory clock.
3. Erasure of a subject that removes what the tenant has no obligation to
   keep, pseudonymises what it must keep, respects holds, reaches the
   archive, and reports all three.
4. A consent announcement on every call the platform records on its own
   decision or a tenant's configuration, heard before the first sample is
   written, with the outcome on the record.
5. Every deletion attributable: who or what removed which data, when, and
   under which policy or request.

Non-goals: backups (a database backup or a bucket's versions hold what
was erased until they age out; that is the operator's retention, stated
in the erasure report), jurisdiction rules (the platform does not know
which law applies; the tenant's policy says what to do), eDiscovery
search and review tooling beyond export, per-party consent negotiation
(§12), and consent for SIPREC to an external recorder (the recorder's
duty, and the SBC has no tenant context).

## 4. Registering and deleting correctly

The groundwork the other sections stand on.

- **Recordings.** Per-participant conference recordings get a row in
  `conference_recordings` (a `participant_id` column and a `kind`), the
  events rooms already emit, and the consent step. Every conference
  recording also records who was in the room while it ran (caller URI and
  display name per participant), so a subject's voice can be found. Call recordings keep
  their reference on the CDR (`recording_url`) and gain a
  `call_recordings` row (tenant, call, path, node, started, stopped,
  consent) so a recording is governable before its call has ended and
  its CDR is stored. The queue's recording is written through the same
  row.
- **Retention by row, not by directory.** Recording retention deletes
  what rows name. The directory prune becomes a platform job with the
  platform's own window (`[retention] orphan_files_days` in `fcp-admin`)
  for files no row names, and never runs inside a tenant's run.
- **Voicemail.** Deleting a message deletes its audio, on every path;
  the cascades from numbers and line bindings collect the files first.
  Retention gains `voicemail_retention_days` on the policy, and the
  mailbox's own `retention_days` is applied as a tighter window. Messages
  marked deleted are purged after a grace period.
- **Traces.** SIP traces get their tenant once the call's tenant is
  known (the handler already resolves it); flow traces gain a
  `tenant_id` column so retention does not depend on the CDR still being
  there.
- **Tenants.** `DELETE /tenants/:id` becomes a request: refused while the
  tenant holds data under a hold, otherwise the tenant is marked
  deleting, a job removes its data class by class (the erasure machinery
  of §6 with the tenant as the subject), and the row goes last.
- **Permissions.** `ComplianceManage` (policy, retention runs),
  `LegalHoldManage`, `SubjectRequestManage` and `AuditView`, granted to
  `super_admin` and `tenant_admin`; the compliance routes check them.

## 5. Legal hold

### 5.1 The hold

```rust
pub struct LegalHold {
    pub id: LegalHoldId,             // hold-{uuid}
    pub tenant_id: TenantId,
    pub matter: String,              // the case or request it serves
    pub reason: Option<String>,
    pub scope: HoldScope,
    pub created_by: String,
    pub created_at: SystemTime,
    pub released: Option<HoldRelease>, // by, at, reason
}

pub struct HoldScope {
    /// Only data from this window (a record's start, a recording's start,
    /// a message's receipt, a trace's capture); open ends allowed.
    pub from: Option<SystemTime>,
    pub to: Option<SystemTime>,
    /// Only data involving these subjects (§6.1's identifiers); empty
    /// means every subject.
    pub subjects: Vec<SubjectIdentifier>,
    /// Only these calls, in addition to or instead of the above.
    pub call_ids: Vec<CallId>,
    /// Which classes; empty means every class.
    pub classes: Vec<DataClass>,
}
```

`DataClass` is `cdr`, `call_recording`, `conference_recording`,
`voicemail`, `sip_trace`, `flow_trace`, `audit_event`, `device_capture`
(`docs/CALL_DIAGNOSTICS.md`), `conference_chat` (chat in a room,
`docs/WEBINARS.md` §2.5, migration 144), and `webinar_question` and
`webinar_registrant` (a webinar's questions and registrations,
`docs/WEBINARS.md` §2.5–2.6, migration 147). An empty scope holds the
whole tenant.

### 5.2 Enforcement

`fcp_core::legal_hold::HoldSet` is a tenant's active holds, loaded once
per operation (a retention run, a sweep, a request) and asked
`covers(&DataRef) -> Option<&LegalHold>`. The Postgres backend compiles
the same scope to a predicate its bulk deletes carry, so a CDR retention
batch deletes `WHERE tenant_id = $1 AND start_time < $2 AND NOT (held)`.
Where a delete meets a hold:

| Path | Under a hold |
|---|---|
| Retention, every class | The held items are skipped; the run's report counts them per hold |
| CDR partition drop | Nothing to add: a partition holding held rows is not empty, so it is not dropped |
| Conference node's own sweep | Asks the store for held recordings and skips them |
| `DELETE` routes (recordings, voicemail messages and greetings, meetings with recordings) | `409 Conflict` naming the hold |
| Cascades (number, line binding, mailbox) | Refused with `409` when anything below is held |
| Tenant deletion | Refused while any hold is active |
| Phone voicemail menu "delete" | The message is marked deleted for the listener and kept |
| Subject erasure | Held items are kept and listed in the report as held |
| CDR archive | A rewrite only adds late records, so it is allowed; with `object_lock` set, the objects for held days get an S3 legal hold, removed on release |
| Redis streams, spools | Not held: they are transit, and their contents are in the table |

A hold placed after an item was deleted cannot bring it back; the
deletion log (§7) shows when it went.

### 5.3 API

`POST/GET /api/v1/compliance/holds`, `GET /compliance/holds/:id`
(with counts of held items per class, computed), `POST
/compliance/holds/:id/release {reason}`. `LegalHoldManage`. The events
`compliance.hold_placed` and `compliance.hold_released`.

## 6. Subject export and erasure

### 6.1 Who the subject is

```rust
pub enum SubjectIdentifier {
    /// A platform user: expands to their extension, their numbers (line
    /// bindings), their AORs on the tenant's SIP domains, their devices'
    /// URIs, their mailboxes and their email.
    User { username: String },
    /// A telephone number, internal or external.
    Number { e164: String },
    /// A SIP address of record.
    Aor { aor: String },
    Email { address: String },
}
```

`fcp_core::subject` holds the one canonical form every class matches
against, replacing the copies (§2): a URI or name-addr reduces to its
user part and host; a number compares by digits against E.164, with the
tenant's dialling defaults applied to national forms; an AOR is
lower-cased without scheme, parameters or port. A `User` is expanded
once, when the request starts, and the expansion is in the report.

Matching the record table is a walk, not an index: the job reads the
tenant's records month by month by keyset and matches in Rust, with the
same canonical form the export and the erasure use. A tenant with ten
million records a month takes minutes, inside a clock measured in days,
and no index is added to the hottest table for a rare request.

### 6.2 The request

`subject_requests (id, tenant_id, kind, identifiers, expanded, status,
received_at, due_at, requested_by, completed_at, report, bundle)`:
`kind` is `export` or `erasure`; `due_at` defaults to thirty days after
receipt (`subject_request_days` on the policy); `status` runs `received`,
`running`, then `completed` or `failed`, with `preview` and
`confirmed` between them for an erasure. Verifying that the person asking
is the subject is the tenant's job before it files the request.

Requests run in `fcp-admin`, beside retention, under the lock
`fcp:subject-requests`: it already holds the recordings directory and
reaches the conference servers. `POST /api/v1/compliance/subject-requests`,
`GET` (list and one), `POST …/:id/confirm` for an erasure,
`GET …/:id/bundle` for an export. `SubjectRequestManage`.
`compliance.subject_request_completed` when one finishes, and the open
requests past `due_at` are a metric (§9).

### 6.3 What each store gives up

| Store | Export | Erasure |
|---|---|---|
| `users` and the user's own rows (tokens, avatar, devices' ownership, line bindings' ownership, queue membership, meeting ownership) | Profile without secrets; owned objects listed | Delete the user, tokens and avatar; clear ownership; the mailbox as below |
| Voicemail: the user's mailboxes, and messages the subject left elsewhere (`caller`) | Metadata and audio | Delete messages, greetings and files |
| `cdrs` | Every matching record | Pseudonymise (§6.4); the recording as below |
| Call and queue recordings (`call_recordings`) | Audio for matching calls | Delete |
| Conference recordings | Audio of rooms the subject was recorded in (a participants column, §4) | Delete |
| Chat in rooms (`conference_chat`, `docs/WEBINARS.md` §2.5) | Messages the subject posted, by where they called from and their name | Delete |
| Webinar questions (`webinar_questions`, `docs/WEBINARS.md` §2.5) | Questions the subject asked, by where they called from and their name | Delete |
| Webinar registrations (`webinar_registrants`, `docs/WEBINARS.md` §2.6) | The subject's registrations, by email and name | Delete |
| `sip_trace_messages` | Raw messages of matching calls | Delete |
| `flow_execution_traces` | Traces of matching calls | Delete |
| `registrations`, presence (presentities, subscriptions, policies, lists, history), Redis presence keys | Current state and history | Delete |
| `emergency_calls` | Every matching call | Pseudonymise; kept (emergency records) |
| `admin_audit_log`, `config_audit_log` (the subject as actor) | Their actions | Pseudonymise actor and path; kept |
| `port_requests` (authorised contact) | The request | Kept, reported (a carrier record) |
| `webhook_delivery_logs` | Deliveries whose payload names the subject | Delete those logs |
| `phone_numbers` block and allow lists | Entries naming the subject | Remove the entries |
| CDR archive objects | Covered by the table's records | Rewrite the days touched (§6.5) |
| Redis dead letters, spools' `dead.jsonl` | Entries naming the subject | Delete; spool dead letters gain an age limit |
| Event stream, CDR stream, TTS cache, gateway cache, logs, backups | Not exported (transit or operator stores) | Not reachable; the report states each window |

Held items (§5) are exported and never erased; the report lists them
with the hold's matter.

### 6.4 Pseudonymisation

A subject's token is `erased-` and the first twenty hex digits of an
HMAC-SHA-256, keyed by a per-tenant secret sealed with the platform's
encryption at rest, over the canonical identifier. The same subject gets
the same token in every record and every later request, so counts and
billing still add up, and the token cannot be reversed without the key.
In a record the URI keeps its form and host
(`sip:erased-3f…@pbx.example`), display names and `sip_call_id` are
cleared, and the `custom_data` fields that name a party are removed.

### 6.5 The archive

The erasure notes each tenant-day it pseudonymised in
`cdr_archive_late`, the note the archiver already acts on, so the next
run rewrites those objects from the table. A bucket with versioning keeps
the old version until its lifecycle rule expires it; the report says so,
and `docs/COMPLIANCE.md` tells operators to set a non-current-version
expiry on the archive bucket.

### 6.6 Two steps for an erasure

An erasure runs first as a preview: the same resolution, a report of
every item it would delete, pseudonymise, keep or leave held, and no
change. A person confirms the preview; the job then runs again against
the data as it is at confirmation and reports what it did. An erasure
cannot be undone, and a wrong identifier would otherwise erase someone
else.

### 6.7 The bundle

A `tar.gz` with `manifest.json` (the request, the expansion, the classes
and counts, each file's SHA-256, what was not exportable and why),
`data/<class>.jsonl`, and `media/<class>/<id>.<ext>`. Written to
`[compliance] exports_dir` in `fcp-admin` (or an `ArchiveTarget`),
downloaded through the API, removed after `export_ttl_days` (seven by
default). A recording holds other people's voices too; the manifest
marks every recording with other parties so the tenant can review it
before handing it over.

## 7. The deletion log

`deletion_log (id, tenant_id, class, item, detail, cause, actor, request_id, hold_skipped, at)`:

- `cause` is `retention`, `manual`, `cascade`, `erasure`, `tenant_deletion`
  or `node_sweep`; `actor` the user, token id or host.
- Per item for recordings, voicemail, erasure and tenant deletion; per
  batch for record-table retention (`item` a range, `detail` the count).
- Written in the same transaction as a database delete; for a file, the
  row is written when the file is gone.
- Retained under `audit_retention_days`, and itself never deleted while a
  hold covers the tenant.

The API audit middleware fills `detail` for deletes (the route's ids) and
records an API token's id.

## 8. Recording consent

### 8.1 One way to start a call recording

`fcp_sip::recording::start_call_recording(call, source)` is the only way
the call manager starts recording a call:

1. Resolve the call's media session from `media_session_id` (fixing the
   REST route's lookup) and refuse if the call is not anchored.
2. Load the tenant's policy. With `announce`, play the consent prompt to
   both legs with `AudioTarget::Both` and **wait for it to finish**
   (`wait_playback_completion`); a prompt with no audio, or no prompt
   store, falls back to the synthesised sentence the conference server
   already uses.
3. If the announcement failed and the policy's
   `recording_consent_on_failure` is `skip`, do not record: the outcome
   is `refused`. Otherwise start the recorder.
4. Write a `call_recordings` row and set the call's `recording_path` and
   `recording_consent` (`announced`, `failed`, `not_required`,
   `refused`), which the CDR carries (a new record field and column).

`source` is `api`, `flow`, `user` (`record_all_calls` on either party's
user), `number` (`recording_enabled` on the dialled or calling DID) or
`platform` (`media.enable_recording`). The automatic sources start when
the call is answered and both legs are bridged, so the prompt is heard by
both parties; a call recorded for several reasons is recorded once. The
file goes under `<recording_dir>/<tenant>/<yyyy>/<mm>/<dd>/<call_id>.wav`,
so the tenant is in the path as well as the row.

A call's recording that stops and starts again (the API's stop, then
start) announces again, and is a second row.

### 8.2 The queue

The queue bridges raw RTP and has no media session, so it gets the same
steps in its own terms: the tenant's consent prompt decoded and encoded
to each leg's payload type, played to the caller's and the agent's
sockets together and awaited before the bridge starts; the same policy,
failure rule, row and CDR field. Its recorder writes the two directions
mixed into one track, not one after the other — the current file is not
a usable recording of the conversation.

### 8.3 The rest

- **Flow `StartRecording`** goes through §8.1 with source `flow`; the
  node cannot skip the policy. A flow that has already played its own
  notice sets the tenant's policy to `none`, which is the tenant's
  decision to make, not the flow author's.
- **Per-participant conference recording** announces to the room, like a
  room recording, and gets its row (§4).
- **Voicemail** is out of the policy: the greeting is the notice, and the
  caller chooses to leave a message.
- **The SBC's local capture and SIPREC** are out of scope (§3).
- **forge's recording routes on the WebRTC gateway and the media server**
  are behind operator authentication and record a bridge that belongs to
  no tenant. They are turned off by default and documented as such.

### 8.4 The policy

`recording_consent` keeps `none` and `announce`. New:

- `recording_consent_on_failure`: `record` (today's behaviour, default)
  or `skip`.
- `recording_notice_interval_secs`: an optional tone played to both
  parties at that interval while a recording runs, for regimes that
  require a continuing notice.


## 9. Observability

- Metrics on `fcp-admin`: active holds, items retention skipped for a
  hold (per class), subject requests open and past due, requests by kind
  and outcome, the deletion log's rows by cause.
- Metrics on the call manager and the queue: recordings started by
  source, consent outcomes.
- Events: `compliance.hold_placed`, `compliance.hold_released`,
  `compliance.subject_request_completed`; recording events carry the
  consent outcome.

## 10. Testing

- **Holds**: a matrix test in `fcp-api-core/tests` placing a hold and
  running every deletion path of §5.2 against it, then releasing it and
  running them again; the conference node's sweep in the conference
  server's tests; retention with a hold per class in the Postgres suite.
- **Deletion**: two tenants with different recording windows keep each
  other's files; a voicemail delete removes the file; tenant deletion
  empties every class.
- **Consent** on the test floor: a user with `record_all_calls` calls a
  phone; both phones hear the prompt (a tone the scripted phone detects),
  the recording exists, its first second does not contain the prompt's
  tone, and the CDR says `announced`; the same with the prompt missing
  and `skip`; a queue call hears the prompt on both legs and records both
  directions mixed; the REST route on an anchored call.
- **Subject requests**: a seeded tenant with the subject in every store
  of §6.3; the export's manifest and files; the erasure's preview
  changing nothing, then the confirmed run deleting, pseudonymising,
  keeping held items, and the archive object rewritten without the
  subject's identifiers.
- **Canonical form**: property tests that every spelling of a number and
  an AOR reduces to the same key.

## 11. Phases

One PR per phase, reviewed before the next starts.

1. **Foundations.** `call_recordings` rows (call manager and queue), rows
   for per-participant conference recordings and a participants column,
   recording file names without identifiers and with the tenant, recording
   retention by row and the platform orphan sweep, voicemail files
   deleted with their rows and voicemail retention, trace tenancy, the
   compliance permissions, the deletion log and audit `detail`, the admin
   UI's two broken compliance routes (`/audit-logs`, the voicemail message
   delete).

   *Phase 1 shipped 2026-09-14 (v0.25.166).* Migration 062 and the pieces
   around it, with these decisions made on the way:

   - **The deletion log is written by the database.** Statement-level
     `AFTER DELETE` triggers on `cdrs`, the two trace tables,
     `admin_audit_log`, both recording tables and `voicemail_messages`
     write it in the deleting transaction, so no path — the API, a phone
     menu, a sweep, SQL typed by hand — can delete without an entry. The
     cause and actor come from `fcp_core::compliance::deleting_as`, a
     task-local scope the API's audit middleware opens around every
     request and each sweep opens around its run; the Postgres backend
     hands it to the transaction with `set_config`. A delete outside any
     scope is logged as `unattributed`, which is visible rather than
     silent. A cascade cannot be told by trigger depth (a cascaded
     statement's triggers fire at the end of the outer statement), so a
     voicemail message is logged as a cascade when its mailbox is gone in
     the same transaction; it is the only class a foreign key cascades to.
   - **Files are queued, not deleted in place.** The same triggers put a
     deleted recording's or message's file on `pending_file_deletions`;
     a reaper in `fcp-admin`'s hourly round removes it — from the
     directories that host mounts, and only once no row names the path
     (a greeting is rewritten under its path on every mailbox save).
     This covers every path that deletes rows, cascades included, and the
     conference server keeps removing its own files as before.
   - **`call_recordings` is backfilled** from the records that name a
     recording, one scan of `cdrs` on upgrade. Its trigger clears the
     record's `recording_url` when the row goes, which retires
     `CdrStore::list_recordings_before` and `clear_recording_url`; a
     stored record closes its call's open recordings.
   - **A recording with no row is not kept** — the API route, the flow
     node, the queue, room and per-participant conference recordings all
     stop the recorder and remove the file when the row cannot be written.
   - **New file names**: calls and queues under
     `<tenant>/<yyyy>/<mm>/<dd>/`, named for the call (flows) or the
     recording's id (queues; campaign recordings were named for the dialled
     number); the API route's `path` under the tenant's directory. Conference
     recording names are unchanged: they carry a room number, not a
     person, and the row names the tenant.
   - **Trace tenancy** is taken from the stored record by the upkeep round
     rather than at capture, where nothing yet knows the call's tenant; a
     tenant's trace retention also reaches its untenanted traces through
     its records, as before.
   - **Upkeep** is a platform round (`[retention]` in `fcp-admin`): mailbox
     retention, the purge of messages users deleted (seven days after),
     `_system` SIP traces and orphan files (both off unless configured),
     and the reaper.
   - **Permissions**: `compliance_manage` and `audit_view` are checked on
     the existing routes; `legal_hold_manage` and `subject_request_manage`
     exist for phases 2, 4 and 5.
   - **The admin UI's audit page** was written for an API that never
     existed (paged entries by resource type); it now reads the real
     audit log and the deletion log. The voicemail page's message delete
     and mark-heard called routes that do not exist; they use
     `/voicemail/messages/:id`.

   Tests: the Postgres suite `tests/compliance.rs` (two tenants' recordings
   under one window, the record's reference cleared, the reaper and its
   roots, attribution by retention, a person, a cascade and nobody,
   greetings kept, mailbox retention and the purge, orphans, trace
   adoption, room participants), the API suite (the permissions against a
   read-only session, a message deleted through the API and its mailbox's
   cascade in the deletion log, the audit event's route parameters), and
   unit tests for the paths, the roots, the attribution scope and the
   permissions.
2. **Legal hold.** The model, API and events; enforcement in every path
   of §5.2; tenant deletion refused under a hold; S3 Object Lock on held
   archive days.

   *Phase 2 shipped 2026-09-15 (v0.25.170).* Migration 063, `fcp_core::legal_hold`
   and `fcp_core::subject`, with these decisions made on the way:

   - **The rules are written twice and tested once.** `LegalHold::covers`
     in Rust and `fcp_hold_matches` in SQL apply the same scope; the
     canonical subject form is `fcp_core::subject::keys` and
     `fcp_subject_keys`, tested against one list of spellings in both.
     Subjects meet on shared keys: `num:` digits, `tail:` the last ten
     digits, `user:` a name, `aor:` user and host. The tail makes a
     national spelling meet its international one, and makes a hold err
     towards holding; erasure (phase 5) will narrow it.
   - **Two layers.** Retention's deletes carry the hold filter, so a run
     deletes what is free and leaves what is held, reporting counts per
     class. Under them, a statement-level `AFTER DELETE` trigger on each
     governed table raises `HLD01` if the statement removed a held row,
     rolling it back — the API's deletes, cascades from numbers, line
     bindings and mailboxes, the tenant's deletion (a row trigger on
     `tenants`) and hand-typed SQL alike. The backend turns `HLD01` into a
     `LegalHoldViolation`, the API into `409`. The trigger costs one
     lookup of the (small) holds table per statement while nothing is
     held.
   - **File-first paths ask first.** The conference recording route removes
     the file before the row, so it asks `hold_on` before the file; a
     node's sweep lists expired recordings through the hold filter.
   - **Keys a row does not have.** A trace captured under `_system` is
     judged by its call's tenant; a call recording or flow trace takes its
     subjects from its call's stored record, and one with no record yet is
     held by any hold with subjects. Conference recordings match by the
     participants phase 1 put on the row; voicemail by caller and
     `mailbox:<id>`; audit events by actor.
   - **A user is expanded when the hold is placed**, and the keys are
     stored on the hold; a number bound to them later is not in it.
   - **The phone's voicemail menu** marks a held message deleted instead of
     failing, and the purge of deleted messages passes held ones by.
   - **Object Lock** is per archived day, conservative: any active hold on
     records whose window reaches into a day locks that day's object,
     whatever its subjects; `cdr_archives.locked` tracks it, a rewrite
     clears it so the new object version is locked again, and release
     unlocks. `[cdr_archive] object_lock` turns it on; the directory target
     cannot lock. The S3 request is tested for its shape (a signed `PUT
     ?legal-hold` with `Content-MD5`); Garage, the S3 store the tests use,
     has no Object Lock.
   - **Counts** of what a hold holds are computed per class under a
     two-second budget, `null` when a class is too large.
   - **Voicemail greetings** are not a held class: they are the mailbox
     owner's own recordings, rewritten on every save.

   Tests: the Postgres suite (SQL and Rust keys agree; a subject hold on a
   national spelling keeps a record, a recording and a message through
   retention while their neighbours go, the counts, `hold_on`, refusals of
   a direct delete, a number's cascade, raw SQL and the tenant, release;
   a tenant-wide windowed hold on traces and rooms, a `_system` trace
   refused through its call, the node's expired list), the API suite
   (validation, placement with expanded keys, list and counts, another
   tenant's 404, `409` on message, mailbox, number and tenant deletes,
   release with a reason and once only, the permission), the archive
   (locked on placement, again after a rewrite, unlocked on release), the
   voicemail menu under a hold, the S3 request, and the event's shape.
3. **Consent.** `start_call_recording`, the REST route's fix, automatic
   recording by user, number and platform, the flow node, the queue's
   prompt and mixed recording, per-participant consent, the CDR field,
   `on_failure` and the notice tone; the test floor scenarios.
   *Phase 3 shipped 2026-09-15 (v0.25.171).* Migration 064 (the policy's
   `recording_consent_on_failure` and `recording_notice_interval_secs`, the
   record's `recording_consent`), with these decisions made on the way:

   - **One recorder per node.** `fcp_sip::recording::CallRecorder` is what
     §8.1 calls `start_call_recording`: the SIP handler, flows and the
     node's HTTP API share one, so a call is recorded once whoever asks, and
     a second start while one runs — or is still announcing — is refused.
     The consent plan (`fcp_core::compliance::ConsentPlan`: announce or not,
     the prompt's file, the failure rule, the notice) is resolved the same
     way by the recorder, the queue and the conference server.
   - **The recording starts after the announcement, and the announcement is
     not in it.** The recorder awaits the playback before the recorder
     starts; forge writes injected audio to the sockets, not the recording
     tap. The test floor checks both parties hear the prompt and the file
     does not contain it.
   - **A refusal leaves no row.** A row names a file; a `refused` recording
     has none. The record says `refused`, a `recording.failed` event carries
     `reason = consent_refused`, and the metrics count it.
   - **Automatic recording is decided at the INVITE** — the call must be
     anchored for it — and starts once the 200 OK has gone to the caller
     (and when a pickup answers). User before number before platform; the
     callee is the user or number dialled, by name, extension (its user, its
     device's owner, its number) or DID (the line binding's owner), not a
     later forwarding target. `media.enable_recording` stops gating the
     other sources, which it did while nothing recorded, and becomes the
     platform source; the call manager reads it and `recording_dir` from
     `[media]`.
   - **The recording stops when the call does.** The media session of a
     call ended by BYE lives until its idle timeout, and so did its
     recorder, leaving an unfinished WAV and an open row: the recorder now
     stops on every ending, and closes the row.
   - **The REST route reaches the calls where they are.** The API's route
     runs through the recorder with the media session fixed, but an
     `fcp-admin` process holds no calls; the call manager's own HTTP API
     gains `POST /calls/{id}/recording/start` and `/stop`. Its
     parameterised routes were written in axum 0.8's `{id}` form on axum
     0.7, which reads them literally, so none of them — agents, queues,
     campaigns, supervisors, these — ever matched; they now use `:id`.
   - **The queue** plays the prompt, decoded to 8 kHz and encoded per leg,
     to the caller's and the agent's sockets together before the bridge
     starts, and campaign calls do the same. Its recorder decodes each
     direction in its own leg's codec (G.722 included; telephone events,
     CSRCs, extensions and padding skipped) and mixes frame by frame,
     writing one side against silence only once the other has been quiet
     for half a second — the wait is on the clock, because a loaded node
     hands the recorder a burst from one leg and then a burst from the
     other, and two people who talked at once must not be recorded one
     after the other. The notice is written over the audio the bridge
     forwards, in each direction's codec.
   - **Conferences** refuse a room or participant recording under `skip`,
     announce a participant's recording to the room, beep the notice into
     the room, and put the outcome on `conference.recording_started`.

   Tests: the test floor (a user who records every call — both parties hear
   the prompt, then the notice, the file has both parties and not the
   prompt, the record and row say `announced` and `user`; the node's
   `enable_recording` with a missing prompt under `skip` — the call goes on,
   nothing is recorded, `refused` on the record and in the metrics; a
   number's `recording_enabled` with no announcement asked; the node's route
   — returns after both hear the prompt, once only, `404`, `400` for a path
   out of the tenant's directory, stop closes the row; a recorded queue call
   — both hear the prompt, and the file has caller and agent overlapping),
   the Postgres suite (the plan from a stored policy and prompt, another
   tenant's prompt, a missing file, the record's field), the API suite (the
   new policy fields and their validation), and unit tests for the outcome
   rule, the notice tone, the counters, the RTP payload parser, the decoders,
   the mix, the notice schedule and overwrite, and prompt loading.
4. **Subject export.** The canonical form, identifiers and expansion,
   requests, the worker, the bundle and its download.
   *Phase 4 shipped 2026-09-15 (v0.25.172).* Migration 065 (`subject_requests`,
   the policy's `subject_request_days`), `fcp_core::subject_request` and the
   Postgres `SubjectDataStore`, with these decisions made on the way:

   - **Two matchers.** A hold's keys lean towards matching (§5); an export
     must not hand one person someone else's data, so it decides with
     `fcp_core::subject::SubjectMatcher`: digits, or a national form of ten
     or more digits against its international one; an address by user and
     host; a bare name by the same name. Erasure (phase 5) acts by the same
     matcher. The canonical keys still narrow candidates in SQL, so a store
     returns a superset and the matcher decides.
   - **The expansion keeps more than keys.** `legal_hold::expand` returns the
     identifiers, the user ids and the mailboxes, kept on the request; a
     user's own rows (tokens, devices, lines, extensions, memberships,
     meetings, mailboxes, greetings) are found by ownership, everything else
     by identifiers, and what hangs off a call (its recordings and traces)
     by the calls whose records matched.
   - **The record table is walked, not indexed.** Oldest first by
     `(start_time, cdr_id)` in pages, filtered by the subject's keys, on the
     primary: an export does not miss what a replica has yet to see.
   - **Payloads no column names a party in** — webhook deliveries — are
     narrowed by `LIKE` on the subject's names and number tails and matched
     on every string in the JSON, including JSON held as a string.
   - **Holds do not narrow an export.** Held items are exported and counted,
     and the report names the holds' matters.
   - **Review before handing over.** A call recording (two parties), a
     message in the subject's mailbox left by someone else, and a room with
     more than one participant are listed under `review`.
   - **Files come from where they are.** Call recordings, voicemail audio
     and the profile picture from the directories `fcp-admin` mounts (the
     reaper's roots), conference recordings through the conference server;
     a file that cannot be read is reported with why, and the rest of the
     export goes on.
   - **The bundle is a file on a shared directory**, `[compliance]
     exports_dir`, not an `ArchiveTarget`, which cannot be read back.
     Downloads are repeatable until `export_ttl_days` rather than once: an
     interrupted download can be retried, and every one is in the audit log
     (the audit middleware now audits `GET …/bundle`). Expired bundles are
     removed and the request says so (`410`).
   - **The worker** runs under `fcp:subject-requests`; holding the lock and
     idle, it takes any request marked running as left by a stopped replica
     and queues it again. Erasure requests are refused at filing until
     phase 5.
   - **Not exported, and said so in the manifest:** the event and CDR
     streams (in transit), the CDR archive (copies of the table), dead
     letters on nodes and in Redis (failed records the operator holds;
     erasure reaches them), live call state, caches, logs and backups.

   Tests: the Postgres suite (a subject seeded across every store — profile
   without secrets, her call and not Bob's with Carol nor a namesake's
   elsewhere, the recordings and traces of her call, a recording on a host
   this one does not mount, messages in her box and left in another, not a
   foreign number that ends like hers, a room she was in, registrations,
   emergency calls, audit events, a webhook delivery, a block list entry;
   a hold counted with its matter; the review list; every manifest file in
   the bundle; the bundle removed on expiry), the API suite (validation,
   the policy's deadline, list and scope, `409` before it runs, the worker,
   the report, the download with its checksum, the audit entry, a read-only
   session refused, `410` once removed), and unit tests for the matcher,
   the request model, media names, local file roots, packing, the payload
   patterns and JSON leaves, the event and the metrics.
5. **Subject erasure.** Preview and confirm, the per-store actions of
   §6.3, pseudonymisation, the archive rewrite, dead letters, the report;
   tenant deletion as a job on the same machinery.

   *Phase 5 shipped 2026-09-15 (v0.25.173).* Migration 066 (the request's
   confirmation, preview and erasure reports; `deletion_log.request_id`;
   `subject_pseudonym_keys`; the policy's `cdr_erasure`),
   `fcp_core::subject_erasure` and the Postgres `SubjectEraseStore`, with
   these decisions made on the way:

   - **One walk, three uses.** The erasure walks the classes the export
     walks, with the same candidates and the same narrow matcher, in an
     order that acts on records before what hangs off their calls, a
     mailbox's contents before the mailbox, and the user last. A preview
     counts and samples; a confirmed run acts page by page, so memory stays
     flat however many records a subject has.
   - **The request is the state.** Filed as `received`, its first run leaves
     it `preview`; `POST …/confirm` records who and when and makes it
     `confirmed`; the worker's next run erases against the data as it is
     then. A worker that stops mid-run leaves it for the next to take up
     where the status says.
   - **One pseudonym per subject as filed.** The token is keyed per tenant
     and made from the subject's canonical identifier (`user:<name>`,
     `num:<digits>`, `aor:<user@host>`); each subject of a request has its
     own matcher, so a record naming two of them gets two tokens. The key is
     sealed like the other secrets and in the re-encryption pass.
   - **Holds are checked before acting, and enforced after.** Updates are not
     guarded by the hold triggers, so the engine skips held items itself;
     deletes still meet the triggers, and a batch a hold refuses is retried
     item by item so only the held ones stay.
   - **Every change is in the deletion log under its request.** The
     request id is set with the cause and actor and filled in by a column
     default, so the existing triggers log it without being rewritten;
     classes with no trigger, and pseudonymisation, are logged per batch by
     the store.
   - **The archive follows by the note it already reads.** Rewriting or
     deleting a record notes its archived day as late; a day left with no
     records now gets an empty object instead of keeping the old one.
   - **Dead letters.** The CDR stream's are read and deleted by id when
     `fcp-admin` is given Redis. A node's spool is out of reach, so its dead
     letters are kept in files by day and removed after `[cdr]
     dead_letter_days` (30), which the report states. Redis presence keys
     turned out to have no writer (`RedisPresenceStore` is not wired), so
     there is nothing there to erase; the API gateway's cache expires by TTL.
   - **Files where no row deletion reaches.** A conference recording is
     deleted through the conference server before its row, and its row kept
     when the node cannot be reached, so the file stays findable; a profile
     picture is removed from the directory `fcp-admin` mounts.
   - **A tenant's deletion is a request without a preview** — a platform
     admin names the tenant by id, which is not ambiguous. It is refused
     while a hold is active, closes the tenant, and runs by the catalogue:
     every table with a `tenant_id` column but the records the deletion
     itself needs (the request, the deletion log, legal holds, pending file
     deletions, the archive's bookkeeping), the `_system` traces of its
     calls, and the tenant last. `GET /tenants/:id/deletion` reads it once
     the tenant is gone.
   - **Emergency calls keep their location.** It is the site's dispatchable
     address, not the person's; their identifiers are pseudonymised.

   Tests: the Postgres suite (an erasure's preview changing nothing, then the
   confirmed run: the user and what they own deleted or cleared, her
   messages and those she left while Bob's own stays, recordings with their
   files queued and a conference file removed through its node, her
   registration not Bob's, a webhook delivery, a block list entry; her side
   of a record pseudonymised with its display name, `sip_call_id` and
   `custom_data` field, the same token in the emergency call and the audit
   log's actor and path, the day noted for the archive; a held record, a port
   request and Bob's call kept; a dead letter of hers removed and Bob's
   kept; every deletion logged under the request as an erasure by the person
   who confirmed; the pseudonym stable; a tenant's deletion emptying every
   class while another tenant's rows and its call's trace stay), the API
   suite (the policy's `cdr_erasure`, confirm refused before a preview and
   twice, the preview, the erasure, the deletion log by request; a tenant's
   deletion refused under a hold, accepted once, closed, run, readable after
   the tenant is gone), the archive (a pseudonymised record rewritten, a
   deleted one leaving an empty object), the Redis dead letters (pages and
   deletion by id), the spool's dated dead letters and their age limit, and
   unit tests for the pseudonym, the anchors, the order and treatments, a
   record's and an audit event's rewriting, and the identifier forms.

## 12. Open questions

Each with a recommendation.

1. **Hold scope.** A tenant with optional time window, subjects, calls
   and classes (recommended: matters are about people and periods, and a
   tenant-wide hold on a carrier stops all retention) or tenant-wide only
   (simplest, and over-retains).
2. **Who places and releases holds.** `tenant_admin` and `super_admin`
   with `LegalHoldManage` (recommended), or platform only.
3. **Erasing call records.** Pseudonymise (recommended, with a policy
   field `cdr_erasure = "delete"` for tenants that choose deletion), or
   delete always.
4. **Recordings in an export.** Include, marked for review where other
   parties are heard (recommended), or exclude recordings.
5. **Erasure in two steps.** Preview then confirm (recommended), or one
   step.
6. **Automatic recording.** Make `record_all_calls`, `recording_enabled`
   and `media.enable_recording` record (recommended), or remove the
   flags and leave recording to flows and the API.
7. **When the consent prompt fails.** Default `record`, as today, with
   `skip` available (recommended), or default `skip`.
8. **Where requests run.** `fcp-admin` under a lock (recommended: it holds
   the recordings directory and runs retention), or `fcp-jobs`.
9. **Held archive days.** S3 Object Lock legal hold when the bucket has
   Object Lock enabled (recommended), or the hold row alone.
10. **Emergency call records under erasure.** Kept and pseudonymised
    (recommended), or deleted.
11. **Pause and resume of a recording** (payment card details read over a
    recorded call). A separate item after these phases (recommended), or
    part of phase 3.

## 13. Not in scope

Backups and bucket versions (reported, not erased), verifying who is
asking, a rules engine for jurisdictions, consent for SIPREC and the
SBC's local capture, per-party consent, an eDiscovery review tool, admin
UI pages for holds and subject requests (the API comes first; the pages
are a follow-up), and field-level encryption for contact details.
