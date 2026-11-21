# Webinars and town halls

Status: agreed 2026-11-17: every recommendation, except that chat is in (§2.5,
§6 Q5). W1 is done (v0.25.582–583, forge-media v2026.11.17): the roles and the shared
audience mix, measured at 1,000 and 2,000 attendees on one node (§5). W2 is done
(v0.25.584–585, forge-media v2026.11.17.1): the audience's video and roster, measured
at 1,000 and 2,000 watching (§5). W3 is done: W3.1, hands and the stage (v0.25.586–587,
forge-media v2026.11.17.2), and W3.2, Q&A (v0.25.588). W3b, chat in every room, is
done (v0.25.591). W4.1, the webinar's life (practice, live, ended, and the lobby), is
done (v0.25.593–594, forge-media v2026.11.19), and W4.2, registration, attendance
and the audience by page (v0.25.595), and W4.3, the admin UI's webinar pages
(v0.25.596): W4 is complete.

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

As built (W1):
- **forge-mixer** has *listeners* (`AudioMixer::add_listener`, `set_listener`):
  what a listener writes is dropped before voice activity or recording see it,
  the frame clock takes no frame from it, and no mix reads it.
- **forge-conference** owns the shared output, not forge-mixer, because
  encoding needs a codec and forge-mixer has none. `ConferenceRoom::add_listener`
  admits one, and `listen_audience(flavor, make)` subscribes to a flavor. The
  first subscriber's `AudienceEncoder` is the flavor's. On each frame-clock tick
  the room mixes once and encodes once per flavor, before the tick wakes anyone,
  and every subscriber gets the same `AudienceFrame`. A flavor ends with its last
  subscriber.
- **The conference server's audio bridge** names a flavor by codec, mixer rate
  and frame (`PCMU/8000/160`). A leg that is a listener sends the feed's payload
  under its own SSRC, sequence and timestamp, and does not decode what it
  receives. The bridge checks the role every frame, so a promoted attendee
  hears N−1 from the next tick and a demoted one rejoins the feed.
- **What an attendee loses:** the per-listener volume (`*7`/`*8`) does nothing
  on the feed, because everyone hears the same encode. No join, exit or "you
  are alone" tone plays for a listener.
- **Visible state:** the room's JSON has `audience` (listeners, each flavor's
  subscribers and frames encoded, the feed's mean and slowest frame time), and
  each participant has `role` and `listener`.

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
- **Chat.** Every room has it, meetings as well as webinars; a webinar adds its
  scope and moderation (§6 Q5, agreed 2026-11-17).
  - **Scope**, a meeting setting `chat`:
    - `everyone`: an attendee posts to the room or privately to the hosts;
    - `hosts`: an attendee posts to the hosts only;
    - `off`.
    Hosts and panelists always have a private channel of their own. In a
    meeting, `everyone` is the default.
  - **Moderation.** A host or panelist can delete any message, which is gone
    for everyone, and mute one person's chat for the rest of the session.
    Deleting and muting are recorded in the room's events
    (`conference.chat_deleted`, `conference.chat_muted`).
  - **The path.**
    - An INFO package in the call (`fcp-chat`, JSON: post, delete, mute, and
      the messages as they arrive), relayed by the call manager and the gateway
      like `fcp-view` and `fcp-qa`.
    - `/me/conferences/live/{room}/chat` for the app.
    - A SIP client that does not ask for the package gets no chat.
  - **Kept.** Messages are rows (`conference_chat`, per session, with sender,
    scope, time and deletion). They are kept under the tenant's retention policy
    (`docs/COMPLIANCE.md`), held by a legal hold, and found by a subject export
    and erased by a subject erasure. They are exported with a webinar's
    attendance.
  - **Joining late.** A client joining late is sent the last 100 messages it may
    see.

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

- **Polls, reactions and breakout rooms.**
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

- **W1: roles and the shared mix** (forge-media, FCP). **Done** (v0.25.582–583,
  forge-media v2026.11.17: forge-mixer 0.6.0, forge-conference 0.18.0).
  - `meeting.kind`, `JoinRole::Panelist` and `Attendee`, `panelist_pin` and the
    meeting's `panelists`.
  - In forge-mixer and forge-conference, a listener's leg on the shared
    audience mix.
  - Attendees muted at the room, not in the contributor set.
  - fcp-loadgen measures 1,000 attendees on one node, audio only: CPU per
    attendee and the mixer's frame time.
  - As built:
    - **Storage and API.** Migration 141 adds `kind`, `panelist_pin` (sealed at
      rest like the other PINs) and `panelists` (user ids). The admin's and the
      owner's routes take them; a panelist may be named by username or id. The
      three PINs must differ. A purged user leaves every panelist list.
    - **Join tokens.** A panelist or attendee token is refused for a meeting. A
      webinar's panelist mints a panelist token for themselves through
      `POST /me/conferences/{id}/join-tokens`, under their own name; the owner
      still mints any role.
    - **Joining.** A host token or the host PIN gives a host, and a panelist
      token or the panelist PIN a panelist. A guest token, the guest PIN, or `#`
      alone gives an attendee. A webinar asks for a PIN whenever a host or
      panelist PIN is set. A guest token for a room that is no longer a webinar
      admits a guest.
    - **Migration and the admin UI.** A migration ticket carries `listener`. The
      admin UI's meeting form has the kind, the panelist PIN and a panelist
      checklist.
    - **Measured** (`fcp-loadgen/tests/webinar.rs`, release, G.711, the phones
      on the same 12-core box as the node):

      | Room | Attendees | Node CPU | Per attendee | Feed frame (mean) | Delivered |
      |---|---|---|---|---|---|
      | webinar | 1,000 | 29.8% of a core | 0.30 ms/s | 22.7 µs | 100% |
      | webinar | 2,000 | 70.5% of a core | 0.35 ms/s | 32.5 µs | 100% |
      | meeting (N−1) | 1,000 | 185.5% of a core | 1.86 ms/s | — | 100% |

      So 1,000 attendees cost a webinar 6.2× less than the same 1,000 as a
      meeting. One core carries about 3,000 listeners. What remains per attendee
      is packet work: building, encrypting and sending one packet every 20 ms.
      The feed's slowest frame was under 7 ms, well inside the 20 ms frame.
      W5's edges are wanted for egress and resilience before CPU.
