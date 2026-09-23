# Distributed Conferencing

One conference spread across several conference nodes, for size (a
thousand-participant town hall) and for resilience (a node failure loses
only its own participants). The design is **cascaded mixing**: each node
mixes its local participants and exchanges one mixed stream per peer node,
so a room with participants on three nodes costs three extra audio streams,
not three hundred.

The work lands in five phases. This document tracks what is in place.

| Phase | What | Status |
|---|---|---|
| 1 | Conference registry + cluster-aware placement | ✅ v0.25.82 |
| 2 | Inter-node trunks (cascaded mixing) | ✅ v0.25.83 |
| 3 | Cross-node control (kick, gain, lock, announce, end) | ✅ v0.25.84 |
| 4 | Resilience (registry expiry, trunk teardown, draining, trunk SRTP) | ✅ v0.25.85 |
| 5 | Participant migration off a draining node | ✅ 5a v0.25.146 (the node), 5b v0.25.147 (the call manager), 5c v0.25.148 (the WebRTC gateway) |

Decisions (2026-09-02): full-mesh trunks between hosting nodes; new
participants stay on a hosting node until it reaches a capacity setting;
trunk INVITEs authenticated by **mutual TLS only**; one PR per phase.

## Phase 1: registry and placement

### Conference registry

`fcp_cluster::ConferenceRegistry` keeps one Redis hash per room,
`cluster:conference:<tenant>:<room>`, mapping node id to that node's local
participant count. Writes refresh a 60 s TTL, so a node that stops
publishing disappears from the room on its own.

Every conference node publishes:

- on every join and leave (and when a room is destroyed), from the
  conference handler through the `RoomRegistry` hook;
- every cluster heartbeat interval (5–30 s) for all active rooms, which
  also keeps the TTL alive.

Only real callers count; the `__…` virtual participants (audio feedback,
AI, and later the inter-node trunks) are excluded.

### Placement

When the call manager routes a call to a conference room it asks its
`ConferenceNodeSelector`, which applies
`fcp_cluster::choose_conference_node`:

1. Among nodes already hosting the room that are alive in cluster
   membership and under `conference_node_capacity`, pick the fullest, so a
   room stays on as few nodes as possible.
2. Otherwise the live conference node with the fewest active calls.
3. If no conference node is in the cluster, fall back to the configured
   `b2bua.conference_server_uri` (and its hot-reload override), exactly as
   before.

Since v0.25.87 the rule is region-aware (`docs/MULTI_REGION.md`): steps 1
and 2 are tried first among the nodes in the call manager's own region,
then across the whole cluster. A caller therefore joins on a conference
node in their region even when another region already hosts the room; the
cascade trunk between the two hosts carries the cross-region audio once.
Nodes without a `[cluster] region` are never "local" and only take part in
the cluster-wide pass.

Since v0.25.103 the rule is also budget-aware for video meetings
(`docs/VIDEO_CONFERENCING.md` §9.3): every conference node advertises
`video_budget` — per device, the codecs it runs and the work units left
and total — and when the meeting allows video the "fewest active calls"
choice in step 2 becomes "most video budget left on a device with one of
the meeting's codecs", a node known to have none coming last and a node
advertising no budget in between (unknown, neither preferred nor
excluded). Hosting nodes in step 1 keep their priority whatever their
budget: inter-node trunks carry audio only, so spreading a video room
would cost its participants each other's video. On the node itself a join
whose video would not fit is refused with 503 like the participant cap;
the budget is re-priced from the running rooms every heartbeat and
published with it.

`conference_node_capacity` lives in the call manager's `[b2bua]` section.
`0` (the default) never spreads a room, which is the right setting until
phase 2 lands: without trunks, participants on a second node would not hear
the first.

```toml
[b2bua]
conference_server_uri = "sip:conference@10.0.0.20:5074"   # fallback only
conference_node_capacity = 100                              # after phase 2
```

Conference nodes advertise themselves in cluster membership with node type
`conference` (the `[service] service_type`) and their SIP address — the
`[sip] udp_bind` with an unspecified IP replaced by `local_ip`, so a node
bound to `0.0.0.0` advertises somewhere a caller can reach; the selector
returns that address as the target host, and a migration's `Refer-To`
names it too.

## Phase 2: trunks

When two conference nodes host the same room, the node with the lower id
dials the other over SIP/TLS, and the callee admits the call as a hidden
`__trunk__<node>` participant. Each trunk carries only the sending node's
local participants, so a room spread over three nodes is a full mesh of
three trunks and no participant is ever mixed twice.

### What proves a trunk is a trunk

A trunk participant bypasses every caller-facing admission check — PIN,
lock, waiting room, room capacity, the per-node participant limit. It can,
because the *connection* proves who is calling, not the message:

- The trunk INVITE arrives on the callee's TLS listener (`sip.tls_bind`),
  which requests a client certificate and verifies it against the cluster
  CA (`sip.tls_client_ca_path`; siphon-rs `v2026.09.03` mutual TLS). The
  verified identity travels with the request as `TransportContext ::
  peer_identity` into the conference handler.
