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
| `recording_retention_days` | Recording files referenced by old CDRs (the reference is cleared too) and, when the admin service knows the recordings directory, any file in it older than the window |
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

## Recording consent

`recording_consent` on the policy is `none` (default) or `announce`. With
`announce` and a `recording_consent_prompt_id`, starting a recording
through `POST /api/v1/calls/:id/recording/start` first plays that prompt to
both parties; the response reports `consent_announced`. The prompt must
belong to the tenant and have audio.

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

## Checklist for a tenant

1. Decide the windows your regime requires (many US rules: CDRs 18 months
   or more, recordings per industry) and set them on the policy.
2. If you must announce recording, upload a prompt and set
   `recording_consent` to `announce` with its id.
3. Run `POST /compliance/retention/run` once and read the report.
4. Give your compliance officer `GET /audit-log` and `GET /cdrs/export`.
