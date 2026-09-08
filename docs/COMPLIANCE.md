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
| `flow_trace_retention_days` | Flow execution traces (found through the tenant's CDRs) |
| `recording_retention_days` | Recording files referenced by old CDRs (the reference is cleared too); when the admin service knows the recordings directory, any file in it older than the window; and **conference recordings**, which their own node deletes (see below) |
| `audit_retention_days` | Admin audit events |

Windows are applied independently and in an order that keeps referential
sense: flow traces, SIP traces, and recordings go before the CDRs they hang
off; audit events last.

### When it runs

- **Hourly** in `fcp-admin`, for every tenant that has at least one window
  set. Set `call_recordings_dir` in the admin config to let the sweep
  prune the recordings directory as well.
- **On demand** with `POST /api/v1/compliance/retention/run`, which
  returns the report for that run.

Every run is recorded; `GET /api/v1/compliance/retention/runs` lists them
with per-category counts and any non-fatal errors (a file that could not be
removed, a store that is not configured). A run never stops at the first
problem.

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

## Admin audit log

A middleware inside the API's auth layer records every request that is not
a `GET`/`HEAD`/`OPTIONS`, plus every CDR export, as an audit event:

| Field | Meaning |
|---|---|
| `actor`, `actor_kind` | The username of a JWT session, `api_token`, `static_token`, or `anonymous` (pre-auth routes such as login) |
| `tenant_id` | The tenant the request acted on: the caller's own, the `x-fcp-tenant-id` override, or the default tenant for the platform token |
| `action` | Method plus route template, e.g. `PUT /api/v1/users/:username` — stable across ids, use it to filter |
| `path`, `method`, `status` | The concrete request and how it ended (a 401 on `/auth/login` is a failed login) |
| `source_ip` | From `X-Forwarded-For` / `X-Real-IP` when a proxy sets them |

Writes are asynchronous and best-effort; the response is never delayed or
failed by the audit log.

`GET /api/v1/audit-log?actor=&action=&since=&limit=` lists events. Tenant
admins see their tenant; the platform token sees everything unless it
selected a tenant with the override header. The older `GET /config/audit`
(configuration-store change history) is unchanged.

## CDR export

`GET /api/v1/cdrs/export` returns `text/csv` with the same filters as
`GET /cdrs` (`call_id`, `from_uri`, `to_uri`, `disposition`,
`start_time_from`, `start_time_to`, and `tenant_id` for platform admins),
capped at 100,000 rows. Times are Unix seconds. Every export is itself an
audit event.

## Conference recordings API

`/api/v1/recordings` is the registry, scoped to the caller's tenant:

| Route | What it does |
|---|---|
| `GET /recordings` | Newest first. Filter by `room_number`, `meeting_id`, `since`/`until` (Unix seconds), `running`; page with `limit`/`offset` |
| `GET /recordings/{id}` | One recording |
| `GET /recordings/{id}/download` | The file, streamed from whichever node holds it, as `audio/wav` or `video/webm` |
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
3. Run `POST /compliance/retention/run` once and read the report.
4. Give your compliance officer `GET /audit-log` and `GET /cdrs/export`.
5. For conferences, decide `recording_format` on each node (`auto` records
   video meetings to WebM, audio-only ones to WAV) and let
   `recording_retention_days` do the rest.
