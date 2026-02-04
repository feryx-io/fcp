# FCP Production Readiness Audit & Live Integration Test Plan

## Part 1: Production Readiness Audit

### Component Readiness Matrix

| Component | Rating | Tests | Key Strength | Critical Gap |
|-----------|--------|-------|-------------|-------------|
| **SIP Stack (siphon-rs)** | | | | |
| Parser (sip-core/parse) | PRODUCTION | 1,082 | Fuzz-tested, comprehensive | None |
| Auth (sip-auth) | PRODUCTION | 33 | Constant-time, replay protection | None |
| Transport (sip-transport) | FUNCTIONAL | 40 | UDP/TCP/TLS with security hardening | SCTP stub, modest test count |
| Transactions (sip-transaction) | FUNCTIONAL | 37 | Transport-aware timer optimization | Lower test count |
| Dialog (sip-dialog) | FUNCTIONAL | 41 | Full state machine, PRACK, session timers | In-memory only |
| Registrar (sip-registrar) | FUNCTIONAL | 79 | Pluggable trait, strong validation | No persistent LocationStore |
| Proxy (sip-proxy) | FUNCTIONAL | 25 | Stateful forking, response selection | No loop detection |
| UAC (sip-uac) | FUNCTIONAL | 62 | Complete methods, auto-auth | None |
| UAS (sip-uas) | FUNCTIONAL | 28 | Dialog/subscription creation | Lower test count |
| SDP (sip-sdp) | FUNCTIONAL | 47 | Full RFC 4566, negotiate | None |
| DNS (sip-dns) | FUNCTIONAL | 61 | NAPTR/SRV/A/AAAA, ENUM | None |
| **FCP Core** | | | | |
| Identifiers | PRODUCTION | 14 types | Strong typing, serde | None |
| Permissions/RBAC | PRODUCTION | 5 | Exhaustive match, 28 perms, 6 roles | None |
| Phone Numbers | PRODUCTION | 11 | E.164, international, E911, CNAM | None |
| Devices | PRODUCTION | 7 | Full model, capabilities | None |
| Line Bindings | PRODUCTION | 6 | CUCM-style, 10 binding types | None |
| Porting | PRODUCTION | 8 | Full state machine | Store only stubbed |
| Storage Traits | FUNCTIONAL | 4 | 11 traits, DynStorage | No pagination |
| Config | FUNCTIONAL | 2 | Serde-based | None |
| Tenants | FUNCTIONAL | 3 | Limits, lifecycle | No limit enforcement |
| Voicemail Types | FUNCTIONAL | 4 | Mailbox, messages, MWI | Only 1 backend |
| **Storage Backends** | | | | |
| SQLite | FUNCTIONAL | 6 | 12/12 traits (3 stubs), migrations | TenantStore/GeoRouting/Porting stubbed |
| PostgreSQL | STRUCTURAL | 0 (ignored) | 8/11 traits, best TenantStore | Missing Voicemail/GeoRouting/Porting |
| MySQL | **BROKEN** | 0 (ignored) | 8/11 traits | **$1 placeholders, reversed binds, no-op updates** |
| MongoDB | STRUCTURAL | 0 (ignored) | 8/11 traits | Missing Voicemail/GeoRouting/Porting |
| **API & Server** | | | | |
| fcp-api-core | FUNCTIONAL | Few | 60+ real handlers, full CRUD | hangup_call missing BYE, no JWT |
| fcp-server | FUNCTIONAL | — | All-in-one, real transport+storage+API | Monolithic only |
| fcp-call-manager | FUNCTIONAL | — | Most complete binary, registrar+B2BUA+trunks | None critical |
| fcp-registrar | FUNCTIONAL | — | Real UDP+TCP, persistent storage, cleanup | Single-purpose |
| fcp-admin | FUNCTIONAL | — | REST API without SIP overhead | None |
| fcp-api-gateway | FUNCTIONAL | — | Reverse proxy, Redis cache, rate limit | Global-only rate limit |
| fcp-events-server | FUNCTIONAL | — | WebSocket, tenant isolation, batching | In-memory bus only |
| fcp-jobs | STRUCTURAL | — | Redis queue, CDR processing real | 4/5 job handlers stubbed |
| **B2BUA & Media** | | | | |
| fcp-b2bua-core | FUNCTIONAL | Yes | Full call lifecycle, CDR, events | None critical |
| fcp-media (embedded) | FUNCTIONAL | — | Real RTP via forge-media | TTS providers stubbed |
| fcp-media (remote) | STRUCTURAL | — | — | 11 todo!() placeholders |
| fcp-sip handler | FUNCTIONAL | — | 9000-line complete B2BUA flow | 8 Phase 2 TODOs |
| **PBX Features** | | | | |
| Parking | PRODUCTION | Yes | DashMap, timeouts, auto-retrieval | None |
| Pickup | PRODUCTION | Yes | Group and directed | None |
| Intercom | PRODUCTION | Yes | Auto-answer headers | None |
| Hunt Groups | PRODUCTION | Yes | All ring strategies | None |
| Feature Codes | PRODUCTION | Yes | Star-code parsing | None |
| CMC/FAC | PRODUCTION | Yes | Code validation | None |
| **Voicemail & IVR** | | | | |
| fcp-voicemail | FUNCTIONAL | — | Metadata, recording orchestration | play_audio stubbed, MWI stubbed |
| fcp-voicemail-server | PRODUCTION | — | Fully wired binary | Inherits voicemail gaps |
| fcp-ivr | PRODUCTION | — | Fully wired binary | Depends on flow engine |
| fcp-flow engine | FUNCTIONAL | — | 13/15 nodes real, DTMF, TTS, branching | Recording placeholder, Fork not parallel |
| fcp-flow schema | PRODUCTION | — | 15 node types, full serde | None |
| **SBC & Routing** | | | | |
| fcp-sbc | PRODUCTION | Yes | Enforcement, SIPREC, HMR, STIR/SHAKEN | None critical |
| fcp-sbc-core | PRODUCTION | Yes | LCR, admission control, trunk health | None critical |
| fcp-routing | PRODUCTION | Yes | Policy engine, YAML, security hardened | None critical |

