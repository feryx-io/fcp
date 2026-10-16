# Enterprise calling

Designed 2026-09-27, after the hybrid control plane (`docs/HYBRID_CONTROL_PLANE.md`)
closed, as the second recommendation of the 2026-09-25 feature review against
BroadSoft, CUCM, Zoom, Webex Calling and Twilio: the checklist of calling
features every enterprise PBX has and FCP does not — shared call appearance,
do not disturb, call waiting, find-me/follow-me, music on hold, class of
service, schedules, executive-assistant and hoteling. Nine features, and
under them one structural gap: FCP has no per-user calling model. This is
the design for the model and the nine features on it. Agreed 2026-09-27:
the ten questions in §5, each settled on its recommendation.

## 1. What the code has, and what it lacks

Three surveys of the code on 2026-09-27, each finding checked against the
file. The live call path is `crates/fcp-sip/src/handler.rs`; the call
manager binary only wires it.

### 1.1 There is no user calling model

A call reaches a user by one of four paths, and they agree on nothing:

- **A number bound to a user** (`forward_invite_to_b_leg`, handler.rs:4513):
  the line binding's enabled devices, every registered contact of each,
  forked at once. The binding's `no_answer_timeout` and `forward_on_no_answer`
  are honoured (:8114, `check_cfna_destination` :13786). Its
  `forward_unconditional`, `forward_on_busy`, `ring_strategy`,
  `DeviceBinding.priority`, `line_appearance`, `shared_line`,
  `outbound_caller_id` and `outbound_cnam` are stored, shown in the admin
  UI and the self-service forwarding page, settable through
  `/v1/line-bindings` and `/v1/me/forwarding/:id`, and read by nothing.
- **An extension bound to a user** (:5709): every device the user owns,
  falling back to the user's own AoR. This path sets no `line_binding_id`,
  so it gets the node's default ring timeout, no forwarding and no
  voicemail. A caller who dials the extension of a user with voicemail
  hears 480.
- **A flow's `Dial`** (`flow_handler.rs:686`): an extension pointing at a
  user is "not supported" (:957) and falls back to `sip:<ext>@local`,
  which re-enters the B2BUA.
- **A hunt group member** (`execute_hunt_group_plan`, :13571): the member's
  AoR at the local address, with `skip_busy` testing the presence state
  `Busy`, which is DND, never OnThePhone (`presence.rs:1341`).

No path asks whether the user is on a call, in DND, or wants a second call.
No contact registered means 480 at once (:4636, :5797), with no forwarding.
`fcp-core` holds a second, fuller routing engine — `routing_engine.rs`,
`forwarding.rs` (rules by type, time range and caller list),
`call_state.rs` (a device state machine with Busy and OnHold),
`call_forking.rs`, `sip_handler.rs` — that nothing outside its own tests
constructs.

### 1.2 The B2BUA does not know about hold

`handle_reinvite` (handler.rs:9367) passes every re-INVITE to the other
leg, rewriting only the address when media is anchored. `a=sendonly` and
`a=inactive` pass through untouched; no state records that a leg is held;
`fcp_b2bua_core::CallState` has no held state; `MediaManager::hold_session`
is a no-op in the forge backend (`backend.rs:931`); no `call.held` event
exists. When media is anchored, the answer returned to the holder is
rebuilt from the A-leg's original offer (:9587), so a hold is answered
`sendrecv`. Park (`handle_park_call`, :12517) sends the parker's own leg a
`sendonly` re-INVITE and hangs up the far party, and a parked caller hears
silence.

REFER is always passed to the far leg as a blind transfer (:10535);
`Replaces` on an incoming INVITE is never parsed; attended transfer from a
phone therefore fails. Pickup works on ringing calls only.

### 1.3 Audio into a leg is one-shot PCMU

`fcp_media::MediaManager::play_file` plays a file once, as PCMU at 8 kHz
regardless of the negotiated codec (`forge-engine/src/injection.rs:347`),
as a second RTP stream beside the relayed audio, with `MixMode` stored and
ignored. The queue's hold music is a hand-built RTP loop in the call
manager (`queue_audio.rs`, node-local files under `queue_audio_dir`); the
conference server has a second, codec-aware player. `PromptCategory::Moh`
exists (`prompt.rs:22`) and only the conference server reads it. The flow
`Play` node's `loop_count` is ignored.

### 1.4 Presence is one tenant deep

`run_presence_listener` (`fcp-call-manager/src/main.rs:3107`) turns
`call.*` events into dialog state for BLF, for `default_tenant_id` only
(:1769), tracking the callee side only. The dialog-info it sends carries
FCP's internal call id and no SIP call-id or tags, so a phone cannot pick
up by `Replaces`. The standalone presence server receives state and note
over Redis and no calls at all.

### 1.5 Devices have no identity of their own

A device registers with its owner's credentials (`provisioning.rs:181`); the
registrar does not check that the digest user matches the AoR it registers
(`registrar.rs:980`); every built-in template renders exactly one account
(`config_template.rs:81`). A device's owner can be changed by
`PATCH /v1/devices/:id` but never cleared, and the admin UI cannot set it.
The registration binding stores no `+sip.instance` or `reg-id`.

### 1.6 Nothing restricts what a user dials, and nothing knows the time

Outbound calls pass the node's YAML policy (roles and departments, file per
node, default deny when set), the tenant's DB routing rules (pattern, IP,
time window; allow, deny or trunk) and balance admission. There is no class
of service, no destination classification, and the number's stored
`block_anonymous`, `allow_list` and `block_list` are never read. The
FAC/CMC code in `fcp-pbx/src/codes.rs` is never called.

Time windows exist on routing rules and on TOML-defined queues
(`is_queue_open`); DB-defined queues get an empty schedule
(`queue_runtime_config.rs:46`). There is no schedule entity, no holidays,
no tenant timezone, and nothing that changes where an inbound call goes
by time. `fcp-flow` has no time node.

### 1.7 What is sound

The B2BUA's parallel fork, the registrar and its cache, the tenant
resolution and trusted-edge claims, the flow engine, the media relay with
its per-packet transcoders, the prompt store with tenant uploads, the
feature-code parser and dispatch, the presence SUBSCRIBE/NOTIFY machinery
for `dialog` and `presence`, the cluster's call registry, the event bus,
the CDR pipeline, and the testkit phone that can hold, resume, REFER, take
a second call and hear a tone. The features below are built on these.

## 2. The model

Three ideas carry all nine features.

**The user's calling settings.** One row per user, `user_calling_settings`,
created with the tenant's defaults the first time it is read: do not
disturb, call waiting, four forwards (all, busy, no answer, unreachable)
with the no-answer time, the find-me steps, the class of service, the hold
music, the executive and assistant relations, and the schedule that
switches the forwards after hours. Every feature is a field here or a row
that points here. The line binding keeps what belongs to the number — its
type, its destination, its devices, its schedule — and loses the forwards,
ring strategy and caller ID it never applied (§5 Q1).

**The ring plan.** One function, `plan_for_user`, builds from the settings,
the user's devices, the hoteling sessions and the live call count an
ordered plan: steps, each a set of targets rung together for a time, and
a treatment when the steps are exhausted. Every path that reaches a user —
number, extension, direct AoR, flow `Dial`, hunt group member, an
assistant's push, a find-me step naming another user — asks for the plan
and executes it. DND, call waiting, forwarding, schedules, find-me and
executive-assistant are all ways the plan is different.

**The effective user of a device.** A call from a phone is from a device;
the device has an owner, or a hoteling guest, and that user's settings are
the ones that apply outbound: caller ID, class of service, the CDR's user,
call-on-behalf. `effective_user(device)` is the one lookup, and the ring
plan's targets are devices found the same way in reverse.

