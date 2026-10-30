# Parking lots

D8 finding 35 (2026-10-30): "I do not see a way to manage parked spaces and configure
park ranges."

Agreed the same day:

1. **Lots per tenant.** A tenant has one or more named parking lots, managed in the
   API and the admin UI, with the calls parked in them visible live.
2. **The number range is the tenant's own.** We use 7xx today; a customer may want
   5xx or 432X. Each lot's range is set by the tenant.

## 1. What exists

Parking is a node setting, not a tenant's:

- **The range is in each call manager's TOML** (`[pbx.parking] orbit_start`,
  `orbit_end`, `timeout_seconds`; 701–720 and 60 s by default,
  `fcp-pbx/src/config.rs`). No API reads or writes it; the admin UI never mentions it.
- **Every tenant shares the range.** `ParkingManager` (`fcp-pbx/src/parking.rs`) is one
  map per node keyed by orbit; a parked call carries its `tenant_id` but nothing
  checks it. `*71701` from tenant B retrieves tenant A's call in 701.
- **Every node allocates on its own.** `next_available_orbit` looks only at the node's
  map, then the cluster store writes `park:{orbit}` with a plain `SET`
  (`fcp-storage-redis/src/parking.rs`). Two call managers parking at once take the
  same orbit, and the second write replaces the first: the first parked call can no
  longer be retrieved from another node.
- **Orbits are `u16`.** `050` cannot exist, and a lot's number is not a string the way
  an extension is.
- **One way in, one way out.** `*70` parks in the next free orbit and the parker hears
  it read out (`park_announcement`); a transfer to `*70` does the same; `*71<orbit>`
  retrieves. Dialing the orbit itself (`701`) reaches nothing, a transfer cannot name
  the orbit to park in, and a phone's line key cannot watch one.
