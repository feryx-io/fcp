# Presence a person sets

D8 finding 33 (2026-10-29): "Right now there is a 'ready button' that always says
ready. Even when on a call the presence status doesn't change. That should be updated
to a presence setting that automatically updates with the user and allows for the
user to create their own status: away, busy, WFH, etc."

Agreed the same day:

1. A user's own statuses are kept **on their account**, so every app they sign in to
   offers the same list.
2. Presence changes automatically for **calls** (ringing, on a call), **do not
   disturb**, **idle** (away after a while with no keyboard or mouse) and **signing
   out** (offline when no device is registered).
3. **Only do not disturb stops calls.** Away, Busy and a user's own statuses are what
   others see; calls still ring.

## 1. What exists

Anvil's "Ready" chip is not presence. It is the SIP registration (`home.dart`
`_RegistrationChip`): ready while the app is registered, whatever the person is doing.

FCP's presence is per SIP address, not per person, and only in memory:

- **One `PresenceManager` per binary** (admin, call manager, presence server), each
  fed differently and kept in step only by Redis pub/sub. In the shipped images only
  the call manager has the pub/sub compiled in (`fcp-admin` and `fcp-presence-server`
  build without the `redis` feature). The presence server also sets its pub/sub after
  handing clones to its SIP handler, so those clones never publish.
- **Automatic signals write over each other.** A registration sets `open`, an
  unregistration `closed` (one device of two signing out shows the person offline),
  a call `ringing`/`on_the_phone` and its end `open`, whatever was there before.
  Expired and refreshed registrations are ignored, so a phone that drops off stays
  open. After a restart nobody is open until they register again.
- **The admin's mirror files everyone under the default tenant**
  (`fcp-admin/src/main.rs` `run_presence_listener`), so `/me/directory` in any other
  tenant shows no presence.
- **Nothing is stored.** `presence_presentities` and `PresenceStore` exist and are
  never written.
- **Calling DND and presence know nothing of each other.** `CallingSettings.dnd`
  (`*78`/`*79`, `/me/calling`) sends calls to the busy treatment; presence `busy`
  takes an agent out of queues (`presence_checker.rs`) and hunt groups (`is_busy`).
- **PIDF** carries a note and RPID activities for away, busy and on-the-phone;
  parsing a PUBLISH reads the state from the note's exact text, so `busy` with the
  note "WFH" is read as `open`.
- **`presence.status_changed`** on `/me/events` carries the state, not the note.

## 2. The model

**Presence belongs to a person** (a user), not to an address. A person has one
presence, made from what they chose and what is happening:

| Layer | Source | Shown as |
|---|---|---|
| Signed out | no registration on any of the user's addresses (the registration store, not events) | `offline` |
| A call | ringing or up on one of their lines | `ringing` / `on_call`, over the chosen status |
| Do not disturb | `CallingSettings.dnd` | `dnd` |
| Idle | an app says its person is idle | `away`, noted "Idle", while the chosen status is `available` |
| Chosen | the status the person picked | its state and label |

The first that applies wins, except that a call keeps the chosen status's label beside
it ("On a call · WFH").