### Critical Issues (Fix Before Testing)

1. **MySQL backend is broken** — PostgreSQL-style `$1` placeholders, reversed bind order, no-op `ON DUPLICATE KEY UPDATE`. Do not use for testing.
2. **hangup_call API doesn't send SIP BYE** — `media.rs:233` marks call terminated but doesn't signal the SIP layer. Will cause ghost calls.
3. **No persistent registration store** — sip-registrar uses in-memory only. FCP's fcp-registrar wraps it with persistent storage, so use fcp-registrar or fcp-call-manager for testing, not raw siphond.

---

## Part 2: Live Integration Test Plan

### Infrastructure

| Resource | Purpose |
|----------|---------|
| **Twilio SIP Trunk** | PSTN origination/termination, inbound DID routing |
| **Signalwire** | Paging, mass notification, intercom testing |
| **FCP Server** (fcp-call-manager) | Primary SUT — registrar + B2BUA + trunks + PBX |
| **Softphones** (2+) | Obi200 / Obi2182 / Obi1032/1062 or Obi504/508 registered to FCP |
| **Physical IP Phones** (2+) | Cisco/Yealink/Polycom registered to FCP |
| **Network** | FCP on LAN with port forwarding or public IP for Twilio trunk |

### Pre-Requisites

Before any live testing:

```bash
# 1. Build release
cargo build --release -p fcp-call-manager

# 2. Verify it starts and binds
./target/release/fcp-call-manager --help

# 3. Start with SQLite (simplest)
./target/release/fcp-call-manager \
  --bind 0.0.0.0:5060 \
  --storage-type sqlite \
  --storage-url sqlite:fcp-test.db \
  --api-bind 0.0.0.0:8080 \
  --log-level debug
```

---

### Phase A: Registration & Basic Connectivity

**Goal**: Verify devices can register and FCP tracks them.

#### A1. Softphone Registration (UDP)

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure softphone with: server=FCP_IP:5060, username=alice, password=test123 | — |
| 2 | Create user via API: `POST /api/v1/users` with username=alice, password=test123 | 201 Created |
| 3 | Register softphone | 200 OK response, Contact with expires |
| 4 | Check registrations: `GET /api/v1/registrations?aor=sip:alice@FCP_IP` | Registration visible |
| 5 | Check TUI registrations tab | Registration appears with countdown |
| 6 | Wait for re-registration | Timer resets, no gaps |
| 7 | Unregister softphone (close app) | Registration expires or removed |

#### A2. Physical Phone Registration (TCP)

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure IP phone with: outbound proxy=FCP_IP:5060, TCP transport | — |
| 2 | Create user via API: username=bob, password=test456 | 201 Created |
| 3 | Phone registers over TCP | 200 OK, TCP connection pooled |
| 4 | Verify in TUI | Shows TCP transport indicator |