Two smaller ideas: **a typed call target** (`CallTarget`: a user, an
extension, an E.164 number, a user's voicemail, a flow, a queue, a SIP URI),
used by every forward, every schedule branch and every find-me step, with
one executor; and **the held state of a leg**, known to the B2BUA, which
music on hold, appearances, transfer and pickup all read.

## 3. The features

### 3.1 Foundations: settings, the ring plan and forwarding

`user_calling_settings` (migration 080): `tenant_id`, `user_id`, `dnd`,
`call_waiting` (default true), `forward_all`, `forward_busy`,
`forward_no_answer`, `forward_unreachable` (each a `CallTarget` or null),
`no_answer_secs` (default 30), `updated_at`; later phases add columns. A
`CallingDefaults` on the tenant (JSONB) seeds new rows. The binding's
forwards and timeout are copied into the owner's row by the migration where
the binding is the owner's first, and the old columns are dropped by a
later contraction (`scripts/check-migrations.sh` rules).

`fcp_sip::ring_plan::plan_for_user(tenant, user, ctx) -> RingPlan` where
`ctx` carries the caller, the number or extension dialed, whether the call
is external, and the time. The plan:

```
RingPlan { steps: Vec<RingStep>, then: Treatment }
RingStep { targets: Vec<RingTarget>, ring_secs: u32, confirm: bool }
RingTarget { Contact(device, contact) | External(e164) }
Treatment { Voicemail | Forward(CallTarget) | Refuse(code) }
```

The executor in the handler runs each step as a parallel fork with the
existing machinery (`send_b_leg_invite`, `handle_parallel_forking`) and the
step's own timeout, cancels the step's legs when the next begins, and
applies the treatment: `route_cfna` becomes the `CallTarget` executor.
Rules applied by `plan_for_user`, in order: `forward_all` set → no steps,
forward; `dnd` → no steps, `forward_busy` else voicemail else 486; no
registered contact and no external step → `forward_unreachable` else
voicemail else 480; on a call and call waiting off or at the class's call
limit → busy treatment; otherwise the user's devices as step one (later:
the find-me steps), then `forward_no_answer` else voicemail else the
callee's last code.

All four paths of §1.1 call it. The extension-to-user path gains what it
lacked (timeout, forwarding, voicemail). Flow `Dial` to a user extension
runs the plan on the flow's leg and reports `dial_status` as now. Hunt group
members that are users get DND and busy honoured. A device extension
(`ExtensionTarget::Device`) rings the device with its effective user's
treatment.

The presence listener runs for every tenant, and `is_busy` counts
OnThePhone. Feature codes `*78`/`*79` (DND on/off), `*72<number>`/`*73`
(forward all on/off), `*90<number>`/`*91` (busy), `*92<number>`/`*93` (no
answer), all in `[pbx.feature_codes]` with these defaults (§5 Q6). API:
`GET/PUT /v1/users/:username/calling` (admin) and `GET/PUT /v1/me/calling`
(self-service), replacing `/v1/me/forwarding`; the user page's Calling
tab and the self-service Forwarding page rewritten. Events:
`user.calling_changed`.

#### 3.1.1 As built (phase 1, v0.25.276)

The row is `user_calling_settings (user_id PK, tenant_id, settings_json,
updated_at)` — the settings object as JSON, so the phases that follow add
fields without a migration; migration 082 seeds it from the owner's first
line binding (`voicemail`, a `+` number as `number:`, digits as `ext:`, a
SIP URI as it is) and journals it for sites under the user's id, which the
link applies with `put_calling_settings`. The binding's four columns stay,
unread, for a later contraction, and `LineBinding` no longer has the
fields. `CallingDefaults` on the tenant was not needed: the defaults are
`CallingSettings::defaults`, and a ring time left unset is the node's
`[b2bua.forking] timeout`, as before (the API clears it with `0`).

`fcp_sip::ring_plan::plan(settings, contacts, has_mailbox)` is the pure
rule; the path that reaches the user resolves the contacts
(`contacts_for_binding` for a number's devices, `contacts_for_user` for
an extension, the registrations found for a direct address) and the
handler's `ring_by_plan` either applies an immediate treatment or stores
`callee_user_id`, `treatments` and the ring time on the call and forks.
When every leg has failed, the call's treatment for busy (486, 600, 603)
or for no answer applies through `apply_treatment`: `route_cfna` for a
forward or voicemail — a number or an extension is a new call into this
node, now at the port it listens on — and a final response otherwise, the
callee's own code when there was one. A forward's own legs carry no
treatments, so a forward that fails is the caller's answer. A flow or a
queue as a target was refused by the API until phase 5 placed it.

Since v0.25.406 (deep testing D4, `docs/DEEP_TESTING.md`):
- **A flow's `Dial` to a user runs the whole plan.** It used to run a
  copy of its own: the first registered phone only, no call waiting, no
  find-me, no executive screening, no push, and a forward's target dialed
  into the node without its token, so `auth_invites` refused it. The leg
  now goes back into this node at the user's extension, carrying the
  node's own-leg token, the flow's call in `X-FCP-Forwarded-For` (its
  tenant) and `X-FCP-Flow-Dial`. The plan rings the user as it would any
  call; with `X-FCP-Flow-Dial` a voicemail treatment answers 480 and a
  refusal its code, so the flow still takes its own `busy` and
  `no_answer` branches.
- **A forward loop is refused.** A call whose forwards reach a user they
  already reached — two users forwarding to each other, at once or after
  ringing — is answered 482 Loop Detected, as is a chain of more than ten
  forwards. Before, each hop was a new call into the node, and they went
  on until the node's own rate limit refused one.
- **A forward rings for the target's time.** The forward's leg used to
  keep the forwarding user's ring time, which had already run out: a
  forward to a user was cut off as their own ring time began, so their
  treatment (voicemail, their own forward) never applied. A leg into the
  node now waits for the new call's own plan, up to ten minutes.

The feature codes answer the INVITE 200 and end the call, as the agent
login codes do; a spoken confirmation waits for the media work of phase 3.
The presence listener keeps each call's tenant from its `call.initiated`
and takes the tenant from every registration event, so BLF works for
every tenant; MWI notes still fall back to the default tenant.

### 3.2 Hold, transfer and pickup in the B2BUA

`CallLeg.held: Option<Held { by: Leg | System, since, private }>`. A
re-INVITE's direction is read: `sendonly`/`inactive` from a leg holds it,
`sendrecv` resumes. When media is anchored the B2BUA answers the holder
itself (`recvonly` or `inactive` in the relay's SDP), stops relaying
between the legs, and plays the far party what §3.3 decides; the far leg
is not re-INVITEd. When media is not anchored the re-INVITE passes through
as today, with the holder's answer derived from the far end's answer, not
the original offer (the bug at handler.rs:9587). Park becomes a system
hold of the far party: the parker's party is parked, the parker's leg ends,
and retrieve moves the held leg to the retriever the way conference
migration moves a leg (`update_session_b_leg`, a re-INVITE only when
unanchored). Events `call.held { call_id, leg, by }` and `call.resumed`.

Transfer: the B2BUA executes a phone's REFER. Blind: it dials the
Refer-To for the far party (one INVITE, `X-FCP-Transferred-From` kept for
the CDR link), NOTIFYs the transferor, ends the transferor's leg on 200.
Attended: a Refer-To with `Replaces` names a dialog the B2BUA holds (the
consult call); the far party of the referring call is bridged to the far
party of the consult call, both phone legs end. An incoming INVITE with
`Replaces` for a dialog the node holds replaces that leg (a BLF pickup by
`Replaces`, later an appearance retrieve). The dialog-info sent to BLF
watchers carries the SIP `call-id`, `local-tag` and `remote-tag`, both
sides of every call, and `+sip.rendering=no` on a held dialog.

#### 3.2.1 As built (phase 2, v0.25.277)

**Hold.** `Call.held: Option<LegSide>` names the held leg: a re-INVITE
whose offer is `sendonly`, `inactive` or `c=0.0.0.0` (`fcp_sip::hold`)
holds the other leg once the far end accepts it, and one that is not
releases it; `call.held` and `call.resumed` are published from
`update_call`, which also publishes `call.parked` and `call.unparked`
(a retrieve used to publish a second `call.answered`). `/calls` shows
`held`. When media is anchored the answer to the holder is now built
from the holder's own offer, so a hold is answered `recvonly`. The
re-INVITE is still passed to the far phone in both cases: answering it
locally only pays once there is something to play into the held leg, so
it moves to phase 3 with the music.

**Transfer.** `[b2bua.transfer] execute` (default on) makes the node
carry out a phone's REFER (`fcp_sip::transfer`): the target is dialed
back into this node on the transferred party's behalf — its identity as
the caller, `X-FCP-Transferred-From` for the record — so it is routed as
any call, a user's plan included; on a 2xx the referrer's slot takes the
new leg, the media session's side is moved (anchored) or the transferred
party re-INVITEd with the answer (not), a held party is taken off hold,
the referrer is sent the sipfrag NOTIFY and then a BYE. A failure is
NOTIFYed and the referrer keeps the call. With `execute = false` the
REFER is relayed as before. An attended transfer's Refer-To carries
`Replaces`, which the node's INVITE carries, and the node's own handling
of it joins the calls: the result is two anchored hops through a leg the
node dialed to itself. Because such a leg is one SIP dialog with an end
in two calls, in-dialog requests are now matched by Call-ID and both tags
(`CallManager::find_by_request_dialog`), not Call-ID alone.

**Replaces.** An INVITE with `Replaces` naming an established leg of a
call of the same tenant takes that leg's slot (`take_over_leg`): the
other party is pointed at it, off hold, the INVITE answered, the old leg
dropped with a BYE that touches no call (`drop_leg`: the slot already
holds the new leg), and the INVITE's own call ended as completed,
pointing at the call it joined. An unknown dialog is 481; an early one is
486 — a ringing call is picked up with the pickup codes, and pickup by
`Replaces` from a BLF key needs the dialog-info to carry SIP ids, which
moves to phase 9 with the rest of the dialog state.

**Park.** A REFER to the park code parks the transferred party: an
orbit, the party moved into the A slot (the retriever takes B), the
referrer NOTIFYed and dropped. The direct `*70` parks the far party of
the dialer's established call on either leg (it used to find only a call
the dialer had placed, and parked the dialer). `*71<orbit>` is the
retriever's own INVITE taking the empty slot. An orbit that times out is
a transfer from the empty slot to the parker, through the parker's plan;
if that fails the call ends.