- The INVITE names the dialing node in `X-FCP-Trunk-Node` and the tenant in
  `X-FCP-Trunk-Tenant`. The node header is never trusted on its own: the
  certificate must **name that node** — as its Common Name, a DNS SAN, or
  the user or host part of a `sip:` / `sips:` URI SAN (RFC 5922 §7.1).
  No certificate, or a certificate for another node, is a 403.
- The room must already have local callers on the callee (480 otherwise);
  a trunk to a node with nobody to mix is pointless and the caller's own
  reconcile would not hold it.

Outbound, the dialing node presents its own certificate (`sip.tls_cert_path`
/ `sip.tls_key_path`, or the `tls_client_*` overrides) and verifies the
peer's certificate against the cluster CA. Peers are dialed by the address
they advertise in cluster membership while their certificates name the
node, so a certificate chaining to the cluster CA is accepted under any DNS
name it carries; public servers under the system roots keep the ordinary
name check. Outbound TLS connections are pooled and read (siphon-rs
`TlsPool`), so responses and in-dialog requests come back on the connection
the INVITE went out on.

### Certificates

Every conference node needs, from one cluster CA:

- a certificate whose CN or a DNS SAN is the node's `service.node_id`
  (a `sip:<node_id>@…` URI SAN also works for the inbound check, but the
  outbound name check needs a DNS SAN);
- extended key usage covering both `serverAuth` and `clientAuth` (or no
  EKU extension at all) — the same certificate is presented both ways;
- a private key file readable only by its owner (siphon-rs refuses to load
  anything more permissive).

`sip.tls_client_auth = "required"` (the default when a CA is set) refuses
any TLS peer without a certificate at the handshake. Use `"optional"` only
while rolling certificates out across a fleet; a trunk INVITE from a peer
that presented nothing is still rejected.

### When trunks connect and disconnect

The trunk manager reconciles each room against the conference registry
(phase 1) on every local join and leave, and every
`cluster.trunk_reconcile_interval_secs` (default 2 s) for every room the
node hosts:

- for each other node the registry shows hosting the room (with local
  participants, and alive in cluster membership), the node with the lower
  id dials if no trunk exists yet;
- a trunk whose peer no longer hosts the room, or is no longer a live
  member, is hung up;
- when this node's last local participant leaves, all of its trunks are
  hung up.

A failed dial is retried after 10 s. A trunk INVITE from a peer this node
believes it already has a trunk with replaces the old one (the peer would
not redial if it thought its trunk was up). Trunk sessions carry no SIP
session timer and are exempt from the session-expiry reaper; the
reconciler is what tears them down. Trunks publish no participant events
and are excluded from placement counts (the `__` prefix rule).

`GET /conferences/{room}` on a conference node lists the room's `trunks`
(peer node ids) alongside its participants.

### Media: the frame-clock mixer

Cascaded mixing needs each trunk's outbound audio to be a mix of the local
callers only — not the other trunks, or a mesh would echo every node's
audio back to it — while every local caller hears everyone. forge-media
`v2026.09.03` adds a frame-clock mode to `forge-mixer` for this, and the
conference server runs every room on it:

- one clock task per room takes one frame from every participant per
  20 ms tick and snapshots it; nothing is consumed by reading;
- each participant's outbound loop awaits the tick and reads its N-1 mix;
  a trunk reads `mix_frame_filtered(not a trunk, not the feedback channel)`;
- the recorder and an attached AI participant read the same snapshot.

This also fixes a defect in single-node conferences: the previous mixer
drained a participant's buffer on read, and with three or more callers each
frame of speech reached only the listener whose tick came first.

Trunk media is RTP with the codec negotiated from `G722`, `PCMU`, `PCMA`
(wideband first, filtered by `conference.allowed_codecs`) and no DTMF;
SRTP per section under `cluster.trunk_srtp` (phase 4).

### Media: video on a trunk

Video follows the same rule, drawn. Since v0.25.113 a trunk carries a
second RTP section when the room's meeting has video and the dialling node
can encode a codec the meeting allows. On it:

- **The peer's composite is one tile.** `VideoRoom::add_remote` registers
  it as a source that draws a tile without being a participant — it is not
  in the audio room, draws no join or leave, and raises no
  `video_state_changed`. It is named for the node behind it, and the API's
  `GET …/video` marks it `remote`.
- **What goes back is local callers only.** The trunk subscribes with
  `OutputScope::LocalOnly`, the exact analogue of
  `mix_frame_filtered(not a trunk)`: a composite sent to a peer carries no
  tile that came from a peer. Exclude-self would do for two nodes, but with
  three a tile forwarded through one peer would be drawn again via another,
  and the same caller would appear twice.
- **One tile per node, not per caller.** A three-node room shows each
  peer's whole composite in one cell. Per-participant tiles across nodes
  stay out of scope.
- **Who is speaking behind a remote tile** is the peer's to say, and until
  it does (phase 5c) a remote tile is never the room's active speaker: the
  energy behind it is a node's mix, not a person's.

The video section is negotiated **when the trunk is dialled**, so a meeting
that turns video on later gets it when its trunks are next re-established —
which reconcile does on the hour, or when a node drains. SRTP applies per
section: a peer that answers video without the key on an otherwise
encrypted trunk gets audio only, and one that drops the audio key is hung
up, as before. Liveness stays on the audio section.

