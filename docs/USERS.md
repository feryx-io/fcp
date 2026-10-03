# Users

A user's lifecycle, as it stands after phase 5a of
`docs/IDENTITY_AND_DEVICES.md`. Later parts of phase 5 add
deprovisioning (5b), groups (5c) and renaming (5d).

## 1. One status

A user's `account_status` is one of three values.

| Status | Web and API | Phones, voicemail, provisioning, hoteling | API tokens they made | How it is set |
|---|---|---|---|---|
| `active` | yes | yes | yes | — |
| `suspended` | no | no | no | an admin (`PUT /users/{username}`), later SCIM |
| `locked` | no (password sign-in refused) | **yes** | yes | wrong passwords or codes, `max_failed_attempts` times; `POST /users/{username}/unlock` or the lock's expiry clears it |

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
- **The status is checked.** A value that is not one of the three is
  refused with 400. It used to become `active`; that was how the admin
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
