# Users

A user's lifecycle, as it stands after phase 5 of
`docs/IDENTITY_AND_DEVICES.md` (5a status, 5b deletion, 5c groups, 5d
renaming).

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

## 5. Groups

A tenant's groups hold users. A group has a name (unique in the tenant),
and optionally:

- **a role** its members hold besides their own;
- **an identity provider's group** it follows (`idp_group`);
- **queues** that take their agents from it.

`/groups` makes, changes and deletes them; `/groups/{id}/members` adds
and removes members by username; `/users/{username}/groups` lists a
user's. Only a tenant administrator makes and fills groups
(`group_manage`); whoever reads users may look (`group_read`).

### Roles from groups

A user may do what their own role allows **and** what each of their
groups' roles allows. The roles are not a ladder (a user admin manages
users and a telecom admin does not; a telecom admin manages queues and a
user admin does not), so someone in both groups may do both.

- A group cannot give `super_admin`, nor `department_admin` (which needs
  the member's department).
- It takes effect at once, both ways: every request reads the user's
  groups afresh, so removing someone from a group takes its role away on
  their next request.
- Where one role is shown (the sign-in answer, `/auth/me`, the events
  server), it is the most privileged one held.
- A token a user makes may carry what any of their roles' tokens may,
  and loses what they lose.
- The password policy and the second-factor requirement count group
  roles: someone who is an administrator by a group is one.

### Groups that follow an identity provider

A group with `idp_group` set follows the group of that name at the
tenant's identity providers. At every sign-in through one that carries a
groups claim, the user is added to each following group the provider
names and taken out of the rest. Groups without `idp_group` are the
tenant's own; no sign-in touches them. See `docs/SSO.md` §2.

### Queues from groups

A queue's `agent_group_id` names a group whose members may log in to it
with the agent login code (`*50`), besides the extensions in `agents`.
Membership is asked at each login, so it follows the group as it is.
Changing which group a queue names reaches a call manager when its
queues are reloaded (`POST /queues/reload` or SIGHUP), as any change to a
queue's settings does. Deleting a group leaves its queues with none.

### Sites and purges

Groups and memberships are part of a site's slice: a site's users hold
their groups' roles there too, and a site's admin cannot change them. A
purged user's memberships go with them.

## 6. Renaming a user

`PUT /users/{username}` with a new `username` renames the user. SCIM's
renames (phase 6) come through the same door.

- **They are the same user.** Everything that names them by id is theirs
  still: sign-in sessions, API tokens, groups, devices, lines, extensions,
  calling settings, hoteling, their mailbox. Their sessions from before
  carry on.
- **They sign in by the new name** (or by email, as before).
- **Their phones keep their SIP name.** A phone's digest password is a
  hash over the name it registers as, which FCP cannot recompute without
  the password. So a user has a SIP name (`sip_username` in the API), which
  is their username until a rename and the old username after it. Their
  phones register, call and are called exactly as before; their
  registrations, voicemail box, presence and queue seats, all named by it,
  carry on untouched. Provisioning gives phones the SIP name.
- **What named them by username follows them**: meetings they own, and API
  tokens recorded under their name (rewritten to their id).
- `user.renamed` is published with the old and the new name, and
  `user.updated` names `username` and `sip_username` among its changes.

The SIP name belongs to them while they exist:

- no new user may be created with it, nor anyone renamed to it, nor a
  device given it as its own SIP name;
- they may be renamed back to it.

A new SIP password (`PUT /users/{username}/password` when the web
password is the SIP one, or `/digest-password`) is hashed over the SIP
name. There is no way yet to change the SIP name itself: it would move
their registrations, mailbox and presence, and needs their phones
re-provisioned.

A rename reaches sites with the user's row, matched by id.

## 7. A user's extension

A user's `extension` (`POST`/`PUT /api/v1/users`, or SCIM's FCP extension)
is the extension that dials them: routing reads the extensions table, and
the API keeps a row there for the number the user has — made with the user,
moved when the number changes, removed when it is cleared (an empty
string). A number that already dials something else (another user, a
number, a device, a service) is refused with 409, a malformed one with
400, before anything is saved. Before v0.25.354 the field was a label only,
and an extension had to be made separately (`/api/v1/extensions`).

## 7a. Listed in the directory

Every user is in the tenant's directory for its users (`GET /me/directory`,
the apps' Directory tab) unless taken out (D8 finding 27, v0.25.471):
`directory_listed` on `POST`/`PUT /api/v1/users`, a switch on the user's
page, on by default. An unlisted user still rings on their extension, still
lights busy lamps others have set up, and is seen by administrators
everywhere; only the users' directory leaves them out.

A user changes their own listing only when the tenant lets them: the
tenant's `directory.users_may_unlist` (off by default; "Users may change
their own listing" on the tenant form). Then `directory_listed` on
`PUT /me/profile` changes it, as the portal's Settings page and the app do;
otherwise that answers 403. `GET /me/profile` and `/me/softphone` say both
`directory_listed` and `may_change_directory_listing`. The column is
`users.directory_listed` (migration 124), synced to sites with the user.

## 7b. First and last name

A user has a `first_name` and a `last_name` (#343, migration 138) beside
their display name. They are set:

- by an administrator, on `POST`/`PUT /api/v1/users` and the Users page;
- by the user, on `PUT /me/profile` and the portal's Settings page. A user
  the tenant's SCIM provisioner keeps is refused (403), because their name
  is the directory's;
- by SCIM, from `name.givenName` and `name.familyName` (`docs/SCIM.md` §3);
- by single sign-on, from `given_name` and `family_name` or the SAML
  equivalents, at the user's creation and at every sign-in that carries
  them (`docs/SSO.md` §2).

Names are trimmed, an empty string clears one, and each is at most 100
characters on one line.

**The display name stays, and reads as the names when unset.** Every
answer that shows a user has `display_name` as they are shown: their own
display name, else "first last", else whichever of the two they have. That
holds on `/users`, `/me/profile`, the directory, provisioning's line
labels, the routing engine and SIP's names for the user. A display name
that is only the first and last name is not kept. Unset, it follows the
names when they change. To give someone a name other than their own, set
the display name to it.

A change of name makes the user's personalized voicemail greeting again
(`docs/VOICEMAIL.md`).

## 8. Who may give a role

Making a user, changing their role, setting their password or SIP
password, removing their SIP password, resetting their second factor,
unlinking their identity, unlocking, deleting or restoring them all need
two things: the route's permission, and that the user's role — and any
role being given — holds nothing the caller does not hold
(`fcp_core::permissions::roles_may_assign`). The platform administrator
may give any role; no one else may give `super_admin`. A caller's roles
are their own and their groups' together (§5).

What that comes to, by role:

| Caller | May give, and act on users holding |
|---|---|
| `super_admin` | every role |
| `tenant_admin` | every role but `super_admin` |
| `telecom_admin` | `telecom_admin`, `end_user` |
| `user_admin` | `user_admin`, `end_user` |
| `department_admin` | `department_admin`, `end_user` |
| `read_only` | `read_only`, `end_user` |

An API token is held to its own permissions the same way, and never gives
`super_admin`; a personal token gives nothing. A group's role is checked
when the group is made or changed and when someone is added to it.
Refusals are 403, naming the role.

Before v0.25.405 a user administrator could set a tenant administrator's
password and sign in as them, and a tenant administrator could make a
`super_admin` — a platform administrator over every tenant.