Capacity needs no special case: the remote tile is a decode and the
local-only composite an encode plus a compositor, and `room_cost` prices
both from the room's own status, so a node's video budget already counts
what its trunks cost. Admission does not price a trunk in advance — a
trunk is infrastructure and is never refused for budget; the periodic
re-pricing picks it up.

### Configuration

```toml
[service]
node_id = "conf-a"                      # what the certificate must name

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
tls_bind = "0.0.0.0:5061"               # trunks arrive here
tls_cert_path = "/etc/fcp/conf-a.crt"    # CN or DNS SAN = conf-a
tls_key_path  = "/etc/fcp/conf-a.key"    # mode 0600
tls_client_ca_path = "/etc/fcp/cluster-ca.pem"
tls_client_auth = "required"            # default when a CA is set
local_ip = "10.0.0.20"                  # advertised with tls_bind's port

[cluster]
enabled = true
trunk_reconcile_interval_secs = 2
```

and, on the call manager, `b2bua.conference_node_capacity` set to the
per-node room size at which new joiners spread to another node (phase 1).
A conference node without `tls_bind`, a client CA and a certificate logs a
warning at startup and hosts rooms on its own only.

## Phase 3: cross-node control

A control request can be sent to **any** conference node — the one an
operator's tooling happens to know — and takes effect across the whole
room. Nodes talk to each other over the HTTP API address each advertises
in cluster membership (`http.advertised_address`, or the bind address
with an unspecified IP replaced by `sip.local_ip`), on `/internal`
routes authenticated with a shared `cluster.secret`. A forwarded request
is applied by the receiving node only and never forwarded again, so every
request travels exactly one hop.

| Operation | Endpoint | Reach |
|---|---|---|
| Lock / unlock | `POST /conferences/{room}/lock`, `/unlock` | every hosting node, plus the registry |
| End | `DELETE /conferences/{room}` | every hosting node |
| Kick | `POST /conferences/{room}/kick/{pid}` | the node the participant is on |
| Mute / unmute | `POST /conferences/{room}/mute/{pid}`, `/unmute/{pid}` | the node the participant is on |
| Gain | `POST /conferences/{room}/participants/{pid}/gain` | the node the participant is on |
| Announce | `POST /conferences/{room}/announce` | one hosting node (the trunks carry it) |
| Video status | `GET /conferences/{room}/video` | the first hosting node that has the room |
| Video layout | `PUT /conferences/{room}/video/layout` `{"layout": …}` | every hosting node |
| Pin / unpin | `POST /conferences/{room}/video/pin/{pid}`, `DELETE …/video/pin` | the node the participant is on / every hosting node |
| Spotlight / clear | `POST /conferences/{room}/video/spotlight/{pid}`, `DELETE …/video/spotlight` | the node the participant is on / every hosting node |
| Participant video | `POST /conferences/{room}/participants/{pid}/video/{on\|off}` | the node the participant is on |
| Start recording | `POST /conferences/{room}/recording/start` | the first hosting node that has the room |
| Stop recording | `POST /conferences/{room}/recording/stop` | every hosting node (whichever is recording stops) |
| Recording file | `GET`/`DELETE /recordings/{file}?node={node}` | the named node, over `/internal/recordings/{file}` |

**Room-wide operations** (lock, unlock, end) are applied locally and, at
the same time, on every other node the registry shows hosting the room
(with participants, and alive as a conference node in cluster
membership). The response succeeds when any node applied it and lists
each peer's answer:

```json
{
  "status": "locked", "room": "8000",
  "node": "conf-a", "hosted_here": true,
  "peers": [ { "node": "conf-b", "ok": true, "status": 200, "body": { "status": "locked", "room": "8000" } } ]
}
```

`DELETE` also reports `participants_removed` summed across the cluster,
with this node's share in `participants_removed_here`. A peer that could
not be reached shows `ok: false` with no `status`; when the registry
itself is unreachable the operation still applies locally and the
response carries `peers_error` instead of `peers`.

**Participant operations** (kick, mute, unmute, gain) are applied locally
when the participant is here. When it is not — this node answers 404 —
the hosting peers are asked one at a time until one has the participant;
its answer comes back with `node` set to that peer. A 404 from every node
returns `peers` with each answer.

**Video controls** (video conferencing phase 3c, `docs/VIDEO_CONFERENCING.md`
§8 and §12) follow the same two shapes. Layout, unpin and clear-spotlight
are room-wide: each node composes its own canvas, so each applies the
change. Pin, spotlight and a participant's video on/off go to the node
the participant is on. A node that has the room but no video yet answers
409 and is not forwarded past; an unknown layout is a 400. Clearing the
spotlight keeps the spotlight layout (the active speaker fills the
canvas) — set the layout to go back to a grid. Every hosting node's
`VideoRoom` emits its own `conference.video_layout_changed`, so a room
spread over several nodes produces one event per node with identical
content. `GET …/video` is one node's view: its participants and its
canvas; the other hosting nodes' local participants appear only as the
trunk carries them.