#### A3. Multi-Device Registration

| Step | Action | Expected |
|------|--------|----------|
| 1 | Register alice on two devices (softphone + physical phone) | Both contacts visible |
| 2 | `GET /api/v1/registrations?aor=sip:alice@FCP_IP` | Two bindings, different q-values |
| 3 | Check line binding with both devices | Ring strategy honored |

#### A4. Digest Auth Verification

| Step | Action | Expected |
|------|--------|----------|
| 1 | Register with wrong password | 401 Unauthorized |
| 2 | Set separate digest password: `PUT /api/v1/users/alice/digest-password` | 200 OK |
| 3 | Register with main password | 401 (main password no longer works for SIP) |
| 4 | Register with digest password | 200 OK |
| 5 | Remove digest password: `DELETE /api/v1/users/alice/digest-password` | 200 OK |
| 6 | Register with main password again | 200 OK (reverted) |

---

### Phase B: Internal Calls (Extension-to-Extension)

**Goal**: Verify B2BUA call routing between registered devices.

#### B1. Basic Two-Party Call

| Step | Action | Expected |
|------|--------|----------|
| 1 | alice dials bob's extension | INVITE routed via B2BUA |
| 2 | bob's phone rings | 180 Ringing relayed to alice |
| 3 | bob answers | 200 OK, SDP negotiated, media bridged |
| 4 | Verify two-way audio | Both parties hear each other |
| 5 | Check TUI calls tab | Call visible with "established" state, duration counting |
| 6 | `GET /api/v1/calls` | Call details with from/to URIs |
| 7 | alice hangs up | BYE sent on both legs |
| 8 | `GET /api/v1/cdrs` | CDR created with duration, hangup cause |

#### B2. Call Rejection / No Answer

| Step | Action | Expected |
|------|--------|----------|
| 1 | alice calls bob, bob rejects (486 Busy) | alice hears busy, CDR with "busy" status |
| 2 | alice calls bob, bob doesn't answer (wait 30s) | No-answer timeout, CDR with "no_answer" |
| 3 | alice calls bob, alice cancels before answer | CANCEL sent, 487 on B-leg, CDR with "cancelled" |

#### B3. Codec Negotiation

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure alice for G.711u only, bob for G.711a only | — |
| 2 | alice calls bob | SDP negotiation succeeds (B2BUA transcodes or selects common) |
| 3 | Verify audio quality | Clear audio both directions |
| 4 | Check SDP in debug view | Codecs properly negotiated |

#### B4. Call Hold / Resume

| Step | Action | Expected |
|------|--------|----------|
| 1 | Establish call alice ↔ bob | Call active |
| 2 | alice puts call on hold | re-INVITE with sendonly/inactive SDP |
| 3 | bob hears hold music (or silence) | Media updated |
| 4 | alice resumes | re-INVITE with sendrecv SDP |
| 5 | Two-way audio restored | Media flowing again |

---

### Phase C: PBX Features

**Goal**: Test star codes, parking, pickup, hunt groups.

#### C1. Feature Codes

| Step | Action | Expected |
|------|--------|----------|
| 1 | alice dials `*70` (DND toggle) | Feature code recognized, DND toggled |
| 2 | bob calls alice | Call rejected (DND active) or sent to voicemail |
| 3 | alice dials `*70` again | DND off |
| 4 | bob calls alice | Normal ring |

#### C2. Call Parking

| Step | Action | Expected |
|------|--------|----------|
| 1 | Establish call alice ↔ bob | Call active |
| 2 | alice dials `*80` (park) | Call parked, alice hears orbit number |
| 3 | Check parking state via debug API | Parked call visible with orbit |
| 4 | charlie dials `*81{orbit}` (retrieve) | Call connected charlie ↔ bob |
| 5 | Wait for parking timeout (if no retrieval) | Call returns to parker or rings all |

#### C3. Call Pickup

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure pickup group with alice + bob | — |
| 2 | External call comes in to alice | alice's phone rings |
| 3 | bob dials `*8` (group pickup) | bob answers alice's ringing call |
| 4 | Verify audio between external caller and bob | Clear two-way audio |

#### C4. Hunt Groups

| Step | Action | Expected |
|------|--------|----------|
| 1 | Create hunt group with alice + bob, strategy=simultaneous | — |
| 2 | Call the hunt group extension | Both phones ring simultaneously |
| 3 | alice answers first | bob stops ringing, call established |
| 4 | Repeat with strategy=sequential | alice rings first, then bob after timeout |
| 5 | Repeat with strategy=round_robin | Different first-ring phone each call |