**Found on the way.** A leg a flow dials that answers as the caller
hangs up was never ended: the BYE path decided the call was over before
the flow recorded the leg. Both now decide under the call's lock — the
BYE path marks the call `Terminating` or forwards to a leg that just
landed, the flow hangs up a leg whose call is ending.

### 3.3 Music on hold

forge-media: `FileSource` looped (`reset()` exists), playback encoded in
the leg's negotiated codec through the transcoders already armed per leg,
resampled from the file's rate, and a `Replace` mix mode that mutes the
relayed stream while the source plays. FCP: `MediaManager::play_file` gains
`looped` and `replace`; a `HoldMusic` resolver picks, in order, the held
party's counterpart's `moh_prompt_id`, the number's `moh_prompt_id`, the
tenant's default `moh` prompt, and the built-in default — a short piece
synthesised at startup like the system prompts, no licensed audio in the
repository (§5 Q4). It plays to the far party on hold and to a parked
party; a tenant flag `hold_music = false` gives silence with the periodic
tone instead. Calls of a tenant with hold music anchor at setup (a reason
in `evaluate_media_anchoring`), since an unanchored hold cannot be played
into (§5 Q3). The queue's `moh_class` may name a prompt as well as a
directory. The flow `Play` node's `loop_count` starts working.

#### 3.3.1 As built (phase 3, v0.25.278)

**The player (forge-media v2026.09.27, #166).** The injection player sent one-shot
PCMU on a stream of its own beside the relayed audio. It now hands the
session's scheduled playout a frame every 20 ms, so music and prompts are
encoded in the leg's negotiated codec, resampled from the file's rate,
SRTP-protected and sent on the leg's own stream; `PlaybackOptions`
adds `looped`, and a `Replace` playback holds back the other leg's audio
toward its target while it plays (`suppress_relay_to`, counted per leg).
fcp-media gains `play_file_looped`, and `stop_playback` stops the one
playback it names instead of every playback on the call.

**Holds.** `[media] hold_music` (default on, in `fcp_core::config::
MediaConfig` as `hold_music`) is an anchoring reason, so every call is
anchored; when it is on and the call is anchored, a hold and its resume
are answered by the node (`answer_hold_locally`) and the far phone is
not told — the phase 2 pass-through remains for unanchored calls and
with the switch off. The music follows the held leg, not the SIP path:
`CallManager::set_on_hold_changed` is told of every change of
`Call.held`, and `fcp_sip::hold_music::HoldMusic` stops or starts the
looped music into the held leg — a hold, a park (the parked party is now
always marked held), a retrieve, a transfer, a take-over.

**The music.** The holder's `hold_music` (a prompt of the tenant, or
`none` for silence; on the calling settings, the API and the Calling
dialog, whose list is the tenant's prompts), else the tenant's first
`moh`-category prompt with audio, else `prompts_dir/hold-music.wav`, a
slow C-major arpeggio synthesised at startup. The holder is the call's
callee from the plan when it is the B leg, the parker of a parked call,
else the user the leg's address names.

**The number's music (v0.25.388).** A number's line binding carries
`hold_music` (a prompt of the tenant, `none` for silence, unset for the
tenant's; migration 114, on the line binding API and its page). An inbound
call remembers its number's binding for its whole life
(`Call.number_binding_id`, which a forward does not clear), and the
resolver's order is the holder's choice, the number's, the tenant's
`moh` prompt, the built-in default. Test: `calling.rs`
`a_numbers_hold_music_plays_to_its_callers_unless_the_holder_chose_their_own`.

**The tenant's music (v0.25.392).** The tenant's calling defaults carry
`hold_music` (a prompt of the tenant, `none` for silence, unset for its
first `moh` prompt; on `PUT /v1/tenants/{id}` under `calling` and the
Tenants page). The resolver's order is the holder's choice, the number's,
the tenant's, the tenant's first `moh` prompt, the built-in default — so a
tenant that wants no music anywhere sets `none`, and a user or number may
still choose music of their own. Test: `calling.rs`
`a_tenants_chosen_hold_music_or_silence_plays_to_every_held_caller`.

**Not built.** The queue naming a prompt as its music —
the queue pre-encodes node-local files and its database configurations
drop their prompt fields, which is a change of its own; the periodic
tone in silence. The flow `Play` node's `loop_count` repeats the prompt
that many times, 0 and 1 being once (every flow written so far carries
0, which the schema had documented as "infinite").

### 3.4 Call waiting and busy

`user_calls`: the cluster call registry (`fcp-cluster/src/call_registry.rs`)
gains a per-user set, `fcp:user_calls:<tenant>:<user>`, written when a
leg of a user's device establishes and cleared when it ends, with the
registry's TTL and refresh. `plan_for_user` reads it (the node's own calls
first). Call waiting on: the second call rings the user's devices as any
call, phones present the second appearance and their own tone. Off, or the
class's `max_calls` reached: the busy treatment. `*56`/`*57` turn it off
and on for the user. A hoteling guest's calls count against the guest.

#### 3.4.1 As built (phase 4, v0.25.279)

The per-user set is a lease subject of the cluster's call counting
(`fcp_cluster::CallLeases`, `LeaseSubject::User("<tenant>:<user>")`),
the same sorted sets the tenant and trunk limits use, rather than a
key of the call registry: a node that dies leaves leases that lapse.
Without a cluster the count is the node's own (`LocalUserCalls`).
`Call::counted_users` is the users a call counts against while it is
up — `caller_user_id` (set at the INVITE when the From names a user of
the tenant, by extension or username) and `callee_user_id` (the plan's
callee) — and `CallManager::set_on_parties_changed` is told whenever
that set changes; `fcp_sip::user_calls::UserCalls` takes and releases
the leases from it and refreshes them every ten minutes. A party that
leaves a call releases it: a transfer clears the referrer's user, a park
the parker's, a take-over puts the joining INVITE's caller in the slot.

The plan gains one input: on a call with call waiting off is the busy
treatment, as do not disturb is. With call waiting on, the second call
rings the phones as any call and the phone presents it. `*56` turns
call waiting on and `*57` off (the codes of §3.4 in on/off order, as
every other pair), and the Calling page and dialog have the switch.
A flow's `Dial` consults it since v0.25.406 (§3.1.1). The class of
service's `max_calls` is phase 7's.

### 3.5 Schedules

`schedules` (migration 081): `tenant_id`, `name`, `timezone`, `weekly`
(JSONB list of `{days, start, end}`), `holidays` (JSONB list of
`{date | from..to, name}`), and `state_at(t) -> Open | Closed | Holiday`.
The tenant gains `timezone` (schedules and reports default to it; the
user's own timezone stays the user's) and `country` (§3.7). Attachments:
a line binding's `schedule_id` with `closed_target` and `holiday_target`
(both `CallTarget`), a user's `after_hours_schedule_id` with
`after_hours_forward`, a DB queue's `schedule_id` and `closed_target`
(the hard-coded empty schedule at `queue_runtime_config.rs:46` goes), and
a flow node `Schedule { schedule_id, open, closed, holiday }`. API
`/v1/schedules`, a Schedules page, the schedule selects on the number,
queue and user pages. The routing rules' own time window stays as it is.

#### 3.5.1 As built (phase 5, v0.25.280)

`fcp_core::schedule`: a `Schedule` is weekly windows (days, `HH:MM`
start and end — an end before its start runs past midnight, an end
equal to its start is the whole day) and holidays (a day, or `from` to
`to`), in an IANA timezone; `state_at` gives holiday before hours,
then open inside any window, else closed, in the schedule's own zone
through its daylight-saving changes. Stored as JSON (`schedules`,
migration 083) and journaled for sites; `/v1/schedules` (create, read
with the state now, replace, delete, under the routing-rule
permissions) and the Schedules page.

Where a schedule acts: a number's line binding (`schedule_id`,
`closed_target`, `holiday_target` — a holiday falls back to the closed
target, no target is 480 "Closed") on the number and the
extension-to-number paths; a user's `after_hours_schedule_id` and
`after_hours_forward` in the plan, after forward-all; a database queue's
`schedule_id` and `closed_target`, decided by the B2BUA before the call
reaches the queue (the queue's own "closed" routing returned an error
and failed the call, and still does for TOML queues); and the flow
`Schedule` node (`open`, `closed`, `holiday`, setting `schedule_state`),
read by the call manager, whose other runners take the open branch. A
schedule that is gone or unreadable is open: a route is never closed by a
database hiccup.