The admin API proxies these under `/api/v1/conferences/live/{room}/…`
(and dial-out, plus `/api/v1/me/conferences/{id}/dial-out` for a
meeting's owner) to the conference server named in the admin's
`[conference] server_url`; any conference node will do as the entry
point, since it forwards as above.

**Announcements** are played on one node only: the announcer is mixed
like any caller, so its audio crosses the inter-node trunks and every
participant on every node hears it once. The request is played here when
the room is active here, otherwise forwarded to one hosting peer. The
`audio_path` is forwarded unresolved, so a bare filename is looked up in
the *peer's* `conference.announcements_dir` — keep announcement files in
step across conference nodes.

### The lock and the registry

The room-wide lock is also written to the conference registry (a
`__locked` field in the room's hash — fields starting with `__` are never
node ids). A node that starts hosting a room other nodes already host
reads the flag when it creates its local copy and starts the room locked,
so placement cannot spread a locked room to a fresh, unlocked node. The
flag is cleared on unlock and otherwise lives as long as the room's key:
it expires with the last host's TTL (60 s after the last node stops
publishing). Trunk INVITEs still bypass the lock, as before.

`RoomLocked` / `RoomUnlocked` events are published once, by the node that
received the request; nodes applying a forwarded lock publish nothing.

### Configuration

```toml
[http]
bind_address = "0.0.0.0:8090"
advertised_address = "10.0.0.20:8090"   # optional; else sip.local_ip + bind port

[cluster]
enabled = true
secret = "…the same value on every conference node…"
```

Without `cluster.secret` the node logs a warning at startup, its
`/internal` routes answer 404, and a control request only affects the
node it was sent to. The API's own bearer token (`http.auth_token`) is
not involved in node-to-node calls, and the `/internal` routes are not
rate limited.


## Phase 4: resilience

What happens when a conference node dies, restarts, or is taken down on
purpose — and what keeps the inter-node media private.

### A node dies

- **Membership** expires its heartbeat after `cluster.heartbeat_ttl_secs`
  (15 s). Placement, trunking and cross-node control only ever consider
  live members, so from then on nothing new is sent to the node.
- **Registry entries** are stamped: each node's field in a room's hash is
  `<participants>@<unix seconds>`, and readers drop any field older than
  the registry TTL (60 s). Before this, the key's TTL was the only expiry,
  and a live node refreshing the room kept a dead node's field alive
  forever — a node coming back with the same id was then "hosting" rooms
  it knew nothing about, and was dialed for them. A bare count written by
  an older version is taken as fresh.
- **Trunks to the dead node** are hung up by the reconciler as soon as the
  peer leaves membership, or sooner by the RTP timeout below. Hanging up
  a trunk tears it down locally first and sends the BYE in the background
  with a 5 s bound, so a peer that will never answer cannot stall the
  reconciler (a BYE over TCP/TLS otherwise waits out the full transaction
  timer).
- **Its callers** are gone with it. They redial and are placed on a
  surviving node. Participant migration (moving a caller between nodes
  without a redial) is not part of this phase.

### A trunk goes quiet

A peer that crashes and comes back inside the membership TTL, or drops a
trunk session without a BYE, leaves the other side holding a trunk that
carries nothing — and, on the dialing side, never redialing because it
believes the trunk is up. So a trunk sends a frame every 20 ms whether or
not anyone is talking (an empty mix becomes a silent frame), and the
reconciler tears down any trunk that has received no RTP for
`cluster.trunk_rtp_timeout_secs` (default 30; `0` disables) and dials it
again at once. The callee side of a redial replaces the stale trunk it
still holds, as in phase 2.

### Taking a node down: draining

Since capacity phase 2b-4 (`docs/CAPACITY_AND_SURVIVABILITY.md` §6) this
is the platform's drain, not the conference server's own: it is the same
sequence every routing binary runs, and it starts either way —

```bash
curl -sX POST   http://node:8090/drain   # leave service, node keeps running
curl -s         http://node:8090/ready   # 503 while draining; /health stays 200
curl -sX DELETE http://node:8090/drain   # serve again, rooms untouched
```

— or on SIGTERM / SIGINT, after which the node stops. Either way it:

1. **Advertises `draining`** in cluster membership, published immediately
   and with every later heartbeat. Placement never sends a new
   participant to a draining node, not even for a room it already hosts;
   trunking and cross-node control still treat it as a host while it has
   callers.
2. **Refuses new callers** (503). Trunk INVITEs from peers are still
   accepted, so the callers already here keep hearing the rest of the
   room until they leave.
3. **Moves the callers it can** to other nodes by REFER (phase 5, below),
   before anyone is waited for.
4. **Waits up to `service.drain_wait_secs`** (default 0) for whoever is
   left to leave on their own, checking once a second.
5. **Hangs up whoever remains**, each with
   `Reason: SIP;cause=200;text="node draining"` and its record; trunks
   are left to the teardown.
6. On a shutdown only: **leaves every room in the registry** (awaited,
   not fire-and-forget) and removes its heartbeat. A drain called off
   before the deadline does none of this — the node is serving again,
   with its rooms as they were.

