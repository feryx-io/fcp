# API tokens and permissions

How programs authenticate to the admin API (`/api/v1`), what a token can
reach, how often it may call, what an error and a time look like, and
what changed in v0.25.293–296 (`docs/DEVELOPER_SURFACE.md` §3, phases 1
and 2).

The whole API is described by its OpenAPI contract, since v0.25.297:
- `GET /api/v1/openapi.json` on any admin serves the spec, no token needed;
- `/admin/api` renders it as a reference;
- `docs/openapi.json` is the same spec, kept equal to the code by a test.

## 1. Three kinds of credential

Every request carries `Authorization: Bearer <credential>`.

| Credential | What it is | Reaches |
|---|---|---|
| The static token (`[api] auth_token`) | The platform's own key | Everything, in every tenant (`x-fcp-tenant-id` picks one) |
| A session (`POST /auth/login`) | A signed-in user's access token, 15 minutes, renewed from the session's cookie until the session ends (§7) | What the user's role permits |
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
- A token that may make or change users gives a role, or acts on a user
  holding one, only when the role holds nothing the token lacks, and never
  gives `super_admin` (`USERS.md` §8).

### The maker answers for the token

A token names its maker by id, and shows them by username (`created_by`).
A user deleted and made again with the same name is a different user, and
not the maker of the first one's tokens. A token made before makers were
ids names a username; the first time it is used it is matched to that
user in its tenant and rewritten to their id. Every time the token is
used, its permissions are checked against what the maker holds now:

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
- **The services' configuration (`/config`)** takes `system_config`, which
  only a super admin holds, to read as well as to change: the nodes seed it
  with their own credentials (the media server's API tokens, an AI key, a
  Redis URL). Until v0.25.404 reading it took `system_view_logs`.
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

`identity_provider_manage` (`/identity-providers`, `docs/SSO.md`) came
later, with single sign-on. Only `super_admin` and `tenant_admin` hold
it: a provider's groups can make anyone a tenant admin.

`group_read` and `group_manage` (`/groups`, `/users/{username}/groups`,
`docs/USERS.md` §5) came with groups. `group_manage` is `super_admin`'s
and `tenant_admin`'s alone: a group's role is its members' too.
`group_read` is also held by `telecom_admin` (who picks a queue's group),
`user_admin`, `department_admin` and `read_only`.

`branding_manage` (`/branding`, `docs/BRANDING.md`) came with the
softphone's branding. Only `super_admin` and `tenant_admin` hold it.

`scim_provision` (`/scim/v2`, `docs/SCIM.md`) came with SCIM. Only
`super_admin` and `tenant_admin` hold it: the provisioner makes users,
administrators among them.

A user's group roles count wherever their own role does: a session's
permissions are what their own role and their groups' hold together,
and a token they make may carry what any of those roles' tokens may.

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
their shape is `docs/API_EVENTS.md`'s (a delivery's envelope is
`docs/WEBHOOKS.md`'s).

### Ids

An id whose type has a prefix is written with it: `tenant-…`, `user-…`,
`dev-…`, `site-…` and so on (`fcp_core::identifiers`). A request may send
a bare UUID or the prefixed form, and both are read.

Six resources have ids with no prefixed type, and these are plain UUIDs:
- audit-log entries;
- billing transactions;
- rate plans;
- voicemail mailboxes;
- voicemail messages;
- webhook deliveries.

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

## 7. Sessions

A sign-in is a session: a row (`login_sessions`) naming the user by id,
their tenant, how they signed in, when it ends and whether it was revoked
(`docs/IDENTITY_AND_DEVICES.md` phase 1).

- **Signing in.** `POST /auth/login` answers an access token
  (`expires_in`: 900 seconds by default) and sets the session's refresh
  secret in a cookie, `fcp_refresh`: `HttpOnly`, `SameSite=Strict`, sent
  only to `/api/v1/auth`, and `Secure` when the request came over HTTPS
  (`X-Forwarded-Proto: https`).
- **Refreshing.** `POST /auth/refresh` takes the cookie and nothing else,
  and answers a new access token with the secret turned over. The previous
  secret is still taken within ten seconds (two tabs refreshing together).
  After that it means two holders, and the session is revoked. The admin
  UI refreshes a minute before its token runs out, and after a 401, so a
  reload keeps you signed in.
- **Ending.**
  - `POST /auth/logout` revokes the session and clears the cookie.
  - A password change revokes the user's other sessions; an
    administrator's password reset revokes them all.
  - A user disabled, suspended or locked is signed out everywhere, and
    stays signed out if enabled again.
  - A deleted user's sessions go with them.
  - A session lasts 12 hours however often it is refreshed.
- **Apps.** FCP's apps (the softphone) sign in through the browser and keep
  a session of their own, refreshed with a token rather than a cookie and
  lasting 90 days from its last use: `docs/APP_SIGN_IN.md`.
- **The keys.** Access tokens are signed with `[api.session] signing_key`
  (32 characters or more, distinct from `auth_token`;
  `FCP_ADMIN_SESSION_SIGNING_KEY`).
  - Unset, the key is derived from `auth_token` as before, and changing
    that token signs everyone out.
  - To rotate: move the old key to `previous_signing_keys` and set a new
    one. Tokens the old key signed are accepted until they expire.
  - The events server verifies the same tokens. Give it the same key as
    `[auth] session_signing_key` (`FCP_EVENTS_SESSION_SIGNING_KEY`) and
    `previous_session_signing_keys`.
- **Lifetimes.** `[api.session] ttl_secs` (43200) and `access_ttl_secs`
  (900, between 60 and 3600).
- **Recorded.**
  - Every sign-in attempt is an audit row under the name tried
    (`actor_kind` `sign_in`), in the user's tenant when the name was
    found.
  - `user.signed_in` and `user.sign_in_failed` are published, the latter
    with its reason: `bad_credentials`, `locked`, `disabled` or
    `suspended`.
  - Ended sessions are deleted a month after they end.
- **Tokens from before.** An access token made before sessions were rows
  names a username and lasts 24 hours. It is honoured until it expires.
- **Users by id.** `GET /api/v1/users/by-id/{id}` reads the user a token,
  a session or an event names.
- **Signing in by email.** A tenant's login domains (`login_domains`, set
  by a platform admin, unique across tenants) make `jane@acme.com` that
  tenant's.
  - Login takes the email, matched as a username and then by the users'
    email in that tenant. An email several users share is refused like a
    wrong password.
  - `POST /auth/discover {login}` answers the tenant an email's domain
    belongs to and the ways in (`methods`). It never says whether a user
    exists; the login page shows "Signing in to <tenant>".