The targets `flow:` and `queue:` are placed now: a flow runs for the
call (the IVR binding's flow start is `start_flow_for_call`), a queue is
dialed at its extension. The tenant's own timezone, which the design
gave schedules as a default, waits for phase 7, which needs the tenant's
country beside it; a schedule names its zone.

### 3.6 Find-me / follow-me

`find_me` on the settings (JSONB): `enabled`, `steps: [{ targets, ring_secs,
confirm }]` where a target is `devices` (the user's own), a user, an
extension or an E.164 number. `plan_for_user` expands the steps into
`RingStep`s; an external number becomes an outbound leg through the
tenant's trunk selection with the original caller's number presented and a
`Diversion` header naming the user's number (§5 Q7 for the alternative).
`confirm` on a step means an external leg that answers hears "call for
<name>, press 1 to accept" (`play_tts` on that leg) and is bridged only on
`1`; anything else, or silence, ends that leg and the step continues — a
mobile's voicemail can no longer swallow the call. Steps with confirmation
anchor the call. Self-service on the Calling page; `*60`/`*61` on and off.

#### 3.6.1 As built (phase 6, v0.25.281)

`CallingSettings.find_me` holds `enabled` and up to five `steps`, each
`targets` (`devices` for the user's own phones, a user, an extension, a
number, a SIP URI), `ring_secs` and `confirm`. The steps ring after the
user's phones have rung for their own time — `devices` in a step rings
them again with the rest — and before the no-answer treatment; when none
of the user's phones is registered, they ring at once and the
unreachable treatment follows them. The call keeps the steps still to
ring (`Call.find_me_steps`), and when a fork's legs have all failed the
next step is forked with its own ring time (`next_find_me_step`): a user,
or an extension that is a user, by their registered contacts; any other
extension, a number or a URI dialed back into this node with the
caller's identity, so a number leaves by the tenant's trunks.

Confirmation: a leg of a `confirm` step that answers has its media set
on the call's session and hears, alone, "Call for <name>. Press 1 to
accept." (a tone where there is no speech), and the caller is connected
only on a 1 within ten seconds; any other digit or none drops that leg
and the call goes on as though nobody had answered — the next step, or
the treatment. A call whose media is not anchored cannot be asked, and
its answer stands. `*60` and `*61` turn find-me on and off; the Calling
page and dialog edit the steps.

`Diversion` (v0.25.389): a forward or a find-me step placed back into
this node names the call it forwards for (`X-FCP-Forwarded-For`), and
the original call keeps why it was forwarded (`Call.forward_reason`).
When that leg leaves by a trunk, its INVITE carries
`Diversion: <sip:<number>@<host>>;reason=<reason>;counter=1` (RFC 5806):
the number is the one the caller dialed (the call's number binding),
else the forwarding user's first number; the host is the trunk's
`from_domain`, else the node's; the reason is `unconditional`,
`user-busy`, `no-answer`, `unavailable` (nothing registered),
`time-of-day` (a schedule) or `follow-me` (a find-me step). The From
stays the caller's, as Q7 agreed; an emergency call carries none. Test:
`trunks.rs`.

### 3.7 Class of service

`classes_of_service` (migration 082): `tenant_id`, `name`, `allowed`
(a set of `internal, emergency, toll_free, national, international,
premium, unknown`; `emergency` cannot be removed), `max_calls` (per user,
default 2), `caller_id` (`line` — the line binding's `outbound_caller_id`
at last applied — or `withheld`, which sends `Privacy: id` and an anonymous
From), and `is_default`. A tenant's default class allows every class, so
an upgrade refuses no call. `classify(tenant, dialed) -> DestinationClass`
runs on the normalised number: an extension or a tenant user is `internal`,
the emergency numbers `emergency`, then by the tenant's `country`: the
country's toll-free and premium prefixes (a built-in table for the
countries the platform has trunked to, extended by the tenant's
`destination_overrides`), the country code `national`, another `international`,
unparseable `unknown`. Enforced in `handle_invite` after the tenant rules
and before billing, for the effective user of the calling device, and for
every find-me external leg and call-on-behalf: refused with 403, a
`Reason` header and `call.failed { reason: class_of_service }`. The
number's `block_anonymous`, `allow_list` and `block_list` are enforced at
the DID lookup in the same phase. Assignment: `class_of_service_id` on the
settings, the class's page and select. FAC/CMC stay unwired (§5 Q8).

#### 3.7.1 As built (phase 7, v0.25.282)

`fcp_core::class_of_service` classifies what a user dials: a feature
code, a name or a short number (up to `[b2bua] max_extension_digits`) is
`internal`, as is a number of the tenant's own; the country's emergency
numbers are `emergency`; the rest is read by the tenant's country — its
international prefix, its trunk prefix, NANP's bare ten digits,
Australia's `1…` service numbers — into E.164, then `premium` and
`toll_free` by the country's ranges, `national` in its country code,
`international` outside it, `unknown` when it cannot be read. The table
holds US, CA, GB, DE, FR, AU and NZ; a tenant with no country reads
numbers as North American, which is how the node normalised them before.
The tenant's `destination_overrides` win by longest prefix, in E.164
(`+1900`) or as dialled (`118`).

The tenant gains `calling` (`country`, `timezone`,
`destination_overrides`; `tenants.calling_json`, migration 084), edited
on the Tenants page; a schedule created without a timezone takes the
tenant's. `classes_of_service` (migration 084, JSON rows, synced to
sites) hold `allowed`, `max_calls` (two unless said, `null` for none),
`caller_id` (`line` or `withheld`) and `is_default`; one class is the
default, and marking another takes the mark from it. A user's
`class_of_service_id` is set by an admin (`/me/calling` refuses it);
unset, the default class applies; with no default, nothing is refused.
`/v1/classes-of-service` (routing-rule permissions) manages them,
`/v1/classes-of-service/classify?dialed=` says what a number is to the
tenant, and the Classes of Service page does both.

Enforced:

- **A call a user places** is checked once the call exists — so it has a
  record, and `call.failed` says `class_of_service: …` — before
  admission: refused with 403 and `Reason: SIP;cause=403;text="class of
  service: premium calls not allowed"`. Not an emergency call, not a
  carrier's, not a site's (the site's node checks its own), and not a leg
  the node placed to itself for a forward: those carry
  `X-FCP-Forwarded-For`, believed only when it names a live call on the
  node, like a transfer origin. The caller is the trusted edge's named
  user, else the From's.
- **A forward or a find-me step to a number or a URI** is checked against
  the forwarding user's class; a forward refused is not followed (the
  caller hears 480), a step's refused target is left out.
- **`max_calls`** counts the user's calls as call waiting does (§3.4): an
  incoming call past it gets the busy treatment, a call placed past it
  is refused.
- **The number's screening**: at the number's lookup, a withheld caller
  (`Privacy: id`, an anonymous From) is refused with 433 when the number
  has `block_anonymous` or an allow list, a caller on the block list or
  off the allow list with 603. Entries match on digits, a national form
  matching its E.164, or a prefix ending in `*`.
- **Caller ID** on a leg a trunk carries: the caller's line's
  `outbound_caller_id` is the From and `P-Asserted-Identity` unless the
  trunk names an identity of its own; `withheld` makes the From
  `"Anonymous" <sip:anonymous@anonymous.invalid>`, adds `Privacy: id` and
  keeps the identity in `P-Asserted-Identity` for the carrier. An
  emergency call presents its callback number whatever the class says.

A site's admin refuses writes to classes of service, and to schedules,
which phase 5 synced without listing, as it does to every synced resource.

Deviations: the check runs after billing admission, not before — a call
the class refuses may first be refused for balance. Call-on-behalf is
phase 10's and will be checked there. A tenant's country covers only the
table's countries; others are refused until the table grows.

### 3.8 Hoteling

Devices gain credentials of their own: `sip_username` and `sip_ha1`
(nullable; absent means the owner's, as today), rendered by provisioning
as `${sip_auth_username}`/`${sip_auth_ha1_hash}`; the registrar checks
that a credential may register the AoR it registers (its own device's, its
owner's, or a line it is bound to). `devices.hoteling_host` marks a host;
`owner_user_id` becomes clearable, and the admin UI can set it.
`hoteling_sessions` (migration 083): `device_id`, `user_id`, `tenant_id`,
`started_at`, `expires_at`, `ended_at`, `ended_by`. Login: `*88` from a
host device — the device is identified from the INVITE's registration, the
call is answered, "extension, then PIN" collected by DTMF (the user's
`pin`), the session made (ending the user's other session and the
device's other guest), a `NOTIFY Event: check-sync` sent so a phone that
re-provisions shows the guest's name (`${effective_user_*}` template
variables), an MWI NOTIFY for the guest's mailbox. `*89` logs out; the
tenant's `hoteling_session_hours` (default 12) expires a session by sweep.
`effective_user(device)` is the guest while a session lives: the ring plan
rings the host among the guest's devices and leaves it out of the owner's
(if any), outbound calls from the host are the guest's for caller ID, class
of service, the CDR and billing, the guest's dialog state is mirrored to
the host's AoR for BLF. E911 stays the device's, so a guest calls with the
host's location. API: `GET /v1/hoteling/sessions`, `DELETE
/v1/hoteling/sessions/:id`, `GET /v1/me/hoteling`, `DELETE /v1/me/hoteling`;
events `hoteling.login`, `hoteling.logout`.

#### 3.8.1 As built (phase 8, v0.25.283)

A device may have credentials of its own: `sip_username`, and
`sip_ha1_hash` and `sip_provisioning_password` sealed at rest like a
user's (migration 085; both are in the re-encrypt pass, neither is ever
returned by the API, subject export leaves them out). The username is the
tenant's alone: the API refuses one a user or another device already is,
and a new username without a new password, since the hash is made of
both. Provisioning renders the device's own credentials when it has them
(`${sip_username}` is then the device's address, `${sip_auth_username}`
its credentials), else its owner's as before, and adds
`${effective_user_username}`, `${effective_user_display_name}`,
`${effective_user_extension}`, `${effective_user_id}` and
`${hoteling_guest}`. A device's owner can be cleared (`null`), and the
admin UI's device dialog sets the owner, the host flag and the
credentials.

**Who a credential may be.** With the registrar given the storage
(`FcpRegistrar::with_identity`, on the call manager and the standalone
registrar), the digest lookup falls back from a user to a device's own
credentials, and one rule (`fcp_sip::hoteling::may_act_as`) decides both
what a REGISTER may register and whose `From` an INVITE may carry under
`auth_invites`: a credential is always its own name; a user is also a
device they own, an extension that is theirs, a number whose line is
theirs; a device is also its own address, its owner's name or extension,
a number whose line it is on. Anything else is 403 (a REGISTER) or the
wrong-user refusal (an INVITE), never a challenge. The SBC's registrar is
not given the storage and is unchanged.

**Sessions.** `hoteling_sessions` (migration 085, not journaled: a site's
logins are its own) hold one active session per host and per guest,
enforced by partial unique indexes; `HotelingStore` starts one in a
transaction that ends the guest's other session and the host's other
guest (`replaced`), ends one (`logout`, `admin`), and expires those run
out (`expired`) returning each to one sweeper only. A session lasts the
tenant's `calling.hoteling_session_hours` (1 to 168, default 12, on the
Tenants page). `fcp_sip::hoteling::Hoteling` starts and ends them, and
after each sends the phone `NOTIFY Event: check-sync` from a task of its
own; at a login it also sends the guest's message-waiting state. Every
node sweeps each `[pbx] hoteling_sweep_secs` (60). `hoteling.login` and
`hoteling.logout` (with `reason`) are `UserEvent`s; a logout the API
made says `resynced: false`, and the call managers that hear it send the
`check-sync` the API cannot (each call manager that hears it, so a
cluster with shared events sends one per node).

**Login and logout.** `*88` from an enabled host (the device the `From`
names, by credentials or address) answers the call into an inline flow
whose `HotelingLogin` node asks for an extension then a PIN, each ended
by `#`, three times at most, checks the PIN (off the async workers), and
logs the user in; the call then hangs up. The inline flow is the one
`*98` runs, now shared (`start_inline_flow`), and its answer now puts the
A-leg dialog on the call before the 200 OK, so the flow's hangup reaches
the phone and a caller's BYE finds the call. `*89` from the host ends its
session and answers 200 at once, as the calling codes do; from a phone
that is not a host both codes are 403. `[pbx.feature_codes]
hoteling_login` / `hoteling_logout`.

**The effective user.** `fcp_sip::hoteling::caller_user` is the user a
SIP name places calls as: the device it names — its guest while a session
lives, else its owner — else the user or extension it names. It is what
marks the call's caller (so the line's caller ID and the class of service
at trunk egress are the guest's), what the class of service checks, and
whose settings the calling codes and `*98` change and open. The ring plan
leaves a host lent to someone else out of its owner's devices and out of
a line's, and adds the host a user is logged in on to the user's own
(`contacts_for_user`, `contacts_for_binding`); an extension that names a
device rings it with its effective user's plan. Presence shows the
guest's calls on the host's address and leaves a lent host out of its
owner's. E911 is the device's: a guest calls with the host's location.

API: `GET /v1/hoteling/sessions` (`?all=true` for ended ones too) and
`DELETE /v1/hoteling/sessions/:id` under the device permissions,
`GET`/`DELETE /v1/me/hoteling`; a device that stops being a host ends
its session. The admin UI's Hoteling page lists sessions and ends them.

Deviations: the CDR has no user of its own, so it carries the host's
`From` as before; billing rates the tenant, as it always has. The MWI
NOTIFY is sent at login only: a message left for the guest afterwards
lights the guest's own phones, not the host (the MWI notifier works by
line binding). Hoteling sessions, like calling settings, are not in
subject export or erasure.

### 3.9 Shared call appearance

The BroadSoft flavour (§5 Q5): the `call-info` event package with
`line-seize`, which Yealink, Polycom, Grandstream and Cisco desk phones
implement. A shared line is a line binding whose `shared` is set with
`max_appearances` (default 4); its AoR is the number's E.164 user at the
tenant's domain. A device bound to it registers that AoR as a second
account with the device's own credentials — the templates render a second
account and its line keys (`account.2.shared_line`, `reg.2.type=shared`, and
their Grandstream and Cisco forms). The appearance table lives in Redis,
`fcp:sca:<tenant>:<line>`, index → `{ state, call_id, node, device }`,
written by the node that holds the call. Phones SUBSCRIBE `call-info` on the
line and receive `Call-Info` NOTIFYs with `appearance-index` and
`appearance-state` (`idle`, `seized`, `progressing`, `alerting`, `active`,
`held`) fanned out by the presence listener from the appearance events. An
outgoing call: the phone SUBSCRIBEs `line-seize` for an index, the server
grants the first free one (or refuses), the INVITE carries it. An incoming
call to the line: every registered device rings, the index is `alerting`,
`active` on the first answer, the others are cancelled. Hold from the
holder's phone marks the index `held` and plays hold music to the far
party; an INVITE to the line with that index from another phone retrieves
it — the held far leg is moved to the new phone as park's retrieve does.
Across nodes: lamps are right because the table is in Redis; a retrieve
that reaches a node not holding the call is forwarded to the owning node
over SIP with `X-FCP-Node-Handoff` (§5 Q9). Barge-in on an active
appearance is §3.11. The `shared_line` toggle appears on the line binding
page; the testkit phone learns `call-info`, `line-seize` and `Call-Info`.

#### 3.9.1 As built (phase 9, v0.25.284)

**The line.** A user's line binding may be `shared` with
`max_appearances` (1 to 24, default 4; migration 086, synced to sites
with the binding). Every device bound to it registers the number as an
account of its own: with the device's own credentials (§3.8.1), or the
owner's, since one "may act as" rule already lets both be the line.
Provisioning renders the account as the device's second
(`${shared_line_enabled}`, `${shared_line_yes_no}`,
`${shared_line_number}`, `${shared_line_appearances}`; set to off on a
device on no shared line so nothing is left unrendered), and the four
starter templates carry it: Yealink `account.2.shared_line = 1`, Polycom
`reg.2.type="shared"` with `reg.2.lineKeys`, Cisco `Share_Ext_2_` and
`Shared_User_ID_2_`, Grandstream account 2 with its SCA switch left to
the operator (its P-value differs between models). The API refuses
`shared` on a binding that is not a user's; the line binding dialog sets
both fields and the list shows the line shared.

**The table.** Appearances live in an `AppearanceTable`
(`fcp_sip::sca`): the node's memory, or — with a cluster — one Redis hash
per line, `fcp:sca:<tenant>:<line>`, index → appearance
(`fcp_cluster::LineAppearances`), claimed by one Lua script so two nodes
cannot take an index. An appearance names its owner (`call:<id>` or
`seize:<SIP Call-ID>`), state, node, the node's SIP URI, a handoff token,
the phone and the far party, and carries its own expiry: a seize lapses
in at most 30 seconds, a call's half an hour after its node last
refreshed it (every five minutes), so a dead node frees its keys.

**Subscriptions.** A SUBSCRIBE for `call-info` or `line-seize` goes to
the node's `Sca` before presence. The line is found by the Request-URI's
number; the subscriber must be the line (its `From`), with credentials
that may be it when `auth_invites` is on (challenged as an INVITE is).
`call-info` watchers are kept per node and sent NOTIFYs inside their
subscription (their Call-ID and tags, our CSeq) — `Call-Info` with each
appearance's `appearance-index`, `appearance-state` and `appearance-uri`,
then `appearance-index=*;appearance-state=idle` — and no body.
`line-seize` claims the index its `Call-Info` names (or the phone's own
seize, or the lowest free), 480 when taken; the phone is sent a
`line-seize` NOTIFY and every watcher the seize. Ending it frees an index
the call did not take.

**Calls.** The call manager's appearance hook
(`CallManager::set_on_appearance_changed`) tells the node every change to
a call's `CallAppearance`: progressing (a call from the line) or alerting
(to it) until answered, active, held when the line's phone holds the far
party (not when the far party holds the line), gone when it ends or is
parked away. Changes are written in order by one task and announced as
`line.appearance_changed`; the node's own watchers are told at once,
every other node's when the event reaches them (so a cluster needs
`[events] use_redis`, and the node warns without it). A call to the
line claims the lowest free appearance once the owner's plan rings the
line's phones (the phones registered at the number are added to the
binding's contacts), and a full line is 486. A call from the line takes
the appearance its INVITE's `Call-Info` names, the phone's seize, or the
lowest free; an index another phone seized, or one in use, is 403; the
appearance never reaches the far party. A phone's `Call-Info` naming an
appearance is believed from an untrusted peer (`claims.rs`); one asking
for an auto-answer is not.

**Retrieve.** An INVITE from the line naming a held appearance takes the
line's leg of that call (`take_over_leg`, as `Replaces` does): the far
party comes off hold to the new phone, the phone that held it is sent a
BYE. When the call is on another node, the INVITE's B-leg goes to that
node's SIP URI carrying `X-FCP-Node-Handoff` with the appearance's token;
the owning node takes it before any tenant, auth or routing work — the
token, which only a node that read the table can know, stands for the
phone's credentials, which the first node checked — and the call runs
through both nodes.

Deviations: the owner's plan decides whether a shared line rings (DND,
forwards and call waiting are the owner's); the devices' INVITEs are not
given the appearance index (phones take it from the lamps); no barge-in
or private hold (§3.11, §7); an appearance's live state is not in the
API; pickup by `Replaces` from a BLF key stays as §3.2.1 left it.

### 3.10 Executive-assistant

`executive_assistants` (migration 084): `executive_user_id`,
`assistant_user_id`, `position`, and the assistant's `opted_in`. On the
executive's settings: `executive: { enabled, filter: all | external | none,
alert: assistants | assistants_and_executive }`. A call to the executive
that the filter selects gets a plan whose first step rings every opted-in
assistant's devices (and the executive's, when `alert` says so) with
`Alert-Info` for a distinct ring and the executive named in the From
display ("for <executive>"); the executive's BLF shows the call as theirs.
Push: `*63` from an assistant on such a call moves the far party to the
executive's own devices, bypassing the filter. Call on behalf:
`*64<number>` from an assistant places the call with the executive's
caller ID and a CDR marking both users, under the assistant's class of
service. `*65`/`*66`: the executive opts in and out of assistant
handling; `*67`/`*68`: an assistant opts in and out. Self-service pages for
both roles; admin sets the relation on the user's Calling tab.

#### 3.10.1 As built (phase 10, v0.25.285)

**The relation** lives on the executive's calling settings, not in a
table of its own: `executive: { enabled, filter, alert, assistants: [{
user_id, opted_in }] }` in `user_calling_settings` (no migration; it is
synced to sites with the row). An admin names the assistants, in order,
by username or id (`PUT /v1/users/:username/calling`, re-naming keeps each
one's opt-in); the executive sets `enabled`, `filter` and `alert` at
`/v1/me/calling` but not who assists them (403); the assistant sees the
executives they assist at `GET /v1/me/executives` and opts in or out of
one at `PUT /v1/me/executives/:user_id`. At most ten assistants, none
twice, never the executive. Which executives a user assists is a scan of
the tenant's settings (`fcp_sip::executive`), asked by a feature code or
a page and never by a call being routed.

**Screening.** When the executive's plan would ring
(`ring_by_plan`), `executive_screening` rings every opted-in assistant's
phones instead — and the executive's too under
`assistants_and_executive` — when the handling is on, the filter takes
the call (`external`: the caller is no user of the tenant) and some
assistant's phone is up. The legs carry `Alert-Info:
<http://127.0.0.1/executive>;info=executive` for a distinct ring and show
the caller as "<caller> for <executive>"; the ring time and the no-answer
treatment stay the executive's. A call from one of the assistants, and a
call an assistant pushed, ring the executive.

