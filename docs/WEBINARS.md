# Webinars and town halls

Status: proposed 2026-11-16. Nothing here is built.

A webinar is a meeting with a stage and an audience:
- a few people speak and are seen;
- hundreds or thousands watch and listen;
- the audience is heard only when a host lets one of them in.

FCP's rooms are built for meetings, where everyone may speak, everyone is mixed
for everyone, and everyone is on the roster. This note keeps everything a room
does well (its mixer, composite, forwarding, recording, trunks and roster) and
adds what a stage and an audience need.

## 1. Where we are

What exists, and what a town hall would hit first:

- **Two roles.** `JoinRole` is `Guest` or `Host` (`fcp_core::join_token`).
  - A host may mute, kick, lock, end, dial out, spotlight and stop a share
    (DTMF, the admin API, `/me/conferences/live/{room}/…`).
  - A guest may do everything else, speaking and sending video included.
  - There is no role that only listens.
- **Every listener has its own mix.** forge-mixer mixes N−1 for each
  participant (`mix_frame_excluding`) and encodes it for them. With 1,000
  listeners that is 1,000 mixes and 1,000 encodes of what is, for all but the
  few who speak, the same audio.
- **Video is capped at 16 tiles** in a composite (`video_max_participants`) and
  25 streams to a forwarding viewer (`MAX_STREAMS`). Everyone else in a large
  room is an audio-only tile. A stage of a few panelists fits these limits; the
  audience does not need tiles at all.
- **The roster lists everyone.** Each subscriber's RFC 4575 document is built
  for them alone, and capped at 1,000 users (`roster.rs`, `MAX_USERS`). Every
  join or leave is a partial NOTIFY to every subscriber. With 2,000 viewers,
  every one of them gets a NOTIFY whenever any of them joins or leaves.
- **Distributed rooms are a full mesh.** Trunks join every node hosting the room
  to every other, each carrying its local mix: N×(N−1) trunk streams, which
  `docs/DISTRIBUTED_CONFERENCING.md` calls fine for the two or three nodes
  placement produces. A room's placement fills one node to
  `conference_node_capacity` before it spreads (default 0: never).
- **Admission** counts participants per node (`max_participants_per_node`, 500),
  per room (`max_participants`), and video work units and egress
  (`video_egress_mbps`).
- **A waiting room**, but only "wait for a moderator": guests wait until a host
  joins. There is no admitting by a host from a phone or the app.
- **Nothing for an audience to say.** No raise hand, Q&A, chat or polls
  (`docs/SOFTPHONE_VIDEO.md` §6 says so).
- **No way out but a recording.** The composite records to WebM or fragmented
  MP4 (`forge-mp4`). Nothing streams to RTMP, HLS or SRT.
- **Join tokens name a room, not a person.** A token is reusable until it
  expires (at most 24 h), so there is no per-person registration or attendance.

Measured, from the forwarding work (`docs/FORWARDING_ROOMS.md` R3b): one core
forwards about 1,300 SRTP streams at 500 kb/s, so a node's link, not its CPU,
is the limit on what it can send.

## 2. The model

### 2.1 Roles