- **A timeout rings the parker** (`fcp-call-manager/src/main.rs`, "Parked call
  expired, routing back to parker"), and nothing else is possible.

## 2. The model

**A parking lot** belongs to a tenant:

| Field | |
|---|---|
| `id`, `tenant_id`, `name` | `lot-…`; "Main lobby" |
| `first`, `last` | The range, as digit strings of the same length: `701`–`720`, `500`–`599`, `4320`–`4329`, `050`–`059`. Two to eight digits, at most 1,000 slots |
| `park_number` | Optional: a number that parks in the lot's next free slot (`700`), as `*70` does |
| `timeout_secs` | How long a call waits (default 60) |
| `on_timeout` | `parker` (default), then `fallback`: a `CallTarget` (an extension, a queue, a flow, the parker's voicemail) if the parker does not answer |
| `music` | The lot's hold music prompt, else the tenant's |
| `location_id` | Optional: a site's lot; `*70` from a phone at that location parks there |
| `enabled` | |

**A slot** is one number in a lot's range. A slot's number is dialable like an
extension, so the lot's range is checked against the tenant's whole dial plan when it
is written: a range overlapping another lot, an extension, a number's short form, a
hunt group, a queue, a conference room or a feature code is refused with the clash
named.

**A parked call** is held by its tenant and slot: `{tenant}:park:{slot}` in the
cluster Redis, claimed with `SET NX` and an expiry of the lot's timeout, so a slot
belongs to one call cluster-wide. The node's in-memory map becomes a cache of the
calls it holds itself, keyed the same way.

**The default lot.** A tenant with no lot gets one built from the node's
`[pbx.parking]` values, so a deployment that never opens the page parks as it does
today. Its first save in the API makes it an ordinary lot.

## 3. Calls

- **Park, next free:** `*70` (during a call or as a transfer target) or the lot's
  `park_number`. The lot is the phone's location's, else the tenant's first. The
  parker hears the slot read out and is hung up on, as today; the 200 OK carries
  `X-FCP-Parked: <slot>`.
- **Park, directed:** a transfer to a slot number parks in that slot when it is free,
  and is refused (486) when it is taken. This is what a VVX park key and an attendant
  console do.
- **Retrieve:** dialing the slot number, or `*71<slot>` as today. An empty slot is 404
  with an announcement.
- **Timeout:** the parker's phones ring for the lot's ring time with the call shown as
  "Parked call from <slot>"; unanswered, the `fallback` target takes it; with no
  fallback the call ends with a short announcement.
- **Watching a slot:** a slot is a presentity (`dialog` event): a desk phone's or
  Anvil's line key on `705` shows occupied, and pressing it retrieves. Provisioning
  writes BLF keys for a lot's slots when a device's `line_keys` ask for them.

## 4. API and UI

- `GET/POST /api/v1/parking/lots`, `GET/PUT/DELETE /api/v1/parking/lots/{id}` under
  new permissions `parking_read` / `parking_manage` (tenant admins hold both; operators
  `parking_read`). A write that clashes with the dial plan is 409, naming the clash.
- `GET /api/v1/parking/calls`: the calls parked now, across the cluster (slot, lot,
  caller, parker, since, node).
- `POST /api/v1/parking/calls/{slot}/retrieve` with a target (a user or extension:
  their phones ring and answering takes the call) and `DELETE` to hang up a parked
  call, through the node holding it, as live call control does.
- `parking.call_parked`, `parking.call_retrieved`, `parking.call_timed_out` events, for
  webhooks and the page.
- **The Parking page** (Calls section): the lots with their ranges, and a live board
  of the slots, empty and occupied, with Retrieve to… and Hang up.
- Lots and their settings are journaled and synced to a site like the rest of the
  dial plan; a site parks in its own Redis.

## 5. Phases

1. **The lot and a slot that is one call's.** `parking_lots` (migration), `fcp_core::parking`,
   the dial-plan clash check, the default lot, slots as strings, the cluster claim
   (`SET NX` per tenant and slot), `*70`/`*71` and `park_number` through the lot, the
   API's lot routes. Fixes the cross-tenant retrieve and the two-node collision.
   Tested in `calling.rs` with two tenants on the same range and two nodes parking
   at once.
2. **Every way in and out.** Dialing a slot retrieves, a transfer to a slot is a
   directed park, the timeout's parker ring and fallback, the lot's music, the events,
   `GET /parking/calls`, retrieve-to and hang-up.
3. **The page and the keys.** The Parking page with its live board; slots as
   presentities; provisioning's BLF keys for slots; Playwright over the page.
4. **Sites.** Lots journaled and synced; a site's parking in its own Redis; the cloud's
   board shows a site's calls through its link.

## 6. Done

**Phase 1 (v0.25.483).** `fcp_core::parking` (the lot, its range as digit strings,
`lot_for`, `ParkingLotStore`), `parking_lots` (migration 128), `/api/v1/parking/lots`
under `parking_read` / `parking_manage` with the clash check against the tenant's other
lots, extensions, queues and meeting rooms (409 naming it). A call manager reads a
tenant's lots at most every 10 s; `*70` and a lot's `park_number` park in the lot,
else in the node's `[pbx.parking]` range as before. A slot is held by tenant and slot
on the node (`ParkingManager`) and claimed in Redis with `SET NX` on
`park:{tenant}:{slot}` before it is taken, with an expiry of the lot's timeout and a
minute; an expired call frees its claim. With Redis unreachable a slot is taken on the
node alone. `*71<slot>` retrieves within the caller's tenant. `call.parked` and
`call.unparked` carry `slot` (and `orbit` as a number for a release; `unparked` used to
say 0). Tested: `a_tenants_own_lot_numbers_the_slots` (calling.rs),
`a_slot_is_one_calls_and_each_tenant_has_its_own` (fcp-storage-redis),
`two_tenants_number_their_slots_alike` (fcp-pbx),
`parking_lots_are_the_tenants_and_clash_with_its_dial_plan` and the route matrix.

**Phase 2 (v0.25.484).** Dialing one of a lot's slots retrieves it (an empty slot is
480, as `*71` is); a transfer to a slot is a directed park, refused 486 when the slot
holds a call; a transfer to a lot's park number parks in the next free slot. A
timed-out call rings its parker shown as "Parked <slot>: <caller>", through the node
so the parker's plan applies, and if that fails the lot's fallback is rung (a number,
extension, user or queue at the node; a SIP address; `voicemail` as the parker's
mailbox), else the call ends. The lot's music plays to the parked party before the
parker's or the tenant's. The events are the calls' own: `call.parked`,
`call.unparked` and the new `call.park_timed_out` (slot, lot, parker, fallback), not a
`parking.*` family. The node answers `GET /parking/calls`,
`POST /parking/calls/:slot/retrieve` and `DELETE /parking/calls/:slot`; the admin API's
`GET /api/v1/parking/calls` (`parking_read`) asks every call manager and puts the lists
together by slot, and `POST …/{slot}/retrieve` and `DELETE …/{slot}` (`parking_manage`)
go to the node holding it. A call sent on that nobody answers ends, as a timed-out one
with no fallback does. Tested: calling.rs
`a_transfer_parks_in_the_slot_it_names_and_dialing_it_takes_the_call`,
`a_parked_call_its_parker_lets_ring_goes_to_the_lots_fallback`,
`the_node_lists_sends_on_and_hangs_up_parked_calls`; api_postgres
`parked_calls_are_listed_sent_on_and_hung_up_through_the_node`; fcp-admin
`parked_calls_are_asked_of_the_call_managers`. The lot's music has no test of its own.

## 7. Questions (agreed 2026-10-30 on their recommendations)

1. **A slot is dialable on its own** (`705` retrieves), as well as `*71705`. Recommended:
   yes; it is what people expect from a park key and an attendant.
2. **Directed park by transfer to a slot**, refused when taken. Recommended: yes.
3. **Timeout:** the parker first, then the lot's fallback. Recommended: yes, with the
   parker's ring time from their calling settings.
4. **Lots per location**, so `*70` at a site parks in that site's lot. Recommended:
   yes, optional per lot.
5. **Who may retrieve:** anyone in the tenant. Recommended: yes; a class of service
   could limit it later.
6. **BLF on slots** in phase 3 rather than phase 2. Recommended: yes.
