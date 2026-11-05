# Voicemail greetings

What a caller hears before leaving a message, and how a mailbox's owner
and its administrators choose it. Covers #343 (v0.25.515).

## 1. The three greetings

A mailbox's `greeting_choice` decides what callers hear.

| Choice | Callers hear |
|---|---|
| `standard` | the node's default greeting: `default_greeting_path` on the voicemail server, `/var/fcp/audio/default-greeting.wav` in the images |
| `personalized` | "Sorry, *name* is not available. Please leave a message after the tone.", spoken by the TTS engine with the owner's name |
| `own` | the owner's recordings: busy when the caller met busy, else an active unexpired temporary greeting, else the active unavailable greeting |
| unset (`null`) | the default: `personalized` when the owner has a first and last name, `standard` otherwise |

The default applies to new mailboxes. Migration 138 sets `own` on every
existing mailbox that already had a busy, temporary or unavailable
greeting, so those callers hear what they heard before.

When the chosen greeting is missing, callers hear the standard one:
- a personalized greeting not made yet (no name, no TTS service, TTS
  down);
- an own greeting never recorded;
- a file not on the node playing it.

`greeting_heard` on the mailbox (`standard`, `personalized` or `own`)
says what callers hear now.

**Recording an own greeting under the default makes it the one heard.**
A busy, temporary or unavailable greeting recorded while the choice is
unset sets the choice to `own`, whether it was uploaded, spoken from text
or recorded from the phone. Otherwise the new greeting would go unheard
behind the personalized one.

## 2. The personalized greeting

`greeting_name` decides the name it says:

| `greeting_name` | The name |
|---|---|
| `full_name` (default) | the owner's first and last name, else their display name |
| `display_name` | the owner's display name, which reads as the first and last name when unset (`docs/USERS.md` §7b) |
| `recorded` | the mailbox's recorded name greeting (`name`), between "Sorry," and "is not available…"; the full name is spoken instead while there is no recording or it cannot be read |

**When it is made.** The greeting is synthesized once and stored as the
mailbox's `personalized` greeting, so a caller never waits on TTS. The
admin makes it again when what it says changes:
- the mailbox is made;
- the mailbox's choice or name source changes (`PUT
  /voicemail/mailboxes/{id}`, `PUT /me/voicemail`, `*98`);
- the owner's first, last or display name changes, from wherever: an
  administrator, the user, SCIM or single sign-on;
- the name greeting is recorded or deleted.

**What it is made from** is kept on the greeting (`made_from`: the
sentence, or the name recording's file and time). It is not made again
unless that changes. When a mailbox stops playing it (another choice, or
no name), the greeting is taken off and the reaper removes its file.

**Where it is made.** The admin makes it, with its own TTS service and
its `voicemail_greetings_dir`. After a choice or a name recorded on
`*98`, the call manager makes it, with its TTS and `[media]
voicemail_greetings_dir`. Without a TTS service nothing is made, and
callers hear the standard greeting.

**Hearing it.** Download `GET
/voicemail/mailboxes/{id}/greetings/personalized/audio` or `GET
/me/voicemail/greetings/personalized/audio`, use the "Hear it" button on
the portal's Greetings tab, or press 2 in `*98`'s mailbox options. It
cannot be uploaded, deleted or switched on and off: it is only made.

**The audio** is 8 kHz mono 16-bit WAV. A recorded name is turned to the
same format whatever its rate, width or channels, and trimmed of the
quiet around it. An eighth of a second of silence goes either side of it.

## 3. From the phone: `*98`

After the PIN, the main menu offers **0 for mailbox options**:

| Key | Does |
|---|---|
| 1 | records the greeting: a tone, then until `#` or two minutes; heard back, then 1 keeps it (an `unavailable` greeting), 2 records again, `*` cancels |
| 2 | plays the greeting callers hear now |
| 3 | chooses it: 1 standard, 2 with your name, 3 your own |
| 4 | records your name, the same way, up to ten seconds |
| `*` | back to the main menu |

Each prompt is a system prompt that can be recorded over on the Prompts
page: `vm_mailbox_options`, `vm_record_greeting`, `vm_record_name`,
`vm_keep_recording`, `vm_recording_saved`, `vm_nothing_recorded`,
`vm_choose_greeting`, `vm_greeting_chosen`,
`vm_greeting_chosen_none_recorded` and `vm_greeting_chosen_no_name`.
Until then they are spoken.

## 4. One greetings directory

The admin writes greetings. The call manager and the voicemail server
play them, and the call manager writes greetings recorded from `*98`.
All three must see the same directory at the same path:

| Binary | Key |
|---|---|
| `fcp-admin` | `voicemail_greetings_dir` (top level) |
| `fcp-call-manager` | `[media] voicemail_greetings_dir` |
| `fcp-voicemail-server` | `[voicemail] greetings_dir` |

A greeting outside a node's directory is not played there, and callers
hear the standard greeting. The compose stack mounts the voicemail
server's `voicemail-data` volume on all three, at
`/var/lib/fcp/voicemail/greetings`. Before v0.25.515 the admin wrote
greetings to its own `/tmp`, where no caller heard them. The single-node
files use `/tmp/fcp/voicemail/greetings` for all three.

## 5. The API

| Where | Fields |
|---|---|
| `POST /voicemail/mailboxes`, `PUT /voicemail/mailboxes/{id}` | `greeting_choice` (`standard`, `personalized`, `own`, or empty for the default), `greeting_name` |
| `PUT /me/voicemail` | the same, for the user's own mailbox |
| the mailbox, both views | `greeting_choice`, `greeting_name`, `greeting_heard` |
| greetings | `greeting_type` `personalized` among them, as listed and downloaded |

A greeting made in the background is stored on its own row
(`VoicemailStore::set_greeting`). A change made to the mailbox meanwhile
is never undone.