```toml
[service]
shutdown_timeout_secs = 30
drain_wait_secs = 300          # let a meeting finish before hanging up
```

### Trunk media: SRTP

Trunk media is SDES-SRTP (`AES_CM_128_HMAC_SHA1_80`) by default. The
dialing node generates its key and offers `RTP/SAVP` with an `a=crypto`
line; the callee answers with its own key whenever the offer carries one,
regardless of `conference.enable_srtp` (which governs callers). Keys travel
inside the mutual-TLS INVITE, so only the two nodes see them. A dialing
node that offered SRTP and gets an answer without it hangs up and logs
why: set `cluster.trunk_srtp` to the same value on every conference node
(`false` for plain RTP on a network you trust).

```toml
[cluster]
trunk_rtp_timeout_secs = 30    # 0 disables the liveness check
trunk_srtp = true
```

### Recording a distributed room

A room is recorded **once**, by one node, to one file (design §11). The
node is the **lowest-id host** — the rule the trunks are dialled by — so
a room hosted on two nodes and started through each of them is still
recorded once: a start that lands on a higher-id host is forwarded down,
and only an unreachable peer leaves it where it landed. A room this node
does not host at all goes on `wherever_it_is`. Stopping is room-wide, so
whichever node is recording stops whichever node the request reaches. That node's mix is the whole room's, because
the trunks already carry every other node's audio into it; its composite,
until trunk video lands (phase 5b), shows the callers on that node.

The file stays on the node that wrote it, and the registry row says which
node that is. A download or delete can therefore be sent to any node:
given `?node=`, a node that is not the holder streams the file from that
peer over `/internal/recordings/{file}`, authenticated with the same
`cluster.secret` as every other cross-node call. The admin API's
`/v1/recordings/{id}/download` uses exactly this, so one URL works
whichever node recorded.

Retention follows the file: the holding node deletes both the file and the
row under the tenant's `recording_retention_days`
(`docs/COMPLIANCE.md`), and the hourly compliance sweep takes only rows
still there a week past the window — a node that never came back.

### One speaker the nodes agree on

A room hosted on several nodes has to name one active speaker, or each
node spotlights its own loudest caller and the layouts disagree. Since
v0.25.115 they agree, without electing a leader:

- **Each node announces its own speaker.** Every
  `cluster.speaker_interval_ms` (400 ms by default, well inside the 800 ms
  a challenger needs to take the floor) a node that hosts a trunked video
  room posts `{node, participant_id, display_name, energy, speaking}` to
  every hosting peer's `POST /internal/conferences/{room}/speaker`. A node
  whose speaker is a peer's caller announces nothing — that peer is the one
  who can see them.
- **Every node runs the same rule over the same candidates:** its own
  callers' levels, plus one claim per peer, applied to the peer's remote
  tile. The energies are the mixer's own smoothed RMS, produced the same
  way everywhere, and an exact tie falls to the lower node id — so the
  answer does not depend on which order a node's map iterated in. They
  converge within a take interval.
- **A claim that stops being refreshed is dropped** after five intervals,
  so a node that goes quiet or partitions does not hold the floor of the
  whole room. A trunk that goes takes its tile and claim with it.
- **`conference.active_speaker_changed {room, participant_id, node_id}` has
  one emitter** — the node the speaker is on — so a listener sees one
  change per change, not one per hosting node. (`video_layout_changed`
  stays per-node: it is a room-wide *write*, acknowledged by each node
  that applied it.)
- **`GET …/video` is a room-wide read.** It asks every hosting node and
  merges: participants are the union, each stamped with the node it is on;
  the speaker and layout are the room's, with `agreed: false` while the
  nodes are still converging; and `nodes` carries each node's own view.
  The admin API's proxy and the Live Video dialog show that.

Turning `speaker_interval_ms` to 0 switches cross-node agreement off: each
node then picks from its own callers and a peer's tile never takes the
floor.

### One shared screen the nodes agree on

A shared screen (`VIDEO_CONFERENCING.md` §15.6) crosses a trunk the way a
composite does, and the room's content floor is one fact across its
nodes. Since v0.25.127:

- **The trunk carries a content section.** Beside its video section the
  trunk offers a third `m=video` with `a=content:slides` (payload type 98,
  its own port pair and, under `cluster.trunk_srtp`, its own key) when the
  meeting allows sharing; the answering node takes it under the same
  rule. What goes down it is this node's own presenter's screen, local
  only — a screen that came from a peer is never sent back — and what
  comes up it is the peer's presenter's screen, a remote content source
  under the trunk's tile that takes the floor here as a caller's would.
  Every node's composite and content views show it.
- **Each node announces its own share.** On the speaker timer, a node
  whose own caller holds the floor posts `{node, participant_id,
  display_name, since_ms}` to every hosting peer's
  `POST /internal/conferences/{room}/content`. A node whose floor is a
  peer's screen announces nothing.