**States** (what colour, what others' apps and phones see): `available`, `away`,
`busy`, `dnd`, plus the automatic `ringing`, `on_call` and `offline`.

**Statuses** are what a person picks from: the four built-in ones (Available, Away,
Busy, Do not disturb) and up to 20 of their own. A status of their own is a label (up to
40 characters, "WFH", "At lunch") on one of the four states. Picking one sets the
person's chosen status; an optional note (up to 100 characters) can be set with it, and
an optional time it clears at (`until`; the apps offer 30 minutes, 1 hour, 2 hours,
today and never). At `until` the chosen status becomes Available, and DND goes off if
the status had turned it on.

**Do not disturb is one switch.** Picking a status whose state is `dnd` turns
`CallingSettings.dnd` on; picking any other turns it off. Turning DND on elsewhere
(`*78`, `/me/calling`, the app's DND switch) shows `dnd` over the chosen status; off
again (`*79`) shows the chosen status, and if that status was itself a `dnd` one, the
chosen status becomes Available.

**Idle** is the app's to say: `PUT /me/presence` with `idle: true` from the app that
saw no input for the person's idle time (a setting in the app, 10 minutes by default),
`false` when they are back. Idle is kept per app session and lapses after 15 minutes
unless repeated, so an app that dies does not leave its person away. Any of the
person's apps not idle makes them not idle.

**Busy, away and own statuses no longer take anyone out of queues or hunt groups.**
Queues and hunt groups skip a member in DND (`CallingSettings.dnd`) or on a call, which
is what "only DND stops calls" means for them.

## 3. FCP

**Storage** (migration 125, journaled so a site has its users' statuses):

- `user_presence`: `user_id` (key, deleted with the user), `tenant_id`, `status`
  (`available`/`away`/`busy`/`dnd` or a status id of the user's), `note`, `until`,
  `updated_at`. No row is Available. The admin clears a status whose `until` has
  passed (a timer per person, and once at start).
- `presence_statuses`: `id` (`status-` and a UUID), `user_id`, `label`, `state`,
  `position`, `updated_at`.

`fcp_core::presence_status` holds the model, the store trait and one pure function,
`effective(chosen, dnd, calls, idle, registered) -> Presence`, which every reader uses.

**Routes** (signed in as the user, `Open` like every `/me` route):

- `GET /api/v1/me/presence`: the person's effective presence, their chosen status and
  note, and their statuses.
- `PUT /api/v1/me/presence`: `{status, note, until}` and/or `{idle}`; sets DND as
  above. `until` is RFC 3339, in the future, within 7 days.
- `GET/POST /api/v1/me/presence/statuses`, `PUT/DELETE …/statuses/{id}`, and `PUT
  …/statuses/order`. Deleting the status in use makes the chosen status Available.
- An administrator reads anyone's at `GET /api/v1/users/{username}/presence`
  (`user_read`) and the admin UI's user page shows it.

**Who works it out.** The admin keeps each person's effective presence, per tenant,
from the database (chosen status, DND, registrations) and the events it already
receives: registrations (registered, refreshed, expired, unregistered: re-read from the
store, so one device of two leaving changes nothing), calls (initiated, answered, ended,
failed; by the user each leg belongs to, in the call's tenant) and `user.calling_changed`.
It replaces the admin's address-keyed `run_presence_listener`. On start it reads every
user's chosen status and registrations, so a restart loses nothing.

**What others see.**

- A change is announced as `presence.user_changed` (`user_id`, `username`,
  `extension`, `state`, `label`, `note`, `tenant_id`), carried by `/me/events` while the
  tenant shows its directory, and (phase 2) by tenant webhooks.
- `/me/directory` gives each person `presence`, `presence_label` and `presence_note`
  from the same map, and stops reading the per-address mirror.
- **Phones.** The call manager feeds each of the person's addresses in its presence
  manager from `presence.user_changed`: PIDF basic `closed` only for `offline`, the
  RPID activity (away, busy, on-the-phone; `dnd` as busy) and the label as the note.
  The dialog package (busy lamps) stays driven by calls. Its own registration and
  call listener stops setting open, closed and on-the-phone on top.
- The admin's presence pub/sub is no longer needed for any of this; the per-address
  routes (`/api/v1/presence/*`) stay for SIP presentities that are not users.

**Sites.** The two tables are journaled down and shipped up like contacts
(`docs/RECORD_AND_CONTACTS.md` §3): a status chosen at a site reaches the cloud and back.
Each side works out presence from its own registrations and calls.

## 4. Anvil

- The "Ready" chip becomes the **presence chip**: the person's state (dot and label),
  "On a call" while one is up. Tapping it opens the status menu: the four built-in
  statuses, the person's own, "Set a note…", "Clear after…" (30 minutes, 1 hour,
  2 hours, today, never), and "Edit statuses…" (add, rename, choose
  the state, reorder, delete). Registration trouble still shows, as the chip's warning
  ("Not connected"), as it does today when not registered.
- **Idle** on a desktop: no keyboard or mouse for the person's idle time (Settings,
  10 minutes by default, or off) sends `idle: true`; input again sends `false`. Read by
  the OS (`GetLastInputInfo`, `CGEventSourceSecondsSinceLastEventType`, the X11 screen
  saver extension; not on Wayland, which says so). A phone app is never idle.
- A plain SIP account (no FCP) keeps the registration chip as today.
- The CLI: `presence [status|note <text>|idle on|off]` and `presence statuses …`.

## 5. Phases

1. ✅ **FCP v0.25.473: the person's presence.** Migration 125 (`user_presence`,
   `presence_statuses`), `fcp_core::presence_status` (the model and `effective`),
   `/me/presence` and its statuses, DND both ways, `fcp_api_core::people_presence` run
   by the admin (registrations read from the store, `call.initiated` for ringing,
   `presence.user_call` from the call managers' counted parties for calls,
   `user.calling_changed` for DND, idle per app session, `until` and a re-read every
   five minutes on a 30-second tick), `presence.user_changed` on the admin's bus and
   `/me/events` (everyone's while the directory is shown, a person's own always),
   `/me/directory` (`presence_label`, `presence_idle`), `GET /users/{username}/presence`
   and the line in the admin UI's calling dialog, the SDKs. Tests: `api_postgres`
   `presence.rs` (`a_user_keeps_their_own_statuses_and_picks_one`,
   `do_not_disturb_is_one_switch`,
   `others_see_signing_out_calls_and_idleness_over_the_chosen_status`,
   `a_status_clears_itself_when_its_time_comes`) and `fcp_core::presence_status`'s own.
   As built: `presence.user_changed` stays on the admin's bus (its apps and UI);
   forwarding it to the events server, for the call managers and webhooks, is phase 2,
   and so are queues and hunt groups, which ask by extension without a tenant and
   cannot wait on the database.
2. **FCP: phones, queues and sites.** `presence.user_changed` forwarded to the events
   server once (no echo back onto the admin's bus) and delivered to tenant webhooks;
   the call manager's presence fed by person, its own overwriting listener and the
   admin's address-keyed one retired, PIDF parsing reading RPID activities; queues and
   hunt groups skipping DND and calls only, from the person's presence the call manager
   now hears; the tables synced with sites. Tests: `fcp-call-manager/tests/presence.rs`
   (a watcher's NOTIFY says away with the label, on-the-phone during a call, and away
   again after; a queue agent who chose Busy still offered calls), `fcp-link/tests`.
3. **Anvil: the presence chip**, statuses, notes, idle, the CLI.

## 6. Agreed (2026-10-29)

1. **Appear offline:** not now. Four built-in statuses and the person's own.
2. **A status that clears itself:** yes, with presets (30 minutes, 1 hour, 2 hours,
   today, never) as `until` on the chosen status.
3. **The admin UI:** read only; only the person changes their status.
4. **Queues and hunt groups:** skip a member in DND or on a call only; Busy, Away and
   a person's own statuses do not take them out.