The meeting gets a kind, `meeting` (today's) or `webinar`. In a webinar,
`JoinRole` gains two roles:

| Role | Heard | Seen | Shares | Gets | Controls |
|---|---|---|---|---|---|
| Host | yes | yes | yes | the stage | everything a host has today, plus the audience's |
| Panelist | yes | yes | under `video_content_share` | the stage | none |
| Attendee | only when promoted | no | no | the stage | raise hand, ask |

- A token, a PIN or a meeting's own list says the role.
  - `host_pin` gives a host, as today.
  - A new `panelist_pin`, or a panelist token, gives a panelist.
  - `guest_pin`, or no PIN at all, gives an attendee.
  - The meeting's `panelists` (users by id) are panelists when they sign in.
- In a `meeting`, `Guest` keeps today's meaning. In a `webinar`, a guest is an
  attendee.
- **Promoted attendees.** A host can bring an attendee to the stage for a
  question, unmuted, and later send them back. A promoted attendee is heard
  like a panelist, and seen only if they send video and the host asks for it.
  The promotion lasts until the host ends it or the attendee leaves, never
  past the call.

### 2.2 Audio: mix once, send many

The stage is everyone heard: hosts, panelists and promoted attendees. Its
listeners fall into two groups.

- **The stage hears N−1**, as every participant does today. Stage members are
  few (16 at most, the composite's own cap), so this is cheap.
- **The audience hears the stage, all of it.** Since no attendee contributes,
  every attendee hears exactly the same mix. The room makes it once per frame
  and encodes it once per codec and packet time. It then sends the packets to
  each attendee, rewriting only SSRC, sequence and timestamp offsets, and
  re-encrypting per leg. This is what forwarding does for video, and costs the
  same per packet.
- **Stage membership changes the mix, not the listener.** A promoted attendee
  moves to the stage: they now hear N−1 and are heard. Sent back, they hear the
  audience mix again.
- **The audience is not in the mixer's contributor set at all.** Its silence
  costs nothing, its voice activity is not run, and no attendee becomes the
  active speaker.

What this changes in forge-media:
- forge-mixer gets a shared output per (codec, ptime) that any number of legs
  subscribe to;
- forge-conference puts a participant on that output instead of their own N−1
  when they are a listener.

The shape is the composite subscribers' (one encoder per flavor, `Subscriber`s
fed from it) brought to audio.

### 2.3 Video for the audience

The stage is at most 16 people, so everything already in place for video serves
it:

- **The composite.** One encode per flavor (resolution × codec), shared by every
  attendee on that flavor, as composite subscribers share one today, with the
  bitrate ladder per subscriber.
- **Forwarding.** In a `video_mode = auto` webinar, an attendee's client that
  asks (`Recv-Info: fcp-view`) is forwarded the stage's own streams, as a
  forwarding viewer is today. The allocator, REMB or transport-cc
  (`docs/TRANSPORT_CC.md`), simulcast and the speaker stream all apply.
- **Nothing from the audience.** An attendee's camera section is answered
  receive-only. A promoted attendee may send video if the host lets them; their
  section is re-INVITEd to send.

The audience's share of a node is egress. At the documented defaults an attendee
costs about:
- **audio only:** 40 kb/s Opus at 20 ms, plus RTP, UDP and IP overhead;
- **forwarded:** the default view (the speaker at 720p plus up to eight at 180p),
  about 1.5–2.8 Mb/s.

So `video_egress_mbps` at 1,000 Mb/s holds a few hundred watching attendees, or
thousands listening only. Placement and admission already price egress (R3b); a
webinar counts an attendee's join at its view.

### 2.4 Rosters and the audience

- **The stage's roster** (hosts and panelists) gets what a roster has today, and
  the audience as a count. A host's client also gets the audience by page (§2.6).
- **An attendee's roster** is the stage and the count, and nothing about the
  other attendees: no attendee is on another attendee's roster.
- **The count is coalesced.** It goes in a partial NOTIFY at most once every
  2 s, not one per join. 1,000 people joining in a minute is about 30 NOTIFYs to
  each subscriber, not 1,000.
- **Attendees who never subscribe** (a plain SIP phone) are unaffected.

This also puts the 1,000-user cap out of reach: no document lists the audience.

### 2.5 The audience's voice: raise hand and Q&A

- **Raise hand.** An attendee raises or lowers their hand.
  - From a phone with DTMF: `*4`, free in the conference's DTMF table (`*9` is
    leave).
  - From Anvil, the join page or a SIP client that asks (`Recv-Info: fcp-qa`):
    an INFO.
  - From the API: `POST /me/conferences/live/{room}/hand`.
- **Hands on the stage.** The stage's rosters show raised hands (a `fcp:hand`
  element in the stage's view of the audience). A host promotes one, or lowers
  all.
- **Q&A.**
  - An attendee asks a question: text, with their display name or "Anonymous"
    where the meeting allows it.
  - Questions are kept: a `webinar_questions` row, `pending`, `answered` or
    `dismissed`. An attendee can upvote others' questions.
  - A host or panelist answers in text, or marks a question "answered live".
  - Attendees see the questions a host has published, and their own.
  - Questions go both ways by an INFO package (`fcp-qa`, JSON) in the call and
    by `/me/conferences/live/{room}/questions` for the app. The gateway
    relays them.
- **Moderation.** A meeting's `qa` is `off`, `moderated` (a question is seen by
  the audience only once a host publishes it) or `open`.
- **No chat.** It is a different moderation problem at a town hall's scale, and
  Q&A covers what the audience needs to say. It can follow as its own project.

### 2.6 The webinar's life

- **States:** `scheduled` → `practice` → `live` → `ended`.
  - **Practice:** the stage is in the room and hears itself; attendees who
    arrive wait in the lobby (§2.7). Nothing is recorded or streamed.
  - **Live:** a host's "go live" (API, the app, or `*95` from a phone, beside
    the host's `*91`–`*94`) lets the
    lobby in. Recording and streaming start if the meeting says to.
  - **Ended:** a host's end, or the stage leaving. Attendees are told and
    released (a BYE after an announcement).
