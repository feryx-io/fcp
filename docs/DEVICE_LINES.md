# Lines on a device

D8 findings 40 and 42 (2026-10-30), both raised on Anvil:

- **40.** "Line text label where we can display the text for a line, i.e. `1001 - Bob`."
- **42.** "Multiline support. Can we support 8–10 unique or shared lines on Anvil?"

Agreed the same day:

1. **A line's label is the admin's.** By default `<extension> - <name>`.
2. **A user may rename their lines only where the tenant allows it.** A per-tenant
   setting, off by default; with it off the apps show the admin's label and offer no
   rename.
3. **Lines work as on a desk phone.** An admin gives a device its lines, the way a VVX
   gets its accounts: the user's own, shared lines, and lines the user acts for. Up to
   ten, each registered.

## 1. What exists

- **A device has one account, or two.** Provisioning's `TemplateContext.accounts`
  (`fcp-core/src/config_template.rs`, built in `fcp-api-core/src/v1/provisioning.rs`)
  holds the device's own account and, when its owner's line is `shared`, the shared
  line as a second (`docs/ENTERPRISE_CALLING.md` §3.9). There is no list of lines an
  admin edits.
- **Line keys are not lines.** `Device.line_keys` (`blf`, `speed_dial`, `park`) are
  watched or dialed numbers, not accounts calls arrive on.
- **Labels exist only in templates.** The VVX template writes `reg.1.label` as the
  extension; Anvil shows the AoR. Nothing stores a label.
- **Anvil has one account.** `/me/softphone` (`fcp-api-core/src/v1/softphone.rs`)
  answers one `account`; anvil-core registers one AoR and keeps one MWI subscription.
- **What a device may register as is already decided in one place.** The hoteling
  work's "may act as" rule (`fcp_sip::hoteling`) lets a device's credentials REGISTER
  and INVITE as its owner's lines, a shared line it is on, or a host's — so several
  lines need no new credentials.

## 2. The model

**A device's lines**, in order, at most ten:

| Field | |
|---|---|
| `kind` | `own` (the owner's line: their extension), `extension` (another extension the owner holds), `shared` (a shared line: an SCA appearance group), `on_behalf` (a user the owner acts for: an executive's line for an assistant) |
| `value` | The extension or the user it names; empty for `own` |
| `label` | The admin's label; empty means the default |
| `ring` | Whether calls to the line ring this device (on by default; off for a line that is only watched and called out on) |

The first line is always `own`. A device with no lines written has the two it has
today (its own, then its owner's shared line), so nothing changes until an admin
edits them. Each line is checked when written against the "may act as" rule: a line
the owner could not REGISTER as is refused, naming it.

**A line's label**, everywhere it is shown (Anvil, a VVX's `reg.N.label`, Yealink's
`account.N.label`):

1. the user's own name for it, when the tenant allows renaming and they set one;
2. else the admin's `label`;
3. else `<extension> - <display name>` of what the line is: `1001 - Bob`; a shared
   line `<number> - <its name>`.

**The tenant's setting** `lines.users_may_rename` (off by default), beside
`directory.users_may_unlist`. A user's names for their lines are kept on FCP
(`user_line_labels`: user, device, line index), so every app of theirs shows them and
an admin can see them; they are ignored while the setting is off.

## 3. FCP

- `GET/PUT /api/v1/devices/{id}/lines` under `device_manage`; the Device page's Lines
  table (add, reorder, label, ring), beside its line keys.
- Provisioning's `accounts` from the lines: every template loops them, so a VVX, a
  Yealink and Anvil get the same lines with the same labels. The starter templates
  write labels and up to their model's account count.
- `/me/softphone` answers `lines: [{index, kind, aor, sip_username, label,
  default_label, shared, appearances, ring, may_rename}]`; `account` stays for one
  release as the first line.
- `PUT /me/lines/{index}/label` (403 unless the tenant allows it; empty clears).
- MWI per line: a line's mailbox is the user's whose line it is, so an assistant sees
  the executive's lamp on that line.
- Ringing: a call to a line rings the devices that have it with `ring` on; the INVITE
  says which line (`X-FCP-Line: <index>` and the Request-URI's user), so the app
  shows the call on it.

## 4. Anvil

- **One registration per line**, with the device's credentials, refreshed apart; a
  line that fails shows so on its own row while the others work.
- **The lines list**: each line's label, its state (ready, ringing, on a call, held,
  for shared lines the appearances in use), its message lamp. Choosing a line before
  dialing places the call from it; incoming calls show the line they came in on.
