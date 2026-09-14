# FCP Implementation Documentation

This document provides implementation details for all major features in the Ferrous Communications Platform. It serves as a reference for understanding how each subsystem works, what it depends on, and where to find the relevant code.

---

## Table of Contents

1. [B2BUA Call Control](#1-b2bua-call-control)
2. [Call Detail Records (CDR)](#2-call-detail-records)
3. [Call Origination API](#3-call-origination-api)
4. [Call Parking](#4-call-parking)
5. [PBX Feature Library](#5-pbx-feature-library)
6. [Session Border Controller (SBC)](#6-session-border-controller)
7. [Routing Engine](#7-routing-engine)
8. [Media Pipeline](#8-media-pipeline)
9. [Flow Engine & IVR](#9-flow-engine--ivr)
10. [Voicemail System](#10-voicemail-system)
11. [Event System](#11-event-system)
12. [REST API](#12-rest-api)
13. [Storage Backends](#13-storage-backends)
14. [Standalone Binaries](#14-standalone-binaries)
15. [Webhooks](#15-webhooks)
16. [Presence System](#16-presence-system)

---

## 1. B2BUA Call Control

**Crate:** `fcp-b2bua-core`
**Path:** `crates/fcp-b2bua-core/src/`

### Overview

The Back-to-Back User Agent is the core call engine. It maintains independent A-leg and B-leg SIP dialogs, bridging them at the signaling layer while delegating media decisions to `fcp-media`.

### Architecture

```
Caller (A-leg) <--SIP--> B2BUA <--SIP--> Callee (B-leg)
                          |
                    CallManager
                     (state machine)
```

### Key Components

| File | Purpose |
|------|---------|
| `call.rs` | `Call` struct with `CallState` machine (`Initiated` -> `Ringing` -> `Established` -> `Parked` / `Terminated` / `Failed`). Each call has A-leg and optional B-leg `CallLeg` structs. |
| `call_manager.rs` | `CallManager` — concurrent call store using `DashMap`. Handles call creation, state transitions, CDR generation on termination, and parking expiry monitoring. |
| `forking.rs` | `ForkingMode` (Serial, Parallel, Hunt) and `EarlyMediaPolicy` for multi-destination dialing. |
| `media_anchoring.rs` | `MediaAnchoringMode` (Always, Never, AsNeeded) — determines whether media flows through the B2BUA or directly between endpoints. |
| `topology.rs` | `TopologyHidingMode` — controls header stripping and URI anonymization. |
| `transfer.rs` | `TransferType` (Blind, Attended, Semi-Attended) with `TransferPolicy` for REFER handling. |

### Call Lifecycle

1. **Inbound INVITE** arrives at `SipMessageHandler`
2. Handler creates `Call::from_invite()` with A-leg populated
3. Routing resolves the destination (extension lookup, LCR, flow)
4. Outbound INVITE sent via UAC, creating B-leg
5. 200 OK from callee triggers `CallState::Established`
6. BYE from either party triggers `CallState::Terminated` and CDR generation

### Call State Machine

```
Initiated -> Ringing -> Established -> Terminated
                |            |
                v            v
              Failed       Parked -> (expiry) -> Terminated
```

---

## 2. Call Detail Records

**Crate:** `fcp-b2bua-core` (generation), `fcp-core` (types), `fcp-jobs` (async processing)
**Path:** `crates/fcp-b2bua-core/src/call_manager.rs`, `crates/fcp-core/src/storage.rs`

### Overview

Every finished call produces a call detail record, which travels a
pipeline from the node that owned the call to the database. The design and
its phases are in [docs/CDR_PIPELINE.md](docs/CDR_PIPELINE.md); this is
what exists.

### On the node

1. The handler ends the call with a `TerminationReason` (hung up by the
   caller or the callee, cancelled, rejected with a status, no answer, a
   platform decision, a setup failure), after taking the media session's
   RTP counters.
2. `CallManager::terminate_call_with` builds the record with
   `fcp_cdr::RecordBuilder`: disposition, hangup cause, SIP status and
   hangup party from the reason; durations; codecs parsed from each leg's
   SDP; trunk, node and region; the call it came from (a transfer, pickup
   or retrieve); a fresh `cdr_id`. The queue handler writes a `queue`
   record under the same call id, the conference server a `conference`
   record per participant leg (under the call manager's or gateway's call
   id, from `X-FCP-Call-Id`), and the WebRTC gateway a `gateway` record per
   browser session (through the stream only).
3. The record is appended to the node's spool (`[cdr] spool_dir`), a
   directory of JSON Lines segments synced every 100 ms, and a shipper
   drains the spool to the sink: the in-process processor
   (`sink = "local"`, the default) or the Redis stream `fcp:cdrs`
   (`sink = "stream"`). A database or Redis outage delays records; a
   crash loses nothing that reached the page cache.

### The processor

In-process on the node, or in `fcp-jobs` with `[cdr] enabled = true`
consuming the stream through the consumer group `cdr-processor`:

1. **Normalize**: validate, fill the durations and the end time.
2. **Enrich**: the call's direction, from the tenant's numbers.
3. **Rate**: the tenant's default plan (cached for a minute in
   `fcp-jobs`), stamping cost, currency, plan, prefix and billable
   seconds. Money is exact decimal (`NUMERIC(18,6)` in the database).
4. **Store**: the batch in one transaction, a replayed `cdr_id` skipped.
   A record the database refuses is kept as a dead letter
   (`fcp:cdrs:dead`, or the spool's `dead.jsonl`).
5. **Settle**: one ledger charge per call, however often a record is
   replayed.
6. **Announce**: `cdr.written` on the event bus, for webhooks and the
   WebSocket stream.

Status and metrics: the call manager's `/cdr/status` and `/metrics`,
`fcp-jobs`' `/status` (under `cdr`) and `/metrics`.

### CDR Fields

`fcp_core::storage::CallDetailRecord` (`fcp_cdr::Record`):

- **Identity**: `cdr_id`, `kind` (`call`, `queue`, `conference`, `gateway`), `schema_version`, `call_id`, `tenant_id`, `sip_call_id`, `correlation_id`
- **Parties**: `from_uri`, `to_uri`, display names, `direction`
- **Timestamps**: `start_time`, `answer_time`, `end_time`, `created_at`
- **Durations**: `setup_duration_ms`, `call_duration_ms`, `total_duration_ms`
- **Disposition**: `disposition` (answered, busy, no_answer, failed, cancelled), `hangup_cause`, `hangup_party`
- **SIP**: `sip_status_code`, `sip_reason`
- **Where**: `node_id`, `region`, `trunk_name`, `gateway_id`, `route_id`
- **Media**: `media_anchored`, codecs, RTP packets and bytes each way, loss and jitter, `recording_url`
- **Billing**: `cost`, `currency`, `rate_plan_id`, `matched_prefix`, `billable_seconds`, `rated_at`
- **Custom**: `custom_data` (the queue record's queue, agent, wait and handle times)

### REST API

- `GET /v1/cdrs` — Query CDRs with filters (tenant, time range, disposition); `?kind=` for `queue`, `conference`, `gateway` or `all` (default `call`, or every kind with `call_id`); pages with `cursor` (the previous page's `next_cursor`) or `offset` up to 10,000; `total` is exact when counting takes under two seconds, else an estimate (`total_exact`)
- `GET /v1/cdrs/export` — Every matching record, streamed, as CSV or `format=jsonl`
- `GET /v1/cdrs/:call_id` — Get a call's record
- `GET /v1/cdrs/export` — CSV export

---

## 3. Call Origination API

**Crate:** `fcp-api-core` (endpoint), `fcp-sip` (implementation)
**Path:** `crates/fcp-api-core/src/v1/media.rs`, `crates/fcp-sip/src/handler.rs`

### Overview

Click-to-call REST endpoint that originates a new call between two parties. The API returns immediately with a call ID; the actual call setup runs asynchronously.

### Flow

```
POST /v1/calls { from: "1001", to: "1002" }
        |
        v
  resolve_contact(from) -> sip:1001@host -> registered contact
  resolve_contact(to)   -> sip:1002@host -> registered contact
        |
        v
  Create Call::new_originated() in CallManager
        |
        v
  Spawn async run_origination() task
        |
        v
  Return { call_id, status: "initiated" }
```

### Origination Task (`run_origination`)

1. INVITE `from` party (A-leg) with generated SDP, 30-second timeout
2. Wait for 200 OK; on failure/timeout, CANCEL and mark call failed
3. Store A-leg dialog in `DialogManager`
4. INVITE `to` party (B-leg) with generated SDP, 30-second timeout
5. Wait for 200 OK; on failure/timeout, BYE A-leg and mark call failed
6. Store B-leg dialog, transition call to `Established`

### Contact Resolution

`resolve_contact()` handles both extensions and full SIP URIs:
- Extensions (e.g., `"1001"`) are converted to `sip:1001@{host}` and looked up in the registrar
- Full SIP URIs (e.g., `sip:user@domain`) are used directly

---

## 4. Call Parking

**Crate:** `fcp-pbx` (orbit management), `fcp-sip` (SIP operations), `fcp-b2bua-core` (call state)
**Path:** `crates/fcp-pbx/src/parking.rs`, `crates/fcp-sip/src/handler.rs`

### Overview

Call parking allows a user to place an active call on hold in a numbered orbit, which any user can then retrieve by dialing the orbit number.

### Park Flow (`*70`)

1. User dials `*70` feature code
2. Handler finds user's active established call via `CallManager::find_established_by_caller()`
3. `PbxHandler::park_call()` assigns an orbit number and starts the expiry timer
4. B-leg is BYE'd; A-leg receives re-INVITE with hold SDP (`a=sendonly`)
5. Call state transitions to `Parked` with `parked_orbit` and `parker_extension` fields
6. `CallEvent::Parked` is published

### Retrieve Flow (`*71{orbit}`)

1. User dials `*71` + orbit number (e.g., `*7101`)
2. `PbxHandler::retrieve_parked_call()` returns the parked call info
3. A-leg receives re-INVITE with unhold SDP (`a=sendrecv`)
4. New outbound INVITE sent to retriever's registered contact (new B-leg)
5. Call state transitions back to `Established`
6. `CallEvent::Unparked` is published

### Expiry

Parked calls have a configurable timeout (default 60 seconds). When expired:
1. The expiry monitor in `fcp-call-manager` detects the timeout
2. `route_expired_parked_call()` looks up the parker's registered contact
3. A-leg is taken off hold and a new B-leg INVITE is sent to the parker
4. If the parker is unreachable, the call is terminated

### SDP Hold/Unhold

- `generate_hold_sdp()` — replaces `a=sendrecv` with `a=sendonly`
- `generate_unhold_sdp()` — replaces `a=sendonly` with `a=sendrecv`

---

## 5. PBX Feature Library

**Crate:** `fcp-pbx`
**Path:** `crates/fcp-pbx/src/`

### Overview

Pure-logic PBX feature library that provides decision-making without directly manipulating SIP messages. The SIP handler calls into `PbxHandler` to resolve feature code actions and then performs the corresponding SIP operations.

### Features

| Feature | Code | Module | Description |
|---------|------|--------|-------------|
| Call Parking | `*70` | `parking.rs` | Park active call to an orbit |
| Call Retrieve | `*71{orbit}` | `parking.rs` | Retrieve parked call from orbit |
| Group Pickup | `*8` | `pickup.rs` | Pick up ringing call in same pickup group |
| Directed Pickup | `*8{ext}` | `pickup.rs` | Pick up ringing call at specific extension |
| Intercom | `*80{ext}` | `intercom.rs` | Auto-answer intercom to extension |
| CMC | (configurable) | `codes.rs` | Client Matter Code — billing classification |
| FAC | (configurable) | `codes.rs` | Forced Authorization Code — restricted dialing |

### Hunt Groups

`hunt_group.rs` resolves a hunt group to a list of target extensions based on ring strategy:

- **Simultaneous** — ring all members at once
- **Sequential** — ring members in order, moving to next on timeout
- **Round-Robin** — sequential with rotating start position
- **Random** — randomized order

### Feature Code Parsing

`feature_codes.rs` parses dialed digits and returns a `FeatureCodeAction`:

```rust
pub enum FeatureCodeAction {
    GroupPickup,
    DirectedPickup(String),    // extension
    ParkCall,
    RetrieveParked(u16),       // orbit number
    Intercom(String),          // extension
    CmcEntry(String),          // code
    FacEntry(String),          // code
}
```

---

## 6. Session Border Controller

**Crate:** `fcp-sbc` (binary), `fcp-sbc-core` (library)
**Path:** `crates/fcp-sbc/src/`, `crates/fcp-sbc-core/src/`

### Overview

Production-grade SBC enforcing security policies, call admission control, and trunk management. Sits at the network edge between internal PBX users and external PSTN/SIP trunks.

### Enforcement Pipeline

Inbound SIP messages pass through a layered enforcement pipeline before reaching the B2BUA:

```
Inbound SIP -> IP ACL Check -> Rate Limiting -> Message Validation
     -> Malformed SIP Detection -> Header Manipulation (HMR)
     -> Topology Hiding -> STIR/SHAKEN Verification
     -> Admission Control -> B2BUA Handler
```

### Core Modules

| Module | Purpose |
|--------|---------|
| `enforcement.rs` | `SbcEnforcementHandler` — orchestrates the full policy pipeline |
| `admission.rs` | `AdmissionController` — CPS limits, concurrent call limits per trunk |
| `hmr.rs` | Header Manipulation Engine — rule-based SIP header rewriting |
| `topology.rs` | Topology hiding — URI anonymization, Via/Record-Route stripping |
| `stir_shaken.rs` | STIR/SHAKEN caller ID verification and signing (RFC 8224/8225) |
| `srtp.rs` | SDES-SRTP enforcement per trunk (RFC 4568) |
| `siprec.rs` | SIPREC recording with selective recording rules |
| `nat.rs` | NAT traversal with STUN/TURN support |
| `local_capture.rs` | Local packet capture for debugging |
| `metrics.rs` | Runtime metrics collection |
| `admin.rs` | Admin HTTP API for runtime monitoring |

### Trunk Management (fcp-sbc-core)

| Module | Purpose |
|--------|---------|
| `trunk.rs` | Trunk definitions — transport, DTMF mode, SRTP, codec preferences |
| `trunk_router.rs` | Route selection based on LCR and quality metrics |
| `trunk_health.rs` | Trunk health state tracking (Healthy/Degraded/Down) |
| `trunk_registration.rs` | Automated outbound REGISTER for trunk authentication |
| `health_probes.rs` | Periodic OPTIONS health probes for trunk monitoring |
| `trunk_quality.rs` | Per-trunk quality metrics (ASR, ACD, NER) |
| `lcr.rs` | Least Cost Routing with failover and load balancing |
| `cps.rs` | Calls-per-second rate limiting |
| `trunk_auth.rs` | Trunk credential management |

### Configuration

SBC configuration is TOML-based with per-trunk settings:

```toml
[[trunks]]
name = "carrier-a"
host = "sip.carrier-a.com"
transport = "tcp"
dtmf_mode = "rfc2833"
srtp = "required"
max_cps = 100
max_concurrent = 500
codecs = ["PCMU", "PCMA", "G729"]
```

---

## 7. Routing Engine

**Crate:** `fcp-routing`
**Path:** `crates/fcp-policy/src/` (engine) and `crates/fcp-routing/src/` (service wrapper + re-exports)

### Overview

Declarative policy-based call routing with a five-layer architecture. All five
layers are implemented, spread across the crates where they naturally live:

1. **Identity** (implemented, `fcp-policy`) — who is calling, who are they calling
2. **Policy** (implemented, `fcp-policy`) — is the call permitted (time windows in the rule's own timezone, IP ranges, user roles, tenant feature toggles)
3. **Reachability** (implemented, `fcp-sbc-core`/`fcp-sip`) — classified route resolution: 404 when nothing is configured for the destination, 503 when routes exist but no trunk is available
4. **Routing** (implemented, `fcp-sbc-core`) — trunk selection, failover, load balancing (`TrunkRouter` + LCR)
5. **Transform** (implemented, `fcp-sbc-core`/`fcp-sbc`) — header manipulation (HMR) and per-trunk egress number transforms (`egress_number_transform`)

### Components

- `RoutingService` — main API for routing decisions
- `PolicyEvaluator` — evaluates rules against identity context
- `PolicyDecision` — Allow/Deny with reasoning
- `YamlRoutingStore` — YAML-backed policy storage

### Extension Routing

The `SipMessageHandler` in `fcp-sip` handles extension-based routing:

- Digits shorter than `max_extension_digits` (configurable) are treated as internal extensions
- Extensions are looked up in the registrar for registered contacts
- Longer digit strings are routed as phone numbers through LCR/trunk routing

---

## 8. Media Pipeline

**Crate:** `fcp-media`
**Path:** `crates/fcp-media/src/`

### Overview

Integration layer bridging SIP signaling with the `forge-media` engine. Supports both embedded (in-process) and remote (distributed) deployment models.

### Components

| Module | Purpose |
|--------|---------|
| `manager.rs` | `MediaManager` — deployment-agnostic API for media operations |
| `backend.rs` | `MediaBackend` trait with `EmbeddedBackend` and `RemoteBackend` |
| `session.rs` | `MediaSession` — per-call media state and lifecycle |
| `decision.rs` | Media anchoring decision logic |
| `sdp.rs` | SDP parsing, codec extraction, SDES crypto line parsing |
| `dtmf_collector.rs` | DTMF digit collection with configurable terminator and timeout |
| `tts.rs` | Text-to-speech provider integration |
| `codec.rs` | Media codec definitions and preferences |

### Media Anchoring

The `MediaAnchoringMode` determines whether media flows through the platform:

- **Always** — all media is anchored (transcoding, recording, DTMF interception)
- **Never** — media flows directly between endpoints (lowest latency)
- **AsNeeded** — anchored only when required (codec mismatch, recording active)

### SDP Handling

- Codec negotiation between A-leg and B-leg SDPs
- SDES crypto line parsing for SRTP
- NAT/SDP rewriting for external-facing addresses
- Hold/unhold SDP manipulation (`sendonly`/`sendrecv`)

---

## 9. Flow Engine & IVR

**Crate:** `fcp-flow` (engine), `fcp-ivr` (binary)
**Path:** `crates/fcp-flow/src/`, `crates/fcp-ivr/src/`

### Overview

The flow engine executes call flow definitions — JSON-based programs that control IVR menus, call routing, voicemail, and other automated call handling.

### Flow Nodes

A flow is a directed graph of nodes. Each node performs an action and routes to the next node based on the outcome:

- **Answer** — answer the inbound call
- **Play** — play an audio prompt
- **Gather** — collect DTMF digits with timeout
- **Say** — text-to-speech playback
- **Dial** — originate outbound call (B-leg)
- **Record** — record audio
- **Voicemail** — route to voicemail
- **Transfer** — transfer call
- **Hangup** — terminate call
- **Condition** — branch based on variables
- **SetVariable** — set flow variables
- **HTTP** — make HTTP request (webhooks)

### Flow Execution

1. Inbound INVITE matches a DID-to-flow mapping
2. `FlowCallHandler` loads the flow definition from `FlowStore`
3. `FlowEngine::execute()` walks the node graph
4. Each node delegates to `SipMessageHandler` for SIP operations
5. Flow variables are available for condition evaluation

### IVR Binary

The `fcp-ivr` binary is a standalone IVR server:
- Accepts SIP INVITEs and matches DIDs to flows
- No registrar (inbound-only)
- No LCR routing (flow-controlled dialing)
- Configuration maps DIDs to flow definitions

---

## 10. Voicemail System

**Crate:** `fcp-voicemail` (library), `fcp-voicemail-server` (binary)
**Path:** `crates/fcp-voicemail/src/`, `crates/fcp-voicemail-server/src/`

### Overview

Multi-tenant voicemail system with mailbox management, message recording, greeting playback, and MWI (Message Waiting Indicator) notification.

### Components

- `VoicemailService` — core voicemail logic
- `VoicemailMailbox` — mailbox configuration (PIN, greeting, limits)
- `VoicemailMessage` — recorded message metadata
- `VoicemailStore` trait — pluggable storage backend

### Mailbox Controls

- **Enable/Disable** — toggle mailbox active state
- **Max message duration** — 1-3600 seconds (default 300)
- **Max messages** — 1-10000 per mailbox (default 100)
- **PIN protection** — 4-8 digit numeric PIN with bcrypt hashing
- **Greetings** — configurable per-mailbox greetings

### REST API

- `GET/POST /v1/voicemail/mailboxes` — list/create mailboxes
- `GET/PUT/DELETE /v1/voicemail/mailboxes/:id` — manage mailbox
- `POST /v1/voicemail/mailboxes/:id/enable` — enable mailbox
- `POST /v1/voicemail/mailboxes/:id/disable` — disable mailbox
- `GET /v1/voicemail/mailboxes/:id/stats` — message counts and storage
- `GET /v1/voicemail/mailboxes/:mailbox_id/messages` — list messages
- `GET/PUT/DELETE /v1/voicemail/messages/:id` — manage messages

### Standalone Server

The `fcp-voicemail-server` binary accepts SIP INVITEs for configured DIDs, programmatically generates Answer -> Voicemail -> Hangup flows, and executes them via `FlowCallHandler`.

---

## 11. Event System

**Crate:** `fcp-events`
**Path:** `crates/fcp-events/src/`

### Overview

Platform-wide event bus for real-time notifications. Events are published by all components and consumed via WebSocket streaming or internal subscribers.

### Event Categories

| Category | Variants | Description |
|----------|----------|-------------|
| **Call** | Initiated, Ringing, Progress, Answered, Ended, Failed, Parked, Unparked | Full call lifecycle |
| **Media** | SessionStarted, SessionEnded, DtmfReceived, QualityWarning | Media pipeline events |
| **IVR** | PromptStarted, PromptCompleted, InputGathered, GatherTimeout | IVR interaction events |
| **Recording** | Started, Completed, Failed | Recording lifecycle |
| **Flow** | Started, NodeExecuted, WaitingForInput, Completed, Failed | Flow engine execution |
| **System** | Started, Stopping, HealthCheck | Platform lifecycle |
| **User** | Created, Updated, Deleted, Enabled, Disabled | User management |
| **Registration** | Registered, Unregistered, Failed | SIP registration changes |
| **Voicemail** | MessageRecorded, MessageDeleted, MessagePlayed, MwiUpdated | Voicemail activity |

### WebSocket Streaming

Events are available via WebSocket at `GET /v1/events`. Clients receive real-time JSON events for monitoring, dashboards, and webhook triggering.

---

## 12. REST API

**Crate:** `fcp-api-core`
**Path:** `crates/fcp-api-core/src/v1/`

### Overview

RESTful API built on Axum providing management and control for all platform resources. All endpoints are prefixed with `/v1/`.

### Endpoint Summary

| Resource | Endpoints | Description |
|----------|-----------|-------------|
| **System** | `GET /health`, `GET /status` | Health checks and status |
| **Events** | `GET /events` (WebSocket) | Real-time event streaming |
| **Debug** | `GET /debug/{summary,dialogs,subscriptions,calls,rate-limits}` | Live system inspection |
| **Tenants** | CRUD on `/tenants` | Multi-tenant management |
| **Users** | CRUD on `/users`, password/digest endpoints | User and credential management |
| **Registrations** | `GET/DELETE /registrations/:aor` | SIP registration viewing |
| **Calls** | `GET/POST /calls`, `GET/DELETE /calls/:id` | Call listing, origination, hangup |
| **Recording** | `POST /calls/:id/recording/{start,stop}` | Call recording control |
| **CDRs** | `GET /cdrs`, `GET /cdrs/:call_id` | Call detail records |
| **Flows** | CRUD on `/flows`, versioning | Flow definition management |
| **Phone Numbers** | CRUD on `/phone-numbers`, activation | DID management |
| **Line Bindings** | CRUD on `/line-bindings` | Device line assignments |
| **Extensions** | CRUD on `/extensions` | Extension management |
| **Devices** | CRUD on `/devices` | Device provisioning |
| **Voicemail** | Mailbox CRUD, enable/disable, stats, messages | Voicemail management |

### State

All handlers share `ApiState` which provides access to:
- `handler: Arc<SipMessageHandler>` — SIP operations
- `call_manager` — call state
- `dialog_manager` — SIP dialogs
- `storage: Arc<DynStorage>` — all storage backends
- `media_manager` — media operations
- `event_bus` — event publishing
- `default_tenant_id` — default tenant

---

## 13. Storage Backends

**Crates:** `fcp-storage-postgres`, `fcp-storage-redis`

### Overview

Pluggable storage backends implementing the trait-based storage layer defined in `fcp-core`. All backends implement the same set of traits and are interchangeable via `DynStorage`.

### Storage Traits

| Trait | Purpose |
|-------|---------|
| `CredentialStore` | User CRUD with password hashing |
| `RegistrationStore` | SIP registration bindings |
| `CdrStore` | Call detail record storage and querying |
| `PhoneNumberStore` | DID/phone number management |
| `LineBindingStore` | Device-to-line bindings |
| `ExtensionStore` | Extension definitions and routing targets |
| `DeviceStore` | Device provisioning data |
| `VoicemailStore` | Mailbox and message management |
| `WebhookStore` | Webhook endpoint CRUD and delivery logs |
| `GeoRoutingStore` | Geographic routing rules |
| `PortingStore` | Number porting records |
| `TenantStore` | Multi-tenant configuration |

### Backend Comparison

| Feature | PostgreSQL | Redis |
|---------|------------|-------|
| Role | Primary storage | Caching, events, distributed state |
| Deployment | External DB server | External Redis server |
| Migrations | sqlx | N/A (schemaless) |
| Recommended for | All production deployments | Event bus, parking, circuit breaker |

### DynStorage

`DynStorage` wraps all storage traits into a single type-erased container:

```rust
let storage = DynStorage::from_all_in_one(postgres_storage);
// or compose from individual backends:
let storage = DynStorage::new(cred_store, reg_store, cdr_store, ...);
```

---

## 14. Standalone Binaries

### fcp-call-manager

**Path:** `crates/fcp-call-manager/`

Combined SIP registrar and B2BUA operating in two modes:
- **Call Manager Mode** (default) — registrar + B2BUA for internal users
- **Session Manager Mode** — trunk termination + routing (no registrar)

Wires together: SIP transport, transaction layer, dialog management, registrar, B2BUA handler, media manager, event bus, storage, and HTTP admin API.

### fcp-sbc

**Path:** `crates/fcp-sbc/`

Session Border Controller for network-edge deployment. Full enforcement pipeline with admission control, trunk management, and security features. See [Section 6](#6-session-border-controller).

### fcp-ivr

**Path:** `crates/fcp-ivr/`

Standalone IVR server. Accepts inbound calls and routes them to JSON-defined call flows. No registrar or LCR routing. See [Section 9](#9-flow-engine--ivr).

### fcp-voicemail-server

**Path:** `crates/fcp-voicemail-server/`

Standalone voicemail server. Accepts inbound calls for configured DIDs and routes them through programmatic voicemail flows. See [Section 10](#10-voicemail-system).

### fcp-registrar

**Path:** `crates/fcp-registrar/`

Standalone SIP registration server. Handles REGISTER requests only, returning 405 for all other methods. Useful for distributed deployments where registration is separated from call handling.

### fcp-jobs

**Path:** `crates/fcp-jobs/`

Async job processing service backed by Redis. Processes CDR storage, recording transcoding, webhook delivery, and cleanup tasks with retry logic and exponential backoff.

### fcp-events-server

**Path:** `crates/fcp-events-server/`

Standalone event streaming server. Collects events from all platform components and exposes them via WebSocket and HTTP APIs.

### fcp-admin

**Path:** `crates/fcp-admin/`

Administrative interface for platform management. Provides HTTP APIs for configuration, monitoring, and operational tasks.

### fcp-api-gateway

**Path:** `crates/fcp-api-gateway/`

API gateway for routing REST requests to appropriate backend services. Handles authentication, rate limiting, and request routing.

---

## 15. Webhooks

**Crate:** `fcp-webhooks` (dispatcher), `fcp-core` (storage trait + types), `fcp-events-server` (REST API)
**Path:** `crates/fcp-webhooks/src/`, `crates/fcp-events-server/src/routes/webhooks.rs`

### Overview

General-purpose webhook delivery system that subscribes to the FCP event bus, matches events against per-tenant endpoint filters, signs payloads with HMAC-SHA256, and delivers them via HTTP POST with exponential-backoff retries. Replaces the earlier presence-specific webhook implementation in `fcp-presence` (now deprecated).

### Architecture

```
Event Bus (all PlatformEvents)
        |
        v
  spawn_webhook_consumer()     ← subscribes to all events
        |
        v
  WebhookDispatcher::dispatch()
        |
        ├─ extract tenant_id from event
        ├─ WebhookStore::list_endpoints(tenant_id)
        ├─ filter by enabled + event type match
        └─ deliver_to_endpoint() for each match
                |
                ├─ HMAC-SHA256 sign payload
                ├─ HTTP POST with headers:
                │     Content-Type: application/json
                │     X-FCP-Event: <event_type>
                │     X-FCP-Signature: sha256=<hex>
                └─ Log delivery result to WebhookStore
                        |
                        └─ On failure: schedule retry with backoff
```

### Key Components

| File | Purpose |
|------|---------|
| `fcp-webhooks/src/dispatcher.rs` | `WebhookDispatcher` — core dispatch and retry logic with concurrency-limited delivery via semaphore |
| `fcp-webhooks/src/consumer.rs` | `spawn_webhook_consumer()` — subscribes to event bus and feeds events to dispatcher |
| `fcp-webhooks/src/payload.rs` | `WebhookPayload` struct and `compute_signature()` HMAC-SHA256 signing |
| `fcp-core/src/storage.rs` | `WebhookStore` trait, `WebhookEndpoint` and `WebhookDeliveryLog` types |
| `fcp-events-server/src/routes/webhooks.rs` | REST API for endpoint CRUD, test delivery, and delivery log inspection |
| `fcp-storage-postgres/migrations/` | PostgreSQL schema for `webhook_endpoints` and `webhook_delivery_log` tables |

### WebhookStore Trait

```rust
#[async_trait]
pub trait WebhookStore: Send + Sync {
    async fn create_endpoint(&self, endpoint: WebhookEndpoint) -> Result<()>;
    async fn get_endpoint(&self, id: &WebhookId) -> Result<Option<WebhookEndpoint>>;
    async fn list_endpoints(&self, tenant_id: &TenantId) -> Result<Vec<WebhookEndpoint>>;
    async fn update_endpoint(&self, endpoint: WebhookEndpoint) -> Result<()>;
    async fn delete_endpoint(&self, id: &WebhookId) -> Result<()>;
    async fn log_delivery(&self, log: WebhookDeliveryLog) -> Result<()>;
    async fn get_pending_retries(&self, limit: usize) -> Result<Vec<WebhookDeliveryLog>>;
    async fn update_delivery_log(&self, log: WebhookDeliveryLog) -> Result<()>;
    async fn get_delivery_logs(&self, endpoint_id: &WebhookId, limit: usize) -> Result<Vec<WebhookDeliveryLog>>;
}
```

Implemented by the PostgreSQL storage backend (with in-memory `FileBackedStorage` for development).

### Event Type Matching

Endpoints specify an `events` filter list. Matching rules:

- Empty list or `["*"]` — matches all events
- Exact match — e.g., `"call.initiated"` matches only that event
- Category wildcard — e.g., `"call.*"` matches `call.initiated`, `call.ended`, etc.

Event type names use dotted notation derived from `PlatformEvent::event_type_name()`:

| Category | Example Types |
|----------|---------------|
| Call | `call.initiated`, `call.ringing`, `call.answered`, `call.ended`, `call.failed` |
| Media | `media.session_started`, `media.dtmf_received` |
| Registration | `registration.registered`, `registration.unregistered` |
| Voicemail | `voicemail.message_recorded`, `voicemail.mwi_updated` |
| System | `system.started`, `system.stopping`, `system.health_check` |
| Presence | `presence.status_changed`, `presence.dialog_changed` |

### Webhook Payload

Every delivery sends a JSON payload with this structure:

```json
{
    "event_id": "uuid",
    "event_type": "call.initiated",
    "timestamp": "2026-02-20T15:30:00Z",
    "tenant_id": "tenant-...",
    "data": { /* full serialized PlatformEvent */ }
}
```

### Retry Logic

Failed deliveries are persisted to `WebhookDeliveryLog` with a `next_retry_at` timestamp. A background task (`spawn_retry_processor`) polls periodically and re-attempts delivery:

- **Backoff**: exponential — `base_delay * 2^(attempt-1)`, capped at `max_retry_delay`
- **Default**: 30s base, 1h max, 5 retries → 30s, 60s, 120s, 240s, 480s
- **Concurrency**: limited by semaphore (default 50 concurrent deliveries)
- **Exhaustion**: after max retries, marked permanently failed with `next_retry_at = NULL`

### REST API

All endpoints are served by `fcp-events-server` when webhook dispatching is enabled in configuration.

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/webhooks` | Create a new webhook endpoint |
| `GET` | `/v1/webhooks?tenant_id=...` | List endpoints for a tenant |
| `GET` | `/v1/webhooks/:id` | Get endpoint by ID |
| `PUT` | `/v1/webhooks/:id` | Update endpoint (partial update) |
| `DELETE` | `/v1/webhooks/:id` | Delete endpoint |
| `POST` | `/v1/webhooks/:id/test` | Send a test event to the endpoint |
| `GET` | `/v1/webhooks/:id/deliveries` | List recent delivery logs |

### Configuration

Webhook dispatching is configured in the events server TOML config:

```toml
[webhook]
enabled = true
request_timeout_secs = 10
max_retries = 5
base_retry_delay_secs = 30
max_retry_delay_secs = 3600
max_concurrent = 50
retry_interval_secs = 60
database_path = "/var/lib/fcp/fcp.db"
```

### Database Schema

Two tables created by migration `019_add_webhooks.sql`:

- `webhook_endpoints` — endpoint registration with tenant scoping, event filters, and HMAC secret
- `webhook_delivery_log` — delivery attempt history with status codes, retry scheduling, and error messages

Indexed for efficient tenant lookups and pending retry polling.

---

## Cross-Cutting Concerns

### Multi-Tenancy

All resources are scoped by `TenantId`. Tenant context propagates through:
- SIP message handling (extracted from configuration or request)
- Storage operations (all queries include tenant filter)
- CDR generation (tenant_id on every record)
- Event publishing (tenant context in events)

### Identifiers

Strongly-typed UUID-backed identifiers with prefix formatting:

| Type | Format | Example |
|------|--------|---------|
| `CallId` | `call-{uuid}` | `call-550e8400-e29b-41d4-a716-446655440000` |
| `TenantId` | `tenant-{uuid}` | `tenant-...` |
| `UserId` | `user-{uuid}` | `user-...` |
| `PhoneNumberId` | `pn-{uuid}` | `pn-...` |
| `VoicemailBoxId` | `vmbox-{uuid}` | `vmbox-...` |
| `DeviceId` | `dev-{uuid}` | `dev-...` |
| `ExtensionId` | `ext-{uuid}` | `ext-...` |
| `WebhookId` | `whk-{uuid}` | `whk-...` |

### Observability

All components use `tracing` for structured logging with span context:
- `call_id`, `tenant_id`, `dialog_id` propagated through request handling
- SIP transaction metrics via `sip-observe`
- Media quality metrics via `forge-media`
- SBC enforcement metrics via `fcp-sbc/metrics.rs`

### Error Handling

- Library crates use explicit error types via `thiserror`
- Binary crates use `anyhow` for application-level errors
- API endpoints return structured `ApiError` responses with appropriate HTTP status codes

---

## 16. Presence System

**Crates:** `fcp-presence`, `fcp-presence-server`, presence integration in `fcp-call-manager` and `fcp-admin`
**Paths:** `crates/fcp-presence/src/`, `crates/fcp-presence-server/src/`, `crates/fcp-call-manager/src/main.rs`, `crates/fcp-admin/src/main.rs`

### Overview

The presence system tracks real-time user availability (Open, Closed, Ringing, OnThePhone, Away, Busy, DND) across the platform. It integrates with SIP SUBSCRIBE/NOTIFY/PUBLISH (RFC 3856/3903/4235) for BLF and client presence, and provides a REST API for the admin UI.

### Architecture

```
                    ┌─────────────────┐
                    │   Redis Pub/Sub │
                    │ "presence:changes" │
                    └───┬─────────┬───┘
                   publish    subscribe
                        │         │
    ┌───────────────────┴─┐   ┌───┴───────────────────┐
    │   call-manager (A)  │   │   call-manager (B)     │
    │  ┌───────────────┐  │   │  ┌───────────────┐    │
    │  │PresenceManager│  │   │  │PresenceManager│    │
    │  │  (DashMap)     │  │   │  │  (DashMap)     │    │
    │  └───────────────┘  │   │  └───────────────┘    │
    │  SIP registrations  │   │  SIP registrations    │
    │  Call events         │   │  Call events           │
    └─────────────────────┘   └────────────────────────┘
              │                          │
         events-server WebSocket
              │
    ┌─────────┴─────────┐
    │   fcp-admin        │
    │  ┌───────────────┐ │
    │  │PresenceManager│ │
    │  │  (read replica)│ │
    │  └───────────────┘ │
    │  Admin UI REST API  │
    └────────────────────┘
```

### In-Memory Only Design

Presence state is **not persisted to the database**. All state lives in in-memory `DashMap`s within each `PresenceManager` instance. This was a deliberate decision to avoid overwhelming the database with frequent state changes (every registration, call, and state transition would trigger DB writes).

State rebuilds naturally on restart:
- SIP registrations re-trigger `Open` state
- Active calls re-trigger `Ringing`/`OnThePhone` state
- No stale data from previous runs

### Key Components

| File | Purpose |
|------|---------|
| `fcp-presence/src/manager.rs` | `PresenceManager` — core runtime state with `DashMap<String, PresenceInfo>` for presentities, watchers, auth policies, active calls, dialog versions |
| `fcp-presence/src/redis_pubsub.rs` | `RedisPresencePubSub` — cross-node presence propagation via Redis Pub/Sub channel `presence:changes` |
| `fcp-presence/src/config.rs` | `PresenceConfig` — rate limits, expiry bounds, composition, federation settings |
| `fcp-presence/src/pidf.rs` | PIDF XML generation/parsing for SIP NOTIFY bodies (RFC 3863) |
| `fcp-presence/src/xcap.rs` | Pres-rules XCAP support (RFC 5025) for authorization policies |
| `fcp-presence/src/events.rs` | Event publishing helpers (StatusChanged, WatcherAdded, etc.) |
| `fcp-presence/src/federation.rs` | Inter-domain presence federation config |
| `fcp-presence/src/rls.rs` | Resource List Subscriptions (RFC 4826) |
| `fcp-presence/src/analytics.rs` | Presence history/analytics consumer |
| `fcp-call-manager/src/main.rs` | `run_presence_listener()` — maps call/registration events to presence state |
| `fcp-api-core/src/v1/presence.rs` | REST API: list, get, update, delete presentities; watcher management; auth policies |
| `fcp-admin-ui/ui/src/pages/PresencePage.tsx` | Admin UI presence cards with real-time refresh |

### Presence State Flow

#### Registration → Open/Closed
When a SIP device registers, the `run_presence_listener` in `fcp-call-manager` handles `RegistrationEvent::Registered` → sets the device AOR and its extension AOR(s) to `Open`. On unregistration → `Closed`.

#### Call Events → Ringing/OnThePhone
- `CallEvent::Initiated` → callee AOR set to `Ringing`, caller AOR set to `OnThePhone`
- `CallEvent::Answered` → callee transitions `Ringing` → `OnThePhone`
- `CallEvent::Ended`/`Failed` → all affected AORs revert to `Open` (if no other active calls)

#### AOR Resolution Chain

When a call or registration event fires for a given AOR, the presence system resolves it to all related identities:

```
Device AOR (e.g. "polycom@host")
  └─→ resolve_device_aor_to_extension_aors()
       └─→ Extension AOR (e.g. "1234@host")

Extension AOR (e.g. "1234@host")
  └─→ resolve_extension_to_device_aors()
       └─→ Device AOR(s) for BLF/NOTIFY

Phone Number AOR (e.g. "15185551234@host")
  └─→ resolve_phone_number_to_extension_aors()
       └─→ Extension AOR(s)
            └─→ resolve_extension_to_device_aors()
                 └─→ Device AOR(s)
```

This ensures that calling a phone number, extension, or device directly all result in consistent presence across the extension and its bound devices.

#### Resolution Functions (fcp-call-manager/src/main.rs)

| Function | Input | Output |
|----------|-------|--------|
| `resolve_extension_to_device_aors()` | Extension AOR (numeric user part) | Device AOR(s) via ExtensionTarget → Device/User/PhoneNumber |
| `resolve_phone_number_to_extension_aors()` | Phone number AOR (10+ digits) | Extension AOR(s) where target is PhoneNumber matching this E.164 |
| `resolve_device_aor_to_extension_aors()` | Device AOR (non-numeric user part) | Extension AOR(s) via `get_device_by_sip_uri` → find extensions targeting this device |

### Multi-Node Clustering via Redis Pub/Sub

**Feature flag:** `fcp-presence/redis` (enabled via `storage-redis` in call-manager, `redis` in admin)

#### How It Works

1. **Local state change:** `PresenceManager::update_presence()` updates the local `DashMap` and fires a local `StatusChanged` event.

2. **Publish to Redis:** If the `redis` feature is enabled and `set_pubsub()` was called, `publish_change_to_cluster()` sends a `PresenceChangeEvent` (JSON) to Redis channel `presence:changes`:
   ```json
   {
     "aor": "1234@192.168.1.225",
     "tenant_id": "tenant-00000000-...",
     "node_id": "cm-local-01",
     "state": "on_the_phone",
     "note": "On the phone"
   }
   ```

3. **Remote nodes subscribe:** `run_pubsub_listener()` spawns a background task that:
   - Subscribes to `presence:changes`
   - Filters out self-published messages (via `node_id`)
   - Applies the remote state to the local `DashMap`
   - Fires local `StatusChanged` events (for admin UI and SIP NOTIFY)
   - Does **not** re-publish to Redis (prevents infinite loops)

4. **Self-filtering:** Every node has a unique `node_id` (from config). Messages originating from the same node are skipped.

#### Configuration

**call-manager** (`fcp-call-manager-local.toml`):
```toml
[storage]
redis_url = "redis://localhost:6379"    # enables Redis Pub/Sub for presence

[service]
node_id = "cm-local-01"                # unique per node, used for self-filtering

[presence]
enabled = true
```

Build with `--features storage-redis` to enable Redis presence sync.

**admin** (`fcp-admin-local.toml`):
```toml
[cluster]
redis_url = "redis://localhost:6379"    # enables Redis Pub/Sub for presence

[service]
node_id = "admin-local"
```

Build with `--features redis` to enable Redis presence sync.

#### Key Types

| Type | Location | Purpose |
|------|----------|---------|
| `PresenceChangeEvent` | `fcp-presence/src/redis_pubsub.rs` | JSON-serialized event for Redis Pub/Sub (aor, tenant_id, node_id, state, note) |
| `RedisPresencePubSub` | `fcp-presence/src/redis_pubsub.rs` | Handles publish + subscribe on Redis channel `presence:changes` |

### Admin UI Merging

The `list_presences` API endpoint (`fcp-api-core/src/v1/presence.rs`) merges related AORs so each extension appears as a single card:

- **Device AORs** belonging to an extension are hidden when the extension has its own presence entry
- **Phone number AORs** targeting an extension are hidden similarly
- **Watcher counts** from merged AORs are aggregated into the extension entry
- **Standalone AORs** (no extension association) appear as-is

### Presence States

| State | Description | Triggered By |
|-------|-------------|-------------|
| `Open` | Available | SIP registration, call ended |
| `Closed` | Offline | SIP unregistration, device removed |
| `Ringing` | Incoming call ringing | `CallEvent::Initiated` (callee) |
| `OnThePhone` | Active call | `CallEvent::Initiated` (caller), `CallEvent::Answered` (callee) |
| `Away` | User away | API/PUBLISH |
| `Busy` | User busy (DND) | API/PUBLISH, conference joined |
| `DND` | Do not disturb | API |

### Dependencies

- **Redis** (optional): For multi-node Pub/Sub sync. Not required for single-node.
- **Event Bus** (`fcp-events`): For local event propagation between components.
- **Storage** (`fcp-core::DynStorage`): For resolving extensions, devices, phone numbers, and line bindings during AOR resolution.