- **The earliest share wins.** A node that hears a claim older than its
  own presenter's share takes the floor from them with reason `replaced`
  — the presenter is told so (a browser through its gateway's `INFO`), and
  may share again later — so the peer's screen can have it. An exact tie
  falls to the lower node id, then the lower participant id: the same
  answer everywhere. A claim that stops being refreshed for five intervals
  releases the floor the peer's screen holds here; so does a trunk that
  goes.
- **The presenter's name.** A peer's screen holds the floor under its
  trunk's tile; the browsers on this node are told the presenter's name
  from the peer's claim, and told again if the packets beat the claim.
- **`conference.content_started` / `content_stopped` have one emitter**,
  the presenter's own node, as the speaker change does. **`GET …/video`**
  names the presenter and their node in `video.content` (`node_id`), each
  node's `nodes[]` entry says whose screen it shows, and `trunks[]` says
  whether a trunk carries `content`.

### Not in this phase

- Participant migration off a draining node (REFER back into the room, or
  a call-manager-side re-INVITE) — callers on a node that goes away redial.
  Phase 5, below.
- Registry and membership when Redis itself is unreachable: every node
  keeps what it has (trunks are left as they are on a registry read
  failure) and reconciles when Redis returns.

## Phase 5: participant migration

Phase 4's drain keeps a node's callers until they leave on their own or
`drain_wait_secs` runs out, then hangs them up; they redial and land on a
surviving node. Phase 5 moves them instead. A drained node asks whoever
holds each caller's leg to re-place the call on another node, hands that
node everything it needs to admit the caller as the same participant, and
drops its own leg once the new one is up. From the caller's side the room
goes quiet for a moment and comes back; nobody redials, nobody re-enters a
PIN, a locked room does not lock them out.

### Who holds a caller's leg

A conference node's SIP peer is rarely the phone. The legs a draining node
holds, and what each needs:

| Leg | Peer | How it re-places |
|---|---|---|
| A SIP caller through the call manager (the common case) | the call manager's B2BUA B-leg | 5b: the B2BUA dials the new node as a fresh B-leg, re-INVITEs the caller with the new node's answer (or re-points the anchored media session), swaps legs |
| A browser through the WebRTC gateway | the gateway's own SIP leg (it dials the node itself and relays the browser's media) | 5c: the gateway dials the new node, swaps the relay's SIP side underneath the browser; the browser sees nothing |
| A SIP endpoint that dialed the node directly, or a dial-out leg | the endpoint | it follows the REFER as any transfer, if it supports REFER; otherwise it is hung up at the end of the drain as today |
| An inter-node trunk (`__trunk__<peer>`) | a peer node | not migrated: the peers' reconcilers re-dial trunks as the registry changes |
| A caller still in the waiting room | any of the above | not migrated: never admitted, hung up at the end of the drain |

### The mechanism: REFER with a ticket

The node sends each caller's leg an in-dialog **REFER** (RFC 3515) whose
`Refer-To` names the room on a surviving node and carries a **migration
ticket** as a URI header:

```
REFER sip:… SIP/2.0
Refer-To: <sip:8000@10.0.0.21:5074?X-FCP-Migrate=eyJ2IjoxLCJ0ZW5…>
Event: refer
```

The leg's holder dials the new node with `X-FCP-Migrate: <ticket>` as a
request header — RFC 3261 §19.1.5's rule for URI headers, restricted to
this one header (the URI-header channel is otherwise stripped, as
`SipUri::without_uri_headers` does today, so nothing else can be smuggled
through a `Refer-To`). It reports the outcome with the REFER's implicit
subscription, a **NOTIFY** with a `message/sipfrag` body: `SIP/2.0 200 OK`
once the new leg is answered, a failure code otherwise. On success it also
BYEs the old leg; the draining node treats either the 2xx NOTIFY or the
BYE as "gone to the new node".

Why REFER and not a call-manager-side watch on `draining` in membership:
the dialog is the one channel that reaches whoever holds the leg, whatever
it is — the call manager, the gateway, a phone — and the draining node is
the one party that knows which of its legs are callers. Nothing has to
record, per call, which node a B-leg went to (the call manager keeps no
such record today; the node lives only in the B-leg dialog's remote
target). And a phone that understands REFER gets migration for free.

### The ticket

The ticket is what lets the new node admit the caller **as the participant
they already were**, past every check a fresh caller would face. It is a
JSON payload, base64url-encoded and signed with HMAC-SHA256 under the
**`cluster.secret`** — the same shared secret that authenticates
cross-node control (phase 3) — in the join token's wire form
(`payload.signature`, `fcp_core::join_token`):

| Claim | Meaning |
|---|---|
| `v` | ticket version, 1 |
| `tenant`, `room` | the room; the new node takes both from the ticket, not the request |
| `participant` | the participant id, kept across the move so operator tooling and events keep naming the same participant |
| `name`, `caller_id` | display name and caller id as the old node had them |
| `host` | moderator role (host PIN or a host join token on the old node) |
| `muted`, `gain` | mixer state, reapplied on the new node |
| `video` | whether the participant's video was on (the new node's answer negotiates video as usual; this restores the on/off control) |
| `token_join` | the caller came through the WebRTC gateway (`joined_by_token`), so the new node speaks the gateway's floor `INFO` |
| `from` | the draining node's id, for the events |
| `exp`, `nonce` | 60 s expiry; the nonce is remembered by the admitting node for that long so a ticket admits once |

