# Distributed Conferencing

One conference spread across several conference nodes, for size (a
thousand-participant town hall) and for resilience (a node failure loses
only its own participants). The design is **cascaded mixing**: each node
mixes its local participants and exchanges one mixed stream per peer node,
so a room with participants on three nodes costs three extra audio streams,
not three hundred.

The work lands in four phases. This document tracks what is in place.

| Phase | What | Status |
|---|---|---|
| 1 | Conference registry + cluster-aware placement | ✅ v0.25.82 |
| 2 | Inter-node trunks (cascaded mixing) | ✅ v0.25.83 |
| 3 | Cross-node control (kick, gain, lock, announce, end) | ✅ v0.25.84 |
| 4 | Resilience (registry expiry, trunk teardown, draining) | planned |

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
`conference` (the `[service] service_type`) and their SIP bind address; the
selector returns that address as the target host.

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

Trunk media is plain RTP with the codec negotiated from `G722`, `PCMU`,
`PCMA` (wideband first, filtered by `conference.allowed_codecs`), no DTMF,
no SRTP — the inter-node network is assumed private; trunk SRTP is a phase
4 item.

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
