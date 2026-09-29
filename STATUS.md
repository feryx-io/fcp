# FCP Status

**The single source of truth for what is done, partial, or absent.**
Update this file when feature status changes; keep other docs describing scope, not status.

- **Last reviewed:** 2026-09-01 (full code audit against ARCHITECTURE-v0.2.md)
- **Version:** v0.25.206 · 39 workspace crates
- **Submodules:** siphon-rs `v2026.09.20` · forge-media `v2026.09.17` (both at their repos' current release tips)
- **Health:** CI green (fmt, clippy `-D warnings`, tests, cargo-audit) · 0 Dependabot alerts · 0 future-incompat warnings

## Phase scoreboard

| Phase | Status | Notes |
|---|---|---|
| 1 — MVP foundation | ✅ Complete | Core types, siphon-rs stack, B2BUA, call-manager, REST API, Postgres, Docker |
| 2 — Programmable voice | ✅ Complete | Flow engine, IVR, voicemail, webhooks, events, admin UI, DB-backed flow store, TTS service |
| 3 — Clustering & apps | ✅ Complete* | Redis-heartbeat cluster, queue, presence, conference, WebRTC, Redis. *Lua plugin runtime and gRPC API from the original Phase 3 scope were never built (see Descoped/undecided) |
| 4 — Enterprise | ✅ Complete | Every row of the audit below is done (last one 2026-09-01) |
| 5 — Carrier grade | ✅ Complete (2026-09-24) | E911 blocks 1–2, compliance and its follow-ups, distributed and video conferencing through the GPU backend and MP4, the CDR pipeline, multi-region with its follow-ups; each row below and its design doc says what was left out |

## Phase 4 feature audit (verified against code, 2026-09-01)

| Feature | Status | Evidence |
|---|---|---|
| STIR/SHAKEN | ✅ Done | ~1,900 lines in `fcp-sbc-core`/`fcp-sbc`: ES256 sign + verify, cert cache, enforcement-pipeline wiring, `/stir-shaken` admin route, ~25 tests |
| SBC enforcement + trunks | ✅ Done | `fcp-sbc-core` / `fcp-sbc` |
| Policy-based routing + LCR | ✅ Done | `fcp-routing` (rule evaluator, ~1,000 lines) |
| Provisioning — serving layer | ✅ Done | `fcp-api-core/src/v1/provisioning.rs`: token auth (constant-time), HTTPS enforcement, template rendering, migrations 045/046 |
| TTS service | ✅ Done | `fcp-tts-server`: synthesize/voices/providers/cache routes, Redis feature-gating |
| Audio transcoding | ✅ Done | B2BUA/SBC path wired (2026-09-01): `fcp-media` rebuilds the payload-type map from each leg's negotiated SDP and arms forge-engine's per-packet transcoders on codec mismatch, in both single-session and cross-wired bridge modes; queue path already had it. Transcodable set: PCMU/PCMA/Opus/PCM (Opus needs the `opus` feature; G722/G729 pass through untranscoded). Gated by `media.enable_transcoding` (default off). `RemoteBackend` stubs now return errors instead of panicking (remote mode remains unimplemented) |
| Provisioning — vendor templates | ✅ Done (2026-09-01) | Built-in starter templates for Yealink, Polycom, Grandstream, Cisco ship in `fcp-core/src/config_template.rs`; tenants browse them at `GET /v1/config-templates/library` and instantiate a customizable copy via `POST /v1/config-templates/library/:vendor` (never overwrites an existing copy) |
| Policy engine (`fcp-policy`) | ✅ Extracted (2026-09-01) | Dedicated crate: rule model, evaluator, YAML store with hot reload. `fcp-routing` re-exports everything (no consumer changes). Extraction also fixed the ignored `TimeWindow.timezone` (rules now evaluate in their own timezone, overnight windows supported) and added per-tenant feature toggles (`require_features` on rules + `feature_toggles` in YAML). SBC admission stays in `fcp-sbc-core` (it is rate/capacity control, a different concern); number manipulation remains future (SBC transform stage) |
| Billing | ✅ Done (2026-09-01) | Dedicated `fcp-billing` crate (rating, balance admission, settlement, hooks; model stays in `fcp-core/src/billing.rs`). Rate plans + longest-prefix rates (migration 047, `/v1/rate-plans`); answered calls rated and **settled** at CDR creation (atomic debit, one charge per call). **Balance enforcement** per tenant (migration 048, `/v1/billing/balance`): 402 at admission when the first billing increment is unaffordable, mid-call cut-off when the affordable duration runs out, fail-open on storage errors. **External hooks**: `billing.*` platform events (`call_rated`, `balance_changed`, `balance_low`, `balance_depleted`, `call_rejected`, `call_cut_off`) → webhooks/WebSocket, plus the `BillingHook` trait for in-process integrations; credits/adjustments via `/v1/billing/balance/transactions`. See `docs/BILLING.md`. Not yet: multi-currency, sub-balances, invoicing |
| SBC pipeline stages 3–5 | ✅ Done (2026-09-01) | **Reachability**: classified route resolution (`RouteResolution`) — unroutable destinations now get 404 (nothing configured) vs 503 (trunks exist but unavailable) instead of generic 480/500. **Routing**: trunk selection/failover/LB already existed (`TrunkRouter` + LCR); fixed a concurrency-accounting leak that recorded every failover candidate as a started call. **Transform**: headers via HMR (pre-existing) + new per-trunk egress number transforms (`egress_number_transform`: strip prefixes / prepend, applied at route build) |
| Encryption at rest / KMS | ✅ Done (2026-09-01) | `fcp-crypto` crate: AES-256-GCM field cipher, envelope-wrapped DEKs (`data_encryption_keys`, migration 049), KEK providers `local` (env master key) and `aws-kms` (feature-gated, GenerateDataKey/Decrypt). Postgres backend seals SIP digest/HA1/provisioning passwords, trunk passwords, conference PINs and webhook secrets; legacy plaintext reads through, writes always seal. Platform-admin API: status / rotate / reencrypt at `/v1/system/encryption`. See `docs/ENCRYPTION.md`. Not yet: rewrapping DEKs under a new KEK, media-file encryption |

## Phase 5 reality check

| Item | Status |
|---|---|
| Multi-region | ✅ Done — and its leftovers (`docs/MULTI_REGION_FOLLOWUPS.md`, closed 2026-09-23 with the registration cache, a visible partition and `docs/MULTI_REGION_DEPLOYMENT.md`). Read replicas done (2026-09-04, `docs/MULTI_REGION.md`): every binary's `[storage]` takes `read_replica_url` + `max_replica_lag_secs`; configuration reads go to a healthy replica, everything else and every write to the primary; lag probe every 5 s, read-after-write window per process. CockroachDB dropped (see doc). Region-aware membership done (2026-09-04, v0.25.87): `[cluster] region` / `FCP_REGION` on every node's heartbeat, `local_members` / `members_in_region` / `least_loaded_near` on `ClusterManager`, conference placement prefers the caller's region (local host → local node → any host → any node), admin cluster API and UI show regions (`?region=` filter, `nodes_by_region`). Cross-region event fan-out done (2026-09-04, v0.25.88): the events server's `EventBridge` forwards each region's stream to every `[[event_bus.peer_regions]]` Redis (per-peer consumer groups, at-least-once with retry and stale-claim; `origin_region` + `relayed` on the envelope stop loops), relayed events reach WebSocket clients and state but not webhooks. Not done: region affinity beyond conference placement, per-peer event filters |
| E911 | ✅ Done — block 2 complete (v0.25.181, `docs/E911_BLOCK_2.md`). Block 1 done (2026-09-02, `docs/E911.md`): emergency-number detection (tenant-configurable, `911`/`933` + `9` prefix by default) before any admission control, dispatchable-location resolution (device override → device site → user site → DID address) with ELIN callback numbers per site, emergency-capable trunks tried first, callback number as caller ID + `P-Asserted-Identity` + `Priority: emergency`, `call.emergency` event with notification contacts, `emergency_calls` audit table, admin API (config, readiness report, validate-address, per-number validation) and a pluggable `E911AddressValidator`. Block 2 phase 1 done (v0.25.176, `docs/E911_BLOCK_2.md`): flow `Dial` nodes and browsers through the WebRTC gateway are emergency paths too, emergency routes are tried one at a time, and records are completed at hangup. Phase 2 done (v0.25.177): PIDF-LO location by value per trunk with structured civic addresses. Phase 3 done (v0.25.178): networks, trusted edge client addresses, device-supplied and declared locations. Phase 4 done (v0.25.179): every address validated (sites and device overrides too), structured normalisation, an HTTP validation provider with a sealed token, validation cleared on edit. Phase 5 done (v0.25.180): on-site notification delivered by the platform — typed contacts, the notifier in `fcp-events-server` (email, SMS, a spoken call, webhook), every attempt recorded and retried, `933` notices marked as tests. Phase 6 done (v0.25.181): the admin UI's emergency pages (configuration and contacts, readiness, networks, the call log with its notifications) and the site, device, number and trunk fields it never showed. **Block 2 is complete**; what it left out is tenant trunks that route, named carrier validation adapters, and per-trunk `emergency_location` in the API |
| Video conferencing | ✅ Done — phases 1–9 through the GPU backend and the MP4 sink (see the entries below and `docs/GPU.md`). Design agreed 2026-09-04 (`docs/VIDEO_CONFERENCING.md`): mixing (MCU) core with a forwarding fast path, native codecs behind features, GPU (FFmpeg hardware contexts) designed in as a per-device backend and built last, 15 fps / 720p defaults, WebM recordings, eight phases; phase 0 CPU measurements done (per core: H.264 8–17, VP8 3–6, SVT-AV1 2–4 720p30 encodes; decodes 19–45; composition negligible; rav1e dropped), GPU spike pending a GPU box. Phase 1 substrate done (2026-09-05, forge-media `v2026.09.05`): RTCP feedback, payload formats for all five codecs, frame assembler, retransmission cache, stream rewriter, SDP video negotiation. Phase 2 mixer core done (2026-09-05, forge-media `v2026.09.05.1`): `forge-video` (frames with device residency, scaling, four layouts, compositor, room clock, flavors, codec traits, raw test codec) and `forge-video-codecs` (libvpx, OpenH264, dav1d, SVT-AV1 behind features, built and tested in CI). Phase 3a signaling foundations done (2026-09-05, v0.25.96): SDP answered per section (video rejected properly with port 0 until 3b, or accepted when a caller allows it), section-aware anchoring and codec filtering, per-section endpoints, hold/resume on every RTP section, `RTP/AVPF` in sip-sdp, `Meeting.video_*` settings with migration/API/admin UI, `has_video` on `participant_joined`. Phase 3b conference video done (2026-09-06, v0.25.98; forge-media `v2026.09.06`): `VideoRoom` beside the audio room in forge-conference (codec thread pool, per-participant decode into frame slots, composite per layout output encoded once per flavor, per-subscriber streams with retransmission, keyframe and feedback gates, decoder limits, active speaker from the mixer's energy) and the conference server's video leg per participant (second socket pair, its own SRTP key, RTP and RTCP both ways, join / re-INVITE add and remove / BYE, `video_state_changed` and `video_overload` events, `video` in participant and room JSON). Native codecs need the `video-*` features on fcp-conference-server; without them callers get audio only unless `video_native_codecs_only = false` (raw test codec, CI). WebRTC gateway, trunks and dial-out still audio-only. Phase 3c-1 controls done (2026-09-06, v0.25.102): layout / pin / spotlight / participant video on-off and a located `GET …/video` on the conference server with `/internal` mirrors, `video_layout_changed`, the admin API's `/conferences/live/…` proxy (`[conference] server_url`) and a "Live Video" dialog per meeting in the admin UI. Phase 3c-2 capacity done (2026-09-06, v0.25.103; forge-media `v2026.09.07`): startup self-benchmark (`forge_video::bench`), per-device work-unit budget with admission 503 and periodic re-pricing, `video_budget` in cluster membership, budget-aware placement for video meetings, `/status` and admin cluster page show it. Phase 3 complete. Phase 4 (WebRTC) planned 2026-09-07 (§15.3: three PRs); 4a done (2026-09-07, forge-media `v2026.09.07.1`): forge-webrtc 0.6.0 negotiates a BUNDLE'd video section (H.264/VP8/VP9/AV1, `nack`/`nack pli`/`ccm fir`/`goog-remb`), sorts inbound RTP into audio and video, parses inbound RTCP, keeps per-source statistics, sends SR/RR + SDES and feedback, carries a room subscription's packets under its own video SSRC, and speaks G.722; forge-rtp 0.5.0 keeps SRTP state per SSRC. 4b done (2026-09-07, v0.25.107): the gateway finally carries media — each session (a room join via `POST /v1/rooms/{room}/join`, or a call via `POST /v1/calls`) owns a peer connection offering G.722/G.711 and, when asked, video, and once the browser connects INVITEs the conference server directly (cluster placement or `[conference] server_uri`, SDES SRTP offered per section, display name in P-Asserted-Identity, the join token in `X-FCP-Join`) and relays audio, video packets and RTCP feedback between the legs; a SIP listener answers far-end BYE; `fcp_media::construct_offer` / `SrtpOffer` are the offerer-side SDP and SRTP helpers. 4c done (2026-09-07, v0.25.108): join tokens (`fcp_core::JoinToken`, HMAC-SHA256; minted by `POST /conferences/meetings/{id}/join-tokens` and `/me/conferences/{id}/join-tokens` with `[conference] join_secret`, verified by the conference server from `X-FCP-Join`, which then takes tenant, room, name and role from the token and skips the PIN), the gateway's join page at `/join` with session-scoped tokens for the browser's own signalling (`/v1/rooms/{id}/answer`, `/ice-candidate`) and `[http] public_join`, a "Join in browser" action on the admin and user meeting pages (`[conference] gateway_url` turns a token into a link), deploy files (local config, Dockerfile, compose service, Prometheus jobs) and `docs/WEBRTC.md`. Phase 4 complete; 5 (recording and cascade) planned 2026-09-07 as three blocks (§15.4: 5a WebM recording with a recording registry, 5b trunk video, 5c cross-node speaker state). 5a done (2026-09-08, v0.25.110–112; forge-media `v2026.09.08`): forge-media wrote `forge-webm` (a dependency-free Matroska writer for VP8/VP9 + Opus, with a reader for its tests), `VideoRoom::record` (a sink of coded composite frames that is not a participant and so draws no tile, sharing a subscriber's encoder), `ConferenceRoom::tap_mix` and `VideoRoom::started_at`; PR #145 (released as `v2026.09.08`) then made a dropped sink really stop the recording. FCP added `recording.rs` in the conference server (the mix resampled to 48 kHz and Opus-encoded beside VP8/VP9 composite frames, one WebM on one timebase, behind the `recording-webm` feature since Opus builds libopus from source), `conference.recording_format` (`auto`/`wav`/`webm`) with `recording_video_codec` / `recording_resolution` / `recording_audio_bitrate`, the `conference_recordings` table (migration 053) and `RecordingStore` written at start and completed at stop, `/internal` twins for recording start/stop (a start forwarded to the lowest-id host so a room is recorded once, a stop room-wide) with a recordings map replacing the size monitor's filename parsing and orphaned rows closed off at startup, the tenant's consent announcement to the room and to each later joiner, `recording_id` / `format` / `node_id` / `duration_secs` / `size_bytes` on the events, retention by the tenant's policy on the holding node with the hourly compliance sweep as a week-late backstop (`conference_recordings_deleted`, migration 054), node-addressed `GET`/`DELETE /recordings/{file}?node=` that streams from the holding peer, the admin API's `/v1/recordings` (list, get, download, delete) and a Recordings page with a player in the admin UI. 5b done (2026-09-08, v0.25.113/114; forge-media `v2026.09.08.1`): forge-conference 0.7.0 gained a remote tile (`VideoRoom::add_remote` / `remove_remote` / `is_remote` — a source that draws a tile without being a participant) and `OutputScope` (`All`, `Excluding(id)`, `LocalOnly`) in place of `OutputKey::exclude`; FCP's trunks now negotiate video when the meeting has it and the dialling node can encode a codec it allows (`dial_trunk` builds its offer with `construct_offer` and a `VideoOfferSection`, SRTP per section under `cluster.trunk_srtp`; `handle_trunk_invite` answers with `build_answer` and a `VideoAnswer` from the same meeting lookup), attach the peer's composite as one remote tile and subscribe the trunk `LocalOnly` so no peer's tile is sent back to a peer, and report it all in `GET …/video`'s new `trunks` array. Capacity needed no new code — `room_cost` already prices the tile's decode and the local-only composite's encode. 5c done (2026-09-08, v0.25.115; forge-media `v2026.09.08.2`): forge-conference 0.8.0 gained `VideoRoom::set_remote_speaker` (a peer's claim — node, participant, name, energy, speaking — stands in this node's own election under the remote tile's id, so the layouts are unchanged while `active_speaker()` names the person and `active_speaker_via()` the tile), `set_local_node`, and a node-then-id tie-break with the candidates sorted so every node reaches the same answer; FCP announces its own speaker to the hosting peers every `cluster.speaker_interval_ms` (400 ms) through `POST /internal/conferences/{room}/speaker`, expires a claim not refreshed for five intervals, emits `conference.active_speaker_changed {room, participant_id, node_id}` once from the speaker's own node, and turns `GET …/video` into a room-wide read that merges the nodes' views (participants stamped with their node, the agreed speaker and layout, an `agreed` flag while converging, per-node views and the trunk list) for the admin proxy and the Live Video dialog. Phase 5 complete. Phase 6 (passthrough fast path, bitrate ladder, overload shedding and fuzzing) planned 2026-09-08 as three blocks (§15.5); 6a done (2026-09-08, v0.25.117/118; forge-media `v2026.09.08.3`): forge-conference 0.9.0 forwards a subscriber whose view is a single source it can already decode — that source's own packets through `StreamRewriter` under the subscriber's SSRC, switched on a keyframe, sent on ingress rather than on the compose clock — and skips rendering and encoding an output every subscriber has left, while `PassthroughMode` (`off`/`auto`/`on`, `auto` = spotlight only, since a forwarded frame has no name banner) comes from the new `Meeting.video_passthrough` (migration 055, API, admin UI) and `video_capacity` stops charging for an encode and a composite that are not happening. The decode still runs (tile state is read off decoded frames), so skipping that is a follow-up. 6b done (2026-09-08, v0.25.119; forge-media `v2026.09.08.4`): `forge_video::ladder` holds the rungs at or below a room's size and the 70%-down / 120%-up rule, and `VideoRoom` moves a subscriber whose link has been asking for five seconds (down) or fifteen (up) — keeping its SSRC, payload type and sequence, so a move is a resolution change and a keyframe on the wire and nothing in signaling — instead of letting one poor link drag its flavor's encoder down for everyone sharing it; the rung is reported per participant (`rung`, `rung_kbps`) and shown in the Live Video dialog. 6c done (2026-09-10, v0.25.121; forge-media `v2026.09.10`): the room sheds before it slows — three overruns in a row take its largest output down a rung of the ladder (a ceiling on that output's scope, which the ladder will not climb through), each later streak the next largest, and only once every output is at the bottom rung does the frame rate halve; ten calm seconds undo a step in reverse, frame rate first. `conference.video_overload` now says what was given up — `action` `shed` (with `output`, `from_resolution`, `to_resolution`) or `fps` — and an event from an older node without `action` reads as `fps`. forge-media also has its first fuzzing: eight `cargo-fuzz` targets (the five depacketizers, the frame assembler, compound RTCP, the WebM reader) on seed corpora derived by tests, ten minutes a target nightly. Phase 6 complete. Phase 7 (screen share) planned 2026-09-10 as §15.6 in five blocks: 7a the content source kind, the per-room floor and the `Composite` / `Cameras` / `Content` views in forge-media; 7b the RFC 4796 content section on SIP with a third socket pair, `video_content_share` / `video_content_layout`, API, events and UI; 7c RFC 8285 header extensions and a second video mid in forge-webrtc, the gateway's content leg and the join page's share button; 7d content across trunks with a room-wide floor; 7e BFCP (`forge-bfcp`) for SIP room systems. 7a done (2026-09-11, v0.25.123; forge-media `v2026.09.11`): forge-conference 0.12.0 keys sources and channels by participant and `StreamKind`, holds one content floor per room (taken by the first content packet or `request_content`, gated by a `ContentGate`, released on stop / leave / idle-on-packets / host stop, refusals told once), and gives every output a `View` (`Composite` with `Layout::Presentation` or content-only while a screen is shared, `Cameras`, `Content` — the content alone, forwarded verbatim under any passthrough mode but off, capped at 1080p, moved in frame rate rather than resolution, shed last); forge-video 0.5.0 (presentation layout, chrome-less content tiles, box-filter downscale, `ContentHint`), forge-video-codecs 0.1.1 (screen-content encoder modes), forge-sdp 0.2.2 (RFC 4796 `a=content`, RFC 4574 `a=label`, `video_sections`). FCP prices a content source as a decode and gates the floor on the video budget. 7b done (2026-09-11, v0.25.124): fcp-media answers a camera and an `a=content:slides` section each on its own port and key (`parse_video_sections`, `AnswerOptions::content`, `OfferOptions::content`, `AnchorPorts::content`, and the anchoring rewrite classifies sections so two of a kind never share a port); the conference server runs a third leg per participant (`ParticipantSession::content`, join / re-INVITE add and remove / UPDATE / BYE, `[conference] content_idle_secs`), applies `Meeting.video_content_share` (`off` / `hosts` / `anyone`, migration 056 with `video_content_layout`), reports the share in `GET …/video` (`video.content`, per-participant `video.content` and `presenting`), and takes `POST …/video/content/stop` and `PUT …/video/content-layout` with `/internal` twins and admin proxies; events `conference.content_started` / `content_stopped` and `stream` on `video_state_changed`; the admin UI's meeting forms and Live Video dialog carry it. 7c done (2026-09-11, v0.25.125/126; forge-media `v2026.09.11.1`): forge-rtp 0.7.0 reads and writes RFC 8285 header extensions, forge-webrtc 0.7.0 offers a second video section (`a=mid:2`, `a=content:slides`, BUNDLE `0 1 2`) with the `sdes:mid` extension and sorts inbound video by mid, learned SSRC, signalled SSRC or camera into `VideoRtp` / `ContentRtp`; the gateway offers the screen on both legs from the start (no renegotiation), relays it on its own sockets and SSRC pair with its own feedback, and reports `content {sending, state, presenter, reason}` per session; the conference server tells every token-joined participant about the floor with a SIP `INFO` (`application/vnd.fcp.content+json`: granted / released / refused, `mine`, presenter, reason), which the gateway answers; the join page has Share screen / Stop sharing (`getDisplayMedia` + `replaceTrack`), shows the room's screen large while it flows and says who is sharing. 7d done (2026-09-11, v0.25.127; forge-conference 0.12.1 adds `release_content_with`): the trunk offers and answers a content section (`a=content:slides`, payload type 98, its own port pair and key under `cluster.trunk_srtp`) beside its video one, sends this node's presenter's screen down it local-only and takes the peer's up it as a remote content source under the trunk's tile, so every node's composite and content views show it; the floor is one fact room-wide — each node announces its own share on the speaker timer (`POST /internal/conferences/{room}/content` `{node, participant_id, display_name, since_ms}`), the earliest share wins with ties to the lower node then participant id, a node whose presenter loses releases them `replaced` (the browser is told), claims expire after five intervals; browsers are told the presenter's name from the claim; `GET …/video` names the presenter's node and `trunks[].content` says a trunk carries the screen. 7e done (2026-09-11, v0.25.128; forge-media `v2026.09.11.3`): a new `forge-bfcp` crate (RFC 8855: the message codec, a transport-agnostic `FloorServer` for one floor per conference that leaves grant / deny / revoke to its owner, TCP framing, UDP transactions with replayed responses and T1 retransmission, a fuzz target) and, in the conference server, `[conference] bfcp_bind` serving UDP and TCP on one address: a room system's `m=application … UDP/BFCP` / `TCP/BFCP` section is answered as the floor control server (`a=floorctrl:s-only`, `a=confid`, `a=userid`, `a=floorid:1 mstrm:<content label>`, `a=bfcpver`) when it sits beside a labelled content section, a `FloorRequest` is put to the room's content floor and granted or denied at once, an implicit sender on a BFCP endpoint is refused, a host's stop, idle or a peer's older share comes back as `Revoked`, and `bfcp` appears in the participant's video JSON. Phase 7 complete. Phase 8 (GPU backend) planned 2026-09-11 as §15.7 in three blocks — 8a `forge-video-hw` over FFmpeg device contexts (NVDEC/NVENC codecs, the L4's numbers), 8b the pipeline on the device (filter-graph compositor, per-stage host fallback, layout parity), 8c the node (`[conference] video_device`, saturation benchmark, per-device budget and placement, NVIDIA container image) — on the GPU box `deploy/gcp-gpu` (`docs/GPU_BOX.md`, a Spot L4 on GCP rented by the session, proven 2026-09-11). 8a done (2026-09-12, v0.25.133; forge-media `v2026.09.12`): `forge-video-hw` over `ffmpeg-sys-next` — `HwDevice` / `HwFrames`, NV12 device frames with bit-exact upload and download, NVDEC decoders for all five codecs with output left on the device, `h264_nvenc` / `hevc_nvenc` / `av1_nvenc` with keyframe on demand and a retarget by reopen, `scale_cuda`, `probe` / `register`, forge-video-codecs' `hwaccel` feature, and `bench::saturate`; on the L4, H.264 encodes saturate at about 62 real-time 720p30 streams (AV1 79, HEVC 57) and decodes at 237–306, clearing the plan's gate for 8b. 8b done (2026-09-12, v0.25.134; forge-media `v2026.09.12.1`): a room placed on a `DeviceBackend` end to end — forge-video-hw's `DeviceCompositor` (a filter graph per output: per tile in order its chrome plane, its picture through `scale_cuda` and its name band, each an `overlay_cuda`, the graph rebuilt only when its shape changes), the compositor's chrome shared by every device, nine golden parity scenes (the L4 within 39.5 dB or better of the host), per-stage host fallback with one copy per frame (a decoder the device lacks uploads, an encoder it lacks takes the canvas downloaded once a tick, a compositor that does not build composes on the host), the device named per stage in the status and JSON, a fake device for CI. Measured: the device composite costs 0.5–5.7 ms a 720p tick for 1–16 tiles against the host's 3.9 ms, per filter stage rather than per pixel. 8c done (2026-09-12, v0.25.135): `[conference] video_device` and the `video-hwaccel` feature place a node's rooms on a GPU (`docs/GPU.md`); the node's codecs are a map per device (usable = decoded somewhere and encoded somewhere, so a GPU room decodes VP8 on NVDEC and encodes it on the host); the budget is one capacity per device with joins priced on the device each stage runs on and admitted only where they fit; the GPU is priced by saturation; membership advertises every device's budget with the codecs it encodes and placement prefers a device only for a meeting whose first codec it encodes; `/status` and the Live Video dialog name the devices; an NVIDIA image, a GPU compose override and `gpu-box test`. Phase 8 complete. Follow-ups planned 2026-09-12 as §15.8 (8d, the budget's engines: per-engine GPU budgets on the wire, the composite priced by saturation, a warm-up and floor for the saturation run, the share priced at the room's settings; a single-kernel compositor and an MP4 sink deferred). 8d done (2026-09-12, v0.25.137–138; forge-media `v2026.09.12.2`): a GPU's budget is three pools with each stage charged to its engine, a join admitted only where every engine it touches has room, the composite priced by saturation with graphs in parallel, the saturation run warmed up with a one-second floor per step, `engines` on the wire (optional) with placement on the encoder's, the cluster page's three bars, and the share priced at the room's settings; on the L4 a nine-tile 720p composite renders 1,737 times a second steady-state (the 3 ms a tick reported before was the first render), so an H.264 join costs 21 GPU units where 8c charged 110. Phase 9 (the MP4 recording sink: a fragmented-MP4 writer `forge-mp4` for H.264/HEVC/AV1 with Opus, `recording_format = mp4`, `auto` picking MP4 where H.264 is encodable so a GPU room records on NVENC) planned 2026-09-12 as §15.9. 9a done (2026-09-12, v0.25.141; forge-media `v2026.09.12.3`): `forge-mp4`, a dependency-free fragmented-MP4 writer — `ftyp` + `moov` with empty sample tables and an `mvex`, the header written on the first keyframe whose parameter sets make the `avcC` / `hvcC` / `av1C`, Annex B converted to length-prefixed NAL units, Opus with `dOps`, a `moof` + `mdat` per fragment closed at a keyframe after 1 s or at 30 s regardless, the movie duration patched and an `mfra` appended on finish — with a reader (`forge_mp4::read`) for its tests and a fuzz target `mp4_read`. 9b done (2026-09-12, v0.25.142; forge-mp4's reader fixed 2026-09-13, v0.25.144, to read the patched track durations — the writer had been right): the conference server's recorder is one task with a WebM and an MP4 writer behind it (`recording-mp4` beside `recording-webm`), `recording_format` takes `mp4` and `auto` is MP4 where the node encodes H.264 (natively or on its device), WebM where it encodes VP8/VP9, WAV otherwise; `recording_video_codec` takes `auto`/`h264`/`hevc`/`av1`/`vp8`/`vp9` validated at startup against the container; `RecordingFormat::Mp4` in fcp-core (`mp4` on the wire and in the store, migration-free), `video/mp4` on download, the retention sweep and the recordings listing see `.mp4`; the loopback test records real H.264 from OpenH264 (the writer needs the parameter sets off the first keyframe), an on-box test records a 720p room off NVENC and probes it with `ffprobe` (run on the L4: 32 H.264 + 100 Opus frames, nothing dropped, `h264`/`opus`); the NVIDIA image builds with both recording features and `gpu-box test` with `recording-mp4`. Phase 9 complete |
| Regulatory compliance | ✅ Done 2026-09-15 (follow-ups: legal hold, consent on every recording, subject export and erasure, tenant deletion). Basics done (2026-09-02, `docs/COMPLIANCE.md`): per-tenant retention policy (CDRs, SIP traces, flow traces, recordings, audit events) applied hourly by fcp-admin and on demand, with recorded runs; recording-consent mode with a prompt played before API-started recordings; admin audit log written by an API middleware (every mutating request + exports, actor/tenant/route/status) with `GET /audit-log`; CSV CDR export. Extended 2026-09-08 (v0.25.110) by conference recording: the tenant's consent prompt plays to a whole room before recording starts and to each later joiner, `recording_retention_days` reaches conference recordings (the holding node deletes file and row; the hourly sweep takes rows a week past the window and counts them as `conference_recordings_deleted`), and `/v1/recordings` lists, downloads and deletes them per tenant. Not yet: consent announcement on B2BUA auto-recorded calls, legal hold, data-subject export/erasure |
| Distributed conferencing | ✅ Phases 1–4 done (2026-09-03, `docs/DISTRIBUTED_CONFERENCING.md`). Phase 1: Redis conference registry (`fcp-cluster`) published by every conference node on join/leave and each heartbeat, plus cluster-aware placement in the call manager (stay on a hosting node under `b2bua.conference_node_capacity`, else least-loaded conference node, else the static URI). Phase 2: inter-node trunks — the lower-id node dials each peer hosting the room over SIP/TLS; the callee admits the trunk only when the peer's client certificate names the claimed node (mutual TLS via siphon-rs `v2026.09.03`; the SIP-header node id is never trusted alone), as a hidden `__trunk__<node>` participant past PIN/lock/waiting-room/capacity; each trunk carries the sending node's local callers only (forge-media's new frame-clock mixer, which also fixes 3+-party mixing on one node); trunks follow the registry, reconciled on every local join/leave and every `cluster.trunk_reconcile_interval_secs`. Needs `sip.tls_bind`, `sip.tls_client_ca_path` and a node certificate on every conference node. Phase 3 (2026-09-03): cross-node control — lock/unlock/end reach every node hosting the room (over each node's advertised HTTP address, `/internal` routes under a shared `cluster.secret`), kick/mute/unmute/gain find the node the participant is on, announce plays on one node and crosses the trunks; the lock is persisted in the registry and inherited by a node that starts hosting the room. Phase 4 (2026-09-03): resilience — per-node freshness stamps in the registry (a dead node vanishes from a room within one TTL even while others keep the key alive), non-blocking trunk hangup, trunk liveness (`cluster.trunk_rtp_timeout_secs`; trunks send every frame), draining advertised in membership and skipped by placement, `service.drain_wait_secs` before hanging up remaining callers, awaited registry leave on shutdown, SDES-SRTP trunk media by default (`cluster.trunk_srtp`). Phase 5, participant migration off a draining node, planned 2026-09-13; 5a (the node) done 2026-09-13, v0.25.146, 5b (the call manager's B2BUA) done 2026-09-13, v0.25.147, 5c (the WebRTC gateway) done 2026-09-13, v0.25.148 — phase 5 complete (`docs/DISTRIBUTED_CONFERENCING.md`, *Phase 5*): the draining node REFERs each caller's leg to the room on a surviving node with a signed migration ticket in the `Refer-To`; the leg's holder (5b the call manager's B2BUA, 5c the WebRTC gateway, or a phone that follows REFER) dials the new node, which admits the same participant past PIN, lock and waiting room; the old leg drops on the NOTIFY |

## Phase 5 gate decisions (made 2026-09-02)

| Item | Decision | Rationale |
|---|---|---|
| Storage strategy | **Postgres stays the system of record, single writer, streaming read replicas per region. SQLite is dropped. CockroachDB dropped (2026-09-04)** — its compatibility pass was started and abandoned: applied migrations are checksum-frozen so every gap needs a permanent CockroachDB-only SQL rewrite, serializable-only isolation needs a retry loop the backend lacks, and DDL is ~50× slower. YugabyteDB is the untested candidate if active-active SQL is ever required (`docs/MULTI_REGION.md`) | A second dialect backend would be ~7k lines to maintain for single-box deployments that docker-compose already covers. Only ~12 of 45 migrations use Postgres-specific features (triggers, PL/pgSQL, GIN), all supported by current CockroachDB |
| Plugin runtime (WASM/Lua) + `fcp-plugins` SDK | **Descoped** | Flows, webhooks, and the `BillingHook`/policy traits are the extension points. Revisit only on a concrete customer need |
| gRPC API (`fcp-api-grpc`) | **Descoped** | REST (~150 routes) + WebSocket cover the surface; a gRPC mirror would be permanent double maintenance |
| Distributed conferencing (cascaded mixing) | **Kept, scheduled after E911 and compliance basics** | Plan on file in `docs/TO_DO_CONFERENCE_LOADBALANCE_HA.md` (~2–3 weeks). Dispatchable-location and compliance items are legal requirements for US enterprise UC; 1000-participant conferences are a scale feature |
| Dedicated CDR-processor pipeline | **Kept, deferred → designed 2026-09-14** | Inline CDR creation + `fcp-jobs` post-processing was sufficient until carrier call volumes; the design is `docs/CDR_PIPELINE.md` (spool then ship, the processor in `fcp-jobs`, rating off the call path, partitions and rollups), five phases; phases 1 (the record and the spool, v0.25.159), 2 (the processor in `fcp-jobs`, exact money, v0.25.160) 3 (every leg a record, v0.25.161), 4 (reading at volume, v0.25.163) and 5 (the archive sink, v0.25.164) done |
| Standalone proxy worker | **Descoped** | Registrar, B2BUA, and SBC binaries cover signaling; a pure proxy only serves load-distribution topologies the SBC can already front |

Vision items in `docs/ARCHITECTURE-v0.2.md` that are descoped above remain in that document as history; this table is authoritative.

## Known weak spots

- **Test coverage** outside the SIP stack is thin: the Postgres backend and the core REST handlers (users, devices, templates, provisioning) now have database-backed tests, and every binary now has a start/health/SIGTERM smoke test, but the binaries' actual call paths (SIP + media end to end) still have no automated coverage — the **test floor** (`docs/TEST_FLOOR.md`) is closing it: T1 done 2026-09-14 (v0.25.150) — the scripted SIP phone with media in `fcp-testkit` (`Phone`, `Seed`, `Service`, `bins`) and the first scenarios against the running call manager (digest registration and its failures, a call answered with a tone heard each way, hang-up from either side, the CDR), T2 done 2026-09-14 (v0.25.153) — busy, decline, no answer, an unregistered callee, hold and resume, RFC 2833 and INFO DTMF, blind transfer — which found and fixed two product bugs (the anchored session dropped RFC 2833 digits; a busy or declined callee reached the caller as 480) and one in siphon-rs (a concurrent reap lost a client transaction's final, v2026.09.14); T3 done 2026-09-14 (v0.25.154) — rooms on the conference server: join, hear each other through the mixer, PINs, mute by DTMF and API, kick, and the room reached through the call manager; T4 done 2026-09-14 (v0.25.156) — two conference nodes with Redis and mutual-TLS trunks from a test CA: a room spread across both heard over the trunk, and the phase 5 migration end to end; it found four product bugs (placement never spread a room; conference nodes advertised no load; the dialing side of a trunk dropped the answer arriving on its outbound TLS connection; a caller admitted as the drain began was hung up), all fixed; T5 done 2026-09-14 (v0.25.157) — the WebRTC gateway: a forge-webrtc peer standing in for the browser joins a room through the gateway's HTTP API (join token, answer, trickled candidates, DTLS-SRTP) and hears a phone and is heard, and the phase 5 migration under the browser against two real nodes; it found that every join through the running gateway had panicked since the forge-media bump of v0.25.57 (the peer's event channel taken before the offer that creates it), fixed. **The floor is in**: T1–T5 run in CI against the real binaries; the REST surface is covered except for TTS synthesis (needs a provider) and a porting API that does not exist yet. Workspace `TODO`/`todo!()` count is low (~45, mostly `fcp-media`'s RemoteBackend), so the debt is coverage, not stubs.
- `proc-macro-error2`/`redis 0.25` future-incompat warnings were cleared in v0.25.63.

## Next steps (agreed 2026-09-01)

1. ~~Documentation truth pass~~ (done 2026-09-01)
2. ~~Finish transcoding path~~ (done 2026-09-01: PT map + transcoder arming wired through `fcp-media`; `RemoteBackend` stubs return errors)
3. ~~Billing MVP~~ (done 2026-09-01: rate plans + rating + CDR cost stamping + usage cost) → ~~billing follow-ups~~ (done 2026-09-01: `fcp-billing` crate, balance enforcement with 402 + cut-off, `billing.*` events/hooks, ledger API)
4. ~~Ship vendor provisioning templates~~ (done 2026-09-01: built-in library + instantiate API, rather than migration seeding — future tenants get them too)
5. ~~Extract `fcp-policy`~~ (done 2026-09-01: policy engine in its own crate + timezone fix + tenant feature toggles; admission deliberately left in fcp-sbc-core as capacity control)
6. ~~SBC pipeline stages 3–5~~ (done 2026-09-01: reachability classification, routing accounting fix, per-trunk number transforms)
7. Parallel: raise the test floor — ~~containerized Postgres integration tests~~ (done 2026-09-01: `fcp-storage-postgres/tests/integration.rs` runs against a Postgres service in CI, gated on `FCP_TEST_DATABASE_URL`; first run caught two production bugs — `query_cdrs` emitted unnumbered `$` placeholders so every filtered CDR query failed, and `aggregate_usage*` could not decode `SUM(bigint)` (NUMERIC) so usage/billing summaries failed for any tenant with answered calls — both fixed in v0.25.71); → ~~API handler tests against Postgres~~ (done 2026-09-02: `fcp-api-core/tests/api_postgres.rs` drives the full Axum router — bearer middleware, platform-admin tenant override, users, devices, config templates, and the device auto-provisioning endpoint including its uniform-404 failure surface — against the same CI Postgres service; the SIP side of `ApiState` is built on a no-op transport dispatcher, which turned out to need no new harness); → ~~binary smoke tests~~ (done 2026-09-02: every service binary has `tests/smoke.rs` on the new `fcp-testkit` harness — `--version`/`--help`, malformed-config rejection, and start → `/health` 200 → SIGTERM → clean exit on free loopback ports, DB-backed ones against the CI Postgres service, `fcp-jobs` against a Redis service. First run caught five startup bugs: `fcp-registrar` panicked on every start (rustls had no `CryptoProvider` because the tree links both ring and aws-lc-rs — now `fcp-sip` installs one before building TLS configs), `fcp-sbc` could not load any config file (flattened `SbcConfig` never matched the documented `[core]` layout and sub-configs had no serde defaults, so `fcp-sbc-local.toml` failed to parse), `fcp-admin` and `fcp-api-gateway` died on SIGTERM instead of shutting down (Ctrl+C only), and `fcp-presence` had no `--version`); → ~~widen the handler suite~~ (done 2026-09-02: `api_postgres.rs` now also covers JWT login/refresh/lockout/role gating, database `fcp_…` API tokens (scoping, expiry, revocation), CDRs + usage + billing summary, rate plans + the balance ledger, trunks in Carrier and Enterprise modes, queue configs, and voicemail mailboxes — 16 tests. Findings fixed: `fcp_api_core::create_router` never attached the API token store (only fcp-admin's inlined copy did — admin now calls the shared `create_router_with`), platform tokens ignored the `x-fcp-tenant-id` override on `/cdrs`, and `POST /api-tokens` answered 200 instead of 201). → ~~remaining API gaps~~ (done 2026-09-02: 23 tests — added flows + versions, flow traces (call-visibility scoping), registrations, the presence runtime (state, watchers, auth policy, pres-rules XML) on a live `PresenceManager`, the queue runtime (agent login/pause/resume/logout, stats) on a live `QueueManager`, the self-service `/me` routes (profile, password, forwarding, voicemail) as an end-user session, and the `/events` WebSocket first-message auth over a real socket. Fixes: registrations and flow traces ignored the platform-admin `x-fcp-tenant-id` override like `/cdrs` had, `POST /flows` answered 200, and `/me/profile` spelled roles differently from login (`enduser` vs `end_user`). Phone-number porting has no API surface at all — only the `PortingStore` data model — so nothing to test there) → ~~multipart uploads~~ (done 2026-09-02: 26 tests — avatar upload/download/replace/delete with type, size, and field validation; voicemail greetings through both the admin and `/me` routes (upload, activate, audio download, expiring temporary greeting, delete); audio prompts create, upload, download, category filter, tenant scoping, delete. No product changes needed) → ~~SIP trace + debug routes~~ (done 2026-09-02: 28 tests — `/calls/:id/sip-messages` by call id, bare uuid, `?sip_call_id=`, and the CDR fallback, with tenant scoping; `/debug/*` summary, dialogs, subscriptions, calls, and rate limits against real `Dialog`/`Subscription`/`Call` objects inserted into the runtime managers, per tenant. Two bugs fixed: the SIP trace handler stripped the `call-` prefix before querying while the B2BUA stores the prefixed id, so live calls were untraceable until their CDR existed; and `/debug/dialogs` compared SIP Call-IDs against internal call ids, so tenant admins never saw any dialog — it now maps through the tenant's call legs. The trace handler also honours the `x-fcp-tenant-id` override now
8. ~~KMS design~~ (done 2026-09-01: envelope encryption in `fcp-crypto`, local + AWS KMS providers). ~~Phase 5 gate decisions~~ (made 2026-09-02: see the *Phase 5 gate decisions* table above)

## Phase 5 engineering order (agreed 2026-09-02)

1. ~~**E911** block 1~~ (done 2026-09-02, see the Phase 5 table and `docs/E911.md`). Block 2 designed 2026-09-15 (`docs/E911_BLOCK_2.md`, v0.25.175), chosen after the compliance follow-ups closed: every path an emergency call takes made an emergency path (the WebRTC gateway dials 911 with no identity or location, flows bypass detection, headers are dropped on the TLS flow path, the record never learns its trunk or outcome, the API trunk flag decides nothing), then PIDF-LO location by value per trunk, network-based location with trusted edge addresses and declared locations, validation of every address through a provider, and notification delivered, recorded and tested; six phases, the nine open questions settled on the recommendations. Phase 1 done (v0.25.176):
  - One emergency path (`fcp_sip::emergency`) for the B2BUA and the flow `Dial` node, whose emergency calls now try each emergency route in turn.
  - The WebRTC gateway takes a `caller` and refuses emergency calls without one or without a call manager (`[sip] outbound_proxy`, now implemented). The call manager believes its caller headers only from `[e911] trusted_edges`.
  - Records completed at hangup (trunk, outcome, SIP status, answer and end, delivery; migration 067), with `call.emergency_completed`.
  - Emergency routes tried one at a time, never forked in parallel.
  - Multipart A-leg bodies read for their SDP.
  - `GET /emergency/trunks` on the call manager, and the API trunk flag documented as not routing.
  - The `Dial` node's response race fixed.
  - Emergency calls on the test floor.

  Phase 2 done (v0.25.177, siphon-rs v2026.09.15):
  - Location by value per trunk (`emergency_location = ["elin", "pidf_lo"]`): a multipart INVITE with an RFC 5491 PIDF-LO, `Geolocation: <cid:…>` and `Geolocation-Routing: yes`.
  - RFC 5139 civic elements on sites, number addresses and device overrides (migration 068), with a site's coordinates as a point.
  - A refused location (`Geolocation-Error` or 424) recorded, then retried as ELIN or failed over to the next route.
  - The SIP stack's multipart bodies, geolocation header writers and INVITE options, which also carry headers on the flow path.

  Phase 3 done (v0.25.178):
  - A device that moves is placed by where it is: registration source addresses, the INVITE's own source, and `X-FCP-Client-Address` from a trusted edge (the WebRTC gateway, or an SBC with `[core.e911] vouch_client_address`).
  - `emergency_networks` (migration 069) with `/v1/emergency/networks`: most specific range first, `applies_to` nomadic or all.
  - A trusted device's own PIDF-LO is read and used (`trust_device_locations`, per-device `e911_trust_location`).
  - Declared locations at `/v1/me/location`, validated, expiring after the tenant's TTL, and erased with the subject.
  - Readiness flags users whose devices move but whom nothing places; emergency metrics on `/metrics`.

  Phase 4 done (v0.25.179):
  - Every dispatchable address is validated on the same route: `/v1/locations/:id/e911/validate` and `/v1/devices/:id/e911/validate` join the phone-number one, a site keeps what the validator said (migration 070), and a validated site address makes the users placed there ready.
  - Only a validator marks an address: one sent with `"validated": true` arrives unvalidated, and editing an address clears its mark while re-sending the same address keeps it.
  - The basic validator splits the street into RFC 5139 elements (`1455A N Market Street SW`, `Suite 300`, `4th Floor`) and says in `notes` what it could not tell apart.
  - `[e911.validator]` in `fcp-admin` selects `basic` or an `http` provider (a documented POST contract, bearer token, bounded timeout); an unreachable provider is an error, never a validation.
  - The provider's token may be sealed with encryption at rest, in the file or a mounted `token_file`; `POST /v1/system/encryption/seal` produces the sealed value.

  Phase 5 done (v0.25.180):
  - On-site notification is delivered by the platform: typed contacts (`email`, `sms`, `call`, `webhook`) on `/v1/emergency/config`, with the free text of block 1 read by its shape (migration 071).
  - The new `fcp-notify` crate holds the notifier and its channels; `fcp-events-server` runs it on `call.emergency` under `[notifications]` — SMTP, an HTTP SMS provider, a call that speaks the notice (`POST /v1/emergency/notify-call` on a call manager), and the existing webhook dispatcher. Credentials may be sealed at rest.
  - Every attempt is an `emergency_notifications` row carrying the notice; failures are retried with a doubling delay and given up on when the notice is too old to help; the first delivery sets the record's `notified`; a contact that runs out of attempts raises `emergency.notification_failed`.
  - A `933` call's notice says `[TEST]` everywhere, and readiness shows every contact with when anything last reached it and when a test last did; `GET /emergency/calls/:call_id/notifications` lists the attempts.
  - `/metrics` on the events server counts notifications by kind and outcome and the time to first delivery. Found and fixed: `PlatformEvent::tenant_id()` named a tenant only for `call.initiated`, so webhooks never saw `call.emergency`.

  Phase 6 done (v0.25.181), and with it block 2:
  - The admin UI's emergency pages at `/admin/emergency`: configuration and typed contacts, readiness (users and whether anybody would be told, with when a test last reached each contact), networks, and the call log with every notification attempt behind each call.
  - The fields the UI never showed: a site's ELIN and its validation, a device's E911 override and whether the location it sends is believed, a number's validation, a trunk's `emergency_capable`; addresses are validated from the row menus, which show what the validator said.
  - A device's override and trust setting can now be cleared (`null` on the PATCH), so a phone that goes back to its site stops carrying where it used to be.
2. ~~**Compliance basics**~~ (done 2026-09-02, see the Phase 5 table and `docs/COMPLIANCE.md`). Follow-ups designed 2026-09-14 (`docs/COMPLIANCE_FOLLOWUPS.md`, v0.25.165), chosen after the CDR pipeline closed the agreed order: legal hold, per-subject export and erasure, and the consent announcement on calls recorded without an API request. The survey found the recording-directory prune deletes other tenants' files, tenant and user deletion leave their data behind, voicemail deletes leave the audio, the per-user and per-number recording flags record nothing, and the REST recording route misses the B2BUA's media session. Five phases: foundations, legal hold, consent, subject export, subject erasure. Phase 1 done 2026-09-14 (v0.25.166): every call, queue and conference recording a row before it is kept, under the tenant's directory; recordings and voicemail deleted by row with their files queued and reaped by `fcp-admin` only from the directories it mounts, so one tenant's window no longer reaches another's files; voicemail retention (policy and mailbox), the purge of deleted messages, trace tenancy; a deletion log written by database triggers in the deleting transaction, attributed to the person, token, sweep or cascade (`GET /compliance/deletions`); `compliance_manage` and `audit_view` permissions; audit events naming the token and the deleted ids; the admin UI's audit page rebuilt on the real API. Phase 2 done 2026-09-15 (v0.25.170): legal holds (`/compliance/holds`, `legal_hold_manage`, `compliance.hold_placed`/`hold_released`) scoped by window, subjects, calls and classes, subjects matched in one canonical form in Rust and SQL; retention keeps held items and reports them, and a trigger on every governed table refuses any other deletion of a held row (API `409`, cascades, the tenant); the voicemail menu keeps held messages; archived days a hold reaches get an S3 Object Lock legal hold with `[cdr_archive] object_lock`. Phase 3 done 2026-09-15 (v0.25.171): consent on every recording — one recorder per call manager node through which the API, flows and automatic recording start (user `record_all_calls`, number `recording_enabled`, node `[media] enable_recording`, decided at the INVITE and started on answer), the prompt heard by both parties and ended before the recorder starts, `recording_consent_on_failure` (`record`/`skip`) and a notice tone, the outcome on the record and the recording's row; queue calls announced to caller and agent before they are connected and recorded as a real two-way mix; conferences honour `skip` and the notice; recordings stop with their calls; the call manager's HTTP routes with parameters, dead under axum 0.7's syntax, work. Phase 4 done 2026-09-15 (v0.25.172): subject export — requests filed at `/compliance/subject-requests` (`subject_request_manage`, due `subject_request_days` after receipt, open and overdue on `/metrics`, `compliance.subject_request_completed`), run by `fcp-admin` under `fcp:subject-requests`: the subject expanded and matched narrowly, every store walked (profile without secrets, owned objects, voicemail, records and the recordings and traces of their calls, conference recordings, registrations and presence, emergency calls, audit logs, port requests, webhook deliveries, number lists), a tar.gz with a checksummed manifest, recordings with other people marked for review, held items counted; downloads audited, bundles removed after `export_ttl_days`. Phase 5 done 2026-09-15 (v0.25.173): subject erasure — a preview then a confirmed run (`POST …/confirm`), the subject's data deleted, owners cleared, call records, emergency calls and audit logs pseudonymised with a per-tenant keyed token sealed at rest (`cdr_erasure = "delete"` to delete records instead), port requests kept, held items untouched, every change in the deletion log under its request, archived days rewritten (an emptied day gets an empty object), CDR stream dead letters removed and node spool dead letters aged out; `DELETE /tenants/:id` a request refused under a hold that removes every row of the tenant and then the tenant. The compliance follow-ups are complete.
3. **Distributed conferencing**: phase 1 (registry + placement) done 2026-09-02; phases 2–4 (mutual-TLS trunks and cascaded mixing, cross-node control, resilience) done 2026-09-03. Phase 5, participant migration off a draining node, is planned (2026-09-13): 5a the node (ticket, REFER, NOTIFY, admission) done 2026-09-13 (v0.25.146); 5b the call manager's B2BUA (a fresh B-leg to the new node, the caller re-INVITEd, the old leg dropped) done 2026-09-13 (v0.25.147); 5c the WebRTC gateway (the relay's SIP side swapped under the browser) done 2026-09-13 (v0.25.148). Phase 5 is complete; distributed conferencing has no open item.
4. ~~**Multi-region**~~: read replicas, region-aware cluster membership and cross-region event fan-out done 2026-09-04 (CockroachDB dropped; see the Phase 5 table and `docs/MULTI_REGION.md`). Follow-ups done 2026-09-23 (`docs/MULTI_REGION_FOLLOWUPS.md`): the registration cache, a partition made visible, the deployment guide; the regional-registrations decision point closed in favour of the cache. Unscheduled: per-peer event filters (measured, not filtered).
5. **Video conferencing**: design in `docs/VIDEO_CONFERENCING.md` (2026-09-04); phases 0–2 done in forge-media, 3a (signaling foundations), 3b (conference video legs), 3c-1 (live controls) and 3c-2 (capacity) done in FCP; phase 4 (WebRTC) done (4a forge-webrtc video, 4b gateway media path and direct join, 4c join tokens, join page and surface), phase 5 (recording and cascade) planned (§15.4), 5a (recording), 5b (trunk video) and 5c (cross-node speaker state) done 2026-09-08 (v0.25.110–115); phase 5 of the video plan is complete, and phase 6 (6a passthrough fast path, 6b bitrate ladder, 6c overload shedding and fuzzing; §15.5) is done too (2026-09-08 to 2026-09-10, v0.25.117–121). Phase 7 (screen share) is planned as §15.6 (2026-09-10, v0.25.122); 7a (the content source, the floor and the views, in forge-media) is done (2026-09-11, v0.25.123) 7b (the SIP content section and its surface) too (2026-09-11, v0.25.124) 7c (the browser: header extensions, a second video mid, the share button) as well (2026-09-11, v0.25.125/126) 7d (content across trunks with a room-wide floor) too (2026-09-11, v0.25.127) and 7e (BFCP for SIP room systems) as well (2026-09-11, v0.25.128), so phase 7 is complete; phase 8 (the GPU backend) is planned as §15.7 (2026-09-11, v0.25.132) on the GPU box `deploy/gcp-gpu`; 8a (`forge-video-hw`: device contexts, NVDEC/NVENC codecs and the L4's numbers) is done (2026-09-12, v0.25.133) and 8b (the pipeline on the device: compositor graph, per-stage fallback, parity) too (2026-09-12, v0.25.134), and 8c (the node: `video_device`, `video-hwaccel`, saturation budget, placement, the NVIDIA image) as well (2026-09-12, v0.25.135), so phase 8 and the video plan are complete; the follow-ups are planned as §15.8 (2026-09-12, v0.25.136) and done as 8d (v0.25.137–139); phase 9 (the MP4 recording sink) is planned as §15.9 (2026-09-12, v0.25.140). The dedicated CDR pipeline is next, designed 2026-09-14 (item 6).
6. **Dedicated CDR pipeline**: design in `docs/CDR_PIPELINE.md` (2026-09-14, v0.25.158), written after the test floor closed and after a survey of the CDR path found that a record whose insert fails is lost, rating runs in the hangup task, the queue handler writes an unjoinable second record, conference and gateway legs write none, `hangup_party` and the RTP counters were never filled, and the table has neither partitions nor a composite index. Phase 1 done 2026-09-14 (v0.25.159): the `fcp-cdr` crate (record builder, typed termination reason, local spool, shipper, in-process processor), migration 057 (`cdr_id` primary key, `kind`, node, region, trunk, RTP loss and jitter, rating columns), `hangup_party` and the RTP counters filled at last, and `[cdr] spool_dir` on the call manager and the SBC; the test floor found busy, declined and unanswered calls recorded as failures with no cause on the forking path, fixed, and proves a record survives the database going away under a call. Phase 2 done 2026-09-14 (v0.25.160): the processor in `fcp-jobs` (`[cdr] enabled`) consuming the Redis stream `fcp:cdrs` that nodes ship to with `[cdr] sink = "stream"`, dead letters in `fcp:cdrs:dead`, trimming that keeps unfinished entries; the direction stored by an enricher; rating off the call path with a one-minute plan cache, the charge and `cdr.written` for every record in a stored batch (replays included, the ledger charging once); money exact everywhere (`rust_decimal`, `NUMERIC(18,6)`, migration 058); `BillingStore` split from `CdrStore`; status and metrics on both hosts; an end-to-end test that kills `fcp-jobs` between two calls and sees the second record land and charge once. Phase 3 done 2026-09-14 (v0.25.161): every leg a record — `conference` records per participant leg on the conference server, `gateway` records per browser session (through the stream; the gateway has no database), queue segments under the B2BUA call's id, all correlated through `X-FCP-Call-Id` and `X-FCP-Transferred-From`; pickup and retrieve linked; the trunk that answered on failover; recording paths from flows and the API; `?kind=` listings defaulting to call records; records flushed on graceful shutdown. It found that calls routed to a queue or a conference room were never ended (left in the call map with no record) and that a queue was unreachable at a Request-URI with a port; both fixed. Phase 4 done 2026-09-14 (v0.25.163): `cdrs` partitioned by month (migration 059, keys and indexes built after the copy), keyset pagination with bounded exact-or-estimated totals, the export streamed with no cap and a JSONL format, hourly usage rollups rebuilt every ten minutes by `fcp-admin` and `fcp-jobs` under an advisory lock, retention batched and locked to one admin replica with emptied months dropped; measured at 10 M records on the dev VM (100 M does not fit its disk): 72 s conversion, 6 ms cursor pages, usage 9 ms from rollups against 797 ms from records. Phase 5 done 2026-09-14 (v0.25.164): the archive sink — `fcp-jobs` `[cdr_archive]` writes each tenant's records for a closed UTC day as gzipped JSON Lines to an S3-compatible bucket (SigV4 written in `fcp-cdr`, multipart above 256 MiB, tested against Garage) or a directory, confirms each object by size and SHA-256 before recording it, and announces `cdr.archived`; migration 061 keeps the bookkeeping (`cdr_archives`, the walk's `next_day`, and a trigger on `cdrs` that notes records landing on a day already walked, so the day is written again, with the walk locked against inserts in flight); `cdr_archive_required` on the compliance policy holds CDR retention and partition drops to what the archive has confirmed, shown as `cdr_archived_before`. The CDR pipeline is complete. Five phases, one PR each: 1 the record and the local spool (`fcp-cdr`), 2 the processor in `fcp-jobs` on a dedicated Redis stream with rating moved into it and a `cdr.written` event, 3 a record for every leg (queue, conference, gateway), 4 reading at volume (monthly partitions, keyset pagination, streamed export, hourly rollups, retention by partition drop), 5 the archive sink. The eight questions in its §14 were settled on their recommendations when the design merged.

## What is next (proposed 2026-09-16)

**Tenant trunks** — chosen after E911 block 2 closed. The trunks `/v1/trunks`
manages do not carry calls: outbound routing is built only from each node's
TOML `[[trunks]]`, and nothing outside the API and the storage layer reads a
database trunk's digits, codecs, caller ID, CPS, SRTP, DTMF, session timer,
outbound proxy, trunk group or header rules. Block 2 had to work around it
(`docs/E911_BLOCK_2.md` §12.1: `emergency_capable` on a database trunk decides
nothing). Design in `docs/TENANT_TRUNKS.md` (v0.25.182): one trunk model,
selection over the tenant's trunks then the platform's, the trunk's own
behaviour applied to the calls it carries, inbound tenancy from the trunk,
one node registering a registering trunk, and `emergency_capable` made real;
six phases, nine open questions with recommendations, every recommendation taken. Phase 1 done (v0.25.185): one trunk model — `fcp_core::Trunk` is what a tenant manages, what a node's `[[trunks]]` parses into and what the call path holds (`TrunkConfig` an alias of it, migration 072 giving the stored trunk the columns it lacked), identified by `TrunkId` rather than by name so two tenants' `carrier-a` never share health or capacity; no routing change yet. Phase 2 done (v0.25.186): tenant trunks route — every routing node holds a `TrunkSet` of its own `[[trunks]]` and every tenant's, read at startup, on the `system.config_changed` the trunks API now publishes, and on a backstop tick (`[trunk_routing] refresh_secs`); selection takes the caller's tenant and is that tenant's trunks then the platform's unless the tenant refuses them (`tenants.platform_trunk_fallback`); a node with no `[[trunks]]` of its own still routes; health probes follow the set; the record names which trunk carried the call (`cdrs.trunk_id`); and the floor routes two tenants to two carriers on one node. It found that on a node with a database an outbound call to an E.164 that is nobody's DID never reached a trunk at all (only the no-storage path asked the route provider), and that `platform_trunk_fallback` read back `true` whatever was stored; both fixed in the phase. Phase 3 is split in two: 3a done (v0.25.187) — the route carries the trunk itself (`OutboundRoute.trunk`), and the leg it carries presents the trunk's identity (`caller_id_override`, `from_domain`, `P-Asserted-Identity`), asks for its session timer (RFC 4028, with the 90-second floor), is offered RFC 2833 only if the trunk takes it, and goes to the trunk's `outbound_proxy` when it has one; asserted on the wire by `fcp-call-manager/tests/trunks.rs`. 3b is three PRs: 3b-1 done (v0.25.188) — a trunk whose `srtp` is `preferred` or `required` is offered `RTP/SAVP` with the key this node sends with, a carrier answering a `required` trunk in the clear gets a BYE and the caller a `488`, a `required` trunk on media this node does not anchor is refused rather than sent in the clear, and the media backend stopped arming the B-leg with a freshly generated local key when an answer carried `a=crypto` (RFC 4568 §5.1.2: that key must be the one this node advertised, or the carrier hears noise); 3b-2 done (v0.25.189) — the compiled HMR engine moved from the SBC binary into `fcp-sbc-core`, a call manager reads the same `[header_manipulation]` rules, a trunk names the ones that run on its calls (`apply_request_egress_named`, since a tenant's trunk was never in the engine's trunk map), and they run on the finished INVITE through `InviteOptions::filter`, a last look added to siphon-rs for this (sip-uac 0.7.3, v2026.09.17) so a rule can rewrite or remove a header this node itself wrote; 3b-3 done (v0.25.190) — `StirSigner` moved from the SBC binary into `fcp-sbc-core`, a call manager reads the same `[stir_shaken]` key and certificate, and a trunk whose `stir_shaken` asks for signing gets an `Identity` header built over the numbers the carrier will see (after the caller ID override and the digit plan), with `orig.tn` written as RFC 8225 §5.2.1 says — one number as a string, not the array it had been, which a strict verifier rejects; 3b-4 done (v0.25.191) — DTMF converted, not only offered: each leg's way of taking DTMF is settled when the trunk answers, telephone-event is relayed only to a leg that negotiated it, a caller's RFC 2833 digit reaches a trunk that takes SIP INFO as an `application/dtmf-relay` INFO, and an INFO digit reaches a leg that takes RFC 2833 as telephone-event (forge-engine 0.6.1, forge-media v2026.09.17: digits by leg, relay per direction, `send_dtmf` to one leg); the media backend's `send_dtmf`, a no-op until now, plays the digit. **Phase 3 is complete.** Phase 4 is split in two: 4a done (v0.25.192) — the trunk a carrier's call arrives on decides whose call it is. Each node indexes where every trunk takes calls from (new `inbound_sources` addresses and CIDR ranges, else its hosts, with names resolved at refresh); a match comes after a trusted edge and a registered device and before the domain. On a carrier address two tenants share, the owner of the dialled number decides (`404` when neither owns it), and a platform trunk carries whoever owns the number. Inbound honours direction: migration 074 moves stored `upstream` trunks to `bidirectional` and makes that the default. The record names the trunk a call arrived on, and a carrier's call to a number of ours that nothing answers is no longer sent back out to a carrier. 4b done (v0.25.193): a trunk's `max_concurrent_calls` and `cps_limit` count its calls in and out together, per node. A carrier's call past either limit gets `503` (emergency calls excepted), an outbound call counts from answer, and every count is released wherever the call ends, through a per-call ledger. Found on the way: the router's own count was never released on a call manager (a trunk with a channel limit stopped being chosen), and `cps_limit` was ignored there. The header rules a trunk names run on its inbound INVITE, with Via, Call-ID and CSeq kept as they arrived. **Phase 4 is complete.** Phase 5 done in two parts. 5a (v0.25.198/199, siphon-rs v2026.09.18): a registering trunk is registered by one node of the cluster, the lowest live node id advertising it in its heartbeat (non-draining preferred), and as the trunk's own account rather than the node's identity. 5b (v0.25.200): each node's view of every trunk (`GET /trunks`, published to Redis) is joined into `/v1/trunks`, plus `trunk.health_changed` and `fcp_trunk_calls_total` per trunk and tenant. **Phase 5 is complete.** Phase 6 is split: 6a done (v0.25.201) — an emergency call tries its tenant's own `emergency_capable` trunks first, then the platform's (the last resort for every tenant, even one that refused the platform's trunks for ordinary calls), then the ordinary route. The emergency record and `call.emergency_completed` name the trunk by `trunk_id` (migration 075), and `GET /emergency/trunks` lists tenants' trunks too. This closes `docs/E911_BLOCK_2.md` §12.1. 6b done (v0.25.202): the tenant's routing rules (`/v1/routing/rules`), stored until now and read by nothing on the call path, decide its calls on every call manager. They are held per node and refreshed on change, and run after the node's YAML policy: `deny` answers 403, and `route_to_trunk` confines a call to one of the tenant's trunks or a trunk group, refused with 503 rather than rerouted when that cannot take it. Emergency detection now comes first, so neither kind of rule touches an emergency call (the YAML policy used to run before detection). The API checks rules as a node compiles them, requires the trunk permissions, checks that a named trunk is the tenant's own, and announces each change. 6c-1 done (v0.25.203): an ordinary call with several trunk routes tried every carrier at once, down the parallel-forking path built for ring groups, so a call priced by the SBC's `[lcr]`, or sent to a 6b trunk group, could reach the callee two or three times. Only emergency calls went one route at a time. Now a call to carriers tries one at a time: the next route when the carrier fails (401/403/407, 408, 488, 5xx, no answer at all, or silence until the timeout), never after the callee's own answer (busy, decline, no such number). 6c-2 done (v0.25.204): per-tenant least-cost routes. A tenant prices prefixes on its own trunks at `/v1/lcr-routes` (migration 076, cost as `NUMERIC(18,6)`, for ordering only), and each node orders that tenant's trunks by them (longest prefix, then cost, then priority) before the platform's, which keep the node's `[lcr]`. Two defects fixed on the way: a platform trunk with a better priority outranked a tenant's own trunk, and the node's `[lcr]` priced tenant trunks that shared a platform trunk's name. 6d done (v0.25.205): the admin UI shows what routes and what each node sees. A live route preview (`GET /routes/preview` on each call manager, proxied by fcp-admin's `/v1/routing/preview` through `[call_manager] url` or cluster membership) shows a number's outcome, the policy and rule that decide it, and the trunks in order with their prices. Routing rules get their own permission (`routing_rule_read` / `routing_rule_manage`, held by tenant admins in every mode; a trunk route also needs `trunk_update`). There is a Least-Cost Routes page, and each node's view is on the Trunks page. The Routing page's edits can now clear a field. Found on the way: with `auth_enabled = false`, every permission-checked endpoint answered 500. **Phase 6, and the tenant trunks project, are complete.**

## What is next (chosen 2026-09-20)

**Capacity and survivability** — chosen after tenant trunks closed, from four
candidates (this, the multi-region leftovers, a ship-and-operate project, and
an edge-security pass). Design in `docs/CAPACITY_AND_SURVIVABILITY.md`
(v0.25.206, agreed 2026-09-20): what a node can carry, what it refuses, and how it
leaves service. The survey behind it found that `[calls] max_concurrent_calls`
on the call manager is parsed, logged and enforced nowhere; that a tenant's
`max_concurrent_calls` is stored, served and edited but read by nothing on the
call path (the same shape of defect that started tenant trunks); that
admission control — CPS, concurrent calls and registrations, overload action —
exists only inside the `fcp-sbc` binary, so the call manager, the edge in
every single-binary deployment, has none; that the registration rate limiter
siphon-rs offers is never installed, and `fcp_core::transport::RateLimiter`
has no callers, leaving REGISTER floods to reach Postgres; that SIGTERM on a
call manager drops every call in flight — no draining state, no refusal of new
work, no wait, no BYE, while the conference server has all four; and that
capacity is measured for video and for nothing else, with no load generator or
benchmark anywhere in the workspace. Five phases, one PR each: 1 the limits
that exist, enforced (admission moved into `fcp-sbc-core` and wired into every
routing binary, the tenant limit real, the registrar's rate limiter
installed); 2 leaving on purpose (drain state, `/drain` and `/ready`, SIGTERM
drains, `drain_wait_secs` then a BYE with a `Reason` and a `node_draining`
record, test floor T6); 3 the harness (`fcp-loadgen` on `fcp-testkit`: register
storm, call ramp, soak, mixed, with a JSON report and budget assertions);
4 what the numbers say (run it, fix what it finds, publish a measured capacity
budget); 5 across the cluster (tenant and trunk limits leased in Redis,
placement by headroom). Phase 1 done (v0.25.208): the limits that exist, enforced. The admission
controller moved from the `fcp-sbc` binary into `fcp-sbc-core` (as the HMR
engine and `StirSigner` did in tenant trunks 3b) and the call manager admits
through it: an `[admission]` section of optional limits, `[calls]
max_concurrent_calls` honoured at last with a warning naming the new key, the
tenant's own `max_concurrent_calls` enforced per node from the tenants the
trunk set already reads, REGISTER floods refused by source address and by AOR
before any credential work, and every refusal a `503` with `Retry-After` and
a `Reason`, counted by reason on `/status` and `/metrics`
(`fcp_admission_*`). Emergency calls are exempt from all of it. Three defects
found on the way: `[admission]` read into the SBC's own config type gave a
call manager the SBC's limits the moment it wrote the section (now its own
optional-field section); a challenged registration is two REGISTERs, so the
smallest useful per-source rate is two, not one; and zero, which used to mean
"refuse everything", now means no limit. `fcp-call-manager/tests/capacity.rs`
is T6's first half. Phase 2a done (v0.25.209): the call manager leaves service without dropping
calls. `POST /drain` or SIGTERM tells the cluster, refuses new calls with a
`503`, keeps the calls already up, and at `[service] drain_wait_secs` ends
what is left with `Reason: SIP;cause=200;text="node draining"` and a record
whose cause is `node_draining`; `/ready` answers `503` while draining (a
load balancer stops choosing the node) while `/health` stays `200` (a
liveness probe does not kill it mid-drain); `DELETE /drain` returns it to
service untouched; a device registering while the node drains gets at most
60 seconds so it comes back somewhere that is serving; `system.node_draining`
and `system.node_drained` are published. siphon-rs v2026.09.20 (sip-uac
0.7.5) added `bye_with_headers`, since a BYE had no way to say why. A second
SIGTERM now cuts a long drain short rather than letting SIGKILL take the
calls with no BYE and no record. Phase 2b, the rest of the fleet, split into
four on 2026-09-21: 2b-1 lifts the call manager's drain into one shared
implementation (`fcp-sip`) and puts the SBC and the registrar on it, 2b-2 the
IVR, voicemail and presence servers, 2b-3 the WebRTC gateway (which has no
health server yet, and whose sessions want handing over rather than
refusing), 2b-4 the conference server's own drain aligned on the shared
state, the admin UI's drain badge and the rolling-restart runbook. Phase 2b-1
done (v0.25.213): `fcp_sip::drain` is the sequence every routing binary runs,
with a `DrainTarget` per binary saying what it carries and how to end it —
calls and a BYE for the call manager, the relayed calls for the SBC, the
devices registered for the registrar, which ends nothing because a
registration lapses. `/ready`, `POST /drain` and `DELETE /drain` answer the
same JSON on all three (the SBC's behind its API token). Found on the way: a
drain started over HTTP never told the cluster, because 2a advertised in the
SIGTERM path rather than in the sequence; the refusal and the shortened
expiry lived in the call manager's handler, so the standalone registrar had
neither, and both now run one `register_gate` (which also gave the registrar
`[admission]` and the source address on its bindings); and `AdmissionSection`
moved into `fcp-sbc-core`, since 2b-2 needs it three more times. Phase 2b-2
done (v0.25.214): the IVR, the voicemail server and the presence server read
`[admission]`, answer `/ready` and `/drain`, and drain on SIGTERM. The B2BUA
target the IVR and the voicemail server share with the call manager moved
into `fcp_sip::drain::NodeCalls` (and the heartbeat behind it into
`fcp_sbc_core::ClusterAnnouncer`, since `fcp-sip` cannot depend on
`fcp-cluster`), so `fcp-call-manager/src/drain.rs` is gone. The voicemail
server hangs up and then waits for the messages already spoken to be written:
a recording ends a second after the caller does, and a node that stopped at
the BYE would lose the file, the row and the MWI. The presence server drains
the way the registrar does — short subscriptions while draining, from the
same cap (`drain_expiry_cap`), then the `terminated;reason=probation` NOTIFY
to whoever is left, which `POST /drain` now sends as well as SIGTERM. Found
on the way: a SUBSCRIBE with `Expires: 0` was clamped up to the configured
minimum, so an unsubscribe was answered with a subscription and neither
unsubscribe path could run. Phase 2b-3 done (v0.25.215): the WebRTC
gateway. It had no FCP health server at all, so `/ready`, `/status` and
`/drain` joined its own router (`/ready` open for a load balancer, the
others behind the operator token) while forge-api's `/health` stays as it
was; `[admission]` and `[service] drain_wait_secs` are new. Its browsers
cannot be moved by the platform — a session's peer connection, SIP leg and
relay are all in the one process — so a draining gateway names a peer in
the session status the browser already polls
(`"handover": {"reason": "node_draining", "url": "https://gw2…"}`), and the
join page joins there with the same room, token and camera before leaving
the old session. The heartbeat gained `public_url` (`[http] public_url`)
because `http_address` is a bind address and no use to a browser, and
`fcp_cluster::choose_handover_node` picks among serving peers of the same
kind, in region, least loaded. A `/v1/calls` session is not moved: the far
end is a phone, and redialling would be a second call. Phase 2b-4 done
(v0.25.216) and with it phase 2: the conference server — the node that had
a drain before the platform did — is on the platform's. It keeps the part
that is its own (REFER its callers to other nodes before waiting, through
a new `DrainTarget::on_drain_start` hook) and gives up having a drain of
its own: its `draining` flag is gone, `is_draining()` is the node
admission's answer, so `POST /drain` refuses INVITEs exactly as SIGTERM
did and `DELETE /drain` lifts the refusal — which the old flag had no way
back from — with `Retry-After` on the refusal and `[admission]` read at
last. The admin UI's cluster page shows a `draining` badge per node and a
count, from the heartbeat field nothing had ever displayed; and
`docs/CLUSTER_OPERATIONS_RUNBOOK.md`, which said FCP had no drain mode and
that a rolling restart drops calls, now carries the procedure and what
each kind of node does with what it is carrying. Phase 3 done (v0.25.217):
`fcp-loadgen`, a workspace member (`publish = false`) driving
`fcp-testkit`'s scripted phones — real SIP, real RTP — many at a time,
with the four profiles (register storm, call ramp, soak, mixed), latency
percentiles per milestone, outcomes by status code, per-leg RTP loss and
jitter, the node's `/metrics` and `/proc` sampled beside them, a JSON
report and budget flags that make a run a gate. A soak holds a population
rather than a rate; the ramp is the profile with a rate, and stops at the
first rung that refuses, slows or drops. The testkit gained
`MediaLeg::stream_stats` (RFC 3550 jitter, loss, reordering) and
`Call::timings`. The harness's first run against a real node found two
defects in itself — registrations counted as answered calls, and
answering phones that played nothing, so every leg read as 100 % loss —
which is what a measuring tool is for. CI runs a six-second soak and a
five-second storm for behaviour, never a rate. Phase 4 ran (v0.25.218):
the harness against a release-built call manager on the dev VM (12 cores,
2 GB, Postgres and the harness on the same box). Every rung from 2 to 20
calls/s held — 2208 of 2210 answered, INVITE→200 p50 4 ms and p99 7 ms,
4419 media legs with 0.00 % loss and 0.37 ms jitter, ≈0.3 MiB of node
memory a call, the node at 3 % CPU — and the ramp ended because the run
did, not because the node did, so 20 calls/s is a floor and not a
capacity claim. A 300-device register storm was answered in full. Its
p50 of 1525 ms turned out to be the harness, not the registrar: the same
storm at 30 and 100 phones measured 10 ms and 18 ms with the node at 1 %
CPU throughout, so one process saturates somewhere past 100 phones on
that box. Four defects found in the harness itself and fixed
(registrations lapsing mid-run and reading as a capacity cliff at 6
calls/s; a rung's rate counting the drain after it; a run's rate counting
its own setup; a refused caller spinning instead of waiting). Left open
with their evidence: two calls in 2210 answered 480, three silent legs in
4419. The heartbeat's capacity and load fields wait for phase 5, which
brings their reader. Phase 5 split in two on 2026-09-21 after 5a's size
showed: 5a the tenant's contract counted across the cluster, 5b trunk
channels, the heartbeat's capacity and load, placement by headroom and
the admin UI. Phase 5a done (v0.25.219): `fcp_cluster::CallLeases` holds
a lease per call in a Redis sorted set scored by expiry, so a dead node's
calls fade rather than holding a tenant's contract for ever; `admit` is
async and the controller takes the lease on the way in, gives it back
when the call ends (queued for a settling tick, since a call ends in a
callback that cannot await) and refreshes what is still up. Redis being
unreachable falls back to this node's own count — phase 1's behaviour,
not no behaviour. `[cluster] lease_tenant_calls` is on wherever a cluster
is configured. Phase 5b done (v0.25.220) and **the project with it**: a
trunk's channels take the same leases (`TrunkCallCounter` async too, an
answered outbound leg counted like an inbound call, the same per-node
fallback); the heartbeat carries `capacity` — the node's limits, its
registrations and its calls per second — beside `active_calls`; placement
prefers room over idleness (a node carrying ten of a hundred beats one
carrying two of four), with a full node last, a node advertising no limit
ranked on its calls as before, and ties by node id so two choosers agree;
and the admin UI's cluster page has a Headroom column reading the same
fields. Ten open questions, every recommendation taken on
2026-09-20: one admission controller in `fcp-sbc-core` for every routing
binary; node limits only where a deployment wrote one down; the tenant limit
per node until phase 5 leases it cluster-wide; a BYE with a `Reason` and a
`node_draining` record at the drain deadline; no migration of ordinary calls
off a draining node (browser legs later); a draining registrar shortening the
REGISTER expiry rather than refusing; CI asserting behaviour from the harness,
never a rate; emergency calls exempt from every limit and admitted while
draining; and registration rate limiting keyed on source address and AOR at
once.

**Edge security** — chosen 2026-09-21, after capacity and survivability
closed. Design proposed in `docs/EDGE_SECURITY.md`: what an
internet-facing node exposes — who may place a call, what a peer may
claim, what a flood costs, and what is actually verified where the code
says "trusted". The survey checked four things sound (inter-node trunks
require mutual TLS naming the peer; the trusted-edge headers are gated on
the peer with IPv4-mapped addresses normalised; the TLS verifier is a
deliberate private-CA-then-public-roots one, not a bypass; REGISTER
floods are bounded per source and per AOR) and found five wanting: an
INVITE is authorised by source address and never authenticated, so a host
sharing an address with a registered device places calls as that user;
the registration check matches the peer as a *substring* of the contact,
so a host at 10.0.0.1 passes as the device at 10.0.0.10
(`fcp-sip/src/handler.rs:1752`); `X-FCP-Call-Id` is believed from any
peer and names the record a leg is billed under, unlike its gated
siblings; the call manager — the edge in every default deployment — has
no IP ACL and no per-source INVITE limit where the SBC, conference and
presence servers each have something; and the stateful surfaces
(transaction, dialog, registrar, offer/answer) are unfuzzed. Five phases,
each ending in a test that fails against today's code; four open
questions for the user, starting with whether INVITE authentication
defaults on. Phase 1 done (v0.25.222): a device is placed by the address
its REGISTER arrived from where the registrar kept one, and otherwise by
its contact's *parsed* host, compared as an address rather than searched
for as text — so a host at 10.0.0.1 is no longer the device at 10.0.0.10,
and neither a port nor a URI parameter that happens to spell an address
authorises anyone. A contact whose host is a name places nobody. The
admission gate reads the stored registration rows rather than the
bindings built from them, since only the rows carry the source address,
and an INVITE naming a user registered elsewhere is logged before it is
refused. Q1 and Q3 settled by the user on 2026-09-21: INVITE
authentication off for one release with a startup warning, then on; and
2.2 fixed both ways, parsed host *and* the stored source address. Phase 2
split in two once phase 2's survey found the digest itself wanting
(§2.7): the authenticator took the nonce out of the client's own
`Authorization` header and never recorded what it had issued, so a
captured header — one packet, one verbose log line, one compromised
device — was valid for ever, on a request whose headers said something
else. Against the registrar that is AoR hijack, since the digest covers
the method and URI but no header, so a replay naming the attacker's
`Contact` takes the victim's calls; the presence server's SUBSCRIBE and
PUBLISH gate took the same header. Phase 2a done (v0.25.223):
challenges come from a tracked `NonceManager`, the digest is checked
first and the nonce second — so a client that knows the password is told
`stale=true` and retries silently while one that does not is refused
rather than looped — and the retransmission check is given the request's
identity (body, `Contact`, `CSeq`, `Call-ID`, `From`, `To`) so a UDP
repeat still passes and a replay that changes what the request does does
not. A nonce lives 300 s and is per node, so a refresh or a request
landing on another node costs one extra challenge. Phase 2b done
(v0.25.224): an INVITE is challenged under `[registrar] auth_invites`,
and where it is, the digest decides *instead of* the source address —
proving the password is stronger than arriving from a plausible place,
and a phone that has moved has not stopped being itself. A carrier on an
enabled trunk and a peer in `[e911] trusted_edges` keep source-based
admission and are never challenged; loopback loses its bypass, which is
the only version the test floor can prove and the reason an embedded
component that INVITEs the node needs a place in `trusted_edges` or
credentials of its own. Alice's password placing a call as Bob is
refused `403`, since no retry fixes that, while missing or stale
credentials get `401` and a challenge (Q2). Two defects found on the way:
the digest store resolved its tenant from the request's `To`, which on an
INVITE is the callee, so on a multi-tenant edge the caller's credentials
were looked for in the callee's tenant and never found — a non-REGISTER
request now resolves by its `From`; and the flag is off for one release
behind a startup warning, so the accepted cost is that a caller who
cannot answer a challenge cannot dial 911 either, exactly as an
unadmitted source already could not. The floor test fails with the flag
off, which is §2.1 in one assertion. Phase 3 done (v0.25.225): one gate
for what a peer claims. `fcp-sip/src/claims.rs` removes at ingress —
before anything reads the request — what an untrusted peer has no
standing to assert, so a reader written later is gated by construction
rather than by somebody remembering. The audit §2.3 asked for found four
readers: `X-FCP-Call-Id` naming the record a conference leg is filed
under, `X-FCP-Transferred-From` naming the call a record is correlated
with, `P-Asserted-Identity` deciding the name and number a room shows
for a participant, and the `Alert-Info` / `Call-Info` /
`X-FCP-Intercom-Mode` trio that is how this platform's own intercom
makes a phone answer itself. The identity one was demonstrated on the
floor before the fix: a phone dialling a room with
`P-Asserted-Identity: <sip:+15125550100@carrier.example>` was listed as
`+15125550100`. The auto-answer copy is narrower than it looks — it
happens on the hunt-group and forward-on-no-answer paths, not an
ordinary call — and was not reproduced end to end, only read in the
code. Headers with a working gate of their own keep it, and a transfer
origin naming a call the node actually has still survives from an
untrusted peer, since a transferred phone carries one legitimately
(RFC 3261 §19.1.5). The conference server gains `[sip] trusted_edges`,
which is how the call manager and the gateway beside it keep naming the
record a leg belongs to. Phase 4 done (v0.25.226): the edge defended.
The call manager is the edge of every single-binary deployment and had
none of the defences the SBC has — no address list, no per-source call
rate, no policy for a message that is not SIP — while `[admission]
max_cps` is a node-wide budget one source could spend entirely.
`fcp-sip/src/edge.rs` is the guard, in the shared handler so every binary
built on it can have the same defences, wired on the call manager from a
new `[security]` section. Before the parser it checks the address list,
the message size and whether this is SIP at all, and each of those
**drops** rather than answering: replying to a denied address confirms
the port is live and makes the node a reflector. On a parsed request,
where there is a transaction to answer on, a per-source call rate is
refused `503` with `Retry-After`, counted only where a call starts so a
long call is not refused for being put on hold twice.
`max_invite_cps_per_source` defaults to 20, settled with the user on
2026-09-22: a bound exists out of the box, several times what a busy
office behind one NAT peaks at, with `[e911] trusted_edges` exempt since
an edge carries other people's calls by design. The known cost is a
carrier trunk above 20 cps from one address, throttled until it is
listed. The address list is allow-by-default and the malformed policy is
log-and-drop, which is what the handler already did, so nothing changes
for a deployment that sets nothing; a typo in a CIDR is a startup error.
The ACL types moved down from `fcp-sbc-core` into the shared handler so
there is one implementation, re-exported under the names the SBC's
configuration has always used. Phase 5a done (v0.25.227): the rest of the
survey (§2.6), which mostly came back clean and now says so explicitly.
Every binary that serves an API already refuses a non-loopback bind
without a token, so the fail-open path is reachable only from localhost;
the gateway, the one meant to face browsers, is fail-closed with only
`/join` and `/ready` open. Tokens are strong — 32 CSPRNG bytes stored as
a SHA-256, HMAC-signed join tokens and migration tickets — and nothing
logs a secret's value, though a browser WebSocket has to pass its token
as a query parameter, which is the part of a URL that logs keep. One
finding: `fcp-presence-server`, `fcp-ivr` and `fcp-voicemail-server`
compared the API token with `==`, which stops at the first differing
byte, while nine other binaries each carried their own correct copy of
the same comparison; `fcp_core::secret::constant_time_eq` is now the one
implementation and both halves are gone. A carrier's identity headers are
already handled — phase 3 strips them and a trunk's rules re-assert what
it is entitled to — and STIR/SHAKEN attestation is static per trunk, so a
carrier cannot drive the platform into vouching more strongly than it was
configured to; the hazard named rather than fixed (the user's call,
2026-09-22) is that a call merely transiting the platform is signed at
whatever its outbound trunk declares, so such a trunk should declare B or
C. Replay: migration tickets are single-use by nonce, join tokens
deliberately are not — a browser that drops has to come back — and are
bounded by a lifetime of between a second and a day. Phase 5b done
(v0.25.228, siphon-rs v2026.09.22), and the project is complete: fuzzing
the stateful surfaces. The first thing it found was that the parser was
not fuzzed either — `fuzz/Cargo.toml` inherited two dependencies with
`{ workspace = true }` while declaring its own `[workspace]`, and carried
no `[[bin]]` entries, so the crate did not compile and one target had
rotted against a `parse_request` that now returns `Option`; nothing
noticed because nothing could build it, and §1 of the design had called
the parser fuzzed on the strength of the files existing. Four targets
added for the stateful surfaces — dialog identity and route sets,
registrar AOR normalisation and binding validation, transaction matching,
and SDP offer/answer — each asserting the invariant that matters rather
than only the absence of a panic. The dialog target found a remote panic
on its first run: `extract_tag` sliced `trimmed[..4]` to test for a `tag=`
prefix, which panics when the fourth byte is inside a multi-byte
character, and that is a `From` header every in-dialog request is matched
through, so any peer could stop the process; `validate_event_package` had
the same shape on an `Event` header. Both use `get` now and both have
regression tests. Seeds are generated rather than committed so they
cannot go stale, which matters more than it sounds — the first unseeded
run managed seventeen million executions that almost all stopped at the
request line. A nightly workflow runs every target for ten minutes and
keeps its corpus between runs.

## What is next (chosen 2026-09-22)

**Multi-region leftovers** — chosen after the edge security project
closed, from the candidates the 2026-09-20 entry named. Survey and design
in `docs/MULTI_REGION_FOLLOWUPS.md` (v0.25.230): the three **Not covered**
lists `docs/MULTI_REGION.md` left behind, checked against the code.

What the survey found sound: the replica split is drawn correctly
(configuration reads to `self.reads()`, writes and must-be-current reads
to the primary), the hot-path config reads are cached, region-aware
placement works and prefers the caller's region even when another region
already hosts the room, and a node with no region is local to nothing so
a rolling upgrade strands nobody.

What it found wrong: **registrations are the one hot-path store with no
regional story.** Every other call-path lookup is replica-served or
cached; registrations are primary-always by design and uncached, so an
ordinary call in a region that does not hold the primary makes at least
two cross-region round trips before the callee rings — the caller's
bindings for admission and the callee's for routing — with a hunt group
reading once per member and domain aliases multiplying every one of them.
The Redis registration backend that exists does not fix it: it reads
`storage.redis_url`, the same Redis cluster membership requires to be
shared across regions, so it moves registrations from one global store to
another; and it is implemented only on the call manager, not on the
standalone registrar or the SBC. Also open: nothing gets a region's
callers to that region's nodes, a partitioned region cannot tell that its
*view* is what is broken, and the event bridge forwards everything
unmeasured.

Four questions settled by the user: registrations get a short-TTL cache
first and the regional question is deferred to a decision point with
measurements behind it; regional ingress is a deployment guide, not a
platform feature; a partition is made visible rather than acted on; and
the event bridge is measured before it is filtered. Four phases: measure,
the registration cache, a partition is visible, the deployment guide.
Phase 1 done (v0.25.231): the cost is a number. Registration lookups are
counted by reason — direct, alias, housekeeping — on `/status` and as
`fcp_registration_reads_total` on `/metrics`, and the floor measures a
two-party call setup at **2 direct reads**, exactly what the survey
estimated, rising to **6** with two domain aliases configured, since each
lookup is repeated at every other alias. Housekeeping is counted apart so
the call-path number is not inflated by REGISTER handling. On the event
bridge, envelopes per peer were already counted and shown on `/status`;
bytes were not counted at all and neither was on `/metrics`. Both are now,
per peer region, with bytes recorded when a batch lands rather than when
it is queued, so a retried batch does not make the number bigger the worse
the link is. Nothing changed behaviour.
Phase 2 done (v0.25.232): the registration cache. The call manager keeps
each registration lookup — empty answers too, which is most of what an
alias costs — for `[registrar] cache_ttl_secs` (default 30, zero is off),
invalidated by its own REGISTERs and deregistrations as the store takes
them and by other nodes' registration events (the admin API's removals now
publish one), with a generation on each entry so a lookup that raced a
write cannot put the old rows back. A repeat call within the TTL costs
**0 round trips**, where phase 1 measured 2, and 6 with two aliases; the
first call after a REGISTER still pays. `/status` shows the cache and
`registration_reads.cached`, `/metrics` `fcp_registration_cache_hits_total`.
A standalone `fcp-registrar` publishes no events, so behind one a new
registration reaches a call manager within the TTL. Whether this closes
§2.1 depends on how much traffic is repeat lookups — the decision point,
with hits against round trips on `/metrics` as its evidence.
Phase 3 done (v0.25.233): a partition is visible. Every node's membership
loop keeps a view of the cluster — `whole`, `store_unreachable` (its
discovery fails or hangs, so its member list is as old as
`view_age_secs`), or `remote_regions_lost` (every node outside its region
went quiet at once, not by shutting down) — and, on the call manager and
conference server, whether it still reaches its database. It is on
`/status` (`cluster_view`), `/metrics` (`fcp_cluster_view`), the heartbeat
record (the admin API per node, `nodes_with_partial_view`, a badge and a
banner in the admin UI) and `system.cluster_view_changed`; nothing acts on
it. A clean shutdown leaves a `cluster:left` marker, so `system.node_left`
says `shutdown` or `heartbeat_expired` and a region taken down cleanly is
not a partition. The survey was wrong on one point, now corrected: a node
cut off from Redis does not lose the other nodes after 15 s — its list
stays as it was. And the partition test found a real fault: `RedisStorage`
used redis-rs 0.27's defaults, which retried a lost connection after 1 s
and then every 60 s with every command waiting on the retry, so a
two-second blip stalled a node's membership and events for minutes;
retries are now at most a second apart and a connection attempt gives up
after two seconds, and the node is back about a second after the link.
Phase 4 done (v0.25.234): the deployment guide,
`docs/MULTI_REGION_DEPLOYMENT.md` — the shape to choose, what each region
runs, which Redis is shared, SRV and carriers, the cost a region away from
the primary pays, what a partition looks like, alerts and a checklist.
Writing it found that a call manager has one Redis for membership, events,
parking, leases and placement, so a deployment is one cluster on a shared
Redis or a cluster per region — `docs/MULTI_REGION.md`'s mix of
global membership and regional events is not configurable on a call
manager. The guide recommends a cluster per region unless rooms, pickup or
tenant limits must span regions, and lists nine gaps (a conference server
advertising `0.0.0.0` when bound there, `fcp-registrar`'s unauthenticated
`/drain`, the SBC's random membership id, HTTP-ingested events never
crossing regions, and others). All four phases are done.
Decision point closed (2026-09-23): the cache closes §2.1 and the regional
registrations project is not written. A cold two-party setup pays two
primary round trips (six with two aliases), a repeat within the TTL none,
and whether that is enough is a ratio on `/metrics` only a deployment can
produce; against it, the regional answer would need a second Redis on the
call manager (phase 4's finding), a backend choice on the standalone
registrar and the SBC, and a cross-region lookup — a multi-phase project
on an estimate. The design doc records the decision and where the project
starts if a deployment's ratio ever says otherwise.

## What is next (chosen 2026-09-23)

**The deployment guide's gaps** — chosen when the multi-region leftovers
closed. Survey and design in `docs/DEPLOYMENT_GAPS.md` (v0.25.235): the
eight gaps `docs/MULTI_REGION_DEPLOYMENT.md` §8 listed after the first,
checked against the code. Confirmed, and mostly worse than the guide
says: the conference server's advertised SIP address is the raw bind and
is read by placement on the call manager, placement on the gateway and
participant migration; the SBC carries four node ids; the registrar's
`/status` is open beside its `/drain`; the gateway's probe fails against
an authenticated admin backend on any path, since the API's `/health` is
behind its bearer layer; HTTP-ingested events reach neither peer regions
nor a second events server in the same region; no binary keeps a handle
it could ask for the replica's health; and a trunk's INVITE goes to its
first host only — the guide's "list every carrier address in `hosts`"
gives no failover — while trunk REGISTER already consults SRV, TCP-first.
Three questions settled by the user: hosts become a failover list and
then an unset port means RFC 3263 (a siphon-rs change to keep every
resolved target); ingested events join Redis at `/ingest`; the API's
`/health` needs no token. Seven phases: identities and advertised
addresses, the registrar's HTTP token, configuration that does not do
what it says, ingested events reach Redis, replica health said, a
trunk's hosts as a failover list, SRV for trunks.
Phase 1 done (v0.25.236): identities and advertised addresses. Every
heartbeat — call manager, conference server, IVR, voicemail, presence,
gateway, SBC — now carries its SIP and HTTP address with an unspecified
bind IP replaced by the node's `local_ip`
(`fcp_core::transport::advertised_address`), so placement on the call
manager and the gateway, and a migration's `Refer-To`, send callers to a
conference node bound to `0.0.0.0` at an address they can reach. The SBC
has one node id: `[service] node_id` (a `String` with an `sbc-<uuid>`
default, like every other binary) names its membership record, its
drain's events, its calls and its trunk registrations, where it had four.
The presence server's `[redis] node_id` the survey called dead is its
Pub/Sub identity and stays. Two binary tests prove it: a conference
server bound to `0.0.0.0` heartbeats `127.0.0.1`, and an SBC's drain
marks the record its configured id names.
Phase 2 done (v0.25.237): the registrar's HTTP surface. `fcp-registrar`
takes `[http] api_token`; `/status` and `/drain` answer 401 without it,
`/health` and `/ready` never ask, and a non-loopback HTTP bind is refused
without a token, as on every other binary. Its default HTTP bind moves to
`127.0.0.1:8083` to match them — a deployment that relied on the old
`0.0.0.0` default now binds explicitly and sets the token. The two
documents that said the call manager's `/drain` was open, or left it off
the list of binaries needing the token, are corrected.
Phase 3 done (v0.25.238): configuration that does not do what it says.
The API gateway's backend health path was a route nothing served, and the
one that exists sat behind the API's bearer layer, so an authenticated
admin backend read unhealthy whatever was configured. Now
`health_check_path` is optional and defaults to each backend kind's own
route (`/api/v1/health` for the admin, `/health` for the events server
and jobs), `/api/v1/health` needs no token, and `health_check_interval`
— parsed and never read — drives a background probe per backend whose
last result, with its path, HTTP status and age, is what
`GET /gateway/status` reports; a status request probes nothing itself.
The call manager's `[events] redis_channel` and `redis_stream_key`, read
nowhere, are refused at startup with a message naming the events
server's and `fcp-jobs`' `fcp:` prefix as the only knob. The wrong path
is gone from the example configs, the compose config and the docs.
Phase 4 done (v0.25.239): ingested events reach Redis. What a conference
server, IVR, voicemail server or the admin posts to the events server's
`/ingest` used to reach that one process's subscribers and no one else —
not the region's other events servers, not the peer regions. With Redis
as the bus the events server now writes an ingested event to its
region's stream in an envelope carrying its region and the posting node
(`X-Node-ID`, which every HTTP publisher now sends), and delivers it
through its own consumer like any other event: once here, once in every
other events server on the stream, once in every peer region through the
bridge, webhooks in the origin region only. A stream that cannot be
written answers 503 so the poster retries rather than the event being
lost. Proved by two events servers in two regions on two Redis databases.
Phase 5 done (v0.25.240): replica health said. The backend probed its
read replica every five seconds and no binary could ask, having dropped
the concrete backend into its trait objects at construction. Each now
keeps a `ReplicaHealth` handle beside them, and the replica's last probe
— host, reachable, healthy, lag, limit, and how old the probe is, so a
stuck probe no longer reads as healthy for ever — is `replica` on
`/status` (call manager, conference server, IVR, voicemail server,
registrar) and `/ready` (SBC, presence server), `null` without one, and
`fcp_replica_reachable`, `fcp_replica_healthy`, `fcp_replica_lag_seconds`
and `fcp_replica_probe_age_seconds` on `/metrics` where a binary has one.
The deployment guide's alerts gain "a region reads from the primary" and
"a replica probe is stuck". The media server is left out: its HTTP is
forge-media's, and its database is a config store read at startup.
Phase 6 done (v0.25.241): a trunk's hosts are a failover list. An
outbound INVITE went to a trunk's first host only, while the probe
passed if any host answered, so a trunk whose first host was dead read
healthy while every call to it failed — and the deployment guide's
advice to list every carrier address in `hosts` bought nothing. Now one
parser (`fcp_core::trunk::TrunkHost`) reads every host the same way —
`host`, `host:port`, `[v6]:port` — and the API refuses what it cannot
read; a trunk yields one route per host in the order written, each on
its own port or the trunk's, with the hosts this node's last probe found
down after the rest, and the ordered resolver walks every host of a
trunk before the next trunk; the probe asks every host and keeps each
answer, shown as `hosts_down` on the trunk report; `transport_port` is
nullable (migration 077), unset meaning the transport's default, so a
TLS trunk is on 5061 in the database, the API and both probes, and the
TLS trunks stored on 5060 by the old default are unset; the admin-side
probe says it is UDP. Proved on the floor: one trunk with two hosts, the
first refusing, the second answering, the other trunk never rung.
Phase 7 done (v0.25.242): SRV for trunks — and the project is complete.
A trunk host given by name with no port is now what RFC 3263 says it is:
the trunk set asks the system resolver at each refresh, on the trunk's
own transport (NAPTR, SRV, then A/AAAA), and every target it gets back —
the SRV targets in priority order, or the name's addresses — is one
route the call tries in turn, on a transport error, a timeout or a 503,
before the next host and the next trunk; the probe asks every target and
the inbound index takes the carrier's calls from every one. Done without
the siphon-rs change the design named: phase 6's per-host routes and the
B2BUA's existing one-at-a-time failover already gave every target its
turn on all three conditions, which keeping the targets inside one SIP
transaction would not have. A registering trunk's REGISTER names its
transport, so a UDP trunk no longer registers over TCP; the flow
engine's emergency dial takes the transport its route names instead of
UDP. The deployment guide's last gap bullet is struck.

## What is next (chosen 2026-09-23, after the deployment gaps closed)

**Ship and operate** — the last of the four candidates named on
2026-09-20. Survey and design in `docs/SHIP_AND_OPERATE.md` (v0.25.243).
The question: does what we ship start, and does what we tell an
operator work? Both no. Every shipped compose configuration was started
against its binary: five of nine are refused before they touch the
network (the admin, call manager, both conference configs and the events
server, each for the token or authentication rule edge security added on
a public bind), and the compose file could not bring the stack up anyway
— an external volume nothing creates, no Postgres service, the call
manager and conference server both on host port 5060, the media server
on the admin's 8080, TLS paths to a directory that does not exist, and
every producer posting events with no token. No configuration struct
outside the SBC's `[admission]` refuses an unknown key, so seven shipped
or example files carry keys the code never reads and start as if
configured. None of the five operator documents gives a working setup:
the token rules are in none of them, every operator route in the
runbook is called without a token, binary names and ports are wrong,
and the root README configures sections the call manager never had while
listing billing, E911, video and compliance as future. What is sound:
the Dockerfiles, the binaries' own validation, every root
`fcp-*-local.toml`, and `docs/MULTI_REGION_DEPLOYMENT.md`. Cause: nothing
starts a shipped configuration, no CI job parses one, and the binaries
accept any key, so three projects that each added a right startup rule
left the packaging and the documents behind. Four questions settled by
the user: unknown keys are refused everywhere; the root packaging is
deleted; every token gets an environment override so the compose stack
carries no secrets; the five operator documents are rewritten from the
code and the four crate READMEs and five dead deployment documents go.
Six phases: a configuration cannot lie (`deny_unknown_fields`,
`--check-config`, CI over every TOML); secrets from the environment; the
stack comes up; the stack is proved by a compose-shaped floor test; the
single-node guide and the runbook; the READMEs.
Phase 1 done (v0.25.244): a configuration cannot lie. Every
configuration section of every binary, and the sections the shared
crates own, refuse an unknown key at startup with a message naming it;
every binary takes `--check-config`, which loads and validates the file,
prints the effective configuration with every secret masked (by key,
and the password of any URL) and exits 0 when the node would start, 1
when a startup rule refuses it, 2 when it does not parse. Run over
every shipped, root and example TOML it refused exactly the seven files
the survey named, all now fixed, and the call manager's dead
`cluster_secret` is gone. Each binary's smoke suite now runs the files
it ships through the flag, so a configuration that stops parsing fails
the PR; the five compose files the token rules still refuse are held at
"parses" until phase 3 gives them their tokens.
Phase 2 done (v0.25.245): secrets from the environment. Every token and
secret a binary read only from its TOML — the events server's
`[auth] token`, the call manager's `api_token` and events token, the
conference server's HTTP token, join secret and cluster secret, the
registrar's, IVR's, voicemail's and presence server's `api_token`, the
media, WebRTC gateway and TTS servers' token lists, and every database
URL, since a password in one is a secret — now has an environment
variable in the binary's `FCP_<BINARY>_*` style, listed by `--help` and
never echoed, that wins over the file. A secret store, Kubernetes or
the compose stack's `.env` can inject them and the shipped TOMLs carry
none. Left in the file: the events server's notification providers'
credentials, TURN passwords and webhook signing secrets, which are
nested lists.
Phase 3 done (v0.25.246): the stack comes up. `deploy/docker-compose/`
has a Postgres service, its own volumes and a port plan with no
collisions; the host-network services reach Postgres, Redis and the
events server through the ports those publish on the host and the admin
reaches the call manager and conference server through
`host.docker.internal`; every secret is in `.env` under the binaries'
own `FCP_*` names and the compose file hands the shared ones on, so no
TOML carries a token; the wiring is set — events tokens on every
producer, the admin's call manager, conference and join-secret settings,
the conference server's trusted edge, the call manager's conference URI,
the CDR stream pipeline through `fcp-jobs`; TLS is off rather than
pointed at missing certificates; Prometheus gets its bearer tokens from
the same `.env` and scrapes every metrics endpoint; a registrar profile;
the scripts send tokens and read Compose's output as it is; the root
packaging and the committed binary are gone. Each binary's smoke suite
checks its compose configuration with the stack's `.env`, so all ten
must start; CI validates the compose file alone, with the GPU overlay
and with the registrar profile on every pull request, and builds the
nine images nightly. Not proved: bringing the stack up, since the
development machine has no Docker — phase 4's test does that.
Phase 4 done (v0.25.247): the stack is proved. The testkit reads the
compose file and `.env` and starts the events server, the admin, the
gateway, the call manager and the conference server from the very
binaries, configurations, environment and health checks the compose
file names, on its own ports, against the test's Postgres and Redis;
a phone registers, joins a room on the conference server, the events
server lists the call, the gateway reads it back through the admin and
reports its backends healthy. It runs in CI, so the compose file and
its configurations are checked by a call from now on. Its first run
found that the shipped `routing.yaml` listed no users and the YAML
routing store knows only the users it lists, so the compose call
manager had answered every INVITE with 500: the stack could not have
carried a call. The file is gone from the stack; that a caller missing
from a YAML routing store is a 500 rather than a refusal is noted.
Phase 5 done (v0.25.248): the single-node guide and the runbook,
rewritten from the binaries — their `--help`, their `--check-config`
defaults, their routers and their log lines — rather than from the old
text. The guide names every binary by its real name, states the startup
rules with the key and the environment variable for each, gives the
port plan the root local files share (their RTP ranges made disjoint),
maps what connects to what with which token, and shows the first
tenant, user and extension through the API; the runbook lists every
binary's routes and token, and gives the drain, rolling restart and
partition procedures as the code does them.
Phase 6 done (v0.25.249): the READMEs — and the project is complete.
The root README is the map: what runs, how to build and test with the
three CI gates, where to run it, the configuration facts every binary
shares, and which document answers which question; its roadmap is this
file. It no longer claims a license the repository does not carry. The
compose README is written against the rebuilt stack. The four crate
READMEs are gone; the five dead deployment documents are archived with a
row each, and the capacity numbers measured on 2026-09-21 that one of
them held are now §12 of the single-node guide. What ships now starts,
is checked by CI, and is described by documents written from the code.

## What is next (chosen 2026-09-24)

**The production-shaped stack** — chosen when ship-and-operate closed,
from that project's own leftovers. Design in `docs/PRODUCTION_STACK.md`
(v0.25.250): five binaries with no image and no service (the SBC, IVR,
voicemail, presence and TTS servers; the TTS image is gigabytes because
its engine brings PyTorch), no TLS anywhere (the SIP binaries and the
media and WebRTC servers can serve it themselves; the admin, gateway and
events server cannot, so provisioning cannot require HTTPS as it does by
default), encryption at rest never turned on, credentials in nested lists
that no environment variable reaches, and a routing engine that maps a
caller to a tenant only through a YAML `users:` list and answers a
stranger with a 500. Five phases: every binary an image and a service;
secrets by reference (`${NAME}` in any configuration string); TLS in the
stack with a one-shot certificate service and a reverse proxy; encryption
at rest on; a stranger refused with 403 and users taken from the
database. Four questions settled by the user: the TTS image under an
opt-in profile, `${NAME}` references in every configuration string, a
reverse proxy for HTTPS, users from the database. Phase 1 done
(v0.25.251): Dockerfiles for the SBC, IVR, voicemail, presence and TTS
servers (the TTS image on `python:3.12-slim-trixie` with CPU PyTorch,
Kokoro and `espeak-ng`, its worker script copied in), compose services
for each — the SBC on the host at SIP 5078 and health 8094 with one
bidirectional trunk to the call manager, the IVR, voicemail and presence
servers on the host at the guide's ports, the TTS server on the bridge
under the `tts` profile — the call manager sending unanswered calls to
voicemail and speech requests to the TTS server, `images.yml` building
fourteen, every new configuration in its binary's configuration check,
and the stack test starting all twelve default services and waiting for
each to be healthy. Phase 2 done (v0.25.252): every binary loads its
file through `fcp_core::config::parse_toml`, which resolves `${NAME}` in
every string from the environment before deserializing — a name the
environment lacks refuses the file naming the key and the variable
(`--check-config` exits 1), `$${…}` is a literal — so the nested
credentials no per-field variable reaches (`[notifications.smtp]
password`, a TURN password, a webhook secret) stay out of the file; unit
tests in `fcp-core`, a reference resolved, masked and refused-when-unset
test in all fourteen smoke suites, and the stack test's call manager
taking its node id by reference. Phase 3 done (v0.25.253): a `certs`
service (`alpine/openssl`, `scripts/make-certs.sh`) makes a CA and a
certificate per TLS service into the `fcp-certs` volume once and keeps
them; SIPS 5061 and WSS 5081 on the call manager (5063/5083 on the
registrar profile), SIP/TLS 5075 on the conference server and 5079 on the
SBC, HTTPS on the WebRTC gateway's one listener (8095) and beside the
media server's HTTP (8185, a new `[tls] bind`); nginx (`fcp-proxy`) in
front of the gateway (8443), admin (8444) and events server (8445) with
`X-Forwarded-Proto`, so the admin requires HTTPS for provisioning again
and sends browsers to `https://${HOST_IP}:8095` — the stack's first
`${NAME}`; the testkit's phone talks SIPS (`Phone::new_tls`) and its HTTP
helpers HTTPS under a trusted CA; the stack test registers over SIPS and
reads the gateway and media server over HTTPS (the proxy, a container,
is checked by compose and its own health check). Phase 4 done
(v0.25.254): the `certs` service also makes `master.key` (`openssl rand
-base64 32`) into the same volume and keeps it; all nine services that
open Postgres run with `FCP_ENCRYPTION_PROVIDER=local` and
`FCP_MASTER_KEY_FILE`, so the first to start creates the initial DEK and
every write of a secret is sealed; the README says how to run the
one-time `reencrypt`, that the key is part of the backup and what losing
it means; the stack test's services run the same way and it reads
`GET /api/v1/system/encryption` for `local` and an active DEK. Phase 5
done (v0.25.255): `fcp_routing::DatabaseRoutingStore` — the engine's
users from the platform's `CredentialStore`, looked up within the tenant
the SIP handler resolved, the YAML file supplying policies only; the
handler answers a caller that is not one of the tenant's users (or is
disabled) with 403 and a reason, never a 500, a trusted edge's call being
its named user's and a carrier's call on a trunk evaluated without a
user; the compose stack ships `routing.yaml` again (one allow policy for
the default tenant, no users) and the stack test's call goes through it;
the floor has the 403. The production-shaped stack is complete.

## What is next (chosen 2026-09-24, after the production stack closed; complete 2026-09-25)

**Backup, restore and upgrade** — design in `docs/BACKUP_RESTORE_UPGRADE.md`
(v0.25.256): the stack can be run and cannot yet be kept. What the survey
found: every piece of state and where it lives (Postgres, the certs
volume with the master key, files on each node, Redis), the call
manager's recordings written to the container's own filesystem in the
compose stack, no backup procedure or tool anywhere, migrations that
refuse an older binary (`VersionMissing`) so rollback is a restore
nobody can take, three migrations that drop or rename with nothing
stopping the next, a node's version in every heartbeat and shown
nowhere, and two storage documents describing backends that do not
exist. Five phases: every stateful path in a volume and the state
documented; `backup.sh`; `restore.sh` proved by a test that restores
into a fresh stack and registers the old user — and refuses without the
key; upgrade rules, the version on the node list, a migration-discipline
check in CI; the skew proved. Five questions settled by the user, each
on its recommendation: shell scripts, Redis not backed up, an old binary
keeps refusing a newer schema, the stale storage documents deleted, a CI
check on new migrations. Phase 1 done (v0.25.257): the compose call
manager's `recording_dir`, `prompts_dir` and `tts_cache_dir` under its
`call-manager-data` volume (they defaulted to the container's own
filesystem); `docs/BACKUP_AND_RESTORE.md` written from the inventory —
what lives where and whether it is re-derivable, the directories per
binary with their defaults, what a backup must hold, what Redis loses,
what a restore needs — and pointed at by the compose README, the guide
and the runbook; `docs/STORAGE_BACKENDS.md` and `docs/SQLITE_STORAGE.md`
deleted. Phase 2 done (v0.25.258): `deploy/docker-compose/scripts/backup.sh
<directory>` — `pg_dump -Fc` through the `postgres` container, a tar per
stateful volume (the certs volume with the master key first; a volume
whose service never ran is named as absent, a backup without `fcp-certs`
is refused), a manifest naming the release (the admin image's binary
asked `--version`), the project, the key ring's active KEK id and the
time, and `SHA256SUMS`; the operator document's §3 says what it writes,
that the dump is a snapshot and the volumes are copied live after it,
which services to stop for a backup with nothing in flight, and the
from-source procedure with the same commands. Phase 3 done (v0.25.259):
`scripts/restore.sh <directory> [--no-up]` into a fresh stack — the
checksums verified, no volume of the stack may exist (`down -v` is the
operator's step), the master key in `fcp-certs.tar.gz` fingerprinted
against the manifest's KEK id before anything is touched, each volume
created as compose labels it and filled from its tar, `postgres` started
alone and the dump restored with `pg_restore --exit-on-error`, the
restored key ring checked against the manifest, then `up -d --build`;
proved by `crates/fcp-call-manager/tests/compose_restore.rs`, which runs
the same commands against the binaries' directories: a user made sealed
through the admin API calls a room, the backup is taken, the database
dropped and the directories deleted, the restore made into a new
database and directory, all nine database services refuse a wrong
master key naming both KEKs, and with the right one the same DEK is
active, the old user registers with the sealed password and calls; the
operator document's §4 has the compose and from-source procedures.
Phase 4 done (v0.25.261): the version was already on every heartbeat,
the node list and the cluster page's column — nothing compared them;
now every node groups the members by release at each discovery
(`fcp_cluster::versions`), `cluster_versions` on the call manager's and
conference server's `/status`, `fcp_cluster_version_skew` and
`fcp_cluster_nodes_by_version` on `/metrics`, `versions` and
`version_skew` on `/api/v1/cluster/stats`, a banner on the cluster page
naming who is on what, and `system.version_skew` when the set of
releases changes while there is or was more than one — said by every
node, never acted on; the rules in the operator document's §5 (back up
first, the first new node migrates, an old binary refuses the new
schema and rollback is the backup, a migration is never edited, a
migration may add but not take away what the previous release reads,
one release at a time), the compose and cluster procedures, and
`scripts/check-migrations.sh` in CI — a new migration that drops or
renames a table, view or column or changes a column's type fails
without `-- fcp: contraction: …, read by nothing since vX.Y.Z`
(migrations up to 077 predate the rule; 047, 058, 059 contract), proved
against itself first. Phase 5 done (v0.25.263): the skew proved —
`fcp-storage-postgres/tests/upgrade.rs` (this release refuses a schema
a later release migrated, naming the migration, and refuses an edited
one), `fcp-cluster/tests/versions.rs` against Redis (a node on another
release is grouped apart, said as `system.version_skew`, counted by the
reader, and said resolved when it shuts down), and the stack test
asserting the compose call manager — now clustered as it ships, with
the admin's `[cluster] redis_url` set, so the cluster page lists it —
on the node list with this release and no skew. The project is
complete: a deployment can be backed up, restored and upgraded, and
each is proved by a test.

## What is next (chosen 2026-09-25, after backup, restore and upgrade closed)

**The nightly proves the shipped stack** — design in `docs/NIGHTLY_STACK.md`
(v0.25.264). Nothing on the roadmap is left; the survey found what is
open is operational, and one item red: both submodules' nightly fuzzing
has never fuzzed anything in CI (forge-media's since 2026-09-11,
siphon-rs's since 2026-09-23) — the prebuilt cargo-fuzz is a musl
binary and builds for musl unless told the target, so every target
fails to build and is reported as crashing — and FCP's CI never looks; the fourteen images are built every
night and never run — the stack is proved over binaries, and the backup
scripts were proved live by hand; tests share one Redis (a database
index drawn at random per process, one `fcp:` prefix for every node), which
leaked records between binaries on 2026-09-25. Four phases: fuzzing back
and watched (the host named as the target, a build failure told from a
crash, a weekly job in FCP that reads the submodules' nightlies); the images pushed to GHCR
and a `stack` job that pulls them, brings the stack up, takes a call
with the testkit's phone, backs up, drops, restores and registers again;
a test's Redis flushed at acquisition; what the nightly finds written
down. Five questions settled on their recommendations: name the target
rather than build cargo-fuzz from source, GHCR under the repository, pull rather than
build, a cargo test rather than a script, a flush rather than a prefix.
Phase 1 done (v0.25.265): both fuzz workflows name the host triple on
`cargo fuzz build` and `run` and report a target that did not build
apart from one that crashed — validated by a twenty-second run of every
target in each repository, millions of executions per target, the
first fuzzing either has done in CI; FCP's weekly CI gains
`submodule-nightlies`, which reads each submodule's latest completed
Fuzz run from the GitHub API and fails when it is not a success or is
two days old. Phase 2 done (v0.25.266): the Images workflow pushes the
fourteen images to `ghcr.io/thevoiceguy/fcp-<name>` as `nightly` and the
sha, every compose service names its image beside its build
(`FCP_IMAGE_TAG`), and a `stack` job pulls them, brings the stack up,
runs `compose_live.rs` (health, a tenant and users through the API,
registration over SIP and SIPS, a call with media, the node list, the
record), backs up, drops, restores and runs it again; run 36177386025
passed end to end. Seven bugs the first runs found, fixed: the proxy's
health check never passed (nginx on IPv4 only, `localhost` is ::1
first); a tenant id as printed was refused when handed back (every id
reads both forms now); the guide's extension example; a registered
phone dialling the node's address named no tenant (the caller's
registration decides now); a user without a device row was unreachable
(rung at their own address of record now); a fork awaited a dead
contact's CANCEL before relaying the answer (losers cancelled in their
own tasks); and every service inserting the first data encryption key
at once on first boot (made under an advisory lock now, and the five
health checks without a start period have one). Phase 3 done (v0.25.287): a test's Redis database is emptied when the
testkit hands it out (`private_redis_url`), so what one test binary left
reaches no other; a binary asking for more than the fifteen databases is
refused. Phase 4 done (v0.25.288): what the nightly found is written down
(`docs/NIGHTLY_STACK.md` §6) — the restore's Postgres readiness race and
the compose health checks (TCP now), the site step's `jq` (the site stack
had never run in the nightly), forge-bfcp's parser (forge-media #168) —
and the packages' visibility left to the owner; `docs/TEST_FLOOR.md` gets
the shipped stack as its top layer. The nightly-stack project is
complete.

## What is next (chosen 2026-09-25, to follow the nightly stack)

**Hybrid: the control plane and the site** — design in
`docs/HYBRID_CONTROL_PLANE.md` (v0.25.267), written after a feature review
against BroadSoft, CUCM, Zoom, Webex Calling and Twilio found the
platform's largest structural gap: a customer's binaries on their own
network cannot use cloud features, because every node opens the one
Postgres and the one Redis directly. The model: the cloud is the system
of record for configuration; a site is the tenant's own stack (every
binary, its own stores and key) plus `fcp-link`, enrolled by a one-time
token into mutual TLS, which keeps the site's database a copy of the
tenant's slice through a trigger-written journal, a snapshot and a
change feed, ships the site's CDRs, events and recording rows up,
carries its health, and lets the site run alone when the link is down;
calls that need the cloud go over a trunk to the cloud SBC provisioned
at enrollment. Seven phases from sites and enrollment to a site stack
in the nightly; nine questions, each settled on its recommendation.
Phase 1 done (v0.25.268): `sites` and `site_enrollment_tokens`
(migration 078), `fcp_core::site` with `SiteStore` in every store bundle,
`SiteRead`/`SiteManage`, `/v1/sites` (made with a one-time token shown
once, listed, read, a new token, disabled, enabled, deleted, in the
tenant), `/v1/cluster/sites` for the cluster page's Sites section, the
admin's `[link]` — a link CA made in `ca_dir` on first start, an enroll
listener over TLS and a link listener under mutual TLS from that CA, a
per-site secret on every heartbeat, a sweep marking a quiet link down,
`system.site_link_changed` — and the new `fcp-link` binary (its image
built nightly), which enrolls with the token, keeps its certificate and
secret in its state directory, and heartbeats what it finds at the site
(every node of the site's cluster with the site's Redis). Proved by
`fcp-link/tests/enroll.rs` against a live admin: enroll, heartbeat,
the spent token refused, a certificate-less connection refused at TLS,
disable and enable, the sweep; the sites API by `api_postgres.rs`. The
compose admin ships with the link on (8446 enroll, 8447 heartbeats).
Phase 2 done (v0.25.269): the journal, the snapshot and the feed.
`sync_journal` (migration 079) is written by one trigger function on
every synced table — the tenant, users, devices, extensions, line
bindings, numbers, routing rules, LCR routes, trunks, queues, flows,
prompts, configuration templates, meetings, locations, emergency
networks and configuration, rate plans and rates (a rate's change
journaled as its plan's) — as (seq, tenant, table, row, put or delete),
so nothing can change a synced row without the journal hearing of it;
`fcp_core::sync::SyncStore` reads it. On the mutual-TLS listener,
`GET /link/v1/snapshot` is every synced object of the site's tenant
with the journal's head as its cursor, and `GET /link/v1/changes?since=`
holds for up to thirty seconds, then answers each journaled change with
the object as it is now (a row since deleted becomes a delete), in pages
with `more`. Proved by `sync_journal.rs` (every table has the trigger;
puts, updates and deletes journal in order under the tenant) and
`fcp-link/tests/sync.rs` against a live admin (the snapshot holds the
tenant, a user made in the cloud is on the feed within a second with
the credentials the site will challenge with, a held feed comes back
empty on time, and the snapshot plus the feed equal a fresh snapshot).
Phase 3 done (v0.25.270): configuration at the site. With `[site]
database_url`, `fcp-link` opens the site's Postgres (migrating its
schema as every binary does), takes the tenant's snapshot and follows
the feed, applying each object through the same stores the cloud writes
with — so the site's store seals the digest credentials under the site's
own key — with its cursor kept in `sync.json` beside the certificate; a
rate plan lands with its rates, a flow as an exact copy
(`FlowStore::put_flow`). Every heartbeat carries `sync` (cursor, applied
at, the last error), and the cloud shows `sync_cursor`, `sync_applied_at`,
`sync_error` and `sync_behind` (entries the site has not applied) on
every site and on the cluster page. The site's admin, with `[api]
managed_by` naming the cloud, refuses a write to any synced resource
with 403 and that address (`fcp_api_core::managed`; reads and runtime
actions such as a trunk test or an agent's login go through). Proved by
`fcp-link/tests/apply.rs` against a live cloud, a site registrar and a
site admin: a user made in the cloud registers at the site within
seconds with the password set there, a wrong one is refused, a password
changed in the cloud replaces the one the site challenges with, the
site's admin lists the user and refuses to change her, and a user
deleted in the cloud is gone at the site.
Phase 4 done (v0.25.271): calls across. When the admin names `[link]
sip_host` (the cloud SBC's SIP-over-TLS address) every site enrolled is
given a link trunk — a tenant trunk with `site_id`, `link = cloud`, TLS,
credentials minted at enrollment in `[link] sip_realm`, registering
every two minutes, emergency-capable, after the site's own carriers —
named by `Site.trunk_id` and disabled and deleted with the site.
`Trunk.site_id` (migration 080) says where a trunk runs: the cloud's
nodes route out on none that has one, the link applies only its own
site's and turns the link's end to `site` as it lands, so the cloud's
carriers no longer reach the site. The cloud SBC loads the link peers
alone, challenges a site's REGISTER against the link accounts
(`fcp_sip::link_peers::SiteAccounts`), keeps each site's binding and
the RFC 5626 flow its TLS registration left (`LinkPeers`, `/admin/sites`
on the SBC), and knows a site's calls by where they come from.
`PhoneNumber.site_id` (`site_id` on `/v1/phone-numbers`) sends a
number's calls down its site's flow with the number as the target's
user, 480 when the site is not bound; a site's own calls are the
tenant's (`Call::link_site`), forwarded to the cloud's call manager with
`X-FCP-Caller-Tenant` and `X-FCP-Caller-Site`, which a trusted edge's
caller may now be, with no user of the cloud's to check. Every leg to
or from a site latches its media on the first packet from the site's
signalling address (`MediaManager::latch_leg`) instead of its SDP. The
site's call manager trusts `[sip] tls_ca_path` for the link, builds its
TLS client whether or not it listens on TLS, and is the one node at the
site that registers the site's end. Proved by `fcp-link/tests/calls.rs`:
a carrier's call rings the site's phone down the flow with audio both
ways, the site phone's call to the outside reaches the tenant's carrier
through the cloud as the tenant's own recorded call, and a site whose
call manager is gone is unbound and its number answers 480. Two
defects fixed on the way: a TLS trunk's INVITE went out over UDP, its
`;transport=tls` stripped as a contact parameter (routes are kept in
angle brackets now), and the media relay told a call's legs apart by
IP alone, so a leg learning its peer from a named address went deaf
beside a peer on the same address (forge-media v2026.09.26 classifies
by exact address first). Left for later: a site binds at one SBC and is
known by the exact connection it registered on; the link carries RTP,
not SRTP; a site's emergency call reaches the cloud without its
location; in-dialog requests other than BYE to a site behind NAT are
proved in phase 6.
Phase 5 done (v0.25.272): records up. Migration 081 gives `cdrs`,
`call_recordings`, `conference_recordings`, `voicemail_mailboxes` and
`voicemail_messages` a shipping sequence (`ship_seq`, bumped on every
change) and a `site_id`, and `pending_file_deletions` the site holding
the file; the link keeps a cursor per table in `ship.json` and ships
after it in batches. `POST /link/v1/cdrs`: the cloud sets the site's
rating aside, marks the record with the site and runs its own
processor — rated, charged, stored idempotently, `cdr.written` naming
the site. `POST /link/v1/records`: recording, mailbox and message rows
written naming the site, the site's logged deletions mirrored under the
cause `site`. Files stay at the site and are reached through
`site_requests`: the cloud writes a fetch or a deletion down, the link
collects it on `GET /link/v1/requests` (a long poll) and answers on
`POST /link/v1/requests/:id`, touching only `[site] file_roots`; behind
it stand the recording download, voicemail playback, subject exports
(the site named in `missing_files` when unreachable), erasures, the
API's delete and the reaper (`files_asked_of_sites`). The link forwards
the site's events from its Redis stream (consumer group `link`, without
`cdr.written`) to `POST /link/v1/events`, which the admin posts to the
events server's ingest with `x-origin-site`, so the envelope names the
site and the tenant's webhooks fire in the cloud once. The heartbeat
carries who is registered at the site, and `/v1/sites` shows it. Proved
by `fcp-link/tests/records.rs` against a live cloud, a site database, a
site Redis and a stub events server. Left for later: a site's own
retention still runs on its copies; a mailbox made at the site and one
made in the cloud for the same user are two; files sent up are held in
memory (512 MB at most); no policy copies a site's files to the cloud's
archive.
Phase 6 done (v0.25.273): survivability, proved. `fcp_testkit::cut::CutLink`
(the partition test's proxy, made shared) stands in front of the admin's
link listener and the SBC's TLS listener; `fcp-link/tests/survivability.rs`
cuts both and watches both sides: the cloud says the site's link is down
within `heartbeat_stale_secs` and the SBC drops its binding as the
connection closes; at the site, Alice re-registers, calls Bob with
audio, and calls the outside on the site's own carrier (a trunk with the
site's `site_id`, priority ahead of the link's) while the shipper's
cursors and the event consumer's entries wait; Carol is made in the
cloud; the link returns, the cloud says so, Carol registers at the site,
the site's two records and its call events reach the cloud naming the
site, and the site binds again so the carrier's call rings Alice. A call
that came up a site's link is now answered, re-invited and hung up down
the connection the site opened: the caller's leg keeps its
registration's flow (`Call::a_leg.flow_token`), a BYE toward it goes via
that flow, and a re-INVITE toward either leg with a flow goes via
`send_reinvite_via_flow`, which the UAC had and the handler never used;
the SBC's log proves both. Left for later: the site's link re-registers
on the trunk registration's own retry (up to 35 s after a refusal), and
a call in progress across the link when it is cut is not proved either
way.
Phase 7 done (v0.25.274): the site as a product. `deploy/docker-compose/site`
is a compose project of its own — the site's Postgres and Redis, its
own CA and master key from the cloud stack's script, the admin under
`[api] managed_by`, the call manager on the host's network on shifted
ports trusting the cloud's CA for the link trunk, and `fcp-link` in its
own image (the fifteenth) — configured from a `.env` that names the
cloud's three addresses and the token; `backup.sh`, `restore.sh` and
`wait-healthy.sh` take `COMPOSE_DIR` and the backup lists `link-data`,
so a site is kept as the cloud is. `docs/SITE_SETUP.md` is the site
operator's document (what to get from the cloud, the bring-up and its
log lines, phones, the site's admin and its refusal, life without the
cloud, the volumes, backup, restore, upgrade, decommissioning, faults);
the runbook's §9 is the cloud operator's. The admin UI has a Sites page
(made with the token shown once, new token, disable, enable, delete,
the nodes and registrations of each) and `site_id` on numbers and
trunks. The nightly's `stack` job makes a site for the live tenant,
brings the site stack up beside the cloud stack from the same images,
and `fcp-link/tests/compose_site.rs` proves it enrolled with the link
up and the call manager on it, a cloud-made user registering at the
site, the site bound at the cloud's SBC, the site's admin refusing a
write with the cloud's address, and nothing behind; then backs the site
up. **The hybrid control plane project is complete.** Left: a call
through the live site in the nightly, the site's own events, voicemail
and conference servers in the site project.

## What is next (chosen 2026-09-27, after the hybrid control plane closed)

**Enterprise calling** — design in `docs/ENTERPRISE_CALLING.md`
(v0.25.275), the second recommendation of the 2026-09-25 feature review:
the calling features every enterprise PBX has and FCP does not — shared
call appearance, do not disturb, call waiting, find-me/follow-me, music
on hold, class of service, schedules, executive-assistant and hoteling.
Three surveys of the code found the structural gap under them: there is
no per-user calling model. A call reaches a user by four paths that agree
on nothing (a number's binding honours only the no-answer forward and its
timeout; an extension to a user gets no forwarding and no voicemail; a
flow cannot dial a user; a hunt group's busy check reads DND), the B2BUA
passes every hold through and holds no held state, park hangs up the far
party and a parked caller hears silence, attended transfer fails because
`Replaces` is never read, audio into a leg is one-shot PCMU, presence is
one tenant deep, devices have no credentials of their own, nothing
restricts what a user dials and nothing knows the time. The model: one
`user_calling_settings` row per user, one ring plan (`plan_for_user`)
executed by every path, and the effective user of a device (its owner or
its hoteling guest) for everything outbound; a typed `CallTarget` for
every forward, schedule branch and find-me step; the held state of a leg
known to the B2BUA. Eleven phases from the foundations to three-party
audio; ten questions, each settled on its recommendation (per user not
per number, the dead `fcp-core` routing engine deleted, hold answered
locally when anchored, a synthesised default hold music, BroadSoft SCA,
per-node feature codes with North American defaults, the caller's number
with `Diversion` on find-me legs, class of service by number
classification not rules, appearances in Redis with retrieves forwarded
to the owning node, monitor/whisper/barge last).
Phase 1 done (v0.25.276): the foundations — `user_calling_settings`
(migration 082, one JSON row per user keyed by the user, journaled for
sites and applied by the link; the line binding's forwards and ring time
seeded into the owner's row, their columns unread from this release),
`fcp_core::calling` (`CallingSettings`, the typed `CallTarget` —
`voicemail`, `number:+…`, `ext:…`, `user:…`, `flow:…`, `queue:…`, a SIP
URI — `Treatment`, `CallingStore` in every store bundle),
`fcp_sip::ring_plan::plan` (forward all; do not disturb; nobody registered;
else ring for the user's time (the node's forking timeout when unset), then the busy or no-answer treatment: a
forward, the mailbox, or the callee's own code) executed by every path to
a user — a number's binding, an extension, the user's own address, a
flow's `Dial` (which rang nothing for a user before), and a hunt group's
members skipped in DND; the call carries `callee_user_id` and
`treatments`, and a forward is a new call into this node at its own port;
the presence listener follows each event's tenant instead of the default
tenant's; `is_busy` counts a member on the phone; the feature codes `*78`/
`*79` (DND), `*72<target>`/`*73` (forward all), `*90`/`*91` (busy),
`*92`/`*93` (no answer) under `[pbx.feature_codes]`; `user.calling_changed`;
`GET`/`PUT /v1/users/:username/calling` and `/v1/me/calling` (replacing
`/me/forwarding`), the Calling dialog on the users page and the user's
Calling page, the line binding page without the forwards it never applied;
the dead `fcp-core` routing engine deleted. Proved by
`fcp-call-manager/tests/calling.rs`: forward all without ringing, busy,
no answer after the ring time, DND as busy or as its forward, a user with
no phone reaching voicemail by extension with `reason=unreachable` and
the unreachable forward over the mailbox, a flow dialing a user who
answers and skipping one in DND, and the feature codes from a phone. A
flow or a queue as a forward target is refused by the API until the
schedules phase places it.
Phase 2 done (v0.25.277): hold, transfer and pickup in the B2BUA — the
held leg on the call (`fcp_sip::hold`, `call.held`/`call.resumed`, `held`
on `/calls`), an anchored hold answered `recvonly` from the holder's own
offer; the node carries out REFER under `[b2bua.transfer] execute`
(default on, `fcp_sip::transfer`): blind by dialing the target back into
itself as the transferred party, to a park code by parking, attended by
carrying `Replaces`; an INVITE with `Replaces` takes the named leg's slot
(`take_over_leg`); park moves the parked party to the A slot, `*70` works
from either leg, `*71` takes the empty slot, an expired orbit is a
transfer back to the parker through their plan; `call.parked` and
`call.unparked` published at last; in-dialog requests matched by Call-ID
and tags; a flow's dialed leg that answered as the caller hung up is now
ended. Proved by `fcp-call-manager/tests/calling.rs` (hold, blind,
attended, park and retrieve, call pull by `Replaces`, orbit timeout) and
`calls.rs`'s relayed transfer under `execute = false`. Hold stays passed
through until phase 3 has music to play; dialog-info's SIP ids and BLF
pickup by `Replaces` move to phase 9.
Phase 3 done (v0.25.278, forge-media v2026.09.27): music on hold — forge-media's
player now plays through the session's scheduled playout (the leg's own
codec and SRTP, looped on request, the other leg held back while a
`Replace` playback runs); `[media] hold_music` (default on) anchors every
call, and the node answers a hold and its resume itself; the music
follows `Call.held` through a call manager hook (`fcp_sip::hold_music`),
so a hold, a park, a retrieve and a transfer all start or stop it; it is
the holder's choice (a prompt, or `none`), else the tenant's hold-music
prompt, else a built-in arpeggio written to `prompts_dir` at startup; a
flow `Play` repeats `loop_count` times. Proved by `calling.rs` (the held
party hears the music and not the holder, the tenant's music and the
holder's silence, the parked party's music) and `calls.rs`'s
pass-through under `hold_music = false`. Not built: the number's and the
tenant's switches, the queue naming a prompt.
Phase 4 done (v0.25.279): call waiting — a user's calls counted as
leases in the cluster's Redis (`LeaseSubject::User`, the same counting
as the tenant and trunk limits; this node's own without a cluster),
kept by a call manager hook on the users a call counts against (its
caller and its callee, released when either leaves by transfer or park);
call waiting off with a call up is the busy treatment; `*56`/`*57` and
the Calling page's switch. Proved by `calling.rs` (a second call rings,
then is refused with call waiting off, a call Bob placed counting too)
and `trunks.rs` (Bob on a call through one node is busy through the
other).
Phase 5 done (v0.25.280): schedules — `fcp_core::schedule` (weekly
windows past midnight or all day, holidays, IANA zones), `schedules`
(migration 083, synced to sites), `/v1/schedules` and the Schedules page;
a number closed or on a holiday goes to its `closed_target` or
`holiday_target`, a user's after-hours forward applies in the plan, a
database queue's hours are decided before the queue, the flow `Schedule`
node branches; `flow:` and `queue:` targets are placed (a flow runs for
the call, a queue is dialed at its extension). Proved by `calling.rs`
(closed, open and holiday numbers; a closed number's flow; a user after
hours; a flow branching; a closed queue) and `api_postgres.rs`.
Phase 6 done (v0.25.281): find-me / follow-me — steps of targets
(`devices`, users, extensions, numbers) rung one after another after the
user's phones, or at once when none is up, each with its own ring time;
a `confirm` step's answerer hears a prompt alone and is connected only on
a 1, so a mobile's voicemail cannot take the call; `*60`/`*61`; the
Calling page's editor. Proved by `calling.rs` (the next step after the
phones, confirmation by 1 and refusal by 2, an unregistered user). The
`Diversion` header is not sent.
Phase 7 done (v0.25.282): class of service — numbers classified by the
tenant's country (US, CA, GB, DE, FR, AU, NZ; the tenant's `calling`,
migration 084) and its prefix overrides into internal, emergency,
toll-free, national, international, premium and unknown; classes
(`/v1/classes-of-service`, a default, synced to sites) name what a user
may dial, how many calls they may be on and whether their caller ID is
withheld; placed calls are refused with 403 and a `Reason`, a forward or
find-me step the forwarder may not dial is not followed, `max_calls`
applies both ways; a number's `block_anonymous`, allow and block lists
refuse with 433 or 603; the caller's line's `outbound_caller_id` leaves
on the trunk, or `Privacy: id` with an anonymous From. A schedule without
a timezone takes the tenant's. Proved by `calling.rs` (the call limit, a
refused forward, screening), `trunks.rs` (the line's caller ID, withheld,
a refused national call never reaching the carrier) and
`api_postgres.rs`.
Phase 8 done (v0.25.283): hoteling — a device may have credentials of
its own (sealed at rest, never returned, rendered by provisioning), and
one rule decides what a credential may register and whose `From` it may
carry: its own name, a user's devices, extension and lines, a device's
address, owner and lines — anything else is refused. A device marked
`hoteling_host` takes a login by `*88` (extension, then PIN, by DTMF, in
an inline flow) and a logout by `*89`; `hoteling_sessions` (migration
085) keep one guest per host, expire after the tenant's
`hoteling_session_hours` (default 12) by a sweep, and each start or end
sends the phone `check-sync` (and MWI at login); `hoteling.login` and
`hoteling.logout`. While a session lives the host rings for its guest and
not its owner, and its calls are the guest's for caller ID, class of
service, calling codes and `*98`; presence mirrors the guest's calls to
it. `/v1/hoteling/sessions`, `/v1/me/hoteling`, the Hoteling page, the
device dialog's owner, host and credentials. Proved by `calling.rs` and
`api_postgres.rs`. The CDR carries the host's `From` (it has no user);
MWI reaches the host at login only.
Phase 9 done (v0.25.284): shared call appearance, BroadSoft flavour — a
user's line may be `shared` with `max_appearances` (migration 086); its
devices register the number as an account of their own, rendered as each
device's second account by provisioning and the four starter templates.
Appearances live in the node's memory or, with a cluster, a Redis hash per
line claimed by one Lua script; phones watch them with `call-info`
(NOTIFYs inside the subscription with `Call-Info`) and claim one with
`line-seize`. A call to the line rings every phone at the lowest free
appearance (486 when full), a call from it takes the phone's seize or the
index it names, the lamps follow alerting, active, held and idle through
the call manager's appearance hook and `line.appearance_changed` across
nodes, and an INVITE naming a held appearance takes the line's leg — on
another node through `X-FCP-Node-Handoff`. siphon-rs gained
`IntegratedUAC::send_request` (sip-uac 0.7.6). Proved by `calling.rs`,
`trunks.rs` (two nodes), `line_appearances.rs` and `api_postgres.rs`.
Phase 10 done (v0.25.285): executive-assistant — an executive's calling
settings list their assistants (named by an admin, each opting in or out
for themself) and which calls they take (`all`, `external`, `none`) and
who rings (`assistants`, or with the executive). Such a call rings the
opted-in assistants' phones with a distinct `Alert-Info` and the caller
shown "for" the executive, the executive's ring time and no-answer
treatment kept; a call from an assistant or pushed by one rings the
executive. `*63` pushes the assistant's call to the executive (a
node-run transfer), `*64<number>` calls for the executive under their
line with the assistant's class and both users on the record, `*65`/`*66`
and `*67`/`*68` switch the executive's handling and the assistant's
opt-in. `/v1/me/executives`, the Calling dialog and page. Proved by
`calling.rs`, `trunks.rs` and `api_postgres.rs`.
Phase 11 done (v0.25.286): monitor, whisper and barge — forge-mixer's
`SupervisionMix` decides who hears whom, shared by forge-engine's new third
party on a session (forge-media v2026.09.28) and the queue's supervisor
bridge, which now mixes instead of interleaving packets. A supervisor is
dialed onto any anchored call through the call manager's
`/calls/:id/{monitor,whisper,barge}` (or the admin API's `/v1/calls/:id/…`
under `call_supervise`, forwarded to the node holding the call), hangs up
or is removed, and leaves with the call; `call.party_joined` and
`call.party_left`. A shared line's `barge_in` lets its phones join an
active appearance. The Active Calls page supervises. Proved by the engine,
mixer and queue-bridge unit tests, `calling.rs` and `api_postgres.rs`. The
enterprise calling project is complete: all eleven phases.

## What is next (chosen 2026-09-29, after the nightly stack closed)

**Developer surface** — design in `docs/DEVELOPER_SURFACE.md`
(v0.25.292), the third recommendation of the 2026-09-25 feature review:
what a developer outside the project meets first. Three surveys of the
code found:
- API keys a `read_only` user can mint with full tenant administration
  (permissions copied unchecked behind `require_admin` alone), permission
  names that match nothing, and no owner, revoke record, rotation or rate
  limit.
- An admin API of ~300 routes with no specification, two error shapes,
  five pagination forms and four timestamp forms, and clients (UI and CLI)
  copied by hand and drifting.
- Webhooks managed only on the events server behind its service token,
  with id routes that axum 0.7 never matches, caller-typed secrets, a
  signature without a timestamp and no SSRF guard.
- Flows that hear only digits: no speech recognition anywhere, forge-engine's
  media bridge never attached, voicemail transcription modelled and never
  produced, the INFO path's Gather reading variables nothing writes, and
  Postgres keeping only a flow's latest version.

The model:
- one contract generated from the code (`utoipa`, `docs/openapi.json`
  checked in CI);
- one shape for errors, times, ids and lists;
- keys as scoped credentials with an owner;
- webhooks owned by the tenant and signed over a timestamp;
- speech recognition with the same shape as TTS.

Eight phases, from keys made safe to live call control and the guide;
eleven questions (§5), agreed 2026-09-29 on their recommendations.

## Historical documents

Superseded status docs live in [`docs/archive/`](docs/archive/README.md) — they describe
Dec 2025–Jan 2026 state and must not be used for planning. `ARCHITECTURE-v0.2.md` remains
the architecture *vision*; its §12 roadmap checkboxes are not maintained — this file is.
