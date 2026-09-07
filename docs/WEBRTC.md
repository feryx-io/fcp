# Browsers and the WebRTC Gateway

`fcp-webrtc-gw` puts browsers in conference rooms and on SIP calls. A
browser talks WebRTC to the gateway (ICE, DTLS-SRTP, one bundled
transport); the gateway talks SIP and RTP to the conference server or to
a SIP destination, and relays media between the two legs without
decoding it. Audio is carried in a codec both sides speak (G.722, then
G.711), so nothing is transcoded; video is carried packet for packet with
the payload type mapped and the SSRC set to what each side expects; RTCP
feedback (NACK, PLI, FIR, REMB) crosses in both directions. This is
phase 4 of `VIDEO_CONFERENCING.md` (§15.3), shipped 2026-09-07.

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
   and, when asked, video (H.264, VP8, VP9, AV1), and returns the offer
   with a **session token**. The page answers through
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
```

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

`POST /v1/calls` `{destination, connection_id?}` calls a SIP destination
the same way, audio only: without `connection_id` the gateway creates the
connection and returns the offer; with one (from forge-api's
`POST /v1/webrtc/connections`) the existing connection is used, and its
browser must have picked G.722 or G.711.

## Ports and firewalls

| Port | Protocol | Who | What |
|---|---|---|---|
| 8095 | TCP | browsers, admin | HTTP API and join page (put TLS in front, or set `[tls]`) |
| 5077 | UDP | conference server | SIP |
| 34000–36000 | UDP | browsers, conference server | ICE and RTP |

## Not yet

- Recording, cascaded video and the WebRTC gateway's video on trunks:
  phase 5.
- transport-cc, RTX, header extensions, FEC, simulcast: phase 6.
- Re-offering the browser to drop an idle video section when the
  conference node declined video.
- A join token bound to one use; consent freshness in forge-webrtc.
