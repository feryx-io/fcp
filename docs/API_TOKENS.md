# API tokens and permissions

How programs authenticate to the admin API (`/api/v1`), what a token can
reach, how often it may call, what an error and a time look like, and
what changed in v0.25.293–296 (`docs/DEVELOPER_SURFACE.md` §3, phases 1
and 2).

## 1. Three kinds of credential

Every request carries `Authorization: Bearer <credential>`.

| Credential | What it is | Reaches |
|---|---|---|
| The static token (`[api] auth_token`) | The platform's own key | Everything, in every tenant (`x-fcp-tenant-id` picks one) |
| A session (`POST /auth/login`) | A JWT for a signed-in user, 24 hours | What the user's role permits |
| An API token (`fcp_…`) | A key a program keeps | See §2 |

## 2. API tokens

A token has a **scope**:

- **Admin tokens** are made at `/api/v1/api-tokens`. They reach the admin
  routes their permissions name, and nothing else.
- **Personal tokens** are made by any user for themself at
  `/api/v1/me/api-tokens`. They act as that user on the user's own `/me`
  routes: profile, voicemail, calling settings, conferences. They reach
  nothing else, and in particular cannot reach:
  - the password;
  - `/auth/refresh` (a personal token cannot be turned into a session);
  - any admin route;
  - the user's own token list.

### Who may make an admin token, and with what

- Making, listing, rotating and revoking admin tokens takes the
  `api_token_manage` permission. Only a tenant admin (and a super admin)
  holds it.
- A token may carry only permissions its maker holds, and only ones a
  tenant admin holds. So even a super admin's token is a tenant admin's,
  in one tenant.
- `GET /api/v1/api-tokens/permissions` lists what the caller may grant,
  grouped by area. Asking for `admin` means all of them.
- An unknown permission name is a 400. A permission the maker lacks is a
  403 that names it.
- An API token cannot make another token.

### The maker answers for the token

A token names its maker (`created_by`). Every time the token is used, its
permissions are checked against what the maker holds now:

- If the maker is demoted, the token loses what the maker lost.
- If the maker is disabled, suspended or deleted, the token is refused.
- A maker locked out by failed passwords keeps their tokens: a lockout is
  about the password.
- A token made with the static token has no maker, and keeps what it was
  given.

### Revoking and rotating

- `DELETE /api-tokens/:id` revokes the token. The row is kept, with
  `revoked_at` and `revoked_by`, and the token is refused from then on.
  Revoking the same token again does nothing.
- `POST /api-tokens/:id/rotate` gives the token a new secret, shown once.
  - The old secret keeps working for `grace_seconds`: one day by default,
    at most a week, and `0` retires it at once.
  - `previous_secret_valid_until` says until when.
  - The caller must be able to grant everything the token carries.
  - A personal token is rotated only by its maker, at
    `/me/api-tokens/:id/rotate`.
- A user makes, lists and rotates their personal tokens from a signed-in
  session. A personal token can revoke itself
  (`DELETE /me/api-tokens/:id`), so a leaked one can be used to kill
  itself.

The secret is `fcp_` followed by 32 random bytes in base64url. Only its
SHA-256 hash is stored.

## 3. Every admin route names a permission

`crates/fcp-api-core/src/route_permissions.rs` lists every authenticated
route with the permission it needs. A middleware checks the caller against
that list before the handler runs, for sessions and tokens alike. A test
fails the build when a route is added without an entry.

The permissions added in this release:

| Permission | Covers |
|---|---|
| `api_token_manage` | `/api-tokens` |
| `emergency_read`, `emergency_manage` | `/emergency/*` and the `…/e911/validate` routes |
| `billing_read`, `billing_manage` | `/usage`, `/billing`, `/rate-plans` |
| `cdr_read` | `/cdrs` |
| `flow_read`, `flow_manage` | `/flows`, `/flow-traces` |
| `prompt_read`, `prompt_manage` | `/prompts` |
| `voicemail_read`, `voicemail_manage` | `/voicemail` |
| `conference_read`, `conference_manage` | `/conferences` |
| `queue_read`, `queue_manage` | `/queues`, `/queue-configs` |
| `presence_read`, `presence_manage` | `/presence` |
| `location_read`, `location_manage` | `/locations` |
| `extension_read`, `extension_manage` | `/extensions` |

Where an existing permission already fitted, the route uses it:

- **Line bindings** take the phone-number permissions.
- **Hoteling sessions and provisioning templates** take the device
  permissions.
- **Recordings** take `call_recording_access`.
- **SIP traces and debug** take `debug_view`.
- **Calls**:
  - recording start and stop take `call_control`;
  - monitor, whisper and barge take `call_supervise`.
- **Least-cost routes** take the trunk permissions.
- **Classes of service and schedules** take the routing-rule permissions.
- **The services' configuration (`/config`)**:
  - reading it takes `system_view_logs`;
  - changing it takes `system_config`, which only a super admin holds.
- **The cluster view (`/cluster/*`)**:
  - nodes and stats take `system_view_logs`;
  - sites take `site_read`.
- **The events WebSocket** takes `call_view`.
- **The dashboard and `/status`** are open to any administrator.

Who holds the new permissions:

| Role | Holds |
|---|---|
| `super_admin`, `tenant_admin` | All of them |
| `telecom_admin` | Every new read and manage permission except `api_token_manage` and `billing_manage` |
| `user_admin` | Voicemail and extensions (read and manage); presence, locations and conferences (read) |
| `department_admin` | Extensions, voicemail, presence and locations (read); queues (read and manage) |
| `read_only` | Every new `…_read` permission, and `cdr_read`; nothing that changes anything |
| `end_user` | None; an end user reaches `/me` |