**Codes.** `*63` from an assistant finds the executive's call they
answered (by their leg's dialog, not the dialed address) and has the node
carry out a blind transfer of the far party to the executive's extension
(`TransferRun`, `X-FCP-Transferred-From` naming the call, which is what
spares it the screening); the assistant is let go once the executive
answers. `*64<number>` places the number for the executive the assistant
serves (the first, by username, they are opted in for): the assistant's
class of service decides what may be dialed, the executive's line is the
caller ID at the trunk (`Call.on_behalf_of` in `caller_presentation`),
and the record's `custom_data.on_behalf_of` names both users. `*65`/`*66`
turn the executive's handling on and off (404 with no assistants);
`*67`/`*68` opt the assistant in and out for every executive listing
them (404 for a user who assists nobody).

**Pages.** The user's Calling dialog (admin) and Calling page (self)
carry the handling: on/off, which calls, who rings, and — for an admin —
the assistants. The Calling page lists the executives the user assists
with a switch each.

Deviations: no `executive_assistants` table (above); a call to an
executive whose phones are all down is not screened (the unreachable
treatment applies); the executive's BLF shows the call as the assistant
answering it, not as the executive's; the executive a `*64` call is for
cannot be chosen at the phone.

### 3.11 Monitor, whisper and barge on any call

`fcp-media` sessions gain a third participant: `add_party(call_id, offer,
Mode::Monitor | Whisper | Barge)` implemented in forge-engine through
forge-mixer — monitor hears both, whisper is heard by the A party only,
barge hears and is heard by both — and `remove_party`. The queue's
supervisor path (packet interleaving, `remix_rtp_packet` returns the
packet unchanged) moves onto it. Exposed as `POST /v1/calls/:id/{monitor,
whisper,barge}` under a `CallSupervise` permission, and as an INVITE to an
active shared appearance from a phone with `barge_in` allowed on the line.
Events `call.party_joined`, `call.party_left`. Last, because every other
feature stands without it (§5 Q10).

#### 3.11.1 As built (phase 11, v0.25.286)

Two things the design assumed proved false: the queue's calls do not run
on forge-engine at all (they are raw RTP bridges of their own), and the
admin API holds no calls (they are on the call managers). Settled on
2026-09-28: one mixing core for both media paths, and the admin API
forwarding to the node holding the call.

**The core** is forge-mixer's `SupervisionMix` (forge-media v2026.09.28):
A, B and the supervisor written in as they arrive, one tick per 20 ms
frame saying what each listener hears — the supervisor both parties
summed; the coached party of a whisper, or both parties of a barge, the
other party with the supervisor summed in; a party that does not hear the
supervisor keeps its relay untouched. Sums saturate, a short source is
silence, a runaway one is trimmed.

**forge-engine** gives a session a third party on a port of its own
(`add_party`, `connect_party`, `set_party_mode`, `remove_party`): both
legs' audio, already decoded for every packet the session relays, feeds
the mix; the supervisor's packets are read on their port (latched to
where they send from); each tick the supervisor is sent both legs, and a
leg that hears the supervisor has its relay suppressed and its mix
scheduled through its own playout — its codec, its generated stream, its
SRTP. The mix is 8 kHz, so a leg hearing the supervisor hears narrowband
while they do; the supervisor's own leg is G.711. Stopping the session
removes the party. `fcp-media` wraps it (`MediaManager::add_party` and the
rest, `Supervision`).