- **W2: the audience's video and roster** (forge-media, FCP). **Done**
  (v0.25.584–585, forge-media v2026.11.17.1: forge-conference 0.19.0).
  - Attendees on the composite flavors and as forwarding viewers, camera
    sections answered receive-only.
  - The stage's roster and the attendee's (the count, coalesced).
  - Admission and placement pricing an attendee at their view.
  - Measured: attendees watching per node.
  - As built:
    - **The video room.** A listener is the *audience* there
      (`VideoRoom::is_audience`): never drawn and never forwarded. They
      always get a composite of the whole room, whatever `exclude_self`
      says, so the audience shares one encode per flavor. They are left out
      of the tick's join order, so the size of the audience costs the tick
      nothing. `set_listener` moves a participant in and out.
    - **The answer.** An attendee known at the INVITE (by a migration
      ticket, a token, or a webinar that asks no PIN) gets their camera and
      screen sections answered `sendonly` from the node's side, with no
      simulcast (`fcp_media::with_video_sending_only`). A re-INVITE is
      answered the same way. One the PIN makes an attendee was answered
      before the PIN was heard; the room adds no source for them, so what
      they send is dropped undecoded. An attendee never takes the floor,
      and their screen section receives the slides.
    - **The roster.** It lists the stage, with a panelist's role entry
      `panelist`. `fcp:audience` in the conference state says how many
      watch, and `user-count` counts everyone. A change to the count alone
      goes out at most once every 2 s per subscription, and rides free with
      any other change. `fcp_core::conference_info` reads the count. A peer
      forwarding with this node is told of the stage, not the audience.
    - **Admission** prices an attendee with `audience_join_cost`: no decode,
      the composite's rate (the first watcher pays for the audience's
      composite and its encode) or the forwarded default view of the stage's
      cameras. Placement is unchanged: `VideoNeed` already prices a join at
      the default view (2,800 kb/s), more than an attendee costs. W5 places
      attendees on edges.
    - `audience` in the room's video JSON and on each participant's.
    - The audience by page (§2.6) moves to W4, beside attendance.
    - **Fixed while measuring:**
      - A join's video and screen ports are bound before the answer, as the
        audio's are. A port something else on the host holds is now passed
        over; before, it put the caller in audio only.
      - A node with no port pair left refuses the caller with 503 instead of
        leaving them ringing.
    - **Measured** (`fcp-loadgen/tests/webinar.rs` with `FCP_LOADGEN_VIDEO`,
      release with `video-vpx`, VP8 at 720p15, plain RTP; the host and two
      panelists on camera, the phones on the same 12-core box as the node):

      | Audience | Attendees | Node CPU (audio and video) | Video sent | Per attendee | Peak RSS |
      |---|---|---|---|---|---|
      | composite | 1,000 | 57.7% of a core | 16,447 packets/s | 16 packets/s | 541 MB |
      | composite | 2,000 | 133.4% of a core | 32,494 packets/s | 16 packets/s | 672 MB |
      | forwarded | 1,000 | 81.1% of a core | 60,802 packets/s, 584 Mb/s | 61 packets/s, 584 kb/s | 1,627 MB |

      - Every attendee was sent video in every run. Each composite run had
        one encode and no overruns, with 1,003 and 2,003 subscribers on the
        one flavor.
      - Against W1's audio alone (29.8% of a core for 1,000), the composite
        adds about 17 µs a packet (its one encode included), and forwarding
        about 8.4 µs a packet.
      - The test kit's frames decode to nothing, so the composite drew
        empty tiles and ran far under its 1,200 kb/s. A real 720p composite
        is about 125 packets a second each. At 8.4 µs a packet that is about
        one core per 1,000 watching, and 1.2 Gb/s out. A node's link fills
        before its CPU: about 830 attendees per Gb/s on the 720p composite.
      - A forwarding viewer holds about 1.1 MB (its retransmission cache and
        queue), where a composite subscriber holds a fraction of that. A
        large forwarded audience is a memory budget as well as an egress
        budget, which W5's edges should weigh.
- **W3: hands and Q&A** (FCP), in two parts.
  - Raise hand by DTMF, INFO and API; promote and send back.
  - `webinar_questions` (migration), the `fcp-qa` INFO package, the
    `/me/conferences/live/{room}/questions` routes, `conference.question_*`
    and `conference.hand_*` events.
  - The gateway relaying the package.
  - **W3.1: hands and the stage.** **Done** (v0.25.586–587, forge-media
    v2026.11.17.2: forge-conference 0.20.0). As built:
    - **Raising a hand.** Anyone but a host raises or lowers their own hand:
      - `*4` on a phone. forge-conference's `DtmfCommand::RaiseHand` queues
        the press on the room, and the node takes it within a second, as it
        takes a DTMF dial-out.
      - An `fcp-qa` INFO, `{"op":"hand","raised":true}`
        (`application/vnd.fcp.qa+json`, `fcp_core::qa::QaMessage`).
      - The app's `POST /me/conferences/live/{room}/hand` with
        `{call_id, raised}`. The call is the `X-FCP-Call-Id` its 200 OK named;
        knowing it is what makes the hand the caller's.
      - The browser's `POST /v1/rooms/{id}/hand` on the gateway, and a Raise
        hand button on the join page.
    - **A host's controls**, under `/me/conferences/live/{room}` as a host's
      others are: `DELETE …/participants/{pid}/hand`, `DELETE …/hands`, and
      `POST`/`DELETE …/participants/{pid}/stage`. The conference server has
      the same routes, plus `POST /conferences/{room}/hand` by call.
    - **The stage.** Bringing a webinar's attendee up takes them out of the
      room's listeners: they hear N−1 and are heard from the next frame, and
      the video room stops counting them as audience. Their hand comes down.
      Sending them back makes them a listener again. Only an attendee goes
      up and back (409 otherwise). Their camera stays as it was answered,
      receive-only: Q12's re-INVITE to send is not built yet.
    - **Rosters.** A raised hand in the audience is listed on the stage's
      rosters, with the role `attendee` and `fcp:hand` (RFC 3339, since
      when), and out of `fcp:audience` once, so `user-count` is unchanged.
      An attendee's roster lists their own raised hand and nobody else's.
      A member of the stage (or a meeting's guest) with a hand up carries
      `fcp:hand` too. `fcp_core::conference_info` reads `hand` and
      `attendee`.
    - **Being told.** A participant whose INVITE said `Recv-Info: fcp-qa`
      (or who came by a token: the gateway always asks) is answered
      `Recv-Info: fcp-qa`. They are sent `{"op":"hand","raised":false}` when
      a host lowers their hand, `{"op":"hand",…}` when `*4` or their app moves
      it, and `{"op":"stage","on":…}` when they go up or back. Going up says
      nothing more about the hand. Each INFO's CSeq is taken from the dialog
      as it is cloned, so two sent at once differ. The room refuses a
      `stage` message from a participant (403) and anything that is not a
      message (400).
    - **Relays.** The call manager carries `Recv-Info: fcp-qa` both ways and
      `fcp-qa` INFOs in either direction (`conference_relay::is_qa_info`).
      The gateway asks for the package, keeps `qa: {hand, on_stage}` on a
      room session, and sends the browser's hand as an INFO.
    - **Events:** `conference.hand_raised`, `conference.hand_lowered`
      (`by_host`), `conference.stage_joined` and `conference.stage_left`.
    - `hand` (RFC 3339 or `null`) on each participant in the room's JSON.
    - **Not kept across a move:** a participant migrated off a draining node
      arrives with their hand down. A SIP client that asked for `fcp-qa` and
      did not come by a token is not told after a move either.
  - **W3.2: Q&A.** **Done** (v0.25.588). As built:
    - **The setting.** A meeting's `qa` is `off`, `moderated` or `open`;
      unset is `moderated` in a webinar and `off` in a meeting.
      `qa_anonymous` lets a question go without the asker's name
      (migration 142, `Meeting::qa_mode`).
    - **Asking.** Anyone in the room asks (at most 500 characters, 50 a
      person and 2,000 a sitting) and upvotes another's question they can
      see, never their own:
      - an `fcp-qa` INFO, `{"op":"ask","text":…,"anonymous":…}` or
        `{"op":"upvote","id":…,"on":…}`;
      - the app's `POST /me/conferences/live/{room}/questions` and
        `…/questions/{qid}/upvote`, by call, as the hand is;
      - the browser's `POST /v1/rooms/{id}/questions` on the gateway (the
        `fcp-qa` message as the body), and the join page's Questions panel.
    - **Moderating.** A host or panelist publishes, answers (in text, `live`
      for aloud, or both) and dismisses:
      - by INFO (`publish`, `answer`, `dismiss`), refused 403 from anyone
        else;
      - by `/me/conferences/live/{room}/questions/{qid}/{publish|answer|dismiss}`
        for the meeting's owner, a host token, a panelist token, or a
        webinar's named panelist;
      - and `GET …/questions` lists every question for them. With
        `?call_id=` it lists what that participant sees.
    - **Who sees what.** A host or panelist sees every question; anyone else
      the published ones and their own. In an `open` room a question is
      published as it is asked. A written answer to a question nobody else
      sees reaches its asker alone. A dismissed question drops out of
      everyone's list.
    - **Being told.** A participant who asked for `fcp-qa` is sent the
      questions they may see as they change, coalesced into
      `{"op":"questions","questions":[…]}` once a second, in bodies of at
      most 8 KB. One who joins a room with questions is sent all they may
      see. Each question says, for them, `mine` and `upvoted`. A dismissed
      question is sent once more, `dismissed`, to those who saw it. The
      gateway keeps the list as `qa.questions` on the session, with
      `qa.moderates` from the join token's role, for the page.
    - **Kept.** A room's questions live on the node holding it, for one
      sitting, from its first participant to its last. Each is a
      `webinar_questions` row (`fcp_core::qa::QuestionStore`), written as it
      changes, with upvotes at most once a second and the rest at the
      sitting's end. Anonymous questions keep no name.
    - **Events:** `conference.question_asked` (without the text),
      `conference.question_published`, `conference.question_answered`
      (`live`) and `conference.question_dismissed`.
    - **Not yet:**
      - questions on a room's other nodes: a cascaded room's questions are
        each node's own until W5;
      - questions under retention, holds and subject requests, and in the
        attendance report (W4, done in W4.2);
      - questions surviving a migration off a draining node.