A node with no `cluster.secret` neither issues nor accepts tickets:
without it there is no cross-node trust, and its drain hangs callers up as
in phase 4. Why not the join token: it is a browser-facing credential under
a different secret (`join_secret`, which may be unset), it names a room
rather than a participant, and it deliberately does not bypass the lock or
the waiting room.

What the ticket **bypasses** on the new node: PIN collection, the lock
and wait-for-moderator. What it **does not** bypass: the draining refusal
(placement never picks a draining node, and a node that started draining
in between refuses the ticket like any other caller), the node's own
`max_participants_per_node`, the per-source rate limit, the codec check,
and the video budget — a room's video that does not fit is refused with
503 exactly as for a fresh join. Placement already weighed capacity and
budget when it chose the node, so a refusal is the rare case; the caller
then stays on the old node until the drain hangs them up.

### On the draining node

The shutdown sequence gains one step between "refuse new callers" and
"wait for callers to leave":

1. Advertise `draining`, refuse new callers (503) — as today.
2. **Migrate.** For every countable participant (not a trunk, not in the
   waiting room), choose a target with `choose_conference_node` — the same
   rule the call manager applies, with the room's hosts from the registry
   (the draining node is excluded by the rule itself), live membership,
   capacity 0 (a hosting node is always preferred, so the room stays on as
   few nodes as possible), the draining node's own region as the caller's
   proxy (the caller was placed here because here was local, or nothing
   was), and the meeting's video need. No target — no other live
   conference node — means no migration: the drain goes on as in phase 4.
   Otherwise mint the ticket, send the REFER (32 at a time, each bounded
   to 5 s like the shutdown BYEs), and mark the participant *migrating*
   with a deadline of `cluster.migration_timeout_secs` (default 15).
   A REFER that is rejected (a phone answering 501, a gateway that predates
   5c answering 405) leaves the participant where they are.
3. **Resolve.** A migrating participant leaves when a NOTIFY with a 2xx
   sipfrag arrives (the node then BYEs the leg itself) or when the peer
   BYEs first; either way the leave is a migration, not a hangup. A NOTIFY
   with a final non-2xx, or the deadline, leaves them in place. The step
   ends when every migration has resolved, or at the deadline.
4. Wait `drain_wait_secs` for whoever remains, hang them up, leave the
   registry — as today.

Media continuity: the old node keeps mixing for a migrating participant
until the leg is dropped, and the new node starts mixing them when their
RTP arrives, so the gap is the caller's re-INVITE (5b) or the relay's
swap (5c). While both nodes hold the participant the registry shows both
hosting the room, and the reconcilers may briefly trunk them together;
the caller sends media to one node at a time, so nothing doubles.

State that **moves** with the participant: identity, role, mute, gain,
video on/off. State that **does not**: a screen share (the content floor
is released when the old leg drops; the presenter shares again), BFCP (the
room system re-associates on the new SDP, if it does), a pin or spotlight
on the participant (room-wide layout is already on every node; a
per-participant pin is not), and a **recording** running on the draining
node, which stops at shutdown as today — the operator restarts it on the
new host.

Events: `participant_left` gains `migrated_to` (the target node id) and
`participant_joined` gains `migrated_from`; consumers that count joins and
leaves see one of each, labelled. `GET /status` reports `draining`.

```toml
[cluster]
secret = "…"                  # tickets are signed under it; unset = no migration
migration_timeout_secs = 15   # how long a leg's holder has to report after the REFER
```

*5a shipped 2026-09-13 (v0.25.146; forge-media `v2026.09.13.1`).* As
planned: `fcp_core::MigrationTicket` (the join token's wire form and
signing, shared), `migration.rs` on the conference server (the planner
over `choose_conference_node`, the ledger of REFERs in flight, the
used-ticket cache, the `Refer-To` and sipfrag forms), the drain step in
`main` between "refuse new callers" and the wait, `handle_notify` on the
conference handler trait (the SIP layer offers a NOTIFY to the conference
handler first, as it does a REFER), admission on a ticket in
`handle_invite`, and forge-conference's `add_participant_admitted` (past
the lock and the waiting room, capacity still applied, hosts tracked, no
join sound). One thing the plan's example showed and this does not send:
a `Referred-By` header — siphon-rs's `send_refer` takes no extra headers,
and nothing reads it. Tested: the ticket, planner, ledger, sipfrag and
`Refer-To` forms, and the room method are unit tests; the REFER → NOTIFY
round trip and the two-node move are not automated yet — there is no SIP
test client against a running node in the repo — and are checked by hand
with a phone that follows REFER, then properly under 5b.

### 5b: the call manager

The B2BUA already relays a REFER from either leg to the other, which is
the wrong thing for this one: the phone would be told to dial a conference
node directly, past the call manager. A REFER on the **B-leg of a
conference call** whose `Refer-To` carries `X-FCP-Migrate` is instead
executed by the B2BUA itself:

1. Answer 202. Mark the call migrating; a second REFER while one is in
   flight is answered 491.
