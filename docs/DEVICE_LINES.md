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

## 6. Questions

1. **The user's names are stored on FCP**, so they follow the user to every app.
   Recommended: yes (the alternative is per install, lost on reinstall).
2. **A user's rename also reaches their desk phones' labels.** Recommended: yes, the
   same rule everywhere.
3. **`ring` per line**, so a device can carry a line it only watches and calls out on.
   Recommended: yes.
4. **Ten lines at most** per device, whatever the model; a template stops at its
   model's own account count. Recommended: yes.
