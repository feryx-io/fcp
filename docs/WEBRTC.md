# Browsers and the WebRTC Gateway

`fcp-webrtc-gw` puts browsers in conference rooms and on SIP calls. A
browser talks WebRTC to the gateway (ICE, DTLS-SRTP, one bundled
transport); the gateway talks SIP and RTP to the conference server or to
a SIP destination, and relays media between the two legs without
decoding it. Audio is carried in a codec both sides speak (G.722, then
G.711), so nothing is transcoded; video is carried packet for packet with
the payload type mapped and the SSRC set to what each side expects; RTCP
feedback (NACK, PLI, FIR, REMB) crosses in both directions. A shared
screen is a second video section on both legs, relayed the same way. This
is phase 4 of `VIDEO_CONFERENCING.md` (§15.3), shipped 2026-09-07, with
screen share from phase 7 (§15.6, 7c).

```
browser ──WebRTC (DTLS-SRTP, BUNDLE)──► fcp-webrtc-gw ──SIP + RTP (SDES SRTP)──► fcp-conference
   │  POST /v1/rooms/{room}/join {join_token}      │  INVITE sip:{room}@node  X-FCP-Join: token
   │  POST /v1/rooms/{id}/answer, /ice-candidate   │  P-Asserted-Identity: "name"
   └─ GET /v1/rooms/{id}, DELETE /v1/rooms/{id}    └─ BYE either way
```

## Joining a room