2. Dial `sip:<room>@<host>` with `X-FCP-Migrate` as a header, offering the
   caller's SDP — the A-leg's last offer when media is passed through
   (the normal case for conference calls), the media session's B-side
   offer when anchored.
3. On the new node's 2xx: pass-through media re-INVITEs the A-leg with the
   new node's answer so the phone sends to the new node; anchored media
   re-points the session's B side (`update_session_b_leg`) and the phone
   sees nothing. If the A-leg re-INVITE fails, BYE the new leg and report
   failure.
4. Swap the call's B-leg (dialog, SDP, handle, the `sip_call_id_map`
   entry), NOTIFY `200 OK` on the old leg, BYE it. The old node's own BYE
   may cross; each side answers 481 to a dialog it no longer holds and
   goes on.
5. On the new node's failure: NOTIFY it, keep everything as it was; the
   old node hangs up at the end of its drain, as today.

To know that a call *is* a conference call, `Call` records the room it was
routed to (`conference_room`), set on the three routing branches that
produce a conference target. Migration is on by default and can be turned
off with `[b2bua] conference_migration = false`, in which case the REFER
is refused (603) and the caller is hung up at the end of the drain. It
does not go through the transfer policy (`transfer`, `allow_refer`): it is
not a transfer the caller asked for.

```toml
[b2bua]
conference_migration = true   # false refuses the REFER (603); the caller is hung up at the end of the drain
```

*5b shipped 2026-09-13 (v0.25.147).* As planned, with the SIP in
`fcp-sip`'s handler and the rest in `conference_migration.rs`. The REFER
is answered 202 at once and the move runs on its own task, since the
REFER's transaction must not wait on an INVITE to another node; `Call`
carries `conference_room`, set on the three routing branches that produce
a conference target. One detail the plan left open: the caller is
offered to the new node with its last SDP (early offer) and then
re-INVITEd with the node's answer, so a phone that keeps its ports — the
usual case — needs nothing more, and one whose answer moves its media
(a new port or address on any section) gets that answer forwarded to the
new node in a second re-INVITE. The new leg's session timer is armed as
the original's was, so the conference node's RFC 4028 reaper does not
hang the moved caller up. Tested: the `Refer-To` reading, the moved-media
check and the sipfrag body are unit tests; the move itself is not
automated — the `fcp-sip` harness has no UAC, so the B2BUA's outbound
side cannot be driven in a test — and is checked by hand between two
nodes on the GPU box.

### 5c: the WebRTC gateway

The gateway refuses every inbound request but BYE, INFO, OPTIONS and
CANCEL. It gains REFER, for its own sessions, when the `Refer-To` carries
`X-FCP-Migrate`:

1. Answer 202.
2. Build a fresh SIP side — new port pairs, new SDES keys, the same
   sections the browser has (audio, camera, screen) — and dial the new node
   with the ticket, exactly as `drive` dials the first node.
3. On 2xx, open the new `SipLeg` from the answer and hand it to the running
   relay over a channel; the relay swaps its SIP side between packets
   (sockets, SRTP contexts, payload types). The browser's PeerConnection,
   its SDP and its tracks are untouched.
4. Update the session (`sip_call_id`, `dialog`, the `by_sip_call_id`
   index), NOTIFY `200 OK` and BYE the old leg, release the old ports.
5. On failure, NOTIFY it and keep the old leg.

The join page needs no change; the migration is invisible to it.

*5c shipped 2026-09-13 (v0.25.148).* As planned. The relay takes a swap
channel and replaces its SIP side between packets (the far end's SSRCs
start over); the session keeps the shape the browser negotiated and
builds the new offer from it with the same code the first INVITE used
(`build_sip_offer`, now shared), so the new node is offered exactly the
sections the browser has; the REFER is answered 202 (481 unknown, 491 a
move already in flight, 480 not relaying yet) and the move runs on its
own task; a REFER that is not a move is answered 603 — the gateway
still does not transfer. The old leg's ports go back to the pool two
seconds after the swap, once the relay has dropped its sockets. The
sipfrag NOTIFY is `fcp_sip::conference_migration::report_refer_outcome`,
shared with the call manager. Tested: the relay swap in process
(`tests/relay.rs`: a real peer-connection pair, the SIP side swapped to
a second far end under SRTP with a new payload type, audio both ways
after it, nothing more at the old far end); the SIP side of the move
by hand, as for 5b.

### Order and tests

5a first — the ticket, the REFER on drain, the NOTIFY handling, admission
on a ticket, the events — and it is complete on its own for endpoints
that follow REFER; 5b then makes it real for every SIP caller through the
call manager; 5c for browsers. One PR each.

Tests: the ticket's mint/verify/expiry/replay and the drain planner (which
participant goes where, given a registry and membership) are unit tests;
the REFER → NOTIFY/BYE resolution is tested against a siphon-rs test UAC
playing the leg's holder. The end-to-end move across two nodes needs
Redis for the registry and a meeting in Postgres, so it runs where the
service-gated suites do (`FCP_TEST_REDIS_URL`, `FCP_TEST_DATABASE_URL`):
two conference nodes, a caller joined on the first, SIGTERM, the caller
admitted on the second as the same participant with the room locked.
