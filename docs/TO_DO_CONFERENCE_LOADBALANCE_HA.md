
❯ would it be possible to split conferences between confernece servers.  For example say
there are 3 confernece nodes in the system, can one conference have users on all 3 nodes?
I'm thinking of a way to handle large conferneces, 1000 + users and also have some
resilinacy.

● Great question. This is absolutely possible and it's how large-scale conferencing works.
  Let me check what infrastructure you already have for this:

● Searched for 4 patterns, read 1 file (ctrl+o to expand)

● Good — you already have cluster infrastructure with Redis heartbeats, node membership, and
  a call registry. Here's how distributed conferencing would work:

  Architecture: Cascaded Mixing

                      ┌─────────────┐
      Participants    │   Node A    │    Participants
      P1, P2, P3 ──▶ │  Local Mix  │ ◀── P4, P5
                      │             │
                      └──────┬──────┘
                             │ RTP trunk
                             │ (mixed audio)
                      ┌──────┴──────┐
      Participants    │   Node B    │    Participants
      P6, P7, P8 ──▶ │  Local Mix  │ ◀── P9, P10
                      │             │
                      └──────┬──────┘
                             │ RTP trunk
                             │
                      ┌──────┴──────┐
      Participants    │   Node C    │
      P11..P20   ──▶ │  Local Mix  │
                      └─────────────┘

  Each node:
  1. Mixes its local participants (what it does today)
  2. Sends its local mix to every other node via an RTP trunk (one UDP stream per peer node)
  3. Receives remote mixes from other nodes
  4. Combines remote mixes with the local mix for each participant's output

  This is how FreeSWITCH, Opalvoip, and most large-scale bridges do it.

  What You Already Have

  ┌─────────────────────────────────────────────────────┬─────────┐
  │                      Component                      │ Status  │
  ├─────────────────────────────────────────────────────┼─────────┤
  │ Cluster membership (Redis heartbeats)               │ Working │
  ├─────────────────────────────────────────────────────┼─────────┤
  │ Node discovery (ClusterManager, DashMap of members) │ Working │
  ├─────────────────────────────────────────────────────┼─────────┤
  │ Call registry (which node owns a call)              │ Working │
  ├─────────────────────────────────────────────────────┼─────────┤
  │ Inter-node HTTP (NodeClient)                        │ Working │
  ├─────────────────────────────────────────────────────┼─────────┤
  │ Conference mixer with mix-minus                     │ Working │
  ├─────────────────────────────────────────────────────┼─────────┤
  │ RTP send/receive                                    │ Working │
  ├─────────────────────────────────────────────────────┼─────────┤
  │ Event bus                                           │ Working │
  └─────────────────────────────────────────────────────┴─────────┘

  What's Needed

  1. Conference Registry (Redis)

  Like CallRegistry but for conferences. Maps room_number → [node_a, node_b, node_c] so any
  node knows which peers host participants for a given conference.

  2. Inter-Node RTP Trunks

  When a second node joins a conference, each node pair establishes a single RTP stream
  carrying the mixed audio of their local participants. This is just an audio bridge like
  what you already have — but instead of a phone on one end, it's another conference node.

  3. Remote Mix Injection

  Each node adds a virtual participant per remote peer (like the __audio_feedback__
  participant) that injects the remote node's mix into the local mixer. The local mixer's
  mix-minus naturally excludes the remote mix from being sent back to that remote node.

  4. Participant Routing

  When a new caller joins conference 8000, the system checks Redis for which nodes host this
  conference and routes to the node with the fewest participants (load balancing). The
  SBC/proxy layer handles this.

  5. State Synchronization

  Mute, kick, lock, host status, recording — these need to be coordinated via Redis pub/sub
  or the event bus. When node A mutes a participant, nodes B and C need to know.

  Capacity with 3 Nodes

  ┌────────────────────────────┬─────────────┬───────┬──────────────────────┐
  │          Scenario          │  Per node   │ Total │      Feasible?       │
  ├────────────────────────────┼─────────────┼───────┼──────────────────────┤
  │ 1 conference x 1000 users  │ ~333 each   │ 1,000 │ Yes                  │
  ├────────────────────────────┼─────────────┼───────┼──────────────────────┤
  │ 10 conferences x 100 users │ ~333 each   │ 1,000 │ Yes                  │
  ├────────────────────────────┼─────────────┼───────┼──────────────────────┤
  │ 50 conferences x 100 users │ ~1,667 each │ 5,000 │ Need larger machines │
  └────────────────────────────┴─────────────┴───────┴──────────────────────┘

  The inter-node RTP trunks add minimal overhead — it's just 1 extra RTP stream per peer node
   per conference (not per participant).

  Resilience

  If node B dies:
  1. Redis heartbeat expires (15s)
  2. Nodes A and C detect the loss
  3. Participants on B lose their connection (SIP BYE timeout or SRTP keepalive failure)
  4. Those participants redial and get routed to A or C
  5. The conference continues on the surviving nodes

  For zero-downtime failover, you'd add participant migration: when a node is draining, it
  re-INVITEs its participants to another node before shutting down. The existing shutdown() +
   draining flag in the conference handler is the foundation for this.

  Implementation Effort

  This is a significant feature — probably 2-3 weeks of work. The biggest pieces are:

  1. Conference registry (Redis) — ~1 day
  2. Inter-node RTP trunk (reuse audio bridge pattern) — ~3 days
  3. Remote mix injection (virtual participant in mixer) — ~2 days
  4. State sync (Redis pub/sub for mute/kick/lock) — ~3 days
  5. Participant routing (SBC/proxy conference-aware load balancing) — ~2 days
  6. Testing and tuning — ~3 days