- **Users without a password.** A user made without `password` has none:
  a password never signs them in, and their SIP password is generated
  (`digest_password`, `sip_provisioning_password`).
- **A tenant's password policy.** `sign_in.password_login` is `allowed`,
  `admins_only` or `disabled`; a platform super admin is never shut out.
  - It is checked only after the password is right, so a stranger learns
    nothing from it.
  - Refused at sign-in as 403 with `user.sign_in_failed`
    (`password_not_allowed`).
  - The API takes nothing but `allowed` until the tenant has an identity
    provider (phase 3) to sign in with instead.
- **Single sign-on.** A tenant's people can sign in through its OpenID
  Connect provider (`docs/SSO.md`). The session that follows is like any
  other, with `method: "oidc"`. Once the tenant has an enabled provider,
  `password_login` can be `admins_only` or `disabled`.
- **A second factor.** A user may add a one-time code (TOTP, RFC 6238:
  six digits, thirty seconds, any authenticator app) to their password.
  - Set up from the portal's settings: `POST /me/totp/enroll` answers the
    secret and its `otpauth://` URI (the page shows it as a QR code);
    `POST /me/totp/confirm {code}` turns it on and answers ten recovery
    codes, shown once.
  - Signing in, a right password then answers `mfa_required: "totp"` and
    an `mfa_token` instead of a token: a ticket good for five minutes and
    for nothing but `POST /auth/login/totp {mfa_token, code}` (or
    `recovery_code`). Each code is good once; each recovery code once.
  - Wrong codes count toward the lockout as wrong passwords do, and a
    right password no longer resets the count — only a whole sign-in
    does.
  - `sign_in.require_totp` (`none`, `admins` — every role but end user —
    or `everyone`) makes it compulsory: a user it applies to without one
    is answered `mfa_required: "totp_enrollment"` and sets one up before
    they are in (`POST /auth/totp/enroll` then `/auth/totp/confirm`,
    both with the ticket).
  - `POST /me/totp/disable {code}` turns it off (not when required);
    `POST /me/totp/recovery-codes {code}` replaces the recovery codes.
  - An administrator with `user_update` resets a user's
    (`DELETE /users/{username}/totp`) for a lost phone.
  - The secret is sealed at rest (`user_totp.secret`, re-sealed by the
    encryption job). It is not synced to a site: a site's users set up
    their own there.
  - An expired password changed at sign-in goes through the second factor
    like any password. A password that has expired no longer answers a
    token at all, only `password_expired`.

