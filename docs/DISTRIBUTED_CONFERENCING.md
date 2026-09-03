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
| 2 | Inter-node trunks (cascaded mixing) | ⏳ gated on [siphon-rs #129](https://github.com/thevoiceguy/siphon-rs/issues/129) (mutual TLS) |
| 3 | Cross-node control (kick, gain, lock, announce, end) | planned |
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

## Phase 2 preview: trunks

When node B starts hosting a room that node A already hosts, the node with
the lower id dials the other over SIPS with a trunk header. The callee
admits it as a hidden `__trunk__<node>` participant that skips PIN, lock,
waiting room, and capacity checks. Each trunk carries only the sending
node's local participants (a local-only mix computed in the conference
server from the room's per-participant audio), so a full mesh never
duplicates audio. Trunks follow the registry: connect when a peer appears,
hang up when it leaves or the room empties.

Because a trunk participant bypasses every admission check, accepting one
requires proof of who is calling. That is mutual TLS on the SIP transport,
which siphon-rs does not provide yet: the TLS listener asks for no client
certificate, outbound TLS carries none, and inbound requests expose no peer
identity. siphon-rs issue #129 specifies the transport work; phase 2 starts
once it is released and the submodule is bumped.
