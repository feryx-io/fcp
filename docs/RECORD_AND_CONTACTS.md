# Record on demand and personal contacts

D8 findings 20 and 21 (2026-10-26). The questions in §5 were agreed the same day.

- **#20:** "Add a record button to allow a user to record the call. It should be a
  setting enabled by the admin for the user. It should also play the recording
  started announcement."
- **#21:** "Add an ellipsis sub-menu button that has other features, Create a contact
  for now, but should be able to add more features down the road as needed. We should
  be able to create local and server based contacts."

## 1. What exists

**Recording.** Every recording is a row with a source (`fcp_core::recording::
CallRecordingSource`: api, flow, queue, user, number, platform). One recorder takes
all of them through the tenant's consent step: the announcement plays to both
parties and ends before the first sample (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1).
`POST /api/v1/calls/{id}/recording/{start,stop}` needs `call_control`, an
administrator's permission, and is forwarded to the node holding the call. A user
has `media_preferences.record_all_calls`, but the admin UI does not show it. Only an
anchored call can be recorded. Calls through the call manager are anchored by default
since the NAT work.

**Which call.** Anvil learns FCP's id for an incoming call from `X-FCP-Call-Id` in
the INVITE. It is not told the id for a call it places.

**Contacts.** FCP has no personal contacts. `/me/directory` is the tenant's people
and extensions. Anvil's People tab shows that directory and has no contacts of its
own.

## 2. Record on demand (#20)

**The setting.** `media_preferences.record_on_demand: bool`, false by default, set by
an administrator only. `PUT /users/{username}` now takes `media_preferences` (whole)
and every user view shows it. The user dialog gets both recording switches, "Record
every call" and "May record calls". `/me/softphone` gains `may_record`, so Anvil shows the
button only to users who have it.

**The request:** `POST /api/v1/me/calls/{call_id}/recording/start` and `…/stop`,
signed in as the user (agreed). The call id is FCP's. Anvil has it for an incoming
call from `X-FCP-Call-Id`. For a call Anvil places, the node puts `X-FCP-Call-Id` on
its 200 OK to a caller who is one of the tenant's users. A carrier is not told. The API forwards the request to the node holding the call, as
`/calls/{id}/recording` does (`SupervisionRequest::Control`), naming the user. The
node checks that the user is one of the call's parties: the effective user of either
leg, so a hoteling guest counts as the host.

- **`start`:** checked against the user's `record_on_demand` (403 without it). The
  node starts the recorder with the new source `on_demand` and `started_by` = the
  user. The consent announcement plays to both parties, which is the "recording
  started" announcement #20 asks for. The request returns once the recorder runs.
  It answers 404 when the call is not the user's, 409 when the call is not anchored
  or is already recorded, and 409 when consent failed under a policy that will not
  record without it.
- **`stop`:** stops only a recording that the user started on demand (agreed). A
  recording from record-all, a queue, a number, a flow or the API answers 409 and
  keeps recording. A user cannot switch off compliance recording.
- **Events:** `recording.started` carries the source, and `started_by` names the
  user for an `on_demand` recording. The recording's row has the source
  `on_demand`.
- **Permissions:** the route needs only a signed-in user (`Open`, as every `/me`
  route), and the setting is the gate. The API answers without the file's path.

SIP INFO `Record: on|off`, the request desk-phone record keys send, is left for
later. It would give the VVX's key the same feature.

**Anvil.** A "Record" toggle on the call screen when `may_record` is set, a red
"Recording" mark while it runs, and the request sent through `anvil-fcp` with the
app's session. The toggle follows the answer: 200 sets it, an error shows the
reason. `anvil-cli` gets `record on|off`. A call FCP does not name (a plain SIP
account) has no button.

## 3. The "More" menu and contacts (#21)

**The menu.** A "⋯" button on the call screen, and on each Recents and People row,
opens a menu built from a list of actions. Adding a feature later means adding an
entry. The first entry is **Create contact**, filled in from the other party's
number and name.

**One contact shape** on both sides: `{id, name, company, numbers: [{label, number}],
email, notes}`.

**Local contacts** live in Anvil's data directory (`settings.json`, beside its other
settings). They work with any account, including a plain SIP account, and
stay on that one device.

**Server contacts:** a new FCP resource.