- **W3b: chat** (FCP), for every room.
  - `conference_chat` (migration) under retention, holds and subject requests.
  - The `fcp-chat` INFO package, relayed by the call manager and the gateway.
  - `/me/conferences/live/{room}/chat`, the meeting's `chat` scope, delete and
    mute, and the events.
  - **Done** (v0.25.591). As built:
    - **The setting.** A meeting's `chat` is `everyone`, `hosts` or `off`;
      unset is `everyone` in a meeting and `hosts` in a webinar (migration
      143, `Meeting::chat_scope`, `fcp_core::chat::ChatScope::allows`).
      Under `everyone` anyone posts to the room (`"to":"room"`) or privately
      to its hosts and panelists (`"to":"hosts"`). Under `hosts` an attendee
      or guest posts to the hosts only, and hosts and panelists post to the
      room as well. Under `off` nobody posts to the room. Hosts and
      panelists always have their own channel.
    - **Posting.** Anyone in the room posts, at most 1,000 characters and
      10 messages in 10 seconds (429, or 403 by INFO):
      - by an `fcp-chat` INFO (`application/vnd.fcp.chat+json`),
        `{"op":"post","text":…,"to":…}`;
      - by the app's `POST /me/conferences/live/{room}/chat`, by call;
      - by the browser's `POST /v1/rooms/{id}/chat` on the gateway (the
        `fcp-chat` message as the body), and the join page's Chat panel.
    - **Moderating.** A host or panelist deletes any message, which is then
      gone for everyone. They mute a participant's chat for the rest of the
      sitting (by participant id; another host or panelist is not muted):
      - by INFO (`{"op":"delete","id":…}`,
        `{"op":"mute","participant_id":…,"on":…}`), refused 403 from anyone
        else;
      - by `/me/conferences/live/{room}/chat/{mid}/delete` and
        `…/participants/{pid}/chat-mute`, for those who moderate questions.
        An optional `call_id` names the moderator's own call, so the room
        records them as the one who deleted.
      - `GET …/chat` gives them the last 100 messages; with `?call_id=` it
        gives what that participant sees, and whether they are muted.
    - **Who sees what.** A host or panelist sees every message; anyone else
      sees the room's messages and their own.
    - **Being told.** A participant who asked for `fcp-chat` (a browser
      always does) is sent the messages they may see as they arrive:
      - every 250 ms, as `{"op":"messages","messages":[…]}` in bodies of at
        most 8 KB;
      - the last 100 they may see when they join a room that has some;
      - a deleted message once more, `deleted` and without its text, to
        those who saw it;
      - `{"op":"muted","on":…}` when they are muted or unmuted.

      The gateway keeps the last 500 as `chat.messages` on the session,
      with `chat.muted`.
    - **Kept.** A room's chat lives on the node holding it for one sitting.
      It keeps the last 2,000 messages in memory. Each message is a
      `conference_chat` row (`fcp_core::chat::ChatStore`), written as it is
      posted and again when deleted: the row keeps its text, with
      `deleted_at` and `deleted_by`. The sender's From URI and name let a
      subject request find it. The tenant's `conference_chat_retention_days`
      deletes old rows. The `conference_chat` hold class holds them by the
      sender's keys. Subject export (`conference_chat`) finds them, subject
      erasure deletes them, and every deletion is logged (migration 144).
    - **Events:** `conference.chat_deleted` (the sender, and `by`) and
      `conference.chat_muted` (`on`, `by`). No event per message.
    - **Not yet:**
      - chat on a room's other nodes: a cascaded room's chat is each node's
        own until W5;
      - a mute that follows a person who leaves and joins again;
      - chat in the attendance export (W4, done in W4.2);
      - the meeting's `chat` in the admin UI's meeting form (done in W4.3).
