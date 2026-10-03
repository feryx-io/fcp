# Users

A user's lifecycle, as it stands after phase 5b of
`docs/IDENTITY_AND_DEVICES.md`. Later parts of phase 5 add groups (5c)
and renaming (5d).

## 1. One status

A user's `account_status` is one of four values.

| Status | Web and API | Phones, voicemail, provisioning, hoteling | API tokens they made | How it is set |
|---|---|---|---|---|
| `active` | yes | yes | yes | — |
| `suspended` | no | no | no | an admin (`PUT /users/{username}`), later SCIM |
| `locked` | no (password sign-in refused) | **yes** | yes | wrong passwords or codes, `max_failed_attempts` times; `POST /users/{username}/unlock` or the lock's expiry clears it |
| `deprovisioned` | no | no | no | `DELETE /users/{username}`; only `POST /users/{username}/restore` clears it (§4) |

- **Locked is only the web sign-in's guard.** It stops guessed passwords.
  Phones keep working, because anyone could otherwise take a person's
  phones offline by typing a wrong password five times.
- **`enabled` is the old word for this.** It is kept for one release and
  always says the same as the status.
  - `enabled: false` suspends.
  - `enabled: true` resumes a suspended user.
  - Every user reads with `enabled` equal to "not suspended".
  - Migration 096 brought existing rows into line: a disabled user became
    suspended.
- **The status is checked.** A value that is not one of the four is
  refused with 400, and `deprovisioned` cannot be set by an update. It used to become `active`; that was how the admin
  UI's old "Disabled" choice made a user active. `disabled` is still
  read, as `suspended`.

## 2. Suspending

Suspending a user takes effect at once:

- every web session of theirs ends;
- every registration their phones hold is removed, under their username,
  their extension, and the SIP name of every device they own. Each
  removal is announced (`registration.unregistered`), so the call
  managers forget it;
- their hoteling session ends, and the host phone goes back to its owner;
- `user.disabled` is published.

From then on, while suspended:

- their phones are refused, both the user's own credentials and those of
  devices they own;
- their calls are refused at routing;
- their voicemail cannot be opened with the PIN;
- their devices are not provisioned, and a hoteling session lends them
  no phone;
- the API tokens they made are refused.

Resuming publishes `user.enabled`. Phones register again by themselves.

A call manager remembers credentials for up to a minute. The admin's
`system.config_changed` for the user clears that memory where the event
reaches the call manager (`[events] use_redis`). Elsewhere a suspension
reaches the phones within a minute; the registrations already held are
dropped at once.

## 3. What else changed

- **`user.updated`** names the fields that changed, in `changed_fields`:
  `account_status`, `enabled`, `display_name`, `pin` and so on. A
  secret's name is given, never its value.
- **`max_users`** on a tenant is enforced. A new user past it is refused
  with 409, whether made by the API or at a first sign-in through an
  identity provider (`tenant_full`).
- **Listing.** `GET /users` is answered from SQL for the tenant alone, and
  takes `q` (a word in the username, display name or email) and `status`.
  The dashboard and usage counts are counted in SQL.

## 4. Deleting a user

A delete does not remove a user at once. It **deprovisions** them:

- they are suspended, with everything §2 does;
- `account_status` becomes `deprovisioned`, and `deprovisioned_at` is
  stamped;
- `user.deprovisioned` is published, naming `purge_after`;
- nothing but a restore changes them (an update is refused with 409).

They are **purged** when the tenant's grace period ends. A tenant sets
it with `offboarding` on `PUT /tenants/{id}` or the Tenants page:

| Key | Default | Meaning |
|---|---|---|
| `grace_days` | 30 | Days from the delete to the purge, at most 3650. 0 purges at once. |
| `meetings_owner` | none | The username a purged user's meetings move to; none leaves them without an owner. |

Until the purge:

- `POST /users/{username}/restore` brings the user back, active, with
  everything they had. Phones register again by themselves.
- A platform admin may `POST /users/{username}/purge` to purge at once.
- `GET /users?status=deprovisioned` lists who is waiting.
- The user still holds their username and counts toward `max_users`.

### What the purge does

`fcp-admin` looks once an hour for users whose grace period has ended
(one replica at a time, under the `fcp:offboarding` lock). Each purge is
one database transaction:

| What | Purge |
|---|---|
| Devices they own | Kept by the tenant, unassigned |
| Their lines | Released: the binding is removed, the number stays the tenant's |
| Extensions that reach them | Removed |
| Their mailbox | Kept, closed and off its line, renamed `offboarded-…`; removed once the tenant's voicemail retention has emptied it |
| Their calling settings | Removed |
| Other users' forwards, find-me steps and assistant lists naming them | The entries naming them removed |
| API tokens they made | Removed |
| Hoteling sessions | Removed |
| Queue seats, and queues' agent lists | Removed |
| Their declared emergency location | Removed |
| Registrations and presence under their names | Removed, and announced |
| Meetings they own | Moved to `meetings_owner`, or left without an owner |
| Their profile picture | Queued for the file reaper |
| Sign-in sessions, second factor, identities | Removed with the user |
| Call records, recordings, voicemail messages, audit entries | Untouched: retention and erasure decide these |

The purge writes a `user` entry to the deletion log with cause
`offboarding` and the counts, and publishes `user.deleted`.

- **Erasure runs first.** While an erasure request for the user is open
  (received, running, waiting for confirmation or confirmed), the purge
  waits for it.
- **Sites.** The purge's changes to synced rows (the user, devices,
  lines, extensions, calling settings, meetings) reach a site through
  its link. When the user's own deletion arrives, the site purges what
  only it holds: tokens, hoteling, queue seats, the declared location,
  registrations and the mailbox.
- **A mailbox outlives its line.** A line deleted on its own no longer
  takes the owner's mailbox and messages with it (migration 097).