#### C5. Intercom

| Step | Action | Expected |
|------|--------|----------|
| 1 | alice dials intercom prefix + bob's extension | — |
| 2 | bob's phone auto-answers (Alert-Info header) | Speaker activates |
| 3 | alice speaks, bob hears immediately | One-way or two-way audio per config |

---

### Phase D: Twilio SIP Trunk Integration

**Goal**: Connect FCP to PSTN via Twilio trunk for inbound/outbound calls.

#### D1. Trunk Configuration

| Step | Action | Expected |
|------|--------|----------|
| 1 | Create Twilio SIP trunk in Twilio console | Trunk SID assigned |
| 2 | Configure Twilio origination URI: `sip:FCP_PUBLIC_IP:5060` | — |
| 3 | Configure Twilio termination URI: `FCP_TRUNK.pstn.twilio.com` | — |
| 4 | Configure Twilio credentials (IP ACL or digest auth) | — |
| 5 | Add trunk config to FCP (config file or API) | — |
| 6 | Start FCP with trunk config | Trunk registration (if digest auth) |
| 7 | Check trunk health in debug/metrics | Trunk shows as "up" |

#### D2. Outbound PSTN Call

| Step | Action | Expected |
|------|--------|----------|
| 1 | alice dials external number (e.g., +1-555-xxx-xxxx) | — |
| 2 | FCP routes to Twilio trunk (LCR or prefix match) | INVITE sent to Twilio |
| 3 | External phone rings | 180 Ringing relayed |
| 4 | External answers | 200 OK, media bridged |
| 5 | Verify two-way audio | Clear audio both directions |
| 6 | Check CDR | Trunk identified, caller ID correct |
| 7 | Hangup | BYE, CDR with duration |

#### D3. Inbound PSTN Call (DID Routing)

| Step | Action | Expected |
|------|--------|----------|
| 1 | Assign Twilio DID to FCP | DID routes to FCP origination URI |
| 2 | Create phone number in FCP: `POST /api/v1/phone-numbers` with DID's E.164 | 201 Created |
| 3 | Create line binding mapping DID to alice's device | — |
| 4 | Call the DID from external phone | INVITE arrives at FCP from Twilio |
| 5 | alice's phone rings | 180 Ringing sent back to Twilio |
| 6 | alice answers | 200 OK, media bridged |
| 7 | Verify two-way audio | Clear audio both directions |
| 8 | Check caller ID on alice's phone | External caller's number displayed |
| 9 | Check CDR | Inbound call logged with DID, caller |

#### D4. Inbound → Hunt Group

| Step | Action | Expected |
|------|--------|----------|
| 1 | Map DID line binding to hunt group | — |
| 2 | Call DID from external | Hunt group rings per strategy |
| 3 | First available answers | Call established |
| 4 | CDR shows hunt group routing | — |

#### D5. Inbound → No Answer → Voicemail

| Step | Action | Expected |
|------|--------|----------|
| 1 | Map DID to alice with call forwarding: no_answer → voicemail | — |
| 2 | Call DID from external, alice doesn't answer | — |
| 3 | After timeout, call forwards to voicemail | Greeting plays |
| 4 | Caller leaves message | Recording saved |
| 5 | Check voicemail: `GET /api/v1/voicemail/mailboxes/alice/messages` | New message visible |
| 6 | MWI notification sent to alice's phone | Message waiting indicator lights up |

#### D6. Caller ID / CNAM

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure CNAM on alice's phone number | — |
| 2 | alice calls external number | Twilio sends configured CNAM |
| 3 | External phone shows CNAM | Caller name displayed |

#### D7. E911 (if Twilio supports)

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure E911 address on phone number | — |
| 2 | Verify E911 callback number is set | — |
| 3 | (Do NOT actually dial 911 — verify config only) | E911 address in API response |

---

### Phase E: IVR & Call Flows

**Goal**: Test programmable call flows with DTMF, branching, and API integration.

#### E1. Simple Auto-Attendant

| Step | Action | Expected |
|------|--------|----------|
| 1 | Create flow: Answer → Play("Press 1 for sales, 2 for support") → Gather(1 digit) → Branch → Dial | — |
| 2 | Upload flow: `POST /api/v1/flows` | 201 Created |
| 3 | Map DID to flow via line binding (type=ivr) | — |
| 4 | Call DID from external | Greeting plays |
| 5 | Press 1 | Call transfers to sales extension |
| 6 | Press 2 | Call transfers to support extension |
| 7 | No input (timeout) | Replay or default action |