**The queue's bridge** (`queue_handler::spawn_supervisor_bridge`) decodes
the caller, agent and supervisor into the same core and sends each
destination its mix on an RTP stream of its own in its own codec; the
packet interleaving and `remix_rtp_packet` are gone. Its codecs are the
queue's (PCMU, PCMA, G.722).

**Calls.** `fcp_sip::supervision::Supervisions` puts a supervisor on an
anchored, established call: their phone (a user's extension or name, its
first registered contact, or a SIP URI) is dialed with the party's SDP and
joined when it answers; the same supervisor asking again changes the mode;
a second supervisor is refused. Their BYE, a request to leave, or the call
ending takes them off. `call.party_joined` and `call.party_left`.

**Barge-in on an appearance.** A shared line's `barge_in` (migration 087,
the line binding dialog) lets a phone on the line join an active
appearance: its INVITE naming the appearance is answered with the party's
SDP, heard by both parties, and its BYE takes it off. Off, the appearance
is 403 as before. A barge-in on an appearance held on another node goes
there with the handoff (§3.9.1).

**The API.** The call manager's `POST /calls/:id/{monitor,whisper,barge}`
(`supervisor_extension`, `heard_by` `a`/`b` for a whisper, the callee by
default, `tenant_id`), `GET /calls/:id/supervisor` and `POST
/calls/:id/supervisor/leave` take every anchored call (a queue call still
through the queue), answering 404, 409 (not anchored, already supervised)
or 480 (unreachable, refused) instead of the old 200-with-an-error. The
admin API's `POST /v1/calls/:id/{monitor,whisper,barge}`, `GET` and
`DELETE /v1/calls/:id/supervisor` need `call_supervise` (tenant and
telecom admins) and are forwarded (`fcp_api_core::CallSupervisor`,
`fcp-admin`'s `NodeCallSupervisor`) to `[call_manager] url`, else to each
healthy call manager until one holds the call; the node's answer comes
back as it gave it. The Active Calls page's Supervise action.

Deviations: a supervisor's leg is plain RTP in G.711 (no SRTP); the mix
is narrowband; the queue path keeps the queue's own supervisor bookkeeping
(its `/internal/supervisor` still only changes the mode).

## 4. Phases

One PR each, in order; each proved by end-to-end tests against the running
binaries with the testkit phone (`docs/TEST_FLOOR.md`), and where a
feature crosses nodes, in the two-node cluster harness.

| Phase | What |
|---|---|
| 1 | ✅ v0.25.276 — **Foundations.** `user_calling_settings`, `CallTarget`, `plan_for_user` executed by all four paths (§3.1); forward all, busy, no answer and unreachable working; DND; the extension-to-user path with voicemail; flow `Dial` to a user; the presence listener for every tenant; the feature codes; the calling API and pages. Tests: each forward, DND, a user with no phone reaching voicemail, a flow dialing a user. As built (§3.1.1): one JSON row per user, the plan as `fcp_sip::ring_plan::plan` with the contacts resolved by the path, treatments kept on the call, a forward as a new call into this node at its own port, a flow's `Dial` reporting `busy` or `no_answer` for a plan that does not ring. Test: `fcp-call-manager/tests/calling.rs`. |
| 2 | ✅ v0.25.277 — **Hold, transfer and pickup** (§3.2). The held state, hold answered locally when anchored, the answer bug fixed, park corrected and retrieve as a leg move, REFER executed blind and attended, INVITE with `Replaces`, dialog-info with ids and held. Tests: hold seen by the B2BUA and BLF, attended transfer among three phones, park and retrieve. As built (§3.2.1): the held leg, the anchored answer fixed, REFER carried out by the node (blind, to a park code, attended through `Replaces`), INVITE with `Replaces`, park and retrieve as leg moves, the orbit's timeout as a transfer back to the parker; hold still passed through (answered locally with phase 3) and dialog-info's ids with phase 9. Test: `fcp-call-manager/tests/calling.rs`. |
| 3 | ✅ v0.25.278 — **Music on hold** (§3.3). forge-media's looped, transcoded, replacing player (a forge-media release); the resolver, the built-in default, tenant and per-user prompts, park and hold, the anchoring reason. Test: the phone hears the file's tone while its party is held and while parked. As built (§3.3.1): forge-media's player through scheduled playout (forge-media v2026.09.27); holds answered by the node when media is anchored; the music chosen per holder, per tenant, else built in; the parked party held with music; `[media] hold_music` anchors every call; a flow `Play` repeats `loop_count` times; the queue naming a prompt not built (the number's music v0.25.388, the tenant's v0.25.392). Test: `fcp-call-manager/tests/calling.rs`. |
| 4 | ✅ v0.25.279 — **Call waiting** (§3.4). The per-user call set in the registry, the plan's busy rule, `*56`/`*57`. Tests: a second call rings or is refused by the setting; on two nodes. As built (§3.4.1): a user's calls as leases (`fcp_cluster::CallLeases`, subject `User`) kept by a parties hook on the call manager, this node's own count without a cluster; call waiting off with a call up is the busy treatment; `*56`/`*57`; the Calling page's switch. The class's call limit waits for phase 7. Tests: `calling.rs`, and `trunks.rs` on two nodes. |
| 5 | ✅ v0.25.280 — **Schedules** (§3.5). The entity, the tenant timezone, the number's closed and holiday targets, the user's after-hours forward, DB queue hours, the flow node, the pages. Tests: a number answered by a flow after hours, a queue closed on a holiday. As built (§3.5.1): `schedules` (migration 083, JSON rows, synced to sites), the number's `schedule_id`/`closed_target`/`holiday_target`, the user's after-hours forward in the plan, a database queue's hours decided before the queue, the flow `Schedule` node, `flow:` and `queue:` targets placed, `/v1/schedules` and a Schedules page. The tenant's timezone waits for phase 7 with its country. Test: `calling.rs`, `api_postgres.rs`. |
| 6 | ✅ v0.25.281 — **Find-me / follow-me** (§3.6). Steps, external legs with confirmation, `Diversion`, the page. Tests: a sequential ring reaching a trunk stub that must press 1. As built (§3.6.1): steps after the user's phones (or instead of them when none is up), each forked with its own ring time; confirmation by a prompt to the answering leg alone and one digit; `*60`/`*61`; the Calling page's editor. The `Diversion` header followed in v0.25.389 (`trunks.rs`). Test: `calling.rs`. |
| 7 | ✅ v0.25.282 — **Class of service** (§3.7). The entity, classification, enforcement for the effective user, find-me legs and call-on-behalf, the number's screening lists, the line's caller ID applied, the pages. Tests: a premium number refused for one user and placed for another, a withheld caller ID, a blocked caller. As built (§3.7.1): classification by the tenant's country (US, CA, GB, DE, FR, AU, NZ) and overrides, `classes_of_service` and the tenant's `calling` (migration 084), placed calls, forwards and find-me steps checked, `max_calls` both ways, the number's screening (433/603), the line's caller ID and `withheld` at trunk egress, `/v1/classes-of-service` and its page. Call-on-behalf waits for phase 10. Test: `calling.rs`, `trunks.rs`, `api_postgres.rs`. |
| 8 | ✅ v0.25.283 — **Hoteling** (§3.8). Device credentials and the registrar's check, hosts, sessions, login and logout by DTMF, `check-sync`, the effective user in every path, expiry, the API and pages. Tests: a guest logs in on a host phone, is rung there and calls out as themself, expires. As built (§3.8.1): device credentials sealed at rest and one "may act as" rule for REGISTER and INVITE, `hoteling_sessions` (migration 085) with one guest per host, `*88` through an inline flow's `HotelingLogin` node and `*89`, `check-sync` and MWI to the host, the sweep, the effective user for the caller, the class of service, the calling codes and `*98`, the ring plan and presence, `/v1/hoteling/sessions`, `/v1/me/hoteling` and the Hoteling page. Tests: `calling.rs` (a login after a wrong PIN, the guest rung at the host and calling there under their class and codes, `*89`, a session run out, credentials registering only what is theirs) and `api_postgres.rs`. |
| 9 | ✅ v0.25.284 — **Shared call appearance** (§3.9). Shared lines, second-account templates, the appearance table, `call-info` and `line-seize`, retrieve from another phone, the testkit phone's packages. Tests: two phones on one line, lamps through seize, alert, active and held, retrieve on the other phone; lamps right across two nodes. As built (§3.9.1): `shared`/`max_appearances` on the line binding (migration 086), the second account in provisioning and the four templates, the appearance table in memory or Redis (`fcp:sca:…`, one Lua claim), `call-info` NOTIFYs inside the subscription and `line-seize`, the call manager's appearance hook and `line.appearance_changed`, a retrieve by `Call-Info` taking the line's leg, handed to the owning node with `X-FCP-Node-Handoff` (siphon-rs `IntegratedUAC::send_request`, sip-uac 0.7.6). Tests: `calling.rs` (two phones, lamps, hold and retrieve, seize, a full line), `trunks.rs` (lamps and a retrieve across two nodes), `line_appearances.rs`, `api_postgres.rs`. |
| 10 | ✅ v0.25.285 — **Executive-assistant** (§3.10). The relation, the filter and the plan, push, call on behalf, opt-in codes, the pages. Tests: an external call to the executive rings the assistant, pushed to the executive, a call placed on behalf. As built (§3.10.1): the relation on the executive's calling settings (no table), screening in the plan with a distinct ring and "for <executive>", `*63` push as a node-run transfer, `*64` on behalf under the executive's line and the assistant's class with the record naming both, `*65`–`*68`, `/v1/me/executives` and the pages. Tests: `calling.rs` (screen, push, the four switches, the filter), `trunks.rs` (a call on behalf at the carrier), `api_postgres.rs`. |
| 11 | ✅ v0.25.286 — **Monitor, whisper and barge** (§3.11). The third party in forge-engine (a forge-media release), the API, the queue's supervisor path moved, barge-in on an appearance. Tests: a supervisor hears both parties, a whisper heard by one, the queue supervisor tests still passing. As built (§3.11.1): forge-mixer's `SupervisionMix` shared by forge-engine's third party and the queue's bridge (forge-media v2026.09.28), `fcp_sip::supervision`, barge-in on an appearance (`barge_in`, migration 087), the node's routes for every anchored call and the admin API forwarding to the node holding the call under `call_supervise`, `call.party_joined`/`party_left`, the Active Calls page. Tests: forge-engine and forge-mixer unit tests, the queue bridge's, `calling.rs` (monitor, whisper, barge, leave; a barge-in on an appearance), `api_postgres.rs`. |

## 5. Questions

Ten, settled by the user on 2026-09-27, each on its recommendation,
which is what the phases above assume.

**Q1. Are calling features per user or per number?** — *Per user.* Every
platform in the review attaches DND, forwarding, find-me, class of service
and assistants to the user, and a user reached by extension, by number and
by a flow expects one behaviour. The line binding keeps the number's
destination, devices and schedule. The binding's forwards and ring
strategy are copied into the owner's settings once and their columns
dropped by a later contraction.

**Q2. The unused routing engine in `fcp-core`.** — *Delete it* in phase 1
(`routing_engine.rs`, `forwarding.rs`, `call_state.rs`, `call_forking.rs`,
`sip_handler.rs` and their tests). Reviving it would mean a second call
path beside the handler's; its forwarding rules run on the server's local
time; nothing has needed it since it was written.

**Q3. Hold: pass the re-INVITE through, or answer it?** — *Answer it when
media is anchored, pass it through when not.* Answering locally keeps the
far phone in an ordinary call so hold music can be played into it and the
far party can be moved without its phone knowing; pass-through is the
only option when the phones exchange media directly. A tenant with hold
music anchors its calls.

**Q4. The default hold music.** — *Synthesised at startup*, a short
instrumental phrase generated by code, so the repository ships no licensed
audio and every tenant hears something on day one; a tenant's uploaded
`moh` prompt replaces it, a user's or number's prompt overrides that.

**Q5. Which shared-line flavour?** — *BroadSoft SCA* (`call-info` and
`line-seize`), not the draft bridged-line-appearance (`dialog` with
`Replaces`). It is what the four vendors' desk phones implement first, what
the platform's provisioning templates target, and Kamailio's `sca` module
is a public precedent for the server side.

**Q6. Feature codes: per node or per tenant?** — *Per node, in
`[pbx.feature_codes]`, with the North American conventions as defaults*
(`*72`/`*73`, `*78`/`*79`, `*90`–`*93`), keeping FCP's `*70`/`*71` for park
and choosing `*56`/`*57`, `*60`/`*61`, `*63`–`*68`, `*88`/`*89` where the
conventions collide with park or have no convention. Per-tenant codes are
a small later project on the same parser.

**Q7. Find-me to an external number: whose number does the carrier see?**
— *The original caller's, with `Diversion` naming the user*, which is what
a mobile user expects to see and what carriers accept from a trunk that
carries a `Diversion` header; a trunk with `caller_id_override` set keeps
overriding, as it does for every call.

**Q8. Class of service: classification by the number, or by rules?** — *By
the normalised number against a built-in country table with tenant
overrides.* A rule language already exists twice (the YAML policy and the
tenant routing rules); a class of service is what an administrator assigns
to a person, and it must read as "no international, no premium", not as a
regex. The routing rules stay for what they do. FAC/CMC authorisation codes
stay unwired: they need mid-dial DTMF collection and no reviewed platform
leads with them.

**Q9. Shared lines across nodes.** — *The appearance table in Redis, and a
retrieve forwarded to the owning node over SIP.* Lamps must be right on
every node, which the table gives; a retrieve or barge is rare and may
cost a hop. Moving the call between nodes for it is not worth the
mechanism.

**Q10. Is three-party audio in this project?** — *Yes, last.* Barge-in is
part of shared appearances on every reviewed platform and silent
monitoring is part of executive-assistant, the queue's supervisor path is
an interleaving that only works by accident, and forge-mixer already mixes.
It is the one phase that can be dropped without loosening the others.

## 6. Testing

Phases 1 to 8 and 10 are proved in `crates/fcp-call-manager/tests/`
(new files per feature beside `calls.rs`) with the testkit phone, the seed
and the carrier stub; phases 4 and 9 add scenarios in the two-node cluster
harness. The testkit phone gains what the features need: SUBSCRIBE and
NOTIFY for `dialog`, `call-info` and `line-seize`, `Call-Info` on an
INVITE, an INVITE with `Replaces`, a REFER with `Replaces`, a second
account, and `check-sync` reporting. forge-media's player and mixer changes
are unit-tested in forge-media and proved end to end here. Unit tests
cover `plan_for_user` for every rule and their order, `classify` for every
class per country, `state_at` for weekly, overnight and holiday cases in a
timezone with daylight saving, and the appearance table's transitions.

## 7. Not in scope

Per-tenant feature codes (Q6). Authorisation codes (Q8). Private hold on a
shared appearance and video on a shared line. A web softphone, SSO, SCIM
and firmware, which are the review's later recommendations. Voicemail
changes beyond the treatment reaching it. Distinctive ring per number and
per caller, which is an `Alert-Info` on the plan's INVITEs and a small
follow-up. Simultaneous ring to the WebRTC gateway's users, who join
conferences and do not register. The queue's own hold-music player, which
keeps its files and gains only a prompt as a source.