- **The lobby** is today's wait for a moderator, widened: attendees wait while
  the webinar is not live, with the meeting's hold music or prompt. They are let
  in together when it goes live, at most 50 a second, so a thousand do not
  rejoin the mixer in one frame.
- **Registration.**
  - A webinar can require it. Each registrant gets an attendee token naming
    them (`JoinToken` gains `registrant`, a row in `webinar_registrants`).
  - The token says who joined, and a host can revoke one person's.
  - Without registration, the room's attendee link and PIN work as a guest's do
    today.
- **Attendance.**
  - Every leg's CDR already says who, when and for how long (`CdrKind::
    Conference`). It now says the role and the registrant too.
  - `/conferences/meetings/{id}/attendance` reads it back per session: who
    registered, who came, for how long, their questions and hands.
- **The audience by page.** `/me/conferences/live/{room}/audience?cursor=` and
  the admin's equivalent, for a host who wants to find someone, not a NOTIFY to
  everyone.

### 2.7 Scale across nodes: a stage and its edges

A full mesh does not suit an audience. The audience sends nothing, so it needs
nothing from any node but the stage's output. The model becomes a tree.

- **The stage node** hosts the stage (hosts, panelists, promoted attendees),
  mixes and composes, and records.
- **Edge nodes** host attendees only.
  - Each gets the stage's output once over a trunk: the audience mix, the
    composite flavors its attendees use, and the forwarded stage streams its
    attendees ask for (R7's forwarding trunk, one way).
  - It sends each to its own attendees.
  - It sends back only what its attendees raise: hands, questions, and a
    promoted attendee's voice. That voice crosses on a trunk stream of its own,
    and only while they are promoted.
- **Placement.**
  - A webinar's stage goes where today's rule puts the first host.
  - Attendees fill the stage node to a webinar share of its capacity, then the
    least-loaded edge in the caller's region.
  - `conference_node_capacity` applies to attendees per node, with the egress
    budget the deciding limit.
- **Promotion across nodes.**
  - An attendee on an edge who is promoted is not moved: their voice goes up
    the edge's trunk to the stage, and the stage's mix carries it to everyone.
  - Moving the leg (as draining does) is not needed for a question.

Trunk streams stay at the number of edges, not N×(N−1): each edge has one trunk
to the stage.

### 2.8 Streaming out

For an audience that is not on FCP at all.

- **HLS.**
  - The recorder already writes the composite as fragmented MP4 (`forge-mp4`,
    H.264 and Opus). CMAF segments of the same, two seconds each, with a media
    playlist, are HLS.
  - The stage node writes them to the recording's directory or a bucket
    (`[api.firmware.s3]`'s shape, reusing `S3Target`).
  - The admin serves the playlist, or a CDN in front of the bucket does.
  - The latency is several seconds, which suits an audience that does not
    speak.
- **RTMP push**, for YouTube Live and the like.
  - The composite at one flavor, H.264 and AAC in FLV, pushed to a URL and key
    the meeting holds sealed at rest.
  - AAC is an encoder forge-media does not have, and FLV and RTMP are new code.
    This is its own phase, after HLS.
- **What goes out** is the composite: one picture, the stage's mix. Per-panelist
  tracks out are not in scope.

### 2.9 Clients

- **Anvil (A7).**
  - An attendee's call is receive-only by default: no camera, mic muted at the
    room.
  - It has a raise-hand button, the Q&A panel, the stage's roster with the
    audience count, and the "you are on the stage" moment when promoted.
  - A host's call also has the hands, the questions to publish and answer, go
    live and end.
- **The join page** gets the same attendee view. It already draws a forwarding
  room's grid, which is the stage.
- **SIP phones** get the audience mix and the composite, `*4` for a hand, and
  their host DTMF if they joined as a host.

## 3. What this does not do

- **Chat, polls, reactions and breakout rooms.**
- **Simultaneous interpretation** (a second audio channel per language).
- **Per-panelist tracks** in the recording or the stream out.
- **More than 16 on the stage.** The stage is a meeting's size; the audience is
  what grows.
- **Moving a promoted attendee's leg** to the stage node.
- **Scheduling and invitations.** A webinar has a start time for its
  registration page and attendance report, but FCP sends no email.

## 4. Risks

- **The shared mix's echo.** A promoted attendee who is still listening on their
  own speaker must not hear themselves. Moving them to N−1 at the moment they are
  promoted is the fix; a frame of overlap is possible, and acceptable.
- **A lobby let in at once.** A thousand legs joining one mixer in one frame
  costs a stall. The lobby lets them in at most 50 a second (§2.6).
- **Egress, not CPU, runs out.** A webinar's attendees are priced by their view
  at admission. A node full of watching attendees has spare CPU and no link; the
  placement rule has to read the egress budget first. R3b already does.
- **HLS latency.** Viewers on HLS are 6–10 s behind the stage. Q&A from them
  comes in late. The note says so in the UI, and Q&A is from FCP's own clients.

## 5. Phases

- **W1: roles and the shared mix** (forge-media, FCP).
  - `meeting.kind`, `JoinRole::Panelist` and `Attendee`, `panelist_pin` and the
    meeting's `panelists`.
  - In forge-mixer and forge-conference, a listener's leg on the shared
    audience mix.
  - Attendees muted at the room, not in the contributor set.
  - fcp-loadgen measures 1,000 attendees on one node, audio only: CPU per
    attendee and the mixer's frame time.
- **W2: the audience's video and roster** (forge-media, FCP).
  - Attendees on the composite flavors and as forwarding viewers, camera
    sections answered receive-only.
  - The stage's roster and the attendee's (the count, coalesced).
  - Admission and placement pricing an attendee at their view.
  - Measured: attendees watching per node.
- **W3: hands and Q&A** (FCP).
  - Raise hand by DTMF, INFO and API; promote and send back.
  - `webinar_questions` (migration), the `fcp-qa` INFO package, the
    `/me/conferences/live/{room}/questions` routes, `conference.question_*`
    and `conference.hand_*` events.
  - The gateway relaying the package.
- **W4: the webinar's life** (FCP).
  - Practice, go live, end.
  - The lobby let in gradually.
  - Registration with tokens per registrant (`webinar_registrants`) and
    revocation.
  - The attendance report from CDRs.
  - The admin UI's webinar pages.
- **W5: a stage and its edges** (FCP).
  - Edge nodes on one-way trunks from the stage, placement for attendees, and
    promotion across a trunk.
  - Measured with fcp-loadgen across two and three nodes, to 5,000 attendees.
- **W6: HLS** (forge-media, FCP).
  - CMAF segments from forge-mp4, the playlist, the bucket or directory, and the
    admin serving it.
- **W7: RTMP push** (forge-media, FCP).
  - An AAC encoder, FLV, the RTMP client, and the meeting's sealed stream key.
- **A7: Anvil.** The attendee's and the host's views (§2.9).

W1 and W2 come first. W3 and W4 follow in either order. W5 waits on W1's
measurements to say when one node is not enough. W6 and W7 are independent of
W3–W5, and A7 follows W3.

## 6. Questions

Each has a recommendation.

1. **A webinar as a kind of meeting, or a new resource?** *Recommendation:* a
   kind (`meeting.kind = webinar`). Room numbers, PINs, video settings,
   recording and placement are all a meeting's already; a webinar adds roles,
   Q&A, registration and a life.
2. **New roles, or attendee as a muted guest?** *Recommendation:* new roles,
   `Panelist` and `Attendee`. A muted guest still has an N−1 mix, a roster entry
   and the right to unmute; the shared mix (§2.2) and the roster (§2.4) need the
   room to know who is audience.
3. **One shared audience mix, or N−1 for everyone as today?**
   *Recommendation:* shared. It is what makes a thousand listeners cost one
   encode; nobody in the audience is heard, so nobody needs their own voice
   taken out.
4. **How does an attendee speak?** *Recommendation:* raise hand, and a host
   promotes them. The attendee is moved to the stage's N−1 and heard by all.
   They are not moved between nodes; their voice crosses the edge's trunk.
5. **Q&A, chat, or both?** *Recommendation:* Q&A first, moderated by default
   for a webinar. Chat at a town hall's size is a moderation project of its own.
6. **Attendees on the roster?** *Recommendation:* as a count, coalesced every
   2 s. A host pages through them by API.
7. **Registration?** *Recommendation:* optional per webinar, with a token per
   registrant naming them, so attendance and revocation are per person.
8. **Scaling past a node: full mesh, or a stage and edges?**
   *Recommendation:* a stage and edges (§2.7). Trunks grow with edges, not
   edges squared, and an edge sends nothing back but hands, questions and a
   promoted voice.
9. **Streaming out: HLS, RTMP, or both?** *Recommendation:* both, HLS first.
   The recorder already writes CMAF's shape. RTMP needs an AAC encoder and two
   new formats.
10. **Recording a webinar?** *Recommendation:* the composite, as every room
    records. Under `auto_record` it starts when the webinar goes live, not
    during practice.
11. **The lobby's rate?** *Recommendation:* 50 a second, a setting
    (`[conference] lobby_admit_per_sec`), so a thousand are in within 20 s.
12. **Attendee video from a promoted attendee?** *Recommendation:* allowed when
    the host asks for it. Their camera section is re-INVITEd to send, and drops
    back to receive-only when they leave the stage.