#### E2. Multi-Level IVR

| Step | Action | Expected |
|------|--------|----------|
| 1 | Create nested flow: main menu → submenu → dial | — |
| 2 | Call and navigate: press 1, then press 2 | Correct routing through menus |
| 3 | Press * to go back | Returns to previous menu |

#### E3. TTS (Text-to-Speech)

| Step | Action | Expected |
|------|--------|----------|
| 1 | Create flow with Say node (TTS) | — |
| 2 | Call the flow | TTS audio plays (if TTS provider configured) |
| 3 | If no TTS provider: verify graceful fallback | Error logged, call continues |

#### E4. HTTP Webhook Node

| Step | Action | Expected |
|------|--------|----------|
| 1 | Create flow with Http node calling external webhook | — |
| 2 | Call the flow | HTTP request made, response used for routing |
| 3 | Check webhook received correct call data | caller, callee, call_id in payload |

---

### Phase F: SBC Features

**Goal**: Test SBC enforcement, NAT, topology hiding, STIR/SHAKEN.

#### F1. SBC Enforcement Pipeline

| Step | Action | Expected |
|------|--------|----------|
| 1 | Start fcp-sbc with IP ACL allowing only Twilio IPs | — |
| 2 | Send INVITE from allowed IP | Passes enforcement |
| 3 | Send INVITE from disallowed IP | 403 Forbidden |
| 4 | Send oversized SIP message | Rejected by size limit |
| 5 | Send rapid-fire INVITEs | Rate limited (429 or 503) |

#### F2. Header Manipulation Rules (HMR)

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure HMR: add X-Custom-Header on egress | — |
| 2 | Place outbound call through SBC | Header present in outbound INVITE |
| 3 | Configure HMR: strip P-Asserted-Identity on ingress | — |
| 4 | Receive inbound call | PAI header removed |

#### F3. STIR/SHAKEN (if certificates available)

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure STIR signing with test certificate | — |
| 2 | Place outbound call | Identity header added to INVITE |
| 3 | Configure STIR verification | — |
| 4 | Receive inbound call with Identity header | Signature verified, attestation logged |
| 5 | Receive call with invalid/expired Identity | 438 Invalid Identity Header |

#### F4. SIPREC Recording

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure SIPREC recording server | — |
| 2 | Configure selective recording rules (e.g., all external calls) | — |
| 3 | Place call matching recording rule | SIPREC INVITE sent to recording server |
| 4 | Verify recording metadata (RFC 7865 XML) | Correct parties, codecs, direction |
| 5 | Hangup call | Recording session ended |

---

### Phase G: Signalwire Integration

**Goal**: Test paging and mass notification via Signalwire.

#### G1. SIP Paging

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure Signalwire SIP trunk to FCP | — |
| 2 | Configure paging zone in Signalwire | — |
| 3 | Trigger page from Signalwire | SIP INVITE with auto-answer to FCP phones |
| 4 | Phones auto-answer and play audio | Paging audio heard on all zone phones |

#### G2. Emergency Notification

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure emergency notification in Signalwire | — |
| 2 | Trigger notification | All phones in zone receive notification |
| 3 | Verify audio and visual indicators | Phones display and play notification |

---

### Phase H: Monitoring & Observability

**Goal**: Verify metrics, logging, and debugging tools work during live calls.

#### H1. Prometheus Metrics

| Step | Action | Expected |
|------|--------|----------|
| 1 | `GET /metrics` | Prometheus metrics returned |
| 2 | Place several calls | Call counters increment |
| 3 | Check registration metrics | Active registration count accurate |
| 4 | Import Grafana dashboards from deploy/grafana/ | Dashboards render |

#### H2. TUI Dashboard During Load

| Step | Action | Expected |
|------|--------|----------|
| 1 | Start TUI: `fcp-cli tui` | Dashboard loads |
| 2 | Navigate all tabs during active calls | No crashes, data refreshes |
| 3 | Watch calls tab during call setup/teardown | Real-time state transitions |
| 4 | Watch registrations tab | Expiry countdowns accurate |

#### H3. Debug API

| Step | Action | Expected |
|------|--------|----------|
| 1 | `GET /api/v1/debug/summary` | Active calls, registrations, dialogs |
| 2 | `GET /api/v1/debug/dialogs` | Dialog state for active calls |
| 3 | `GET /api/v1/debug/rate-limits` | Per-IP rate limit state |

#### H4. CDR Verification