1. An admin (or the meeting's owner) mints a **join token** in the admin
   UI ("Join in browser" on the meeting) or through the admin API. The
   token names the tenant, the room, the participant's display name, the
   role (guest or host) and an expiry, and is signed with a secret the
   admin API shares with every conference server. With the gateway's URL
   configured, the admin API returns the join link as well.
2. The person opens the link. The gateway's join page
   (`/join?room=…&name=…&token=…`) takes the token out of the address bar,
   asks for the microphone (and camera), and calls
   `POST /v1/rooms/{room}/join` with `{display_name, video, join_token}`.
3. The gateway creates a peer connection offering G.722, PCMU and PCMA
   and, when asked, video (H.264, VP8, VP9, AV1) — with a second video
   section for a shared screen (`content`, on by default with `video`) —
   and returns the offer with a **session token**. The page answers through
   `POST /v1/rooms/{id}/answer` and trickles candidates through
   `POST /v1/rooms/{id}/ice-candidate`, both under the session token.
4. Once DTLS completes, the gateway INVITEs the conference server —
   placed through the cluster like a SIP caller, or the static
   `[conference] server_uri` — with an SDES-SRTP offer in the browser's
   negotiated codecs, the display name in `P-Asserted-Identity` and the
   join token in `X-FCP-Join`.
5. The conference server verifies the token (signature, expiry), takes
   the tenant and room from it, admits the participant with the token's
   role and skips PIN collection. A bad or expired token, or a token when
   no secret is configured, is refused with 403. An INVITE without the
   header takes the usual path: tenant from the To host, PIN by DTMF.
6. Media flows. `GET /v1/rooms/{id}` reports the state, codecs and packet
   counters; `DELETE` leaves; a BYE from the conference server ends the
   session; a browser that goes silent for `inactivity_timeout_secs` is
   taken as gone.

A meeting whose conference node rejects video (no native codec compiled
in, no budget) joins audio-only: the browser's video section stays
negotiated but idle, and the session reports `video: null`.

### The camera as simulcast

The gateway's offer asks for the camera as three simulcast streams
(`a=rid:h recv`, `a=rid:m recv`, `a=rid:l recv`, `a=simulcast:recv h;m;l`
and the RFC 8852 rid header extension). A browser that agrees sends one
encoding per rid. The join page sizes them: `h` full, `m` half and `l` a
quarter. The browser may send fewer than it agreed to; Chromium sends two
for a 640×480 camera.

The gateway passes the streams on to the conference server as they
arrive, each under its own SSRC. Its offer to the room names the same
rids and uses the browser's own id for the extension, so the packets
cross unchanged. The room decodes only the stream its tile needs
(`docs/FORWARDING_ROOMS.md` §2.4). When the room asks for a keyframe from
one of the streams, the gateway sends that request on to the stream it
names. A browser that does not agree sends one encoding, as before.

`fcp-webrtc-gateway/tests/rooms.rs`
`a_browser_s_camera_reaches_the_room_as_simulcast` drives the join page in
Chromium. It runs when:

- `FCP_TEST_PLAYWRIGHT` names a Playwright package;
- `FCP_TEST_NODE` names a node (else `node` is used);
- the conference server is built with `video-vpx,video-openh264`.

## Sharing a screen

Both legs carry the screen as a second video section from the start, so
sharing needs no renegotiation on either side:

- **Browser leg.** The offer has three sections bundled on one
  transport — audio (`mid 0`), the camera (`mid 1`) and the screen
  (`mid 2`, `a=content:slides`) — each offering the `sdes:mid` RTP
  header extension, by which the gateway tells a browser's camera
  packets from its screen packets (the SSRC each side signalled is the
  fallback). The join page answers the screen section `sendrecv` with no
  track and puts a `getDisplayMedia` track on its sender with
  `replaceTrack` when the person clicks **Share screen**; **Stop
  sharing**, or the browser's own stop bar, takes the track off again.
- **SIP leg.** The INVITE offers a third `m=video` with
  `a=content:slides` and `a=label:2` on its own port pair and, under
  `[conference] srtp`, its own SDES key — the same section a SIP room
  system offers (RFC 4796), answered by the conference server as in
  phase 7b. A conference server that rejects it (the meeting's
  `video_content_share` is `off`, or no port) leaves the session without
  a screen; the browser's section stays negotiated but idle.
- **The relay** forwards the screen's packets packet for packet on the
  content sockets under their own SSRC pair, and re-addresses feedback
  about the screen (NACK, PLI, FIR) to the screen's far end; a REMB,
  which is about the whole transport, goes to the camera's.
- **The floor.** The room's content floor is the conference server's
  (§15.6): the first screen packet takes it, or is refused when someone
  else holds it, when a guest may not share under `hosts`, or when the
  node's video budget is spent. The conference server tells every
  participant that joined with `X-FCP-Join` — a browser through the
  gateway — with a SIP `INFO` of type `application/vnd.fcp.content+json`:

  ```json
  {"floor": "granted", "mine": false, "participant_id": "p-7", "presenter": "Ann", "reason": null}
  ```

  `floor` is `granted`, `released` (with the stop's `reason`: `ended`,
  `idle`, `host`, `replaced`, `left`, `failed`) or `refused` (with the
  refusal's: `held`, `disabled`, `participant_disabled`, `no_source`,
  `budget`), and `mine` says whether the notice is about the recipient;
  a refusal goes only to the participant refused. A participant admitted
  while someone shares is told a moment after joining. The gateway
  answers the INFO, keeps the floor on the session, and the join page
  shows "N is sharing", "You are sharing your screen" or why a share was
  refused, and stops sending when the room ended the share.

`GET /v1/rooms/{id}` reports it under `content`:

```json
"content": {"codec": "VP8", "payload_type": 96, "srtp": true,
            "sending": true, "state": "granted", "presenter": "Ann", "reason": null, "notices": 2}
```

`state` is `free`, `held` (someone else shares), `granted`, `refused` or
`released`; `sending` says whether the browser has sent screen packets in
the last few seconds; `notices` counts the floor notices taken in, so a
page can tell a verdict on its new share from an old one; `content` is
`null` when the SIP leg has no screen section. The stats gain
`content_to_sip` and `content_to_browser`.

## Tokens

| Where | Setting | Purpose |
|---|---|---|
| Admin API (`fcp-admin`) | `[conference] join_secret` | Signs tokens |
| Admin API | `[conference] gateway_url` | Public base URL of the gateway; turns a token into a join link |
| Conference server | `[conference] join_secret` | Verifies tokens; must equal the admin API's |

`POST /api/v1/conferences/meetings/{id}/join-tokens` (admins, tenant-scoped)
and `POST /api/v1/me/conferences/{id}/join-tokens` (owners) take
`{display_name?, role?: "guest"|"host", ttl_secs?}` and return
`{token, room_number, display_name, role, expires_at, join_url?}`. The
default lifetime is 15 minutes, the maximum one day. A token admits any
number of joins until it expires: treat the link as the invitation it is
and keep lifetimes short. The wire form is
`base64url(payload).base64url(hmac-sha256)`; the payload is JSON.

The conference server reads its secret from TOML only; the configuration
store never carries it.

## Gateway configuration

```toml
[http]
bind_address = "0.0.0.0:8095"
enable_cors = true
allowed_origins = ["https://admin.example.com"]
serve_join_page = true   # /join
public_join = true       # joins need no operator token (the join token admits)

[sip]
udp_bind = "0.0.0.0:5077"
local_ip = "10.0.0.5"

[media]
rtp_port_start = 34000   # lower half: ICE sockets and SIP-side RTP
rtp_port_end = 36000     # upper half: forge-api media sessions
public_ip = "203.0.113.10"

[webrtc]
stun_servers = ["stun:stun.l.google.com:19302"]           # the gateway's reflexive candidate
turn_servers = [{ uri = "turn:turn.example.com:3478", username = "fcp", password = "…" }]
browser_ice_servers = [{ urls = ["stun:stun.l.google.com:19302"] }]  # handed to browsers
gather_timeout_ms = 1500   # wait for STUN so reflexive candidates ride in the offer
ice_timeout_secs = 30      # browser must connect within this
inactivity_timeout_secs = 30

[conference]
server_uri = "sip:conference@10.0.0.7:5060"   # static fallback
node_capacity = 100                            # placement, as the call manager's
tenant_id = "…"                                # registry lookups (hosting node) for this tenant
srtp = true                                    # offer a=crypto on the SIP leg
invite_timeout_secs = 30

[auth]
tokens = ["operator-token"]

[cluster]
enabled = true           # with [storage] redis_url: placement by hosting node, load, region, video budget

[service]
drain_wait_secs = 300    # how long a drain gives browsers to move (0: stop where it stands)

[admission]
max_concurrent_calls = 500   # sessions this node will carry; every limit optional, unset is no limit
```

`[http] public_url` is what a *draining* gateway hands its browsers
(below); `bind_address` cannot serve for that, being a bind address.

Two credentials open the gateway's routes. An operator token (`[auth]
tokens`) opens everything, including forge-api's own `/v1/webrtc/…`
routes. A session token opens only its session's routes. With
`public_join` off, creating a room session needs an operator token too.

The gateway's own candidates go inside the offer (host, then
server-reflexive after a STUN round trip, then relay), since browsers
never fetch them later. A static public IP for the gateway's ICE
candidates is not yet supported; STUN and TURN cover a gateway behind
NAT.

## Calls

`POST /v1/calls` `{destination, connection_id?, caller?}` calls a SIP
destination the same way, audio only: without `connection_id` the gateway
creates the connection and returns the offer; with one (from forge-api's
`POST /v1/webrtc/connections`) the existing connection is used, and its
browser must have picked G.722 or G.711.

A destination that is a URI or `user@host` is called as given. A bare
number (`1001`) goes through the call manager named by `[sip]
outbound_proxy` (`host:port`), as `sip:1001@<outbound_proxy>`.

`caller` `{tenant_id, username}` names the tenant user the call is placed
for. The gateway sends it as `X-FCP-Caller-Tenant` and
`X-FCP-Caller-User`, and a call manager that lists the gateway's address
in `[e911] trusted_edges` treats the call as that user's: their tenant,
their location on an emergency call, their name on the record. From any
other address those headers are ignored.

With a `caller`, the gateway also sends the browser's own address as
`X-FCP-Client-Address`, so the call manager can place it by network
(`docs/E911_BLOCK_2.md` §6.2) rather than at the gateway. Behind a load
balancer, list the balancer in `[http] trusted_proxies` and the gateway
believes its `X-Forwarded-For`; otherwise the connection's own address is
used.

```toml
[http]
trusted_proxies = ["10.0.0.4"]   # whose X-Forwarded-For names the browser
```

### Emergency numbers

A browser can dial 911, and the call is placed only when it can be located
(`docs/E911_BLOCK_2.md` §4):

- The destination is classified against `[e911] emergency_numbers`,
  `test_numbers` and `dial_prefixes` (by default `911`, `933` and `9`).
  These should cover every tenant's numbers. The call manager classifies
  the call again against the tenant's own configuration.
- Without `caller`, the call is refused with 403 and a reason the browser
  can show. An unlocated emergency call is never sent.
- Without `[sip] outbound_proxy`, it is refused with 503: only the call
  manager can locate and route it.
- Otherwise it always goes to the call manager, as
  `sip:<dialled>@<outbound_proxy>`, even when the destination named
  another host. The call manager resolves the user's dispatchable
  location and sends the call to its emergency trunks with the callback
  number.

The session shows `"emergency": "live"` (or `"test"`) and `caller`. Its
record carries the caller's tenant.

```toml
[sip]
outbound_proxy = "10.0.0.2:5060"   # the call manager

[e911]
emergency_numbers = ["911"]
test_numbers = ["933"]
dial_prefixes = ["9"]
```

On the call manager:

```toml
[e911]
trusted_edges = ["10.0.0.7"]       # the gateway's SIP address
```

## Ports and firewalls

| Port | Protocol | Who | What |
|---|---|---|---|
| 8095 | TCP | browsers, admin | HTTP API and join page (put TLS in front, or set `[tls]`) |
| 5077 | UDP | conference server | SIP |
| 34000–36000 | UDP | browsers, conference server | ICE and RTP |

## When a conference node drains

A conference node taken down for maintenance moves its callers to
another node rather than hanging them up
(`docs/DISTRIBUTED_CONFERENCING.md`, phase 5). For a browser this is the
gateway's business: the node REFERs the gateway's SIP leg to the room on
another node, the gateway dials it with the migration ticket, opens the
new leg and swaps it under the running relay. The browser's peer
connection, SDP and tracks are untouched; the room goes quiet for about
the time of one INVITE and comes back. The session's `sip_target` then
names the new node. Nothing in the join page or the client API changes.

## When the gateway itself drains

A gateway session lives in the gateway process — the browser's peer
connection, the SIP leg and the relay between them — so, unlike a
conference participant, the platform cannot move it: the browser has to
join again somewhere else (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).

`POST /drain` (operator token), or SIGTERM, starts it:

- `/ready` answers `503`, so a load balancer stops choosing this node,
  while `/health` keeps answering `200` — a liveness probe must not kill
  the process mid-drain. `/status` shows `draining` and what is left.
- New room joins and calls are refused with `503` and `Retry-After`.
- **Every room session is told to move**, in the status the browser is
  already polling:

  ```json
  { "id": "sess-…", "state": "connected",
    "handover": { "reason": "node_draining", "url": "https://gw2.example.com" } }
  ```

  The `url` is a serving gateway from the cluster — same region where
  possible, least loaded first — that advertises `[http] public_url`.
  With no such peer the field is absent, and the browser should join
  again at the address it is already using, which behind a load balancer
  is a serving node.
- The built-in join page does exactly that: it joins the named gateway
  with the same room, name and join token, answers the new offer with the
  same camera and microphone (a share in progress follows it), and only
  then leaves the old session. A move that fails — an expired join token,
  a peer that will not take it — leaves the old session untouched.
- A `/v1/calls` session is not moved: the far end is a SIP phone, and
  redialling it from another gateway would be a second call. Those are
  waited for, and hung up at `[service] drain_wait_secs`.
- `DELETE /drain` returns the node to service; sessions are untouched
  throughout.

A client that is not the built-in page should do the same: read
`handover`, join at `url` (or at its own gateway address), and leave the
old session once the new one is connected.

## Not yet

- Recording, cascaded video and the WebRTC gateway's video on trunks:
  phase 5.
- transport-cc, RTX and FEC. Header extensions arrived with screen share:
  `sdes:mid` is read, and the rid extension is passed on to the room.
- A screen shared from a browser with no camera: the screen section is
  offered only beside a camera section.
- Re-offering the browser to drop an idle video section when the
  conference node declined video.
- A join token bound to one use; consent freshness in forge-webrtc.