- **W4: the webinar's life** (FCP), in three parts.
  - Practice, go live, end.
  - The lobby let in gradually.
  - Registration with tokens per registrant (`webinar_registrants`) and
    revocation.
  - The attendance report from CDRs.
  - The admin UI's webinar pages.
  - **W4.1: practice, live, ended, and the lobby.** **Done** (v0.25.593–594,
    forge-media v2026.11.19: forge-conference 0.21.0's `*95`). As built:
    - **The sitting.** Every room, meetings too, has one per run on its
      node: from its first caller (past their PIN) to its last, the lobby
      included. Its id is the `session_id` of that run's questions and
      chat, which before each made their own. It is a
      `conference_sittings` row (migration 145,
      `fcp_core::webinar::SittingStore`), written as it begins, goes live
      and ends: `kind`, `started_at`, `live_at` (a meeting's is its start),
      `ended_at` and `ended_by` (a host's participant id, `host` for `*00`,
      `api`, `stage_left`, or nothing when its last caller left). A later
      write never clears what an earlier one set. The room's JSON has it as
      `sitting` (`id`, `kind`, `phase`, `started_at`, `live_at`, `lobby`).
    - **Practice.** A webinar's sitting begins in practice. The stage
      (hosts, panelists) is in the room and hears itself. An attendee who
      arrives, by PIN, `#` or token, waits in the **lobby**: answered, on
      the conference's wait announcement and hold music, and not in the
      room at all (not mixed, not on a roster, not counted). The lobby is
      the conference server's, not forge-conference's waiting room, so a
      host joining does not let it in. Nothing is recorded, whatever
      `auto_record` says. An attendee waits at most `[conference]
      lobby_max_wait_secs` (7200; 0 is unlimited). A migrant is never put
      in the lobby.
    - **Going live.** A host's `*95` (forge-conference's host command
      `go_live`: queued on the room and taken within a second, as `*4` is),
      `POST /conferences/{room}/webinar/live` on the node, the admin's
      `POST /api/v1/conferences/live/{room}/webinar/live` (`conference_manage`)
      or a host's `POST /api/v1/me/conferences/live/{room}/webinar/live`.
      The lobby is let in oldest first at `[conference]
      lobby_admit_per_sec` (50), each as a listener; their waiting task
      sees it within half a second and starts their bridges. One the room
      cannot take (full) is sent a BYE. Recording starts if the meeting
      records. Live twice is no change (`changed: false`); a meeting is 409.
    - **Ending.** A host ends it: `*00`, `POST …/webinar/end` on the node,
      the admin's or a host's route. Everyone, the lobby included, hears
      the ending announcement and is sent a BYE, and the sitting is over:
      the next caller begins a new one. While live, a stage with nobody on
      it (no host, panelist or promoted attendee) for `[conference]
      webinar_stage_grace_secs` (120) ends the webinar the same way
      (`stage_left`), so a host whose call drops has that long to call back.
      In practice an empty stage ends nothing: the lobby goes on waiting.
      A caller who arrives while a webinar is being ended is let go.
    - **Being told.** A participant who asked for `fcp-qa` is sent
      `{"op":"webinar","phase":…,"lobby":…}` when they join, when it goes
      live (one in the lobby is told only as they are let in, so the two
      cannot cross) and when it ends. The room refuses the message from a
      participant (403). The gateway keeps `qa.webinar` and `qa.lobby` on a
      session, and the join page says "waiting for the webinar to start".
    - `GET /conferences/{room}/webinar` on the node, and the admin's and a
      host's `GET …/webinar`, give the sitting.
    - **Events:** `conference.webinar_live` (`sitting_id`, `by`) and
      `conference.webinar_ended` (`sitting_id`, `by`).
    - **Not yet:** a lobby across a migration (a migrant is admitted as it
      was).
  - **W4.2: registration, attendance and the audience by page.** **Done**
    (v0.25.595). As built:
    - **Registration.** A webinar's `registration` (migration 146) says an
      attendee is admitted only by a registrant's token. An administrator
      registers a person, `POST
      /api/v1/conferences/meetings/{id}/registrants` (`name`, `email`,
      `ttl_secs`), and is handed the registration and an attendee token
      naming it (`JoinToken::registrant`), with the join link when the
      gateway's URL is known. A registrant's token lives 30 days by
      default, 90 at most (`MAX_REGISTRANT_TOKEN_TTL`). Registrations are
      `webinar_registrants` rows (`fcp_core::webinar::RegistrantStore`), at
      most 10,000 a webinar, gone with the meeting. `GET …/registrants`
      pages them oldest first, `GET …/registrants/{rid}` reads one, `POST
      …/registrants/{rid}/token` mints another token and `DELETE
      …/registrants/{rid}` revokes it.
    - **At the room.** A token naming a registration is admitted while the
      registration stands and is the meeting's; otherwise 403 (`Registration
      Revoked`, `No Such Registration`). In a webinar that requires
      registration an attendee's token naming none is refused (403
      `Registration Required`) and an attendee by the guest PIN or `#` is
      let go after it. Hosts and panelists come in as before. Revoking
      stops the next join; someone already in stays until a host removes
      them. The registration moves with a migrated leg.
    - **The record.** Each conference leg's CDR now names its `sitting_id`,
      its `registrant`, and whether it left from the `lobby`, beside the
      `role` it had.
    - **Attendance.** `GET /api/v1/conferences/meetings/{id}/sittings`
      pages a meeting's sittings, newest first. `GET …/attendance?sitting=`
      (the newest by default) reads one back: each leg from its record
      (who, from where, role, registration, joined, left, how long, lobby,
      questions asked and messages posted), the distinct attendees, the
      peak in the room at once, each registration with whether they came
      and for how long, and the sitting's questions and chat.
      `CdrQuery::custom_data` finds the legs (`custom_data @>`, the GIN
      index); `QuestionStore::questions_of_session` and
      `ChatStore::chat_of_session` the rest.
    - **The audience by page.** `GET /conferences/{room}/audience` on the
      node, the admin's `GET /api/v1/conferences/live/{room}/audience`
      (`conference_read`) and a host's `GET
      /api/v1/me/conferences/live/{room}/audience`: the room's attendees by
      participant id (`limit`, `cursor`), each with their name, caller,
      registration, when they joined, their hand, and whether they are in
      the lobby or on the stage.
    - **Compliance** (migration 147, `docs/COMPLIANCE.md`). Questions and
      registrations are kept under the tenant's retention policy
      (`webinar_question_retention_days` by `asked_at`,
      `webinar_registrant_retention_days` by `created_at`), held by the
      `webinar_question` hold class (the asker's address and name: a
      question now keeps `asker_address`, so an anonymous one is found too)
      and `webinar_registrant` (their email and name), found by a subject
      export and deleted by a subject erasure, every deletion logged. A
      meeting whose registration is held cannot be deleted until the hold
      is released (registrations go with their meeting).
    - **Not yet:** a registrant registering themselves (a public form);
      hands in the attendance report (a hand is not kept).
  - **W4.3: the admin UI's webinar pages.** **Done** (v0.25.596). As
    built:
    - **The meeting's form** (Conference Bridges, create and edit): Questions
      (the kind's default, off, moderated, open), Chat (the kind's default,
      everyone, to the hosts only, off), anonymous questions, and for a
      webinar "Require registration". A webinar row shows a Registration
      badge.
    - **A webinar's dialog** (the row's Webinar action), in three tabs:
      - **Live:** the sitting's phase (practice, live, ended), when it
        started and went live, how many wait in the lobby, Go live and End,
        and the audience a page at a time (name, caller, joined, lobby,
        stage or audience, hand). Followed every few seconds while there is
        a sitting.
      - **Registrants:** register a person (name, email) and copy the link
        handed back, mint a new link, revoke.
      - **Attendance:** a sitting chosen from the meeting's sittings, its
        attendees, peak, registered and came, questions and messages; each
        leg, each registration with whether they came, the questions and
        chat; the legs as CSV and the whole report as JSON.
    - `e2e/webinars.spec.ts` drives the form, the registrants and the empty
      attendance in a browser; the UI suite's admin has a `join_secret`.
- **W5: a stage and its edges** (FCP).
  - Edge nodes on one-way trunks from the stage, placement for attendees, and
    promotion across a trunk.
  - Measured with fcp-loadgen across two and three nodes, to 5,000 attendees.
  - **The plan** (written 2026-11-19 from the code). In four parts, each
    with its own PR.
    - **What is there.** Placement (`fcp_cluster::choose_conference_node`)
      keeps a room on the fullest hosting node under
      `conference_node_capacity`, then spreads it to the least-loaded node,
      preferring the caller's region. Every two nodes hosting a room are
      trunked (the lower node id dials). A trunk is a contributor in each
      room's mixer and carries its node's own callers only
      (`audio_bridge.rs`: every trunk is left out of a trunk's mix). Hands,
      questions, chat, the sitting and the lobby are each node's own.
      Placement knows nothing of roles: the call manager places a caller
      before the PIN, and a token names its role only to the room.
    - **Decisions** (each follows §2.7 and question 8):
      1. **The stage is a field of the room's registry hash**
         (`__stage`, the node id), claimed by the first host or panelist to
         join a webinar while no live node holds it (`HSETNX`, or replacing
         a stage node gone from membership). The stage gives it up when its
         sitting ends or the room is destroyed. A meeting never has one, so
         nothing changes for meetings.
      2. **Placement of a webinar's callers** (`choose_webinar_node`, given
         the meeting's kind with its video need): the stage while it hosts
         fewer than `conference_webinar_stage_share` (percent, default 50)
         of `conference_node_capacity`, then today's rule over every other
         node, so edges pack in the caller's region. Capacity 0 keeps the
         whole webinar on one node, as today. With no stage yet, today's
         rule.
      3. **A host or panelist who lands on an edge is moved to the stage**
         by the drain's migration: a REFER with a signed ticket, executed by
         whoever holds the leg. The ticket gains `panelist`, so the role
         survives the move. Attendees are never moved: a promoted attendee's
         voice crosses their edge's trunk (§2.7).
      4. **Trunks form a tree.** An edge trunks with the stage only; the
         stage with every edge. The lower node id still dials.
      5. **The stage is the hub.** The stage's trunk to an edge carries the
         whole room except that edge's own trunk: the stage's people and
         every other edge's promoted attendees. An edge's trunk carries its
         own contributors, which are its promoted attendees alone, since
         listeners contribute nothing. So audio never loops: only the hub
         relays, and never back down the trunk it came from.
      6. **The stage holds the webinar.** The sitting, its phase, questions
         and chat are the stage's. An edge forwards what its people send
         (a hand, a question, an upvote, a chat post) to the stage over the
         internal routes under `cluster.secret`. It tells its people what
         the stage announces: the phase, the questions and chat each may
         see, and a lowered hand. An edge lets its own lobby in at its own
         `lobby_admit_per_sec` when the stage goes live, and ends when the
         stage ends. The sitting's id is the stage's, so questions, chat and
         attendance read as one sitting.
      7. **Video to an edge** is the stage's composite once over the
         forwarding trunk (R7). The edge's audience is served it by
         passthrough (`PassthroughMode::On` for an edge's audience), so an
         edge encodes nothing.
    - **W5.1: the tree.** Decisions 1–5: the stage in the registry, webinar
      placement, a host moved to the stage, the tree's trunks and the hub
      mix. **Done** (v0.25.597). As built:
      - **The stage.** `ConferenceRegistry::claim_stage` and
        `release_stage` (Lua scripts over the room's hash);
        `RoomRecord::stage` and `live_stage`. A stage is live while its node
        is among the room's fresh hosts. A claim takes it from a holder
        that is not live, has left membership, or is draining. A host or
        panelist joining a webinar claims it
        (`conference_handler/stage.rs`). The stage is given up when the
        room's sitting closes on its node.
      - **Placement.** `fcp_cluster::choose_webinar_node`, used by the
        call manager for a webinar's callers (the selector now takes
        `webinar`) under `[b2bua] conference_webinar_stage_share` (default
        50). The WebRTC gateway does not load the meeting, so it places by
        a stage in the registry at the same default share.
      - **A host on an edge** is referred to the stage (`refer_to_node`,
        tried for five seconds while their leg's dialog confirms). The
        ticket already brought a webinar's non-attendee back as a panelist.
      - **Trunks.** The trunk manager reads the room's record: an edge
        trunks with the stage alone (`trunks::tree_peers`), and it marks the
        room on this node `stage` or `edge` (`webinar_node` on
        `GET /conferences/{room}`). The trunk's audio bridge takes the
        stage's flag: on the stage, a trunk's mix is the whole room but that
        trunk.
      - **Fixed:**
        - an edge's stage is always empty, so an edge no longer ends its
          webinar after `webinar_stage_grace_secs`;
        - a webinar's lobby counts towards the registry, and letting it in
          updates the registry. Before, an edge whose attendees all came
          through the lobby never published them, so placement could not
          see them and the edge was never trunked.
      - **Proved** by `cluster.rs`
        `a_webinar_over_three_nodes_is_a_stage_and_its_edges`, on three
        nodes at a capacity of 1:
        - the host claims the stage, and the two attendees land on an edge
          each;
        - each edge is trunked to the stage alone, and the stage to both;
        - the stage is heard on both edges;
        - an attendee promoted on one edge is heard on the stage and, through
          it, on the other edge;
        - a panelist placed on an edge is moved to the stage and heard.

        Until W5.2, each node's phase is its own, so the test takes every
        node live.
    - **W5.2: the webinar across nodes.** Decision 6, in two parts.
      - **W5.2a: its life, hands and roster.** **Done** (v0.25.598). As
        built (`conference_handler/stage.rs`):
        - **News, once a second** (main.rs's `webinar-tree` task, under
          `cluster.secret`):
          - The stage sends each edge its `StageNews` on
            `POST /internal/conferences/{room}/webinar/stage-news`: the
            sitting's id and phase, the stage's members (its own and every
            edge's promoted attendees), and the audience counted over every
            node.
          - Each edge sends the stage its `EdgeNews` on
            `…/webinar/edge-news`: its audience, its lobby, its raised hands
            and its promoted attendees.
          - News unheard for 5 s is dropped. A node takes news only in its
            own part: an edge from its stage, the stage from an edge.
        - **The sitting.** An edge takes the stage's sitting id
          (`follow_stage`): questions and chat take it too, so attendance
          reads one sitting. The row it wrote before it knew it was an edge
          is deleted (`SittingStore::delete_sitting`). An edge keeps no
          sitting row, publishes no `conference.webinar_*` event, and never
          records: the stage does all three.
        - **The phase.** An edge goes live when its stage does, letting its
          own lobby in at its own `lobby_admit_per_sec`. It ends when the
          stage ends, with the ending announcement. A stage that ends queues
          its last news, so its edges hear it even once the stage's room has
          emptied. `POST /conferences/{room}/webinar/live` and `…/end` sent
          to an edge are forwarded to the stage, and the stage's answer is
          returned.
        - **Rosters.** The stage's rosters count every edge's audience,
          list every edge's raised hands for the stage, and list promoted
          attendees wherever they are. An edge's rosters list the stage's
          members and the whole audience.
        - **Hands and the stage across nodes.** A host's `DELETE
          …/participants/{pid}/hand`, and `POST`/`DELETE
          …/participants/{pid}/stage`, for an attendee on another node go
          there (`wherever_it_is`, new internal routes). `DELETE …/hands`
          lowers every node's hands.
        - **The webinar's JSON** (`GET /conferences/{room}/webinar`) gains a
          `tree` object: on the stage, each edge's audience, lobby, hands and
          promoted attendees; on an edge, its stage and whether it has heard
          from it. On the stage, `lobby` counts every node's.
        - **Proved** by the three-node test, which no longer takes each node
          live:
          - the edges are on the stage's sitting and live with it, and the
            stage hears each edge's audience;
          - a hand raised by `*4` on an edge is seen on the stage, and the
            stage lowers it;
          - the host brings an edge's attendee up from the stage's node;
          - ending the webinar from an edge ends every node.
        - **Not yet:** the audience by page
          (`GET /conferences/{room}/audience`) lists the node's own
          attendees only.
      - **W5.2b: questions and chat through the stage.** **Done**
        (v0.25.599). As built:
        - **The stage holds both.** An edge's person asks, upvotes and
          posts on their edge as before: the edge checks it against the
          meeting, shows it to them at once, and queues it. The queue rides
          the edge's news as numbered ops (`EdgeOp`: ask, upvote, post).
          The stage applies each op once and in order, keeps the row,
          publishes the event, and acknowledges the last op it applied.
          The edge holds an op until it is acknowledged, so a lost request
          is sent again, not lost.
        - **Each edge mirrors its share.** The stage numbers every change
          to a question or message. Once a second it sends each edge it has
          heard from a `QaNews` on `…/webinar/qa-news`:
          - the changes since the edge last had them: published questions
            and the room's chat, and the edge's people's own questions and
            private messages, at most 200 questions and 500 messages at a
            time;
          - how far that goes;
          - which of the edge's people are muted.

          The edge keeps that in its own rooms and tells its people from
          them as before (`mine` and `upvoted` from its own record). It
          tells a newly muted person at once and refuses their posts. It
          never writes a question or message row; the stage does.
        - **Moderation is the stage's.** At an edge, `GET …/questions` and
          `GET …/chat` without a call (a moderator's view), and publish,
          answer, dismiss, delete and chat-mute, are done on the stage
          (internal copies of the same routes). The stage mutes someone on
          an edge who has posted.
        - `fcp_core::qa::Question` and `fcp_core::chat::ChatRecord` are
          now serialised between nodes, never by the API.
        - **Proved** by the three-node test:
          - Ann asks on her edge, and the stage has the question;
          - the moderator's view through Ben's edge is the stage's;
          - published from Ben's edge, the question reaches Ben, and his
            upvote is counted on the stage;
          - Ann sees her question published with one upvote;
          - Ann's message to the hosts reaches the stage and not Ben, and the
            host's message to the room reaches Ben;
          - the stage mutes Ann, her edge tells her, and her next post is
            refused (403).
        - **Not yet:**
          - a question an edge showed that the stage then refused (the
            meeting's questions turned off meanwhile, or the limits)
            stays on the edge, pending;
          - a moderator joined at an edge rather than moved to the stage
            does not moderate there.
    - **W5.3: video to edges.** Decision 7. **Done** (v0.25.600–601,
      forge-media v2026.11.20: forge-conference 0.22.0's
      `VideoRoom::set_passthrough`). As built:
      - **An edge forwards the stage's composite.** The trunk already
        carries the stage's composite as one tile. A node learns it is an
        edge after its video room exists, so it now turns passthrough `on`
        then, and puts the meeting's own mode back if it stops being one.
        The trunk manager checks this on every pass, so a room whose video
        starts later is covered too. The audience's view on an edge is that
        one tile, so each attendee is sent the stage's packets as they
        arrive, under their own SSRC. The edge encodes nothing for them,
        and the stage's labels and layout are what they see.
      - An attendee brought to the stage on an edge, with their camera on,
        is a second tile there: while they are up, the edge composes as any
        room does.
      - A forwarding room (`video_mode = auto`) is unchanged: its attendees
        are forwarding viewers, and R7's trunk forwards each person.
      - `passthrough` in the room's video JSON.
      - **Proved:**
        - forge-conference's
          `passthrough_turned_on_forwards_an_audience_its_one_source_and_off_composes_again`
          shows the mechanism, on frames that decode;
        - `cluster.rs` `a_video_webinar_s_edge_forwards_the_stage_s_composite`
          shows the node: the edge turns passthrough on, the stage keeps
          `auto`, and the edge's attendee is sent video from the stage. The
          test kit's frames do not decode, so this test cannot show the
          fast path itself; W5.4 measures it.
    - **W5.4: measured.** `fcp-loadgen` across two and three nodes, to 5,000
      attendees: CPU and egress per node, and the trunk's cost. **Done**
      (v0.25.602). As built:
      - `fcp-loadgen/tests/webinar_tree.rs` (`what_a_tree_costs`,
        `#[ignore]`d):
        - `FCP_LOADGEN_NODES`, `FCP_LOADGEN_ATTENDEES` and
          `FCP_LOADGEN_VIDEO` shape the run;
        - the host's token claims node 0 as the stage, and attendees dial
          the edges straight by guest token, split evenly;
        - each node's CPU and memory are sampled, with what each edge's
          audience is sent.
      - The test kit's cluster takes `sip_extra`, `rtp_ports`,
        `first_node_rtp_ports` and `log_level`. The attendees' phones bind
        127.0.0.2, so thousands of their sockets leave the nodes' ports
        alone.
      - **Measured** (release, `video-vpx`, plain RTP; the nodes and every
        phone on one 12-core box, so these are upper bounds):

        | Nodes | Attendees | Video | Stage CPU | Each edge's CPU | Delivered |
        |---|---|---|---|---|---|
        | 2 | 2,000 (one edge) | — | 1.6% of a core | 78.3% | 100% |
        | 3 | 2,000 (1,000 an edge) | — | 2.3% | 37.0%, 39.8% | 100% |
        | 3 | 5,000 (2,500 an edge) | — | 2.7% | 135.8%, 139.1% | 100% |
        | 3 | 2,000 (1,000 an edge) | the composite | 19.1% (the host's camera, its composite and two trunks' video) | 68.0%, 70.6%, 40 Mb/s out each | 100% |

        - **A trunk costs the stage about 0.4% of a core** with its audio
          (1.6% with one edge, 2.3% with two, 2.7% with two edges of
          2,500). The stage's load is its own people, whatever the
          audience's size.
        - **An edge costs what a node holding that audience alone costs.**
          Its 1,000 at 37–40% compares with W1's 29.8% for one node. Its
          2,500 at about 137% is a little above linear: the run's 5,000
          phones share the box's 12 cores with three nodes.
        - So 5,000 attendees take three nodes at about one and a half cores
          on each edge, and each further 2,500 is one more edge. The limit
          per edge is its link before its CPU: 2,500 attendees is about
          3 Mb/s of audio but 300 Mb/s and more of 720p video (W2).
        - The test kit's frames do not decode, so an edge's composite of
          the stage's tile is drawn empty and encoded rather than passed
          through (3 encoders on each edge in the video run). The fast path
          is shown in forge-conference (W5.3), and its saving there is the
          encode a flavor costs. Measuring it end to end needs decodable
          frames from the stage.
        - Five thousand watching (four edges) does not fit on one box:
          their ranges outrun loopback's 30,000 test ports. It needs the
          nodes on machines of their own.
      - **Fixed while measuring.** An attendee's video join was priced with
        the audience composite's encode whenever no composite was running
        yet:
        - on an edge, which encodes nothing for its audience: every join
          there was charged an encode, and an edge refused its audience
          after about 36 joins (503, "Video budget exhausted");
        - in a burst: every join before the budget's next recompute (every
          5 s) paid the encode again.

        Now an edge charges an attendee their picture's egress alone.
        Elsewhere the first join pays for the composite and the joins in
        the next 10 s do not (`audience_join_cost`'s `composite_paid`).
  - **W5 is complete.**
- **W6: HLS** (forge-media, FCP).
  - CMAF segments from forge-mp4, the playlist, the bucket or directory, and the
    admin serving it.
  - **The plan** (written 2026-11-20 from the code). In three parts.
    - **What is there:**
      - `forge_mp4::Mp4Writer` writes one stream, the init segment and
        every fragment to one `W`, and patches durations in at `finish`.
        It writes no `styp`. A fragment opens on a keyframe after
        `min_ms`, or on any sample after `max_ms`, so a fragment can start
        mid-picture. Audio is Opus (`dOps`), and both tracks use a
        timescale of 1 ms.
      - The recorder (`recording.rs`, `recording-mp4`) takes the room's
        composite from `VideoRoom::record` and the mix as Opus, into one
        file in `recordings_dir`.
      - The composite's encoder makes a keyframe every 10 s
        (`keyframe_interval`, room-wide). A recording sink cannot ask for
        one.
      - `fcp_cdr::S3Target` puts a file path as `application/gzip`, with
        no `Cache-Control`.
      - The conference server serves bare file names from
        `recordings_dir`, and the admin proxies a recording's download.
        Neither serves a playlist or a segment.
    - **Decisions:**
      1. **The segmenter is new, beside the writer.**
         `forge_mp4::CmafSegmenter` makes the init segment once, then one
         `styp`+`moof`+`mdat` segment for each keyframe-led group. It cuts
         only on a video keyframe at or past the target duration (2 s by
         default). Each segment is handed back as bytes with its duration
         and sequence number. Audio rides in the segment its time falls in.
         Opus in fMP4 plays in hls.js and Safari, so no AAC is needed for
         HLS.
      2. **A keyframe every segment, asked for by the sink.**
         forge-conference's recording sink keeps its encoder's keyframe
         request (`RecordingSink::request_keyframe`). The stream asks once a
         segment, a frame before its target. The room's interval is left
         alone. A flavor shared with subscribers then sends them a keyframe
         every 2 s too: a little more bitrate for them.
      3. **One stream a sitting, on the stage.** `[conference] hls` (off by
         default) and a meeting's `streaming: hls` start a stream when the
         webinar goes live (never in practice) and end it when the webinar
         ends. It runs on the stage only: an edge has no stream.
      4. **Where it goes.**
         - Locally, `recordings_dir/hls/<sitting>/`: `init.mp4`,
           `seg-<n>.m4s`, and `index.m3u8`.
         - With `[conference.hls.s3]`, the same keys in a bucket. That
           needs `S3Target::put_bytes` with a content type and a
           `Cache-Control`: the playlist `no-cache`, the segments
           immutable.
         - The playlist is a live `EVENT` playlist (`EXT-X-VERSION:7`,
           `EXT-X-MAP`, `EXT-X-TARGETDURATION`, `EXT-X-INDEPENDENT-SEGMENTS`).
           It is rewritten after each segment, and ends with
           `EXT-X-ENDLIST` when the webinar ends, so the stream is its own
           recording for replay.
      5. **Serving.**
         - The conference server serves `/hls/{sitting}/{file}` (m3u8 and
           m4s content types, names checked strictly).
         - The admin serves `GET /api/v1/conferences/meetings/{id}/stream/{file}`
           to anyone holding the meeting's stream link. That is a signed
           viewer token, minted by a host as a registrant's link is. It
           proxies to the stage's node, or redirects to the bucket when one
           is set, through the CDN's public base URL
           (`[conference.hls] public_url`) when that is set too.
         - The meeting's `stream` in the API says its state and its
           playlist's URL.
    - **W6.1: the segmenter and the sink's keyframes** (forge-media).
      **Done** (forge-media v2026.11.20.1: forge-mp4 0.2.0,
      forge-conference 0.23.0):
      - `forge_mp4::cmaf::CmafSegmenter` drives an `Mp4Writer` into memory
        with its cuts on keyframes alone, and hands back the init segment
        once and each fragment as an `styp`+`moof`+`mdat` segment.
      - `RecordingSink::request_keyframe`.
      - Tested:
        - each segment's boxes, and its `tfdt` equal to its start;
        - a keyframe first in every segment, and durations at the target;
        - the init and the segments read back as one file;
        - audio alone cut at the target;
        - the request reaching the encoder.
    - **W6.2: the stream on the stage** (FCP). **Done** (v0.25.603–604).
      As built:
      - **The node:**
        - `[conference.hls]`: `enabled` (off by default), `segment_secs`
          (2), and `dir` (`hls` under `recordings_dir`);
        - it needs the `recording-mp4` build, and one without it says so
          and does not stream.
      - **The meeting's `streaming`** (`off` or `hls`; migration 148) in the
        API and the SDKs; `Meeting::streams_hls` is true for a webinar
        alone.
      - **Its life** (`conference_handler/stream.rs`):
        - the stream starts when the stage goes live, never in practice and
          never on an edge, and ends when the webinar ends or its sitting
          closes;
        - it is the recorder's composite and mix, with the codec chosen as
          for an MP4 recording;
        - a node with no H.264, HEVC or AV1 encoder streams the audio
          alone.
      - **The writer** (`recording/hls.rs`):
        - `init.mp4`, `seg-<n>.m4s`, and a live `EVENT` playlist
          (`EXT-X-VERSION:7`, `EXT-X-MAP`, `EXT-X-INDEPENDENT-SEGMENTS`),
          each written to a temporary name and renamed into place;
        - `EXT-X-ENDLIST` when it stops;
        - a keyframe asked for once a segment, a frame before the target.
      - **Served** at `GET /hls/{sitting}/{file}` on the node:
        - only the playlist, the init segment and `seg-<n>.m4s`;
        - the playlist `no-cache`, the segments immutable.

        The webinar's JSON names the stream under way (`stream`: its
        sitting and playlist).
      - **Proved:**
        - `recording_mp4.rs`: an audio stream in one-second segments, whose
          playlist ends on stop and whose files read back as one file;
          and with real H.264 (`video-openh264`), a room keying every 10 s
          streamed in segments of about 2 s, each starting on a keyframe;
        - `rooms.rs` `a_live_webinar_is_streamed_as_hls_until_it_ends`:
          nothing in practice; from going live, a playlist growing segment
          by segment, fetched from the node with its init and first
          segment; another name refused; ended, the playlist ends.
      - **Not yet:**
        - the bucket (`[conference.hls.s3]`);
        - the stream's recording row, its announcement, and retention
          (W6.3).
    - **W6.3: the stream kept and linked** (FCP), in four parts:
      - **W6.3a: the stream is a recording.** **Done** (v0.25.605). As
        built:
        - **Its row.** A conference recording of kind `stream`
          (`RecordingKind::Stream`, format `mp4`, trigger `auto`), whose
          path is the stream's directory and whose name is the sitting.
          - It is written before the first segment, and nothing is
            streamed without one, as for a recording.
          - It is completed when the stream ends, with the directory's
            size. A node that stopped mid-stream closes it off at start-up
            from the directory's last write.
          - The webinar's JSON names it (`stream.recording_id`).
        - **Its announcement.** The tenant's recording announcement and
          notice tone, as for a recording. When the meeting also records
          (`auto_record` starts first as it goes live), the recording's
          announcement serves both: the room hears it once. Anyone joining
          later hears it too. A `refused` outcome means no stream.
        - **One item.** A stream is retained, held, exported and erased as
          any conference recording, through its row:
          - the node's `/recordings/{sitting}` downloads the directory as
            one fragmented MP4 (the init segment, then each segment in
            order), on its own node or through a peer's;
          - `DELETE` removes the directory, refused (409) while the stream
            runs;
          - the API's download names it `<sitting>.mp4`, and the
            recording's JSON says `kind`;
          - the node's retention sweep and the admin's sweep of abandoned
            rows remove a directory whole
            (`fcp_core::recording::remove_recording_path`);
          - the orphan sweep counts a file under a stream's directory as
            that row's (migration 149, a partial index on stream paths).
        - **Proved:**
          - `rooms.rs` `a_live_webinar_is_streamed_as_hls_until_it_ends`:
            the running row, a delete refused while live, the row
            completed, the download equal to the files in order, and the
            delete removing the directory;
          - `compliance.rs` `a_stream_is_one_recording_its_files_kept_until_retention_takes_it`:
            the stream's files are not orphans and a file beside them is,
            and retention takes the row and the directory with it.
      - **W6.3b: the viewer link.** **Done** (v0.25.606). As built:
        - **The link.** A `StreamToken` (`fcp_core::stream_token`), signed
          like a join token with `[conference] join_secret`, naming the
          tenant and the meeting. Its `purpose` keeps it from passing for
          a join token, or a join token for it.
          - `POST /api/v1/conferences/meetings/{id}/stream/links` mints one
            under `conference_manage`: a day by default, a week at most
            (`ttl_secs`), and only for a webinar whose `streaming` is
            `hls` (409 otherwise).
          - It answers `token`, `playlist_path`, `playlist_url` (when
            `[api.sso] public_url` says where the admin is) and
            `expires_at`.
          - A host's own route (`/me/…`) is left for A7: the link is the
            admin's, as a registrant's is.
        - **The stream.** `GET /api/v1/conferences/meetings/{id}/stream/{token}/{file}`,
          outside the auth layer: the link is the credential.
          - The token is a path segment, not a query: a player resolves a
            playlist's segments relative to its URL and drops a query.
            This deviates from §2.8's route, which had none.
          - Only the playlist, the init segment and `seg-<n>.m4s`.
          - A forged or expired link is 401, another meeting's 403, a
            meeting that no longer streams or has not streamed yet 404.
            None reaches a node.
          - The newest `stream` row of the meeting names the sitting and
            its node. The request goes to the configured conference node
            as `/hls/{sitting}/{file}?node=<node>`, in the meeting's
            tenant. That node serves its own, and fetches a peer's
            through `/internal/hls/…` under `cluster.secret`.
          - The content type and `Cache-Control` come back as the node
            set them, with `Access-Control-Allow-Origin: *` for a player
            on another origin.
          - A stream that ended stays served, its replay, until retention
            takes its row.
        - **The meeting's stream.** `GET /api/v1/conferences/meetings/{id}/stream`
          (`conference_read`), a resource of its own rather than a field on
          every meeting. It gives `state` (`off`, `waiting`, `live` or
          `ended`, from the newest stream's row) with that row's
          recording, sitting, start, end and duration.
        - **Recordings by kind.** `GET /api/v1/recordings?kind=stream`, and
          `RecordingFilter::kind`.
        - **Proved:**
          - `api_postgres.rs` `a_stream_link_opens_the_webinar_s_newest_stream_and_nothing_else`,
            against a stand-in node: the states, the link's limits, the
            playlist and a segment proxied with the node and tenant named,
            the refusals never reaching the node, and replay;
          - `rooms.rs`: `?node=` naming the node itself served, and an
            unknown node 503;
          - `stream_token` unit tests.
      - **W6.3c: the bucket.** **Done** (v0.25.607). As built:
        - **The copy.** `[conference.hls.s3]` (the shape of
          `[api.firmware.s3]`, keys under `hls/` unless `prefix` says
          otherwise) has the stage put every file of a stream at
          `<prefix><sitting>/<file>` as it writes it
          (`recording/hls_bucket.rs`, `S3Target::put_bytes`).
          - The order is the writer's: the init segment, then each segment
            before the playlist that names it, so the bucket's playlist
            never names a segment it lacks.
          - The content type and `Cache-Control` are the node's: the
            playlist `no-cache`, the rest immutable.
          - Uploads run on a task of their own, never holding up the
            writer. A copy more than 128 files behind, or whose upload fails
            after `S3Target`'s retries, stops: the node serves the rest.
        - **The directory stays the recording.** Downloads, retention,
          holds and erasure act on it as in W6.3a. Deleting it, by the API
          or by the node's retention sweep, first deletes the bucket's
          copy of every file it holds (a failure keeps the row, to be tried
          again).
        - **Delivery.** The node answers a request for the init segment or a
          segment the bucket holds with a 302 to `<public_url>/<key>`
          (`[conference.hls] public_url`, a CDN in front of the bucket), or
          to the bucket's own URL when that is unset. A peer's redirect is
          passed on, never followed, and so is the admin's: a stream link's
          player fetches the segments from the CDN.
          - **A deviation from §2.8:** the playlist is never redirected. It
            is small and `no-cache`, so a CDN saves nothing on it. Served
            through the link, a player keeps returning through the link for
            it. Each segment it names is redirected only if the bucket has
            it, and otherwise served by the node. A playlist redirected to a
            copy that had stopped would leave the player on a playlist that
            never grows.
          - The bucket's objects must be readable by players: public, or
            behind the CDN. The sitting's id in their keys is a link
            holder's to see, so a segment can be fetched from the bucket
            after the link expires.
          - Which files are in the bucket is kept in memory. After the node
            restarts, the old stream's segments are served by the node, its
            replay included.
        - **Proved:**
          - `rooms.rs` `a_stream_is_copied_to_its_bucket_and_its_segments_sent_there`,
            against `fcp_testkit::s3::StandInS3`: every file put with its
            headers, the bucket's playlist naming only segments it holds,
            a segment redirected through `public_url` (named by its node
            too), the playlist the node's, a missing segment the node's
            404, the ended playlist, and the delete emptying the bucket;
          - `api_postgres.rs`: the node's redirect passed on to a link's
            holder with CORS;
          - `fcp-cdr/tests/s3.rs` `puts_bytes_with_their_content_type_and_cache_control`
            against Garage.
      - **W6.3d: the admin UI's Stream tab.** **Done** (v0.25.608). As
        built:
        - **The meeting's form:** "Stream out as HLS" for a webinar (the
          meeting's `streaming`), and a "Streamed" badge in the list.
        - **The webinar's dialog, Stream:**
          - the state (not streamed, waiting to go live, live, ended), with
            its start, end and length;
          - a viewer link minted for an hour, a day or a week, the playlist's
            full URL shown to copy;
          - the replay: the stream downloaded as one MP4 through its
            recording.
          - There is no player in the dialog. A browser other than Safari
            needs hls.js to play HLS, and the link is meant for a player of
            the audience's own.
        - **Proved:** `e2e/webinars.spec.ts`. Streaming is turned on in the
          form and the API shows it. The Stream tab waits to go live. The
          link it mints is the meeting's playlist, and the admin answers it
          404 (nothing streamed) rather than 401.
- **W7: RTMP push** (forge-media, FCP).
  - An AAC encoder, FLV, the RTMP client, and the meeting's sealed stream key.
  - **The plan** (written 2026-11-21 from the code). In three parts.
    - **What is there:**
      - The recorder (`recording/mod.rs`) takes the composite from
        `VideoRoom::record` as coded frames (Annex B for H.264), and the mix
        from the room's tap. It resamples the mix to 48 kHz and encodes it as
        Opus in 20 ms frames, handing each to a `Writer` (WebM, MP4 or HLS)
        stamped in milliseconds.
      - `forge_mp4::codec` lifts the SPS and PPS out of a keyframe and builds
        the `avcC` record, the same bytes as FLV's AVC sequence header.
      - `RecordingSink::request_keyframe` (W6.1) lets a sink ask for
        keyframes.
      - forge-media has no AAC, no FLV and no RTMP. The `[api.firmware]`
        and identity providers' secrets are sealed with `fcp-crypto` (a
        column encrypted at rest, never returned).
    - **Decisions:**
      1. **AAC from fdk-aac, built from source** (agreed 2026-11-21).
         `forge-codecs` gains an `aac` feature (off by default) over the
         `fdk-aac` crate, whose `fdk-aac-sys` compiles the library with `cc`.
         It is AAC-LC, 48 kHz, raw access units with their
         `AudioSpecificConfig`, 128 kb/s unless set, in 1024-sample frames.
         Its decoder is there for tests. Like OpenH264's, the licence grants
         no patent rights: AAC's are the deployer's matter, said so in the
         deployment notes. The feature is only in a build that asks for it
         (`[features] rtmp` on the conference server).
      2. **FLV and RTMP are a new crate, `forge-rtmp`**, written here rather
         than taken from crates.io: what is published is either a server,
         unmaintained, or pulls in a runtime of its own.
         - `flv`: tags for `onMetaData`, the AVC and AAC sequence headers,
           and H.264 NAL units as length-prefixed samples (composition time
           0, the composite has no B-frames) and AAC access units.
         - `amf0`, and the chunk stream (4096-byte chunks out, any size
           in).
         - The client: TCP, or TLS for `rtmps://` (rustls, the platform's
           roots). The simple handshake. `connect` (app, `tcUrl`),
           `releaseStream`, `FCPublish`, `createStream`, `publish(key,
           "live")`, and `NetStream.Publish.Start` waited for. Window
           acknowledgements sent, pings answered, and an error status
           turned into an error.
         - A stand-in server (`forge_rtmp::server`) that takes a publisher
           and keeps its tags, for forge-media's tests and FCP's.
         - H.264 only: plain RTMP carries H.264 and AAC, and Enhanced RTMP's
           HEVC and AV1 are not what every service takes.
      3. **The push is a writer.** `Writer::Rtmp` beside MP4 and HLS,
         with the audio encoder generalised (Opus or AAC, each with its own
         frame). The network is a task of its own behind a bounded queue, so
         the recorder never waits on a socket.
         - Behind: video is dropped until the next keyframe, which is asked
           for. Audio is kept.
         - Lost: the connection is made again with backoff (1 s doubling to
           30 s). It starts from a keyframe with the sequence headers again,
           its timestamps going on from where they were.
         - A keyframe every 2 s is asked for, as YouTube asks of an encoder.
      4. **Where it goes.** The webinar's RTMP destination: the service's
         ingest URL and the stream key, sealed at rest and never returned.
         Revised while building W7.2a: it is a sub-resource
         (`…/stream/rtmp`) in a table of its own
         (`webinar_rtmp_destinations`, migration 150), not fields on the
         meeting. The key is then never in a meeting's JSON, and never goes
         to a site with the meeting through the sync journal.
         - The stage pushes when the webinar goes live and stops when it
           ends, as the HLS stream does, under `[conference.rtmp] enabled`.
           Never in practice, and never from an edge.
         - A node without an H.264 encoder does not push and says so.
      5. **Consent.** A push broadcasts the room, so it is announced as the
         stream is: the recording's or the stream's announcement serves, or
         the push announces itself. `refused` means no push. FCP keeps
         nothing of it: no row.
      6. **What is seen.** The webinar's JSON has `rtmp`: its state
         (`connecting`, `live`, `retrying`, `stopped`), since when, the last
         error, and what was sent. The events are `conference.rtmp_live`
         and `conference.rtmp_failed`, the latter once it has retried for a
         minute. The admin UI's Stream tab shows it, and the meeting's form
         takes the URL and key.
    - **W7.1: AAC, FLV and RTMP** (forge-media). **Built** in forge-media
      #233 (forge-rtmp 0.1.0, forge-codecs 0.3.0):
      - `forge_codecs::aac::AacEncoder` (feature `aac`) gives AAC-LC from
        fdk-aac with afterburner on: raw access units of 1024 samples and
        their `AudioSpecificConfig`.
      - `forge-rtmp` has:
        - FLV tag bodies (`AvcTagger` from Annex B), and FLV files;
        - AMF0 and the chunk stream;
        - the `Publisher` over `rtmp://` or `rtmps://`;
        - `server::StandInServer`.
      - Proved against the stand-in (a stream, a refused key, a cut, a
        timeout, RTMPS) and against ffmpeg's RTMP server. There ffprobe
        reads back H.264 640×360 and AAC 48 kHz mono, four seconds, and
        every picture decodes.
    - **W7.2: the push from the stage** (FCP), in two parts:
      - **W7.2a: the destination.** **Done** (v0.25.609). As built:
        - `fcp_core::webinar::RtmpDestination` and `RtmpDestinationStore`,
          implemented on `PostgresStorage` so the key is sealed as every
          secret column is (`SEALED_COLUMNS`, re-encrypted and rewrapped
          with them).
        - `check_rtmp_url`: the URL must be `rtmp://` or `rtmps://`, with a
          host and an application, and no credentials, query or spaces.
        - Under `conference_read` and `conference_manage`:
          - `GET` shows the URL and `key_set`;
          - `PUT` takes the URL and key, and a key left out keeps the one
            there is. Only a webinar has a destination (409 otherwise);
          - `DELETE` removes it.
        - The destination is gone with its meeting.
        - A site's webinar does not push: the destination is not synced.
        - **Proved:**
          - `api_postgres.rs` `a_webinar_s_rtmp_destination_keeps_its_key_to_itself`;
          - `integration.rs`'s sealing test, which shows the key sealed
            at rest and out of another tenant's reach.
      - **W7.2b: the push**: the writer, consent, state and events, the
        stand-in in a binary test, and ffmpeg's RTMP server by hand.
    - **W7.3: the admin UI**: the form's destination and the Stream tab's
      push.
- **A7: Anvil.** The attendee's and the host's views (§2.9), and chat in every
  room.

W1 and W2 come first. W3, W3b and W4 follow in any order; W3b needs no webinar
at all. W5 waits on W1's measurements to say when one node is not enough. W6 and
W7 are independent of W3–W5, and A7 follows W3 and W3b.

## 6. Questions

Each has a recommendation. All were agreed on 2026-11-17 on their
recommendations, except question 5, where chat was added.

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
   **Agreed: both** (§2.5). Chat is in every room, meetings too, with a host-set
   scope (`everyone`, `hosts` or `off`). It is moderated by a host deleting
   messages and muting a person. It travels as an `fcp-chat` INFO package and
   through `/me` routes, and is kept under the tenant's retention policy.
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