| Step | Action | Expected |
|------|--------|----------|
| 1 | Place 10 calls of various types | — |
| 2 | `GET /api/v1/cdrs?limit=10` | All 10 CDRs present |
| 3 | Verify CDR fields: duration, caller, callee, hangup_cause | Accurate values |
| 4 | Filter CDRs: `GET /api/v1/cdrs?caller=alice` | Correct filtering |

---

### Phase I: Stress & Reliability

**Goal**: Verify stability under load and failure conditions.

#### I1. Concurrent Calls

| Step | Action | Expected |
|------|--------|----------|
| 1 | Use SIPp or similar to generate 10 concurrent calls | — |
| 2 | All calls establish with audio | No call failures |
| 3 | Monitor memory and CPU | Stable, no leaks |
| 4 | Ramp to 50 concurrent calls | System remains responsive |

#### I2. Registration Storm

| Step | Action | Expected |
|------|--------|----------|
| 1 | Register 100 endpoints simultaneously | — |
| 2 | All registrations succeed | 200 OK for all |
| 3 | Re-registration cycle | No failed re-registrations |

#### I3. Graceful Restart

| Step | Action | Expected |
|------|--------|----------|
| 1 | Establish several active calls | — |
| 2 | Send SIGTERM to fcp-call-manager | — |
| 3 | Verify graceful shutdown (trunk de-registration) | BYE sent, trunks de-registered |
| 4 | Restart fcp-call-manager | — |
| 5 | Verify endpoints re-register | Registrations restored from persistent storage |

#### I4. Trunk Failover

| Step | Action | Expected |
|------|--------|----------|
| 1 | Configure two trunks (primary + backup) | — |
| 2 | Place call via primary trunk | Call succeeds |
| 3 | Simulate primary trunk failure (block IP) | — |
| 4 | Place call | Routes to backup trunk, call succeeds |
| 5 | Restore primary trunk | Subsequent calls use primary again |

---

## Part 3: Test Execution Order

### Week 1: Foundation
- [ ] Phase A: Registration (softphone, physical phone, multi-device, digest auth)
- [ ] Phase B: Internal calls (basic, reject/no-answer, hold/resume)
- [ ] Phase H1-H3: Verify monitoring works

### Week 2: PBX & PSTN
- [ ] Phase C: PBX features (feature codes, parking, pickup, hunt groups, intercom)
- [ ] Phase D1-D3: Twilio trunk setup, outbound PSTN, inbound DID

### Week 3: Advanced Routing & IVR
- [ ] Phase D4-D6: Hunt group routing, voicemail, caller ID
- [ ] Phase E: IVR flows (auto-attendant, multi-level, TTS, webhooks)

### Week 4: SBC & Integration
- [ ] Phase F: SBC features (enforcement, HMR, STIR/SHAKEN, SIPREC)
- [ ] Phase G: Signalwire integration
- [ ] Phase H4: CDR verification
- [ ] Phase I: Stress testing

---

## Part 4: Known Issues to Fix Before Testing

### Must Fix (blocking)

1. **hangup_call API doesn't send SIP BYE** (`fcp-api-core/src/v1/media.rs:233`)
   - Impact: API-initiated hangups leave ghost calls
   - Fix: Call `handler.send_bye()` on both legs before marking terminated

2. **Voicemail play_audio is stubbed** (`fcp-voicemail`)
   - Impact: Voicemail greeting won't play to callers
   - Fix: Integrate with MediaManager for audio playback

3. **MWI (Message Waiting Indicator) is stubbed** (`fcp-voicemail`)
   - Impact: Phones won't show voicemail notification light
   - Fix: Send SIP NOTIFY with message-summary event

### Should Fix (quality)

4. **Flow engine Fork node not parallel** (`fcp-flow`)
   - Impact: Fork node continues sequentially instead of parallel dialing
   - Fix: Spawn concurrent tasks for fork branches

5. **Recording node placeholder URL** (`fcp-flow`)
   - Impact: StartRecording in flows doesn't actually record
   - Fix: Integrate with media recording backend

6. **TTS providers are stubs** (`fcp-media`)
   - Impact: Say nodes in IVR won't produce speech
   - Fix: Implement at least one provider (e.g., ElevenLabs or local espeak)

### Can Defer

7. **MySQL backend broken** — Use SQLite or PostgreSQL instead
8. **No JWT/OIDC auth** — Bearer token sufficient for testing
9. **Global-only rate limiting in gateway** — Acceptable for testing
10. **No persistent event bus** — In-memory sufficient for single-node testing