- **Shared lines** through the appearance protocol FCP already speaks (`call-info`,
  `line-seize`): seizing, the other appearances' state, barge where allowed.
- **Rename** in a line's menu when `may_rename`; the name is stored on FCP.
- Desktop shows up to ten lines in a side column; mobile a line picker above the
  keypad.

## 5. Phases

1. **FCP: the lines.** `device_lines` and `user_line_labels` (migration), the
   tenant's setting, the label rule, the API and the Device page's table, the
   "may act as" check, provisioning's accounts from the lines, `/me/softphone`'s
   `lines`, `PUT /me/lines/{index}/label`, `X-FCP-Line` on INVITEs, MWI per line.
   Tested with a VVX template rendering four lines and a testkit device registering
   them.
2. **Anvil: many lines.** anvil-core with N registrations and N MWI subscriptions;
   the app's lines list, line choice for outgoing calls, incoming calls on their line,
   labels and rename. Tested against FCP with three lines (own, shared, on behalf).
3. **Anvil: shared lines in full.** Appearances shown and seized, barge, the line's
   held calls retrievable from any device on it.

## 6. Done

**Phase 1a (v0.25.487).** `Device.lines` (`DeviceLine`: `kind`, `value`, `label`,
`ring`; `devices.lines_json`, migration 130), set on the device's create and `PATCH`
(not a route of their own) and shown in its response; the Devices page's edit dialog
takes them as text, one per line (`own`, `extension, 1005, Sales`, `shared,
+15551234567`, `on_behalf, mary, , no ring`). `fcp_sip::device_lines` resolves each
line — what it registers as, its default label `<extension> - <name>` — and refuses one
the device may not be: an extension that is not the owner's, a shared line the device is
not on, a user the owner does not assist. "May act as" lets a device be an executive's
line when its owner is one of their assistants and the line is on it. A user's names
are kept by the line itself (`user_line_labels`: `own`, `extension:1005`, …), not by
device and index, so one name reaches every device with the line; the tenant's
`lines.users_may_rename` (off by default; the Tenants page's switch) decides whether
they are shown, and `PUT /me/lines/{line}/label` (403 while off, 404 for a line none of
the user's devices has; empty clears). Provisioning writes a device's accounts from its
lines when it has any (a hoteling host lent to a guest keeps the guest's), and
`/me/softphone` answers `lines` and `may_rename_lines`; the app's own line is the user's
address, as `account` is, while a desk phone's first account stays the device's own
name. Tested: `a_devices_lines_are_checked_shown_and_named` (checks, the app's lines,
renaming on and off, a VVX written with three accounts), the line rules' unit tests.
A user's names are not synced to sites.

**Phase 1b (v0.25.488).** Every INVITE the node sends a user's phone (never one toward
a trunk) names the line the call is for in `X-FCP-Line`, as a line's key rather than
an index, so it means the same on every device: `on_behalf:<executive>` for an
executive's screened call ringing the assistants, `shared:<number>` for a call on a
shared line, `extension:<number>` for a call to another of the user's extensions,
`own` otherwise (`fcp_sip::device_lines::line_of_call`). A phone finds its line by
that key among `/me/softphone`'s `lines`; the resolver's keys are canonical
(`shared:+e164`, `on_behalf:<username>`) whatever the administrator typed. Message
waiting per line: a line's `message-summary` subscription is the mailbox of the user
the line is — an executive's on their assistant's device, a shared line's owner's.
Tested: calling.rs `a_desk_phone_registers_its_lines_and_is_told_which_a_call_is_for`
(three lines registered with the desk's credentials and another's refused, the line
named for a call to each, the executive's line subscribing to her mailbox).

Phase 1 is done; phase 2 is Anvil's.

## 7. Questions

1. **The user's names are stored on FCP**, so they follow the user to every app.
   Recommended: yes (the alternative is per install, lost on reinstall).
2. **A user's rename also reaches their desk phones' labels.** Recommended: yes, the
   same rule everywhere.
3. **`ring` per line**, so a device can carry a line it only watches and calls out on.
   Recommended: yes.
4. **Ten lines at most** per device, whatever the model; a template stops at its
   model's own account count. Recommended: yes.