## 4. What changed on upgrade

- **Read-only and other limited roles.** Before this release, a
  `read_only` session could write any admin route that did not check a
  permission itself. That covered 37 of 51 handler files: flows,
  voicemail, prompts, queues, presence, the services' configuration and
  more. Those writes now answer 403.
  - Each role now reaches what the table in §3 gives it.
  - A limited role that used a page it no longer reaches will see 403
    there.
- **Scoped tokens now work.** A token that names permissions reaches the
  routes that take them. Before, it reached only the 14 files that checked
  a permission, and everything else wanted `admin`.
- **Existing tokens.**
  - Tokens made before this release have no maker, and keep what they
    were given.
  - A token carrying `admin` or `*` still reaches everything in its
    tenant. The tokens page marks it **Full access (legacy)**, and the API
    marks it `legacy_full_access: true`. Any user with an admin role could
    have made one, including a `read_only` user. Review each and replace
    it with a scoped token.
  - New tokens never carry `admin`: at creation it becomes the list of
    what the maker may grant.
- **Revoking now keeps the token.** It shows as revoked in the list.

## 5. Limits

Since v0.25.294, `[api.rate_limit]` in `fcp-admin.toml` sets how many
requests each API token may make a minute, and how many all of a tenant's
tokens may make together:

```toml
[api.rate_limit]
enabled = true                # the default
per_token_per_minute = 600    # the default
per_tenant_per_minute = 3000  # the default
```

- Where requests are counted:
  - With `[cluster] redis_url` set, in that Redis, so every admin node
    counts together.
  - Without it, in each admin process.
- A count that cannot be made (Redis down) lets the request through.
- A request past either limit is answered 429 with `Retry-After`: the
  seconds until the minute turns.
- Every token request carries, for the tighter of the two limits:
  - `RateLimit-Limit`;
  - `RateLimit-Remaining`;
  - `RateLimit-Reset`, the seconds until the minute turns.
- Sessions and the static token are not limited.
- The API gateway's per-address limit still stands in front, for anonymous
  traffic.

## 6. Errors, request ids, times, ids and lists

Every error under `/api/v1` has one shape, whether a handler, the
authentication layer, the permission check, or a malformed body, path or
query refused the request:

```json
{ "error": "Forbidden", "code": "forbidden", "details": "Insufficient permissions" }
```

- `error` is the status's reason.
- `code` is stable and meant for programs: `bad_request`, `unauthorized`,
  `forbidden`, `not_found`, `method_not_allowed`, `conflict`, `gone`,
  `rate_limited`, `service_unavailable`, `internal`, and so on.
- `details` says what was wrong.

Every response carries `X-Request-Id`: the caller's own, when it sent one,
otherwise a fresh UUID. Quote it when reporting a problem.

A browser client may use `PATCH` and send `X-FCP-Tenant-Id`. It can read
`Retry-After`, `X-Request-Id` and the `RateLimit-*` headers.

### Times

Since v0.25.295, every time in a response is an RFC 3339 string in UTC,
such as `2026-09-29T14:31:44Z`. Fractional seconds appear only when the
time has them.

Before v0.25.295 there were four forms:
- serde's `{"secs_since_epoch", "nanos_since_epoch"}`;
- epoch seconds;
- epoch milliseconds (the SIP and flow traces);
- fractional epoch seconds.

A request may still send a time as epoch seconds, but RFC 3339 is the form
to use. A duration stays a number, and its name says the unit: `_ms`,
`_secs`, `_seconds` or `_days`.

The events WebSocket and webhook payloads are not covered by this rule;
their shape is `docs/API_EVENTS.md`'s.

### Ids

An id whose type has a prefix is written with it: `tenant-…`, `user-…`,
`dev-…`, `site-…` and so on (`fcp_core::identifiers`). A request may send
a bare UUID or the prefixed form, and both are read.

Four resources have ids with no prefixed type, and these are plain UUIDs:
- audit-log entries;
- billing transactions;
- rate plans;
- voicemail mailboxes.

`api_postgres.rs` checks every response its tests receive against these
two rules (`check_shape`).

### Lists

Since v0.25.296, every list answers in one envelope:

```json
{ "data": [ … ], "next_cursor": "b3Bh…", "total": 42 }
```

- `?limit=` sets the page size: 100 by default, at most 1000. A limit of
  0 or over 1000 is a 400.
- `?cursor=` takes the `next_cursor` of the page before. The cursor is
  opaque, and one the API cannot read is a 400.
- `next_cursor` is null on the last page.
- `total` is present where the server knows it. For CDRs it may be an
  estimate, and `total_exact` says which.

Pages are keyset pages, so a row added or removed between two pages
neither repeats nor skips. Each list keeps a fixed order:
- **Named resources** (users, devices, trunks, …) by their name, number
  or address.
- **Routing rules** by priority.
- **Logs** (the audit log, the deletion log, emergency calls, hoteling
  sessions, API tokens, CDRs), newest first.

Flows, recordings, flow traces and billing transactions page by position,
for now, behind the same opaque cursor.

Some things are not lists and keep their own shapes:
- single objects;
- statistics;
- the cluster view (`{available, nodes}`);
- the live SIP trace (`{messages}`);
- downloads.