- `personal_contacts` (migration 122, `fcp_core::contact`): `id` (`contact-` and a
  UUID, made by whoever writes the contact first), `owner_user_id` (a user's contacts
  are deleted with the user), the fields above, `updated_at`, and `deleted_at` (a
  tombstone, so a delete reaches the other side).
- `GET/POST /api/v1/me/contacts` and `GET/PUT/DELETE /api/v1/me/contacts/{id}`,
  paged in the one list shape, searchable with `q`, capped at 5,000 per user.
- Because a contact is personal data, three compliance paths have to cover it:
  subject export includes it (the `personal_contacts` class), erasure deletes it,
  and a purge deletes it (the foreign key's cascade).
- Webhooks get nothing; a contact is not tenant configuration.

**At a site, synced both ways (agreed).** A user's contacts can be written in the
cloud and at their site, and each side sees the other's.

- **Down:** the table is journaled like the nineteen configuration tables
  (`sync_journal`), so the cloud's writes reach the site through the link's feed.
  The site's admin does *not* refuse writes to it under `[api] managed_by`.
- **Up:** the table has a `ship_seq`, as the five record tables do, and the link
  ships rows the site wrote with `POST /link/v1/records`. The cloud applies them
  under the site's tenant.
- **No echo:** each side takes the other's copy only when it is strictly newer
  (`ContactStore::put_contact_if_newer`, the comparison made in SQL on the stored
  microseconds). A change's echo comes back no newer and is let go. The cloud
  journals a site's contact once, when it takes it; the site lets that copy go, and
  nothing more travels. (As built, this replaces the session setting first planned:
  no trigger needed changing.)
- **Conflicts:** the newer `updated_at` wins on both sides, and a tombstone is an
  update like any other. A tombstone is kept only for a contact the side has, so a
  late echo of one never brings back a row already swept. Two people editing one person's contact on both sides
  within one sync is rare, and last-writer-wins is the expected answer for an
  address book.
- **While the link is down,** the site keeps writing and its rows wait with their
  `ship_seq`, as CDRs do.
- **Tombstones** older than 30 days are swept on both sides, by the admin's hourly
  offboarding sweep. A row the cloud removes goes at the site through the journal.

**Choosing where a contact goes.** When the app is signed in to FCP, the create
dialog defaults to "My account". "This device" is the other choice. A local contact
can be moved to the account later, which uploads it and then deletes the local copy.

**Using contacts.** People gets a "Contacts" section above the directory with local
and server contacts merged, each marked with where it lives. Recents and the
incoming-call screen show a contact's name when a number matches one. The match is
done by Anvil, so FCP's caller ID is never rewritten.

## 4. Phases

1. ✅ **FCP v0.25.466:** `record_on_demand`, `X-FCP-Call-Id` on the 200 OK to a
   user's own calls, `/me/calls/{id}/recording/{start,stop}`, the `on_demand`
   source, events, the user page's switches, `may_record` on `/me/softphone`, and the
   SDKs. Tests: `fcp-call-manager/tests/calls.rs`
   `a_party_records_their_own_call_and_only_they_stop_it` (the 200 OK names the
   call, both parties hear the announcement, someone not on the call finds none,
   the other party and an API recording are not the user's to stop) and
   `api_postgres` `a_user_records_their_own_call_once_an_administrator_lets_them`
   (403 until the setting is on, `for_user` sent, no path in the answer).
2. ✅ **Anvil v0.2.14** (anvil #48): the record toggle, `anvil-cli record`, and the
   "More" menu with local contacts, named on calls and recent calls.
3. ✅ **FCP v0.25.467:** `personal_contacts`, `/me/contacts`, export, erasure and
   purge, the SDKs, and the site sync both ways. Tests: `api_postgres`
   `a_user_keeps_their_own_contacts` (made, paged and searched by name, changed,
   deleted to a tombstone, nobody else's) and `fcp-link/tests/contacts.rs` (written at
   each side and seen at the other, no echo, the later edit winning on both sides, a
   delete crossing, a swept row going at the site and not coming back).
4. **Anvil v0.2.15:** server contacts, the merged Contacts section, and name lookup.

## 5. Agreed (2026-10-26)

1. **Record mechanism:** the `/me` API route. SIP INFO `Record` is left for later.
2. **Stopping:** a user stops only what they started on demand.
3. **Server contacts at a site:** synced both ways.
4. **Desk phones:** a remote phonebook from a user's contacts is left for later.
