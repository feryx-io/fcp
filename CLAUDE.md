# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Ferrous Communications Platform (FCP)** is a next-generation, carrier-grade unified communications platform written entirely in Rust. The project aims to be a multi-tenant, API-first alternative to FreeSWITCH/Kamailio/Asterisk with built-in memory safety, modern async I/O, and observability.

The repository contains 39 workspace crates spanning core libraries, application services, storage backends, and standalone binaries, plus two git submodules under `external/`: the siphon-rs SIP stack and the forge-media media stack. Both submodule repositories are public.

> **Current status lives in [STATUS.md](STATUS.md)** — the phase notes below describe scope, but STATUS.md is the maintained source of truth for what is done, partial, or absent.

## Build and Test Commands

### Initial Setup (First Time Only)

If you're cloning this repository, you need to initialize the git submodules (siphon-rs and forge-media):

```bash
# Clone with submodules
git clone --recursive https://github.com/thevoiceguy/fcp.git

# Or if already cloned without --recursive:
git submodule update --init --recursive
```

### Building and Testing

```bash
# Build all workspace crates
cargo build

# Build with all features
cargo build --all-features

# Run all tests
cargo test --all

# Run tests for a specific crate
cargo test -p fcp-core
cargo test -p sip-transaction

# Run tests with output visible
cargo test -- --nocapture

# Run a specific test by name
cargo test test_name

# Lint with clippy
cargo clippy --all

# Format code
cargo fmt --all

# Check without building
cargo check --all
```

### Running siphond (SIP Testing Daemon)

```bash
# Minimal mode - OPTIONS only
cargo run -p siphond

# Full UAS mode - Complete SIP server
cargo run -p siphond -- --mode full-uas

# Registrar mode with authentication
cargo run -p siphond -- --mode registrar --auth --auth-realm example.com

# Proxy mode - Forward calls between registered users
cargo run -p siphond -- --mode proxy --local-uri sip:proxy@192.168.1.81

# B2BUA mode - Bridge calls between registered users
cargo run -p siphond -- --mode b2bua --local-uri sip:b2bua@192.168.1.81 --tcp-bind 0.0.0.0:5060

# See all options
cargo run -p siphond -- --help
```

For detailed siphond documentation, see `external/siphon-rs/bins/siphond/README.md`.

## High-Level Architecture

### Phase 1: Current State (MVP Foundation)

The project is currently in Phase 1 (MVP) with:
- ✅ **fcp-core**: Core types, identifiers, error handling, configuration, services, codecs, transport abstractions
- ✅ **fcp-sip**: Integration layer bridging fcp-core and siphon-rs with type adapters and service wrappers
- ✅ **siphon-rs**: Complete SIP stack with transaction layer, dialog management, authentication, registrar, UAC/UAS helpers, and multi-transport support
- ✅ **siphond**: Multi-mode SIP testing daemon (minimal, full-uas, registrar, proxy, b2bua)

### Phase 2-5: Planned Architecture

The architecture document (`ARCHITECTURE-v0.2.md`) defines a layered platform with:

**Control Plane** (scales independently, internal network):
- Config Store, Routing Engine, Policy Engine, CDR Processor
- Cluster Membership, Admin API, Webhook Dispatcher, Billing Gateway

**Data Plane** (edge-facing, latency-critical):
- Signaling Workers: Registrar, Proxy, B2BUA, SBC
- Media Workers: RTP Engine, Transcoder, Conference, WebRTC
- Application Workers: IVR, Queue, Voicemail, Presence

**Event Bus**: Decouples components via Redis Streams/NATS/Kafka

## Repository Structure

```
fcp/
├── Cargo.toml                    # Workspace root (38 crates)
├── ARCHITECTURE-v0.2.md          # Detailed architecture and roadmap
├── IMPLEMENTATION_DOCS.md        # Implementation details for all subsystems
├── Phase1 plan.md                # Original MVP implementation plan
│
├── crates/
│   │
│   │ # ── Core ──────────────────────────────────────────────
│   ├── fcp-core/                 # Core types, traits, identifiers, storage abstractions
│   ├── fcp-sip/                  # SIP integration layer (bridges fcp-core and siphon-rs)
│   ├── fcp-events/               # Platform-wide event bus (PlatformEvent types)
│   ├── fcp-api-core/             # REST API endpoints (Axum-based, /v1/ routes)
│   │
│   │ # ── Signaling ─────────────────────────────────────────
│   ├── fcp-b2bua-core/           # B2BUA call engine (call state machine, forking, transfer)
│   ├── fcp-sbc-core/             # SBC enforcement library (admission, HMR, topology hiding)
│   ├── fcp-pbx/                  # PBX feature library (parking, pickup, intercom, hunt groups)
│   ├── fcp-policy/               # Dedicated policy engine (rules, evaluator, YAML store)
│   ├── fcp-billing/              # Billing engine (rating, balance enforcement, settlement, hooks)
│   ├── fcp-cdr/                  # CDR pipeline (record builder, spool, shipper, processor, stream consumer)
│   ├── fcp-crypto/               # Encryption at rest (AES-256-GCM field cipher, local/AWS KMS key providers)
│   ├── fcp-routing/              # Policy-based call routing engine (re-exports fcp-policy)
│   │
│   │ # ── Application ───────────────────────────────────────
│   ├── fcp-flow/                 # Programmable call flow engine (JSON-based DSL)
│   ├── fcp-voicemail/            # Voicemail service (mailboxes, greetings, MWI)
│   ├── fcp-queue/                # ACD queue service (ring strategies, agent management)
│   ├── fcp-presence/             # Presence and BLF (SUBSCRIBE/NOTIFY, dialog events)
│   ├── fcp-webhooks/             # Webhook dispatcher (HMAC-signed, retry with backoff)
│   ├── fcp-notify/               # On-site emergency notification (contacts, notice, channels)
│   ├── fcp-cluster/              # Clustering (Redis heartbeats, node membership, regions)
│   │
│   │ # ── Media ─────────────────────────────────────────────
│   ├── fcp-media/                # Media session management, RTP relay, DTMF, recording
│   │
│   │ # ── Storage ───────────────────────────────────────────
│   ├── fcp-storage-postgres/     # PostgreSQL backend (sqlx, JSONB, migrations)
│   ├── fcp-storage-redis/        # Redis backend (event bus, parking, circuit breaker)
│   │
│   │ # ── Test support (dev-dependency only) ─────────────────
│   ├── fcp-testkit/              # Binary smoke-test harness, throwaway Postgres databases
│   │
│   │ # ── Admin & CLI ───────────────────────────────────────
│   ├── fcp-admin/                # Admin HTTP server
│   ├── fcp-admin-ui/             # Admin web UI
│   ├── fcp-cli/                  # Terminal UI for monitoring
│   │
│   │ # ── Binaries (standalone services) ────────────────────
│   ├── fcp-call-manager/         # Main PBX binary (registrar + B2BUA + routing + PBX)
│   ├── fcp-sbc/                  # Session Border Controller binary
│   ├── fcp-ivr/                  # Standalone IVR server
│   ├── fcp-voicemail-server/     # Standalone voicemail server
│   ├── fcp-registrar/            # Standalone SIP registrar
│   ├── fcp-presence-server/      # Standalone presence server
│   ├── fcp-conference-server/    # Conference bridge server
│   ├── fcp-media-server/         # Standalone media server
│   ├── fcp-tts-server/           # Shared TTS service (provider registry, caching)
│   ├── fcp-webrtc-gateway/       # WebRTC gateway
│   ├── fcp-events-server/        # Event streaming + webhook delivery server
│   ├── fcp-api-gateway/          # API gateway (auth, rate limiting, routing)
│   └── fcp-jobs/                 # Async job worker (the CDR processor, webhooks, cleanup)
│
├── external/                     # ── Git submodules (both public repos) ──
│   ├── siphon-rs/                # SIP stack — https://github.com/thevoiceguy/siphon-rs
│   │   ├── CLAUDE.md             # siphon-rs specific documentation
│   │   ├── bins/
│   │   │   └── siphond/          # Multi-mode SIP testing daemon
│   │   └── crates/
│   │       ├── sip-core/         # SIP message types, headers, URIs
│   │       ├── sip-parse/        # nom-based SIP parser
│   │       ├── sip-transport/    # UDP/TCP/TLS transports
│   │       ├── sip-transaction/  # RFC 3261 transaction state machines
│   │       ├── sip-dns/          # RFC 3263 DNS resolution
│   │       ├── sip-dialog/       # Dialog and subscription management
│   │       ├── sip-auth/         # Digest authentication
│   │       ├── sip-registrar/    # REGISTER handler
│   │       ├── sip-uas/          # User Agent Server helpers
│   │       ├── sip-uac/          # User Agent Client helpers
│   │       ├── sip-proxy/        # Proxy primitives
│   │       ├── sip-sdp/          # SDP support
│   │       ├── sip-observe/      # Observability
│   │       └── sip-testkit/      # Test utilities
│   └── forge-media/              # Media stack — https://github.com/thevoiceguy/forge-media
│       ├── CLAUDE.MD             # forge-media specific documentation
│       ├── bins/                 # forge-sip-server, forge-test-daemon
│       ├── crates/               # forge-core, forge-rtp, forge-sdp, forge-codecs,
│       │                         # forge-conference, forge-mixer, forge-webrtc,
│       │                         # forge-siprec, forge-api, forge-transcoder, …
│       └── external/siphon-rs/   # forge-media's own siphon-rs submodule (recursive)
│
├── tests/                        # Integration tests
├── migrations/                   # Database migrations
└── deploy/                       # Deployment configs (Docker, Kubernetes)
```

## Key Concepts

### Multi-Tenancy

Multi-tenancy is a core design principle baked into FCP from day one:
- `TenantId` is a strongly-typed UUID wrapper with a "tenant-" prefix
- All core identifiers (CallId, SessionId, etc.) include tenant context
- `TenantContext` is propagated through all request handling
- Storage, metrics, and policy decisions are tenant-aware

### Identifiers

FCP uses strongly-typed identifiers defined in `fcp-core/src/identifiers.rs`:

```rust
// Macro-generated types with UUID backing
CallId       // call-{uuid}  - Unique call identifier
SessionId    // sess-{uuid}  - Media session identifier
DialogId     // dlg-{uuid}   - SIP dialog identifier
TransactionId// txn-{uuid}   - SIP transaction identifier
RegistrationId// reg-{uuid}  - Registration binding identifier
NodeId       // node-{uuid}  - Cluster node identifier
TenantId     // tenant-{uuid}- Tenant identifier

// SIP-specific identifiers
SipCallId    // SIP Call-ID header value (e.g., "uuid@host")
Aor          // Address of Record (e.g., "user@domain")
```

**Benefits:**
- Type safety prevents mixing different ID types
- Display format includes prefix for debugging
- Serde support for JSON serialization
- FromStr parsing for both "prefix-uuid" and raw UUID formats

### Error Handling

All errors are defined in `fcp-core/src/error.rs`:
- `Result<T>` is aliased as `std::result::Result<T, Error>`
- `Error` is an enum using thiserror for ergonomic error handling
- Categories: Parse, Protocol, Transaction, Dialog, Transport, Storage, Media, Config
- Sub-errors like `ParseError`, `TransactionError`, etc. for granular error handling

### SIP Stack Integration (siphon-rs)

The SIP stack in `external/siphon-rs` is a complete, production-grade implementation:

**Transaction Layer** (sip-transaction):
- RFC 3261 compliant state machines with transport-aware timer optimization
- TCP/TLS transactions complete 5-37 seconds faster than UDP (zero wait timers)
- Comprehensive performance metrics: duration tracking, timer statistics, outcome analysis
- `TransactionManager` handles lifecycle, `TransactionKey` uniquely identifies transactions

**Dialog Layer** (sip-dialog):
- RFC 3261 §12 dialog management: Early/Confirmed/Terminated states
- Subscription layer for SUBSCRIBE/NOTIFY event notifications
- PRACK support (RFC 3262) for reliable provisional responses with RSeq tracking

**Authentication** (sip-auth):
- RFC 7616/7617 Digest authentication: MD5/SHA-256/SHA-512, qop=auth/auth-int
- Nonce management with automatic expiry
- Pluggable `CredentialStore` trait for production backends

**Registrar** (sip-registrar):
- RFC 3261 §10 REGISTER handling with location service
- Multi-contact support, q-value prioritization, wildcard deregistration
- Pluggable `LocationStore` trait with thread-safe in-memory implementation

**UAC/UAS Helpers** (sip-uac, sip-uas):
- High-level APIs for request/response handling
- Automatic authentication (401/407 challenge handling)
- Dialog and subscription management integration
- Call transfer support: REFER (RFC 3515), Replaces (RFC 3891)
- PRACK support for reliable provisionals
- RFC 3264 Offer/Answer (early and late offer)

**Transport** (sip-transport):
- UDP (stateless), TCP (connection pool), TLS (rustls with SNI)
- RFC 3263 DNS resolution: NAPTR → SRV → A/AAAA with failover

**tel URI Support** (sip-core):
- RFC 3966 telephone number URIs: global E.164 and local with phone-context
- Unified `Uri` enum supporting both SIP and tel URIs

See `external/siphon-rs/CLAUDE.md` for comprehensive SIP stack documentation.

## Development Guidelines

### Code Style

- Follow Rust 2021 idioms and conventions
- Use `thiserror` for error types, `anyhow` for application-level error handling
- Prefer explicit error types over `anyhow::Error` in library crates
- Use `tracing` for structured logging (not `log` crate)
- Async code uses `tokio` runtime exclusively

### Adding New Crates

When adding a new crate to the FCP workspace:

1. **Create crate structure:**
   ```bash
   cargo new --lib crates/fcp-new-feature
   ```

2. **Update workspace Cargo.toml:**
   ```toml
   [workspace]
   members = [
       "crates/fcp-core",
       "crates/fcp-new-feature",
   ]
   ```

3. **Add fcp-core dependency in new crate's Cargo.toml:**
   ```toml
   [dependencies]
   fcp-core = { path = "../fcp-core" }
   ```

4. **Follow naming conventions:**
   - Use `fcp-` prefix for FCP-specific crates
   - Use `sip-` prefix for SIP stack crates (in siphon-rs workspace)
   - Use descriptive names: `fcp-routing-engine`, `fcp-media-rtp`, etc.

### Error Handling Patterns

```rust
// Library crate - use explicit error types
use fcp_core::{Error, Result};

pub fn parse_thing(input: &str) -> Result<Thing> {
    // Use fcp_core::Error variants
    Err(Error::Parse(ParseError::InvalidUri(input.to_string())))
}

// Application crate - can use anyhow for convenience
use anyhow::{Context, Result};

async fn main() -> Result<()> {
    let thing = parse_thing(input)
        .context("Failed to parse configuration")?;
    Ok(())
}
```

### Identifier Usage

```rust
use fcp_core::{CallId, TenantId, Aor};

// Creating identifiers
let call_id = CallId::new();           // Generates new UUID
let tenant_id = TenantId::from_uuid(uuid);
let aor = Aor::new("alice", "example.com");

// Display format
println!("Call: {}", call_id);  // "call-abc12345"

// Parsing from strings
let call_id: CallId = "call-abc12345-...".parse()?;
let aor: Aor = "alice@example.com".parse()?;

// SIP Call-ID (different from internal CallId)
let sip_call_id = SipCallId::new();  // "uuid@fcp.local"
```

### Transaction Layer Integration

```rust
use sip_transaction::{TransactionManager, ClientTransactionUser};
use sip_core::{Request, Method};

// Create transaction manager
let manager = TransactionManager::new(dispatcher);

// Send request and get transaction handle
let request = Request::new(Method::Register, uri);
let mut ctx = manager.send_request(request).await?;

// Wait for response
while let Some(event) = ctx.wait_event().await {
    match event {
        TransactionEvent::Response(resp) => {
            if resp.status.is_final() {
                break;
            }
        }
        TransactionEvent::Timeout => break,
        _ => {}
    }
}
```

### Testing Strategy

**Unit Tests:**
- Test pure functions and logic in isolation
- Use `#[cfg(test)]` modules within the same file
- Mock external dependencies via traits

**Integration Tests:**
- Place in `tests/` directory at workspace root
- Test interactions between multiple crates
- Use `sip-testkit` for SIP-specific test utilities
- End-to-end scenarios against a running binary over real SIP and RTP use
  `fcp_testkit::{Phone, Seed, Service}` and the two-node `fcp_testkit::cluster`
  harness (`docs/TEST_FLOOR.md`) and live in the binary crate's `tests/`
  (`fcp-call-manager/tests/calls.rs`, `fcp-conference-server/tests/{rooms,cluster}.rs`,
  `fcp-webrtc-gateway/tests/rooms.rs`)
- Database-backed suites (`fcp-storage-postgres/tests`, `fcp-api-core/tests`) and the
  per-binary `tests/smoke.rs` files are gated on `FCP_TEST_DATABASE_URL` (and
  `FCP_TEST_REDIS_URL` for `fcp-jobs`); they skip when unset and run in CI against
  service containers. `fcp-testkit` holds the shared harness.

**Property Tests:**
- Use `proptest` for testing invariants
- Focus on parser round-trips and canonicalization

**Fuzz Tests:**
- Located in `external/siphon-rs/fuzz/`
- Use `cargo-fuzz` for parser hardening

### Observability

All components should emit structured logs via `tracing`:

```rust
use tracing::{info, debug, trace, error, instrument};

#[instrument(skip(request), fields(
    call_id = %request.call_id,
    tenant_id = %tenant_id,
))]
async fn handle_request(request: Request, tenant_id: TenantId) -> Result<()> {
    debug!("Processing request");

    // Errors are automatically logged with instrument
    let result = process(&request).await?;

    info!(duration_ms = %result.duration.as_millis(), "Request completed");
    Ok(())
}
```

**Span Context Guidelines:**
- Include: `call_id`, `tenant_id`, `transaction_id`, `dialog_id` when applicable
- Use `%` format for Display, `?` for Debug
- Skip large fields with `skip(field_name)`

## Phase-Specific Development Notes

### Phase 1 (Complete - MVP Foundation)

**Focus:** Single-node PBX with basic call control

- ✅ fcp-core types, identifiers, storage traits
- ✅ siphon-rs SIP stack
- ✅ fcp-sip integration layer
- ✅ fcp-b2bua-core call engine
- ✅ fcp-call-manager runtime (registrar + B2BUA + routing)
- ✅ fcp-api-core REST API
- ✅ fcp-storage-postgres backend
- ✅ Docker deployment

### Phase 2 (Complete - Programmable Voice)

**Focus:** CPaaS-like programmability with webhooks

- ✅ `fcp-flow`: Call flow DSL runtime (JSON-based, 12+ node types)
- ✅ `fcp-ivr`: Standalone IVR server
- ✅ `fcp-voicemail` / `fcp-voicemail-server`: Full voicemail system with greetings, MWI
- ✅ `fcp-webhooks`: General-purpose webhook dispatcher (HMAC-signed, retry with backoff, REST API via fcp-events-server)
- ✅ `fcp-events` / `fcp-events-server`: Real-time event streaming
- ✅ `fcp-admin` / `fcp-admin-ui`: Admin interface

### Phase 3 (Complete - Clustering & Applications)

**Focus:** HA clustering and application suite

- ✅ `fcp-cluster`: Redis-heartbeat clustering (membership, call/conference registries, region labels)
- ✅ `fcp-queue`: ACD queue service
- ✅ `fcp-presence` / `fcp-presence-server`: Presence and BLF
- ✅ `fcp-conference-server`: Conference bridge
- ✅ `fcp-webrtc-gateway`: WebRTC gateway
- ✅ `fcp-storage-redis`: Redis for distributed state
- ✅ `fcp-media-server`: Standalone media server

### Phase 4 (In Progress - Enterprise Features)

**Focus:** Production-ready for enterprise

- ✅ `fcp-sbc-core` / `fcp-sbc`: SBC with enforcement pipeline, trunk management
- ✅ `fcp-routing`: Policy-based routing with LCR
- ✅ STIR/SHAKEN caller ID verification (ES256 sign + verify, cert cache, wired into the SBC enforcement pipeline)
- ✅ `fcp-tts-server`: Shared TTS service (provider registry, caching, health)
- ✅ Device provisioning serving layer (token auth, HTTPS enforcement, template rendering in `fcp-api-core/src/v1/provisioning.rs`)
- ✅ Audio transcoding pipeline — `fcp-media` syncs the payload-type map from each leg's SDP and arms forge-engine's per-packet transcoders on codec mismatch (single-session and bridged modes); gated by `media.enable_transcoding`; transcodable set PCMU/PCMA/Opus/PCM
- ✅ Vendor provisioning templates — built-in Yealink/Polycom/Grandstream/Cisco starters (`config_template::builtin_templates`), instantiated per tenant via `/v1/config-templates/library`
- ✅ `fcp-policy`: Dedicated policy engine — rule model, evaluator, and YAML rule store extracted from fcp-routing (which re-exports for compatibility); timezone-correct time-of-day windows and per-tenant feature toggles included. Number manipulation remains with the SBC transform stage (future)
- ✅ Billing — `fcp-billing` crate: rate plans with prefix rates (model in `fcp-core/src/billing.rs`, admin CRUD at `/v1/rate-plans`), calls rated and settled at CDR creation, per-tenant balance enforcement (402 at admission, mid-call cut-off; `/v1/billing/balance`), ledger with credits/adjustments, and `billing.*` events / `BillingHook` for external billing systems (`docs/BILLING.md`)
- ✅ SBC pipeline stages 3–5: reachability (classified 404/503 resolution), trunk routing/failover/LB (`TrunkRouter` + LCR), transform (HMR headers + per-trunk `egress_number_transform` digit plans)
- ✅ Encryption at rest — `fcp-crypto` envelope encryption (DEKs wrapped by a local master key or AWS KMS), secret columns sealed in the Postgres backend, platform-admin rotate/re-encrypt API (`docs/ENCRYPTION.md`)

### Phase 5 (Planned - Carrier Grade)

**Focus:** Carrier-scale deployment

**Key Components (in the agreed order, see STATUS.md):**
- E911 integration — block 1 shipped (`docs/E911.md`): detection, dispatchable location, ELIN caller ID, emergency-capable trunks, audit + event, admin API; block 2 designed 2026-09-15 (`docs/E911_BLOCK_2.md`): every emergency path, PIDF-LO by value, network-based and declared locations, provider validation, notification delivery, six phases; phase 1 done (v0.25.176): one emergency path for INVITEs, flow `Dial` nodes and the WebRTC gateway (caller identity, refusal, `[e911] trusted_edges`), emergency routes tried one at a time, records completed at hangup with `call.emergency_completed`, multipart A-leg bodies; phase 2 done (v0.25.177, siphon-rs v2026.09.15): location by value per trunk (`emergency_location`, a multipart INVITE with an RFC 5491 PIDF-LO and `Geolocation: <cid:…>`), RFC 5139 civic elements on addresses and sites, refused locations retried as ELIN or failed over; phase 3 done (v0.25.178): a device is placed by where it is — registration source addresses, `X-FCP-Client-Address` from trusted edges, `emergency_networks` (`/v1/emergency/networks`), trusted devices' own PIDF-LO, declared locations (`/v1/me/location`), emergency metrics; phase 4 done (v0.25.179): validation — `/v1/locations/:id/e911/validate` and `/v1/devices/:id/e911/validate`, a site's validation kept and cleared when its address is edited, the basic validator splitting the street into RFC 5139 elements, the `http` provider behind `[e911.validator]` with a token sealed at rest (`/v1/system/encryption/seal`); phase 5 done (v0.25.180): on-site notification delivered — typed contacts, `fcp-notify` run by `fcp-events-server` under `[notifications]` (SMTP, an HTTP SMS provider, a spoken call through `/v1/emergency/notify-call`, webhooks), every attempt an `emergency_notifications` row with retries and `emergency.notification_failed`, `933` notices marked `[TEST]`; phase 6 done (v0.25.181): the admin UI's emergency pages (`/admin/emergency`: configuration and contacts, readiness, networks, the call log with its notifications) and the site, device, number and trunk fields it never showed — block 2 complete
- Regulatory compliance features — basics shipped (`docs/COMPLIANCE.md`): retention policy + sweep, recording consent, admin audit log, CDR export; follow-ups designed 2026-09-14 (`docs/COMPLIANCE_FOLLOWUPS.md`): legal hold, subject export and erasure, consent on every recording, five phases; all five phases done (v0.25.166: every recording a row, deletion by row with a file reaper, the trigger-written deletion log, voicemail retention, compliance permissions; v0.25.170: legal holds enforced by retention filters and delete triggers, S3 Object Lock on held archive days; v0.25.171: consent on every recording — one recorder for API, flow and automatic recording by user, number and node, the announcement heard before the first sample, `skip` and the notice tone, queue calls announced to both parties and mixed; v0.25.172: subject export — requests run by `fcp-admin`, the subject matched narrowly across every store, a checksummed tar.gz with recordings marked for review, audited downloads; v0.25.173: subject erasure — preview then confirm, delete/pseudonymise/keep per store with a per-tenant keyed pseudonym, holds respected, the deletion log under the request, the archive rewritten, dead letters reached or aged out, tenant deletion as a request)
- Distributed conferencing (cascaded mixing) — all four phases shipped (registry + placement, mutual-TLS inter-node trunks with cascaded mixing, cross-node control under a shared `cluster.secret`, resilience: registry freshness, trunk liveness, draining, trunk SRTP; `docs/DISTRIBUTED_CONFERENCING.md`). Participant migration off a draining node shipped 2026-09-13 as phase 5: REFER with a signed ticket, executed by the call manager's B2BUA and the WebRTC gateway
- Multi-region support — read replicas and region-aware cluster membership shipped (`docs/MULTI_REGION.md`: Postgres primary + per-region streaming replicas with configuration reads routed to a healthy replica; `[cluster] region` on every node's heartbeat, conference placement prefers the caller's region, admin API/UI show regions; the events server's `EventBridge` forwards each region's event stream to its `[[event_bus.peer_regions]]` with loop-safe envelopes, webhooks firing only in the origin region; CockroachDB dropped)
- Video conferencing — design agreed (`docs/VIDEO_CONFERENCING.md`): mixing core, forwarding fast path, native codecs behind cargo features, GPU backend designed in (device-resident frames, placement by capability); phases 1–2 shipped in forge-media (substrate: RTCP feedback, payload formats, frame assembler, SDP video; mixer core: `forge-video` and `forge-video-codecs`), then phase 3 in FCP (per-section SDP, `VideoRoom` and the conference server's video leg per participant, live controls, the self-benchmark and budget-aware placement), phase 4 (WebRTC: the gateway's media path, join tokens and join page), phase 5 (WebM recording, trunk video, cross-node speaker) and phase 6 (passthrough fast path, bitrate ladder, overload shedding, nightly parser fuzzing in forge-media). Phase 7, screen share, is done (§15.6): 7a the content source kind and floor, 7b the RFC 4796 section on SIP, 7c a second video mid on WebRTC and the join page's share button, 7d content across trunks with a room-wide floor, 7e BFCP floor control for SIP room systems (`forge-bfcp`, `[conference] bfcp_bind`); phase 8 (GPU backend, §15.7) — 8a `forge-video-hw` over FFmpeg device contexts, 8b the pipeline on the device, 8c the node (`[conference] video_device`, the `video-hwaccel` feature, saturation budget, placement, the NVIDIA image; `docs/GPU.md`) — built on the GPU box `deploy/gcp-gpu` (`docs/GPU_BOX.md`); phase 9 (the MP4 recording sink, §15.9) — 9a `forge-mp4`, a fragmented-MP4 writer for H.264/HEVC/AV1 with Opus, 9b the recorder (`recording_format = mp4`, `recording-mp4` feature, `auto` picking MP4 where H.264 is encodable so a GPU room records on NVENC)
- Dedicated CDR pipeline — designed 2026-09-14 (`docs/CDR_PIPELINE.md`): the node spools every record locally and ships it to a dedicated Redis stream, the processor in `fcp-jobs` normalizes, enriches, rates, stores and announces (`cdr.written`), then partitions, rollups and a streamed export; five phases, all done (v0.25.159: `fcp-cdr` with the record builder, typed termination reasons and the local spool, `[cdr] spool_dir`; v0.25.160: the processor in `fcp-jobs` on the Redis stream `fcp:cdrs` with `[cdr] sink = "stream"`, `cdr.written`, money as `rust_decimal`/`NUMERIC(18,6)`, `BillingStore` split from `CdrStore`; v0.25.161: every leg a record — conference legs, gateway sessions, queue segments under the call's id, linked through `X-FCP-Call-Id` and `X-FCP-Transferred-From`; v0.25.163: monthly partitions, keyset pagination, streamed export, hourly usage rollups, locked retention, measured at 10 M records; v0.25.164: the archive sink — each tenant's closed UTC days as gzipped JSON Lines in an S3-compatible bucket or a directory from `fcp-jobs` `[cdr_archive]`, confirmed by size and SHA-256, rewritten when late records land, `cdr.archived`, and `cdr_archive_required` holding retention to what the archive confirmed)

Descoped on 2026-09-02: SQLite backend, plugin runtime (WASM/Lua), gRPC API, standalone
proxy worker. The vision document keeps them as history; STATUS.md is authoritative.

## Working with the Submodules (siphon-rs and forge-media)

FCP links two git submodules under `external/`, each maintained as its own repository:

- **siphon-rs** (SIP stack) — https://github.com/thevoiceguy/siphon-rs
- **forge-media** (media stack) — https://github.com/thevoiceguy/forge-media, which itself
  contains siphon-rs as a nested submodule (hence `--recursive` everywhere)

Both repositories are **public**, so no credentials are needed to clone them. CI checks
them out with `actions/checkout@v5` + `submodules: recursive` using the default
`GITHUB_TOKEN` — no extra secret is required.

### The single sip-sdp rule

forge-media's `forge-sdp` crate depends on `sip-sdp` via a **git dependency pinned to a
siphon-rs release tag** (see `external/forge-media/crates/forge-sdp/Cargo.toml`). FCP's
root `Cargo.toml` then patches that git source to the local submodule checkout:

```toml
[patch."https://github.com/thevoiceguy/siphon-rs"]
sip-sdp = { path = "external/siphon-rs/crates/sip-sdp" }
```

This keeps the whole workspace resolving to a **single copy** of `sip-sdp` (mixing two
copies breaks type compatibility between fcp-sip and forge-media). When bumping the
siphon-rs submodule to a new tag, also update the tag in forge-sdp's git dependency so
the patch and the pin stay consistent.

### Release-tag workflow for submodule changes

Submodules are pinned to **annotated release tags** (`vYYYY.MM.DD`, e.g. siphon-rs
`v2026.08.25`, forge-media `v2026.09.01`), not arbitrary branch commits. To land a change:

```bash
cd external/forge-media            # or external/siphon-rs

# 1. Branch, change, test, push in the submodule repo
git checkout -b feature/my-feature
cargo test
git add . && git commit -m "Add feature"
git push -u origin feature/my-feature

# 2. Merge to main (fast-forward when main is an ancestor — no merge commit needed),
#    then tag the release and delete the merged branch
git push origin feature/my-feature:main   # fast-forward merge (or merge a PR)
git tag -a vYYYY.MM.DD -m "Release notes" && git push origin vYYYY.MM.DD
git push origin --delete feature/my-feature

# 3. Repoint FCP at the release and commit
cd ../..
git -C external/forge-media checkout main
git add external/forge-media Cargo.lock
git commit -m "chore: vX.Y.Z — bump forge-media to vYYYY.MM.DD"
```

Every FCP commit bumps the workspace `version` in the root `Cargo.toml` and names it in
the commit message (`fix: v0.25.60 — ...`); refresh `Cargo.lock` after the bump.

### Checking Submodule Status

```bash
# See which commit/tag each submodule is at
git submodule status

# See if a submodule has uncommitted changes
cd external/forge-media && git status
```

### Important Notes

- Always commit and push changes in the submodule repo first, then update the FCP
  submodule reference — CI can only fetch submodule commits that exist on GitHub
- Never commit submodule code changes directly in FCP; use the submodule's own repo workflow
- When pulling FCP changes, update submodules: `git pull && git submodule update --init --recursive`
- CI scopes `cargo fmt --check` to FCP workspace members, so submodule style is the
  submodule repo's concern, not FCP's

## Common Tasks

### Adding a New SIP Method

1. Add method to `sip-core/src/method.rs`:
   ```rust
   pub enum Method {
       // ...
       NewMethod,
   }
   ```

2. Update parser in `sip-parse`

3. Add handler in `siphond` if needed

4. Write tests in `sip-core/tests/`

### Creating a New Service

1. Create crate: `cargo new --lib crates/fcp-new-service`

2. Define service trait in `fcp-core/src/service.rs`:
   ```rust
   #[async_trait]
   pub trait NewService: Send + Sync {
       async fn process(&self, input: Input) -> Result<Output>;
   }
   ```

3. Implement service in new crate

4. Add integration tests

### Running Interop Tests

```bash
# Start siphond in registrar mode
cargo run -p siphond -- --mode registrar

# In another terminal, test with a softphone
# Configure phone to register at localhost:5060
```

## Documentation

### Key Documents

- **ARCHITECTURE-v0.2.md**: Complete architecture vision and roadmap
- **docs/E911.md**: Emergency calling — location precedence, call flow, admin API, tenant checklist
- **docs/E911_BLOCK_2.md**: Emergency calling block 2 design — the defects in block 1's paths, location by value (PIDF-LO), network and declared locations, validation providers, notification delivery, six phases
- **docs/COMPLIANCE.md**: Retention policy and sweep, recording consent, admin audit log, CDR export
- **docs/COMPLIANCE_FOLLOWUPS.md**: Legal hold, per-subject export and erasure, consent on every recording — the survey's findings, the hold model, subject requests and pseudonymisation, the five phases
- **docs/DISTRIBUTED_CONFERENCING.md**: Conference registry, placement rule, inter-node trunks (mutual TLS, cascaded mixing) and the frame-clock mixer
- **docs/VIDEO_CONFERENCING.md**: Video conferencing design (mixer core, codecs and licensing, layouts, capacity model, phases)
- **docs/MULTI_REGION.md**: Read replicas, region-aware membership, cross-region event fan-out
- **docs/MULTI_REGION_FOLLOWUPS.md**: The leftovers survey (agreed 2026-09-22) — registrations are the one hot-path store that is primary-always and uncached, so a region away from the primary pays two crossings per call; the Redis backend that exists shares the global cluster Redis and is call-manager-only; regional ingress, partition visibility, and an unmeasured event bridge; four phases; phase 1 measured 2 reads per call (6 with two aliases), phase 2 (v0.25.232) cached them — `[registrar] cache_ttl_secs`, invalidated by own writes and registration events — so a repeat call within the TTL costs none; phase 3 (v0.25.233) made a partition visible — `whole` / `store_unreachable` / `remote_regions_lost` on `/status`, `/metrics`, the admin UI and `system.cluster_view_changed`, never acted on — and fixed a Redis reconnect that took minutes; phase 4 (v0.25.234) is the deployment guide (`docs/MULTI_REGION_DEPLOYMENT.md`) — a call manager has one Redis, so a deployment is one cluster on a shared Redis or a cluster per region; all four phases done; the decision point closed 2026-09-23 in favour of the cache, the regional-registrations project not written
- **docs/MULTI_REGION_DEPLOYMENT.md**: Running FCP in several regions — one cluster or one per region (a call manager has one Redis), what each region runs, which Redis is shared, SRV and carriers, the registration cost away from the primary, what a partition looks like, alerts, a checklist, and the gaps found writing it
- **docs/DEPLOYMENT_GAPS.md**: The deployment guide's gaps (agreed 2026-09-23) — the eight gaps `MULTI_REGION_DEPLOYMENT.md` §8 found, checked against the code: advertised addresses and the SBC's four node ids, the registrar's open `/status` and `/drain`, the gateway's health probe and the API's authenticated `/health`, dead `[events]` keys, HTTP-ingested events reaching no other events server, replica health nobody can ask for, and trunks — INVITE to the first host only, REGISTER already on SRV; seven phases, all done (v0.25.236–242), ending in hosts as a failover list and RFC 3263 for an unset port
- **docs/SHIP_AND_OPERATE.md**: Ship and operate (agreed 2026-09-23) — does what we ship start, and does what we tell an operator work: five of nine compose configs refused at startup, a compose file that cannot come up, unknown keys accepted everywhere, operator docs with none of the token rules; four settled questions (refuse unknown keys, delete the root packaging, environment overrides for every token, rewrite five docs and delete the rest); six phases from `--check-config` and CI over every TOML to the rewritten guide and runbook
- **docs/SINGLE-NODE-SETUP.md**: Every binary on one machine, from the root local files — the fourteen binaries by their real names, the startup rules with the key and the environment variable for each, the one SIP realm (`[sip] realm` on the admin, else the call managers'; hashes made again on a change), `--check-config`, the port plan, what connects to what and with which token, the first tenant, user and phone, and what one node carries (measured)
- **docs/CLUSTER_OPERATIONS_RUNBOOK.md**: Running several nodes — what a cluster is (Redis heartbeats), the per-binary `[cluster]` keys, every binary's routes and token in one table, drain, rolling restart, partition and Redis-down behaviour, the log lines as they are
- **deploy/docker-compose/README.md**: The stack in containers on one host — `.env`, the port plan, what runs where, and how the same configurations are checked by CI's stack test
- **docs/BACKUP_AND_RESTORE.md**: The operator's document for keeping a deployment — where every piece of state lives (Postgres, the certs volume with the master key, files per node, Redis) and whether it is re-derivable, what a backup must hold, what a restore needs, what an upgrade does (procedures land with the project's phases)
- **docs/BACKUP_RESTORE_UPGRADE.md**: Backup, restore and upgrade (agreed 2026-09-24) — where every piece of state lives and whether it is re-derivable, the call manager's recordings outside any volume, migrations that refuse to run backward, the invisible node version; five phases from the state inventory to a proved restore and the upgrade rules; phase 1 (v0.25.257) put every compose path in a volume and wrote `docs/BACKUP_AND_RESTORE.md`, phase 2 (v0.25.258) added `deploy/docker-compose/scripts/backup.sh` — the dump through the `postgres` container, a tar per stateful volume, a manifest with the release and the KEK id, `SHA256SUMS` — and the from-source procedure; phase 3 (v0.25.259) added `restore.sh` — checksums, the key's fingerprint against the manifest's KEK id, the volumes as compose labels them, `pg_restore` into `postgres` started alone, `up` — proved by `crates/fcp-call-manager/tests/compose_restore.rs`, which also watches all nine database services refuse a wrong key; phase 4 (v0.25.261) compared the versions the heartbeats already carried — `fcp_cluster::versions`, `cluster_versions` on `/status`, `fcp_cluster_version_skew` on `/metrics`, `version_skew` on the cluster stats and a banner on the cluster page, `system.version_skew` — wrote the upgrade rules and procedures into `docs/BACKUP_AND_RESTORE.md` §5, and added `scripts/check-migrations.sh` to CI (a contraction needs `-- fcp: contraction: …, read by nothing since vX.Y.Z`); phase 5 (v0.25.263) proved it — an older binary refuses a newer or edited schema naming the migration (`fcp-storage-postgres/tests/upgrade.rs`), a node on another release is flagged and resolved against Redis (`fcp-cluster/tests/versions.rs`), the compose call manager clustered as it ships is on the admin's node list with its release; all five phases done
- **docs/HYBRID_CONTROL_PLANE.md**: Hybrid, the control plane and the site (agreed 2026-09-25) — why a customer's site cannot run against the cloud's stores, the model (cloud as the system of record, a site as the tenant's own stack plus `fcp-link`), enrollment into mutual TLS, the trigger-written journal with a snapshot and a change feed, records shipped up, survivability when the link is down, seven phases, nine questions; phase 1 (v0.25.268) built sites and enrollment — `fcp_core::site`, `/v1/sites`, the admin's `[link]` listeners (enroll over TLS, heartbeats under mutual TLS from a link CA the admin makes), `system.site_link_changed`, and the `fcp-link` binary that enrolls and heartbeats, proved against a live admin; phase 2 (v0.25.269) the journal (`sync_journal`, migration 079, one trigger function on nineteen tables, `fcp_core::sync::SyncStore`), `GET /link/v1/snapshot` and the long-polled `GET /link/v1/changes` on the mutual-TLS listener, proved by `sync_journal.rs` and `fcp-link/tests/sync.rs`; phase 3 (v0.25.270) the link applying the snapshot and the feed into the site's Postgres (`[site] database_url`, cursor in `sync.json`, `FlowStore::put_flow`), `sync` on the heartbeat and `sync_behind` on every site, the site's admin refusing writes to synced resources under `[api] managed_by` (`fcp_api_core::managed`), proved by `fcp-link/tests/apply.rs`; phase 4 (v0.25.271) calls across: `[link] sip_host` makes each site a link trunk at enrollment (`Trunk.site_id`, `link`), the cloud SBC takes the site's TLS registration and sends calls for numbers with `site_id` down its RFC 5626 flow, a site's calls reach the cloud's call manager as the tenant's own (`X-FCP-Caller-Site`), media to and from a site is latched, proved by `fcp-link/tests/calls.rs`; phase 5 (v0.25.272) records up: a `ship_seq` on five tables and a cursor per table in the link, CDRs re-rated by the cloud's processor and announced naming the site, recording and voicemail rows shipped with the site's logged deletions mirrored, files fetched and removed through `site_requests` on the link's long poll, the site's events forwarded with `origin_site`, registrations on the heartbeat, proved by `fcp-link/tests/records.rs`; phase 6 (v0.25.273) survivability proved: `fcp_testkit::cut::CutLink` cuts the link listener and the SBC in `fcp-link/tests/survivability.rs` — the site goes on alone (phones, a local trunk, records and events waiting), the cloud says down and back, the catch-up lands, and mid-call BYE and re-INVITE toward a site go down its flow; phase 7 (v0.25.274) the site as a product: the `site` compose project (`deploy/docker-compose/site`, `Dockerfile.link`, the backup scripts taking `COMPOSE_DIR`), `docs/SITE_SETUP.md`, the runbook's §9, the admin UI's Sites page with `site_id` on numbers and trunks, and the nightly's site stack proved by `fcp-link/tests/compose_site.rs`; all seven phases done
- **docs/ENTERPRISE_CALLING.md**: Enterprise calling (designed 2026-09-27) — the nine checklist features (shared call appearance, DND, call waiting, find-me/follow-me, music on hold, class of service, schedules, executive-assistant, hoteling) and the gap under them: no per-user calling model, four ring paths that agree on nothing, a B2BUA that passes hold through and holds no held state, one-shot PCMU playback, one-tenant presence, devices without credentials; the model (`user_calling_settings`, `plan_for_user` run by every path, the effective user of a device, `CallTarget`, the held leg), eleven phases, ten questions (agreed 2026-09-27 on their recommendations); phase 1 done (v0.25.276): the settings row (migration 082, journaled for sites), `fcp_core::calling`, `fcp_sip::ring_plan` executed by every path to a user, the four forwards and DND, the feature codes, `/v1/users/:username/calling` and `/v1/me/calling`, the presence listener per tenant, the dead `fcp-core` engine deleted, proved by `fcp-call-manager/tests/calling.rs`; phase 2 done (v0.25.277): the held leg and `call.held`/`call.resumed`, the node carrying out REFER under `[b2bua.transfer] execute` (blind, park, attended via `Replaces`), INVITE with `Replaces` taking a leg, park and retrieve as leg moves, in-dialog requests matched by Call-ID and tags; phase 3 done (v0.25.278, forge-media v2026.09.27): music on hold — forge-media's player through scheduled playout (looped, in the leg's codec, the other leg held back), `[media] hold_music` anchoring every call and answering holds at the node, the music following `Call.held` through `CallManager::set_on_hold_changed` (`fcp_sip::hold_music`): the holder's prompt or `none`, the tenant's `moh` prompt, a built-in default; phase 4 done (v0.25.279): call waiting — a user's calls as `fcp_cluster::CallLeases` (`LeaseSubject::User`) kept by `CallManager::set_on_parties_changed` (`fcp_sip::user_calls`), off with a call up is busy, `*56`/`*57`; phase 5 done (v0.25.280): schedules — `fcp_core::schedule`, `schedules` (migration 083), `/v1/schedules` and its page, a number's `closed_target`/`holiday_target`, a user's after-hours forward, a database queue's hours, the flow `Schedule` node, `flow:`/`queue:` targets placed (`start_flow_for_call`); phase 6 done (v0.25.281): find-me — `CallingSettings.find_me` steps forked one after another (`next_find_me_step`), a confirming answerer connected only on a 1 (`confirm_answer`), `*60`/`*61`; phase 7 done (v0.25.282): class of service — `fcp_core::class_of_service` classifying by the tenant's `calling` country and overrides (migration 084), `classes_of_service` and `/v1/classes-of-service`, checked for placed calls, forwards and find-me steps (`fcp_sip::class_of_service`, loopback legs marked `X-FCP-Forwarded-For`), `max_calls` both ways, number screening (433/603), the line's caller ID or `Privacy: id` at trunk egress; phase 8 done (v0.25.283): hoteling — devices with credentials of their own and one "may act as" rule for REGISTER and INVITE (`fcp_sip::hoteling`), `hoteling_host` and `hoteling_sessions` (migration 085), `*88` (extension and PIN in an inline flow's `HotelingLogin` node) and `*89`, `check-sync` to the host, the sweep (`[pbx] hoteling_sweep_secs`), the guest as the host's effective user for ringing, caller, class of service and codes, `/v1/hoteling/sessions` and `/v1/me/hoteling`; phase 9 done (v0.25.284): shared call appearance — `shared`/`max_appearances` on a user's line (migration 086), the second account in provisioning and the templates, appearances in memory or a Redis hash per line (`fcp_cluster::LineAppearances`, one Lua claim), `call-info` and `line-seize` answered by `fcp_sip::sca`, the call manager's appearance hook and `line.appearance_changed`, a retrieve by `Call-Info` taking the line's leg and handed across nodes with `X-FCP-Node-Handoff` (siphon-rs `IntegratedUAC::send_request`); phase 10 done (v0.25.285): executive-assistant — the relation on the executive's calling settings (`executive`: enabled, filter, alert, assistants with their own opt-in), screening in `ring_by_plan` (`executive_screening`, a distinct `Alert-Info`, "for <executive>"), `*63` push as a node-run transfer, `*64` on behalf (the executive's line at the trunk, the assistant's class, both on the record), `*65`–`*68`, `/v1/me/executives`; phase 11 done (v0.25.286, forge-media v2026.09.28): monitor, whisper and barge — forge-mixer's `SupervisionMix` shared by forge-engine's third party (`add_party`/`connect_party`/`set_party_mode`/`remove_party`) and the queue's supervisor bridge, `fcp_sip::supervision` (a supervisor dialed onto any anchored call), barge-in on a shared appearance (`barge_in`, migration 087), the node's `/calls/:id/{monitor,whisper,barge}` for every call and the admin API's `/v1/calls/:id/…` under `call_supervise` forwarded to the node holding the call — all eleven phases done
- **docs/DEVELOPER_SURFACE.md**: The developer surface (designed 2026-09-29) — API keys a read-only user can escalate, ~300 routes with no specification and five pagination forms, webhooks a tenant cannot manage or verify against replay, flows that hear only digits; the model (one generated contract, one shape, scoped keys with owners, tenant webhooks with timestamped signatures, speech recognition shaped like TTS), eight phases, eleven questions (agreed 2026-09-29 on their recommendations); phase 1 done (v0.25.293): keys made safe — `api_token_manage`, a token within its maker's permissions and judged against the maker at each use, revoke kept, rotation with grace (migration 088), personal `/me` tokens, every route's permission in `route_permissions::ROUTES` enforced by one middleware (`docs/API_TOKENS.md`); phase 2a done (v0.25.294): one JSON error shape with a `code` from every layer (`shape::json_errors`), `X-Request-Id`, CORS for `PATCH` and the tenant header, `[api.rate_limit]` per token and per tenant counted in Redis or the process; phase 2b done (v0.25.295): every response time RFC 3339 (`fcp_api_core::time` helpers on the handlers' structs, domain structs untouched), ids prefixed where typed, `check_shape` over every response the Postgres suite sees; phase 2c done (v0.25.296): every list `{data, next_cursor, total}` with `limit`/`cursor` (`fcp_api_core::page`, keyset over a stable key, the audit and deletion logs in SQL), the UI's `getAll` and the CLI's `list_all` — phase 2 complete; phase 3a done (v0.25.297): the OpenAPI contract — `utoipa` on every route, `fcp_api_core::openapi::ApiDoc` (+ fcp-admin's cluster routes), `docs/openapi.json` checked by `docs_openapi_json_is_the_spec` (`FCP_WRITE_OPENAPI=1` rewrites it), served at `/api/v1/openapi.json` and `/admin/api`, every Postgres-suite response validated against its schema; phase 3b done (v0.25.298): the UI's types generated from the contract (`npm run gen:api` → `src/api/schema.d.ts`, checked by CI's Admin UI job), the CLI's models checked by `fcp-cli/tests/contract.rs` — phase 3 complete; phase 4 done (v0.25.299): `sdk/typescript` (`@fcp/sdk`, openapi-fetch) and `sdk/python` (`fcp-sdk`: generated `fcp_api` + hand `fcp_sdk`), tested live by `fcp-admin/tests/sdks.rs`, generated code checked by CI's SDKs job (`sdk/README.md`); phase 5 done (v0.25.300): tenant webhooks — `/api/v1/webhooks` under `webhook_manage` (CRUD, `rotate-secret` with an overlap, `test`, deliveries, replay, `event-types` from `PlatformEvent::EVENT_TYPES`), server-made `whsec_` secrets shown once, `X-FCP-Signature: t=<unix>,v1=<hex>`, `fcp_webhooks::target` (URL checked when written, `GuardedResolver` at delivery, `allow_private_targets` for a lab), the events server's routes deleted and its dispatcher on in compose, the spec's `webhooks` section, the UI's Webhooks page (`docs/WEBHOOKS.md`); phase 6 done (v0.25.301): speech recognition — `fcp_media::stt` (Deepgram, OpenAI, a local faster-whisper worker, `remote`; utterances cut by `Endpointer` for a provider without a stream), `MediaManager::listen`/`recognize` over forge-engine's media bridge, `fcp-tts-server`'s `/v1/transcribe` and `/v1/transcribe/stream` under `[stt.providers]`, `[stt]` on the call manager and voicemail server, voicemail transcribed when the mailbox asks and published as `voicemail.transcribed`, proved against `fcp_testkit::stt::ScriptedStt` (`docs/SPEECH.md`); phase 7a done (v0.25.302): flows that listen — Gather's `input` (`dtmf`/`speech`/`dtmf_speech`), `hints`, `language`, `speech_timeout_ms`, `speech_transitions` (`fcp_flow::schema::match_speech`), `speech`/`speech_confidence`, barge-in on speech, the `Transcribe` node publishing `call.transcript` (`fcp_sip::flow_listen`), SIP INFO digits joining the Gather, a quiet leg heard as silence, the flow builder's fields, proved by `fcp-call-manager/tests/listen.rs`; phase 7b done (v0.25.304): the flow fixes — the `Http` node's `send_variables`, `<response_variable>_status`, JSON unpacked into dotted variables and `on_error` for a status ≥ 400, every version kept in `flow_versions` (migration 089, a trigger on `flows`) with `/flows/{id}/versions/{version}`, `webhook_url` removed (`docs/FLOWS.md`) — phase 7 complete; phase 8a done (v0.25.305): live call control — `/api/v1/calls/:id/{hold,resume,transfer,redirect,play,say,hangup}` under `call_control`, forwarded to the node holding the call (`SupervisionRequest::Control`), the node's routes in `fcp-call-manager/src/call_control.rs` over `SipMessageHandler::{hold_call,resume_call,transfer_call,redirect_call,play_into_call,say_into_call}` (`fcp_sip::call_control`), redirect as a transfer to `flow~<ticket>` (`docs/CALL_CONTROL.md`); phase 8b done (v0.25.306): `docs/DEVELOPER_GUIDE.md` quoting `sdk/{typescript,python}/examples/` (a scoped token, a webhook receiver, a listening flow, a live call), run by `fcp-admin/tests/sdks.rs` against an admin and a call manager with a call up, held to the guide by `the_guide_quotes_the_examples_as_they_are` — the developer surface complete
- **docs/API_TOKENS.md**: API tokens and permissions — the static token, sessions and tokens; admin and personal tokens; who may grant what; the maker answering for a token; revoke and rotate; the route-to-permission table and who holds the new permissions; what changed on upgrade
- **docs/IDENTITY_AND_DEVICES.md**: Identity and devices (designed 2026-10-01) — sign-in by password and a hand-made JWT keyed by username, users that own nothing the database knows, provisioning no phone can authenticate to, no firmware; the model (users by id with identities, per-tenant OIDC/SAML with login domains, sessions as rows, a user lifecycle with groups, SCIM 2.0, a tenant provisioning URL with vendor file names, redirection services, real templates, managed firmware), nine phases, twelve questions (agreed 2026-10-01 on their recommendations); phase 1 done (v0.25.308): users by id and sessions as rows — `fcp_core::session` (`SessionKeys` with key ids, `LoginSessionStore`, `authenticate_session` shared with the events server), migration 090 (`users.id` unique, `login_sessions`), the `fcp_refresh` HttpOnly cookie and `/auth/refresh` before the bearer layer, `[api.session]`, API tokens' makers by id, sign-ins audited and published (`user.signed_in`, `user.sign_in_failed`), `GET /users/by-id/{id}`; rename moved to phase 5; phase 2a done (v0.25.309): `tenants.login_domains` (platform-admin-set, unique) and `sign_in_json` (migration 091), `POST /auth/discover`, login by email (`find_users_by_email_in_tenant`), `password` optional with a generated SIP password, `PasswordLogin` enforced after the password checks (the API takes only `allowed` until phase 3); phase 2b done (v0.25.310): TOTP — `fcp_core::totp` (RFC 6238), `user_totp` (migration 092, the secret sealed), a password answering a five-minute `mfa_token` (a claim with a `purpose`) for `/auth/login/totp` or `/auth/totp/{enroll,confirm}` under `sign_in.require_totp`, `/me/totp`, `DELETE /users/{username}/totp`, wrong codes toward the lockout — phase 2 complete; phase 3a done (v0.25.311): OIDC sign-in (`docs/SSO.md`) — `fcp_core::identity` (providers with a sealed client secret, `user_identities` by `sub`, sign-ins in flight), migration 093, `fcp_api_core::oidc` (discovery and JWKS through the webhook guard, PKCE, RS256/ES256 verified with `rsa`/`p256`), `/auth/sso/{id}/start`, the callback bound by an `fcp_sso` cookie, `/auth/sso/exchange`, link by verified email, just-in-time creation, roles by group, `identity_provider_manage` and `/identity-providers`, `password_login` restrictable once a provider exists, `[api.sso] public_url`, proved against `fcp_testkit::oidc::StandInIdp`; phase 3b done (v0.25.312): back-channel logout (`/auth/sso/{id}/backchannel-logout`, sessions ended by the provider's `sid` or the person, migration 094), providers and identities synced to sites (secret sealed with the site's key; no JIT and no roles from groups at a site), `required_claims` and `email_verified_claim`, guides for Entra ID, Okta and Google — phase 3 complete; phase 4a done (v0.25.313): SAML sign-in — `ProviderKind::Saml` by the IdP's metadata XML, `fcp_api_core::saml` over `samael` behind the `saml` feature (libxmlsec1; the admin image and CI carry it), signing certificate required, SHA-256+ only, audience required, transient NameID refused, assertion ids remembered (migration 095), `/auth/saml/{id}` (FCP's metadata) and `/auth/saml/{id}/acs`; phase 4b done (v0.25.314): FCP's own key per SAML provider (sealed as its secret, rotatable at `…/sp-key`) signing AuthnRequests and LogoutResponses, encrypted assertions decrypted inside a signed response, single logout at `/auth/saml/{id}/slo` (Redirect verified over the raw query, POST reduced to its signed parts, `SessionIndex` read directly), metadata by URL with `…/refresh-metadata`, SAML guides for Entra ID, Okta and Google — phase 4 complete; phase 5a done (v0.25.315): one user status (`docs/USERS.md`) — `User::is_suspended`/`may_use_services`/`normalize_status` (migration 096), Suspended refused by SIP, devices of a suspended owner, routing, hoteling, provisioning and the voicemail PIN (Locked stops only web sign-in), `status_changed` dropping registrations, ending hoteling and publishing `user.disabled`/`user.enabled`, invalid status 400, `changed_fields`, `max_users` enforced, `list_users_in_tenant` in SQL with `GET /users?q=&status=`; phase 5b done (v0.25.316): a delete deprovisions — `AccountStatus::Deprovisioned` and `deprovisioned_at` (migration 097), the tenant's `offboarding` (`grace_days`, `meetings_owner`), `/users/{username}/restore` and a platform admin's `/purge`, `fcp_core::offboarding::OffboardingStore::purge_user` as one transaction (devices unassigned, lines and extensions released, the mailbox kept off its line with its FK now `SET NULL`, others' settings forgetting the user, meetings moved, logged as `offboarding`), `fcp_api_core::offboarding` with the hourly sweep in `fcp-admin`, an open erasure first, the link purging a site's own rows; phase 5c done (v0.25.317): groups — `groups`/`group_members` (migration 098, journaled), `fcp_core::group`, roles adding up (own + groups', `roles_have_permission`; deviation from Q7's "highest wins"), group roles read per request via `CredentialStore::group_roles_of_user`, `group_read`/`group_manage`, `/groups`, `idp_group` membership set at SSO sign-in, a queue's `agent_group_id` for `*50` login, the Groups page; phase 5d done (v0.25.318): renaming — the SIP identity separate from the login name (`users.sip_username`, migration 099, `User::sip_name()`), SIP lookups through `get_user_by_sip_name`, `PUT /users/{username}` with `username` (`rename_user`, `user.renamed`), `put_user` renaming by id so renames reach sites — phase 5 complete; phase 6a done (v0.25.319): SCIM users (`docs/SCIM.md`) — `fcp_api_core::scim` at `/scim/v2` (Users with one-`eq` filters, paging, PATCH applied to the SCIM document then taken as a replace, ETags, discovery), `scim_provision`, `users.scim_managed`/`scim_external_id` (migration 100), `sign_in.scim` turning off JIT and signing in only SCIM-made users; phase 6b done (v0.25.320): SCIM groups — `/scim/v2/Groups` over the tenant's groups (`groups.scim_external_id`, migration 101), members by user id as Entra ID and Okta patch them, a group's role on FCP's group extension, setup guides — phase 6 complete; phase 7a done (v0.25.321): the tenant's provisioning URL (`docs/PROVISIONING.md`) — `fcp_core::provisioning` (a key sealed and found by hash, migration 102; vendor file names; Polycom's master file; `TenantProvisioning` with device credentials and networks; the fetch log), `GET /provision/{key}/{file}`, `/provisioning/key`, `/devices/{id}/fetches`; phase 7b done (v0.25.322): templates in Jinja (`fcp_core::config_template::render` over minijinja, `${name}` still read, XML escaping by vendor, `TemplateContext`), picked by vendor and model (`pick_template`, `fcp_core::provisioning::identify`), `[api.provisioning]` for what phones are told, Fanvil and Snom starters, devices' `vendor` and `line_keys`, pending phones claimed (`/provisioning/pending`, migration 103), `/devices/import` all or none, `/devices/{id}/render` and `/config-templates/preview`, the Phone templates page; phase 7c done (v0.25.323): registrations written on devices (`fcp_sip::device_sync`, matched by credentials, MAC or the address the device fetched from, `DeviceStore::record_device_registration`), resync per device and template through a call manager (`/devices/{id}/resync`, the vendor's NOTIFY), `sip_domain` in templates, the old `/api/v1/provisioning/{mac}` route removed, proved by `fcp-admin/tests/provisioning.rs` — phase 7 complete; phase 8a done (v0.25.324): zero-touch — `fcp_api_core::redirection` (`RedirectionConnector`, `YealinkRps` signed HMAC-SHA256 per its guide, `PolyZtp` over its XML API), accounts per tenant and service sealed at rest (migration 104), devices registered with their vendor's service when made, claimed or imported and removed when deleted, `[api.redirection]`, `/provisioning/redirection` and `/devices/{id}/redirection`, DHCP 66/160 documented, tested against `fcp_testkit::redirection` stand-ins; 8b (Grandstream GDMS, Fanvil FDPS) deferred: no published API; phase 9a done (v0.25.325): firmware — `fcp_core::firmware` (images, per-tenant targets with a pilot stage, `target_for`, `FirmwareStatus`, migration 105), upload by the platform administrator to `[api.firmware] directory`, `/provision/<key>/firmware/<image>/<file>`, `firmware_url` in templates and Polycom's master file, the device views' `firmware_status`, `device.firmware_behind`, the Firmware page; phase 9b done (v0.25.326): images in an S3-compatible bucket (`[api.firmware.s3]`, `fcp_api_core::firmware_files::FirmwareFiles`, `fcp-admin`'s `S3Files` over the CDR archive's `S3Target` with new `get`/`delete`); phase 9c done (v0.25.327): firmware at sites — targets journaled and synced with their images (migration 106), the link fetching each image over `GET /link/v1/firmware/<image>` into `[site] firmware_dir` checked by SHA-256 (`fcp_link::firmware`), the site admin serving it, the site compose `site-firmware` volume — phase 9 complete

- **docs/BRANDING.md**: A tenant's brand for its softphones (softphone F3) — assets uploaded as the body and named by SHA-256, `PUT /api/v1/branding` and what it checks, `/brand/v1` (Anvil's contract v1, bearer only, ETag/304, 404), assets public by hash, kept in Postgres (migration 109), synced to sites with the link fetching assets by hash
- **docs/SOFTPHONE_VIDEO.md**: Video in the softphone (designed 2026-11-08) — one-to-one video calls and Anvil as a client of FCP's video rooms: an anchored call sets video to port 0 today (`AnchorPorts { video: None }`), and every Anvil call is anchored; the model (SIP through the call manager for calls and rooms, FCP relaying each video section packet for packet with RTCP passed through, Anvil's `VideoHost` trading coded frames with platform hardware codecs, a REMB estimator, RFC 4575 rosters, the content floor by INFO, a host's controls under `/me`; SIP for the rooms FCP has, with Anvil's video independent of the transport — streams not "the call's video", encodings per source, a pluggable estimator — because per-participant forwarding rooms, over WebRTC, are to be supported later), phases F1–F2 and A1–A5, ten questions (agreed 2026-11-08 on their recommendations); F1a done (v0.25.539): `fcp_media::VideoRelay` per video section of an anchored call (a socket pair per leg, SDES per leg, RTCP passed through, latching), `AnchorKeys`, `splice_relayed_video`, `with_video_sdes`, `MediaManager::anchored_offer`/`video_answered`/`anchored_answer`/`secure_video_offer` on the B-leg offer, the callee's answer and re-INVITEs, `fcp_testkit::video::VideoLeg` and `Phone::enable_video`; F1b done (v0.25.541): video paused through hold, music and park and resumed with the call, offered through transfer, retrieve, find-me and forwarding on no answer, the tenant's `video` (migration 139) and a class's `video` deciding per call (`MediaSession::set_video_allowed`), `/me/softphone`'s `video` block — F1 complete; A1–A3 done in Anvil (v0.2.36–67: the core, desktop and mobile video); F2a done (v0.25.542): the conference roster over RFC 4575 (`fcp-conference-server/src/roster.rs`). A participant subscribes inside their call or names it in `Target-Dialog`, and anyone else gets 403. The first NOTIFY is the full roster and later ones are versioned partial updates: names, roles, audio, camera, muted, `fcp:speaking` and `fcp:presenting`. They are sent in the subscription's dialog, compared every 500 ms and on room events. The test kit gains `Call::target_dialog` and `Phone::subscribe_with`. F2b done (v0.25.544, siphon-rs v2026.11.10): `X-FCP-Join` and `Recv-Info: fcp-content` carried to the room by the call manager, the floor INFO (`Info-Package: fcp-content`) to a SIP client that asked, the roster SUBSCRIBE/NOTIFY and floor INFO relayed in the call (`fcp_sip::conference_relay`, down a TLS flow), a token dialed at another room refused, `/me/conferences/live/{room}/…` for a meeting's owner or a host token — F2 complete. v0.25.547 (for A4c): a re-INVITE turning the shared screen's section `recvonly` releases the presenter's floor at once, and turning it sending again after a stop is a new share (`content_change` in `conference_handler.rs`). A4 done in Anvil (#86–#91, v0.2.68–80; FCP #388–#390 for rooms with video, the floor freed by re-INVITE, a room's re-INVITE and codec), A5 (mobile screen share: MediaProjection, ReplayKit) done (#92–#93, released v0.2.82): all phases done 2026-11-12; macOS and Windows capture type-checked only, not yet run by hand
- **docs/FORWARDING_ROOMS.md**: Forwarding rooms (designed 2026-11-13) — video rooms that forward each participant's own stream beside the composite, chosen per subscriber: streams by a stable SSRC per person in the existing camera section over SIP (the roster's `src-id` says whose), `Recv-Info: fcp-view` and a view INFO, simulcast in by RFC 8853 rids, an allocator over REMB, composing only while a phone, recording or trunk needs it, browsers through K gateway transceivers, an egress budget; phases R1–R5, R7 and A6, twelve questions (all agreed 2026-11-13); R1a done (v0.25.555, forge-media v2026.11.13): a SIP camera's simulcast taken by the conference server (`VideoAnswer.simulcast`, `SIMULCAST_LAYERS`), forge-conference's `add_source_layered` decoding the smallest layer that fills the tile, `layers` in the video JSON; R1b done (v0.25.556, forge-media v2026.11.13.1): browsers — forge-webrtc 0.9.0's `VideoConfig::simulcast`, the gateway asking for `h`/`m`/`l` and offering the room the browser's layers under its own rid extension id, the join page sizing them, proved against Playwright's Chromium (`a_browser_s_camera_reaches_the_room_as_simulcast`); R2 done (forge-media v2026.11.13.2): forge-conference 0.16.0's forwarding viewers — `VideoRoom::forward`/`set_view`, a stable SSRC per stream, the allocator over REMB, VP8 indices rewritten and the sender's extensions stripped, nothing decoded while nothing composes; R3a done (v0.25.558–559): the conference server and the call manager carrying `fcp-view` — `video_mode` (`mix`/`auto`, migration 140), `Recv-Info: fcp-view` answered in kind, the `fcp-view` INFO (`fcp_core::video_view`), each subscriber's roster with `<src-id>`, `fcp:paused`, a `speaker` media and `fcp:spotlight`; R3b done (v0.25.561, forge-media v2026.11.14): the egress budget — `[conference] video_egress_mbps` priced in `room_cost`/`join_cost` and admission, on the host's cluster budget line and in placement — no decode charged where nothing composes (`VideoSourceInfo::decoding`), the test kit sending three rids and REMB, and `fcp-loadgen`'s `what_a_forwarded_stream_costs` (about 13 µs an SRTP forwarded packet, all-in: one core forwards about 1,300 streams at 500 kb/s, so the link is the limit); A6 done in Anvil (#95–#98, v0.2.84–92: the view and roster, simulcast send on every platform, the app's own layout); R4a done (forge-media v2026.11.16: forge-webrtc 0.10.0's `VideoConfig::streams`, send-only sections with an SSRC each); R4b done (v0.25.566): the gateway's `streams` join, `Recv-Info: fcp-view`, each room SSRC on a free browser track (`fcp_webrtc_gateway::forwarding`), the roster followed in the call (`fcp_core::conference_info`), `forwarding` on the session and `POST /v1/rooms/{id}/view`; R4c done (v0.25.568–569, forge-media v2026.11.16.1): the join page's own grid (a tile per member, names, initials, speaking rings, a pin), the view posted as it changes, proved in Chromium; forge-rtp 0.7.4 lays SRTCP under AES-GCM out as RFC 7714 says, so a browser's RTCP reaches the gateway at last; R7 done (v0.25.570–571, forge-media v2026.11.16.2): forwarding across nodes — `X-FCP-Trunk-Forward` on the trunk, each end a forwarding viewer of the other (forge-conference 0.16.2's `forward_trunk`), `POST /internal/conferences/{room}/forwarding` announcing members, whose each SSRC is and what is wanted, the peer's people as remote members in the room and the roster (`fcp-conference-server/src/trunk_forwarding.rs`)
- **docs/TRANSPORT_CC.md**: Transport-cc for forwarding rooms (R5, proposed 2026-11-16) — what REMB cannot do (see above what arrives, judge per packet, live in one place), `forge-bwe` (the GCC core moved from Anvil, REMB and send-side estimators, a feedback writer, a prober, a simulated bottleneck), stamping per viewer after the NACK cache, the send-side estimate while feedback is fresh else REMB, probing only for a held-back upgrade, the gateway translating each side's feedback into the other's numbers so the room estimates and probes the whole path to a browser (Q5 revised 2026-11-16 from ending it and telling the room by REMB), feedback for uplinks, no trunk estimation; phases R5a–R5d with a decision point after R5a's simulation, nine questions (agreed 2026-11-16); R5a done (v0.25.573–574, forge-media v2026.11.16.3: forge-bwe 0.1.0, forge-rtp 0.8.0): the simulation found a REMB viewer never recovers from a dip (held to 1.5× what arrives), transport-cc's probes recover in ~9 s and get under a falling capacity 1.8× as fast — the decision point passed; R5b done (forge-media v2026.11.16.4: forge-conference 0.17.0 numbering, send-side budget and probes paced on the clock's runtime, forge-webrtc 0.11.0 negotiating it); R5c1 done (v0.25.575–576): fcp-media's answer keeps transport-cc (`VideoAnswer::transport_cc`), the conference server wires it into viewers, composites and senders, `estimate`/`budget_kbps` in the video JSON, the test kit's `Phone::set_video_transport_cc` and `VideoLeg::shape`; R5c2 done (v0.25.578): the gateway's `transport_cc::Downlink` renumbers the room's packets for the browser and rewrites the browser's FMT 15 into each SIP section's room numbers (SIP id 5), the gateway writes the browser's uplink feedback itself, proved through the call manager's relay and in Chromium (no REMB once agreed); R5d done in Anvil (#100, v0.2.93: forge-bwe's estimators, transport-cc on the camera's section — the screen's would push an INVITE past UDP's 1,300 bytes) — R5 complete
- **docs/WEBINARS.md**: Webinars and town halls (agreed 2026-11-17 on every recommendation, plus chat in every room) — what a room lacks for an audience (two roles, an N−1 mix and encode per listener, a roster of everyone capped at 1,000, a full-mesh cascade, no raise hand, Q&A or streaming out, join tokens per room not person); the model: a meeting `kind = webinar` with Panelist and Attendee roles, one shared audience mix encoded once and sent many, the stage's composite and forwarded streams for the audience, rosters with the audience as a coalesced count, raise hand (`*4`), moderated Q&A (`fcp-qa`) and chat in every room (`fcp-chat`, host-set scope, delete and mute, kept under retention), practice/live/ended with a lobby let in gradually, registration per registrant and attendance from CDRs, a stage node and one-way edge trunks, HLS from forge-mp4's CMAF then RTMP; phases W1–W7 (W3b chat) and A7, twelve questions; W1 done (v0.25.582–583, forge-media v2026.11.17: forge-mixer 0.6.0's listeners, forge-conference 0.18.0's `add_listener`/`listen_audience` — mixed once, encoded once per flavor): `kind`/`panelist_pin`/`panelists` (migration 141), `JoinRole::Panelist`/`Attendee`, a panelist minting their own token, attendees admitted as listeners by a guest token, the guest PIN or `#`, the audio bridge sending the feed and switching to N−1 when promoted, `audience` and `role` in the room JSON; measured by `fcp-loadgen/tests/webinar.rs`: 1,000 attendees 29.8% of a core (a meeting of 1,000: 185.5%), 2,000 at 70.5%; W2 done (v0.25.584–585, forge-media v2026.11.17.1: forge-conference 0.19.0's audience — never drawn or forwarded, one composite of the whole room for all of it): attendees' video answered `sendonly` (`fcp_media::with_video_sending_only`), no source and never the floor, the roster listing the stage with `fcp:audience` coalesced to one count-only NOTIFY per 2 s, `audience_join_cost` in admission, a join's video ports bound before the answer and 503 when none are left; measured: 1,000 on the composite 57.7% of a core, 2,000 at 133.4%, 1,000 forwarded 81.1% (8.4 µs a packet: the link fills before the CPU); W3.1 done (v0.25.586–587, forge-media v2026.11.17.2: forge-conference 0.20.0's `*4`): hands by `*4`, the `fcp-qa` INFO (`fcp_core::qa`), `/me/conferences/live/{room}/hand` by call and the gateway's `/v1/rooms/{id}/hand`; a host lowering one or all and bringing an attendee to the stage and back (`set_listener`, `…/participants/{pid}/stage`); raised hands on the stage's rosters as `attendee` with `fcp:hand`; `conference.hand_*`/`stage_*` events; relayed by the call manager and the gateway (`conference_handler/hands.rs`); W3.2 done (v0.25.588): questions — a meeting's `qa` (`off`/`moderated`/`open`, unset by kind) and `qa_anonymous` (migration 142), asked and upvoted by `fcp-qa` INFO, `/me/conferences/live/{room}/questions` by call and the gateway's `/v1/rooms/{id}/questions`, published, answered and dismissed by a host or panelist, each participant told what they may see once a second (`{"op":"questions"}`), `webinar_questions` rows, `conference.question_*` events (`conference_handler/questions.rs`) — W3 complete; W3b done (v0.25.591): chat in every room — a meeting's `chat` (`everyone`/`hosts`/`off`, unset by kind; migration 143), posted by `fcp-chat` INFO (`fcp_core::chat`), `/me/conferences/live/{room}/chat` and the gateway's `/v1/rooms/{id}/chat`, deleted and muted by a host or panelist, sent every 250 ms with the last 100 to a late joiner, `conference_chat` rows under retention, holds, subject export and erasure (migration 144), `conference.chat_deleted`/`chat_muted` (`conference_handler/chat.rs`)
- **docs/APP_SIGN_IN.md**: Signing in an app (softphone F1a) — OAuth 2.0 for native apps with PKCE: `/auth/app/authorize` to the admin UI's sign-in, `/auth/app/approve`, `/auth/app/token` (code and refresh), `/auth/app/sessions` for an app without a browser, `/auth/app/revoke`; app sessions (rotated refresh tokens, `sign_in.app_session_days`, one per install), errors
- **docs/SOFTPHONE.md**: The softphone (designed 2026-10-05) — Anvil (`thevoiceguy/anvil`, Rust, on siphon-rs and forge-media) as FCP's own client: what FCP lacks (browser-only sign-in and a cookie refresh, no SIP account for a user, no branding, no `/me` history/directory/events) and what Anvil lacks (stale stack APIs, registration that does not last, 405 to NOTIFY/REFER/SUBSCRIBE, stubbed branding); the model (the app as a device with its own SIP credentials, OAuth 2.0 for native apps with PKCE and a loopback redirect, rotated app sessions, `/me/softphone`, Anvil's `BRANDING.md` as the contract, `.well-known` discovery, `anvil-fcp`), the proof across repos on the nightly images, phases A0–A5 and F1–F4, eleven questions
- **docs/APP_SIGN_IN.md**: Signing in an app (softphone F1a) — OAuth 2.0 for native apps with PKCE: `/auth/app/authorize` to the admin UI's sign-in, `/auth/app/approve`, `/auth/app/token` (code and refresh), `/auth/app/sessions` for an app without a browser, `/auth/app/revoke`; app sessions (rotated refresh tokens, `sign_in.app_session_days`, one per install); the install's device (F1b: its own SIP account returned at each sign-in, `/me/devices`, removal ending its session and registrations); what the app is told (F2: `/me/softphone`, rotate-password, `/.well-known/fcp-provisioning`, `X-FCP-Provisioning-Url` from `[registrar] provisioning_url`), errors
- **docs/PUSH.md**: Push (designed 2026-10-09) — ringing a sleeping phone app: RFC 8599 push addresses in REGISTER (555, Feature-Caps, long-lived bindings by instance), APNs and FCM senders (`fcp-push`, `[push]` with the publisher's keys), a call pushed and held for the woken app's re-REGISTER within the ring time, cancel pushes, a flow on another node reached through its node, sites pushing through the cloud; phases P1–P4 (fcp) and A1–A3 (anvil), eight questions (agreed 2026-10-09 on their recommendations); phase P1 done (v0.25.377): `+sip.instance`/`reg-id` kept and an instance's new binding replacing its old, `pn-*` stored beside the contact (migration 111, `fcp_core::push`, `fcp_sip::push_address`), 555 with `Feature-Caps`, a week-long push binding past siphon-rs's day, `flow_node`, a device's `push_provider`, `[push]` on the call manager and the standalone registrar; phase P2 done (v0.25.378): `fcp-push` (APNs HTTP/2 with an ES256 provider token, FCM HTTP v1 with a service account; once, five seconds), a gone token taking the push address off its binding and `device.push_failed`, `fcp_push_total`, `POST /api/v1/devices/{id}/push-test` through the first call manager with `[push]`, stand-ins `fcp_testkit::push::{StandInApns, StandInFcm}`; phase P3 done (v0.25.379, siphon-rs v2026.10.09): wakeable bindings pushed instead of rung (`split_wakeable`), a 180 while only woken apps ring, `fcp_sip::push::PushWaits` and `registration.registered` handing a woken app's new binding to its fork (`LateLegs`, `FuturesUnordered`), cancel pushes for apps that never came back, unreachable at once when nothing can be woken; testkit `Phone::new_app`/`set_push`/`sleep`; phase P4a done (v0.25.380): a flow on another node — `FlowRegistry::set_owner`, `classify_bindings` ringing through the flow's node (`X-FCP-Flow`, `X-FCP-Flow-Contact`, `X-FCP-Call-Id`; members' SIP addresses via `with_node_sip_address`), the receiving node's leg `Call::relay_for` (no record, events or parties), 430 Flow Failed for a gone flow, deferred push targets pushed on that 430 (`PushWaits::undefer`/`forgo`), woken apps back at another node rung through it; two-node tests in `push.rs`; phase P4b done (v0.25.381): a site's pushes through its cloud — `[push.relay]` on the site's call manager (apps listed without keys), the link's `/push` behind `[site] push_token`, `POST /link/v1/push` held to the site's tenant, the cloud's call manager's `POST /push`, the outcome back down so a gone token is let go at the site; `fcp-link/tests/push.rs`; A1 done (anvil #25); v0.25.382: every INVITE to a registered phone names its call in `X-FCP-Call-Id` as the push did, so a woken app knows which INVITE is the call on its screen
- **docs/DEEP_TESTING.md**: Deep testing (agreed 2026-10-13, every recommendation) — the bugs per-feature tests could not find: our phone sharing the server's SIP stack, 87 of 412 API operations never called (measured in D1), features tested one at a time, faults mid-call, nothing running for hours, tenant isolation checked only where remembered, a UI that only has to compile, hostile SIP never sent to a binary; phases D0–D8 (nightly health, measure, SIPp/baresip, every route/tenant/role, interactions, faults, soak, Playwright, the real world by hand), ten questions; D0 done (v0.25.394): the admin's startup stack overflow, images on Rust 1.98, failed scheduled runs opening a `nightly-failure` issue; D1 done (v0.25.395): `FCP_ROUTE_LOG` and `fcp_api_core::route_log`, `scripts/check-routes-called.py` failing CI on an uncalled operation not in `scripts/routes-uncalled.txt`, the weekly `coverage.yml`; D2a (v0.25.396): SIPp against the call manager (`fcp-call-manager/tests/sipp.rs`, `fcp_testkit::sipp`, `tests/sipp/scenarios.py`, `FCP_SIPP_TESTS=required` in CI) — CANCEL racing the answer, CANCEL before the call exists, CANCEL after the answer, 481 for no dialog fixed; siphon-rs `CallHandle::invite_outcome`; v0.25.397: `[registrar] auth_invites` on by default (own legs via `fcp_sip::own_leg`'s per-process token, loopback trusted in the single-node file, upgrade note `BACKUP_AND_RESTORE.md` §5.4.1); D2b (v0.25.398, siphon-rs v2026.10.14): SIPp inside a call — UPDATE, reliable ringing (the stack offers 100rel and PRACKs, the node relays ordinary provisionals and answers PRACK 481; a caller requiring 100rel is #310), a node-run blind transfer, session timers (422 under Min-SE, the caller's interval and refresher kept, an expired session ending both legs through the serving handler, the non-refresher's BYE before expiry)
- **docs/NAT_AND_TLS.md**: Phones on any network (agreed 2026-10-24, every recommendation) — D8 found one-way and no audio for two phones behind one router, with and without a SIP ALG: forge-engine gives a call one RTP port for both legs and guesses a packet's leg from its source; the model (a socket pair per leg as rtpengine/FreeSWITCH do, sockets moving with a parked phone, the transcoder's timestamp rescaled; TLS that lasts — flows toward callers and for NOTIFYs, pongs, keepalives, idle timeouts, certificate reload — and TLS by default for Anvil and the templates, SRTP never keyed over UDP); phases N1 (forge-media), N2 (fcp), T1, T2, A1 (anvil), then D8 by hand; eight questions; N1 and N2 done (v0.25.459, forge-media 2026-10-24: forge-engine 0.8.0): `forge_engine::LegSockets`, a packet's leg the socket it arrived on, `swap_leg_sockets` from fcp-media's `move_peer`, `MediaSession::ports_for(Leg)` at every fcp-sip site, the transcoded timestamp on the receiver's clock, `calls.rs` `two_phones_behind_one_address_*`
- **docs/RECORD_AND_CONTACTS.md**: Record on demand and personal contacts (D8 findings 20 and 21, agreed 2026-10-26) — `media_preferences.record_on_demand`, `/me/calls/{call_id}/recording/{start,stop}` checked at the node (`for_user`, the `on_demand` source, only the starter stops it), the caller's 200 OK naming the call (`X-FCP-Call-Id`), `may_record` on `/me/softphone`; personal contacts local in Anvil and on FCP (`fcp_core::contact`, `/me/contacts`, synced both ways with a site — down by the journal, up by the link's shipping, each side taking only a strictly newer copy so an echo is let go, deletes as tombstones swept after 30 days); four phases, phase 1 done (v0.25.466), phase 2 = anvil #48 (v0.2.14), phase 3 done (v0.25.467)
- **docs/PRESENCE.md**: Presence a person sets (D8 finding 33, agreed 2026-10-29) — presence per person, not per address and not only in memory: the chosen status (four built-ins and up to 20 of the person's own, a note, `until`) on the account, with calls, DND (one switch both ways), idle from the apps and offline from the registration store layered over it; `presence.user_changed`, `/me/presence`, the admin working it out per tenant, phones fed by person, queues and hunt groups skipping DND and calls only; three phases (FCP person, FCP phones, queues and sites, Anvil); phase 1 done (v0.25.473): migration 125, `fcp_core::presence_status`, `/me/presence` and statuses, `fcp_api_core::people_presence` in the admin (registrations from the store, `presence.user_call` from the call managers, DND, idle, `until`), `presence.user_changed` on `/me/events`, the directory and the admin read; phase 2 done (v0.25.474): `presence.user_changed` posted to the events server once, `fcp_sip::person_presence` laying it on each person's addresses as PIDF with RPID and their label as the note (read from the database for a first watcher), queues and hunt groups skipping DND and calls only, PIDF parsing reading RPID activities, both tables synced with sites (migration 126: journal, `ship_seq`, tombstones, a cleared status as an `available` row)
- **docs/SERVICES.md**: Seeing every service (D8 finding 34, agreed 2026-10-29) — the Cluster page lists opt-in cluster members, so D8 showed one node of eighteen; the model: a service beacon from every binary with a Redis, apart from membership (`fcp:service:<id>`: kind, version, state, checks of its database, Redis and dependencies, load), the admin's kept record (`service_instances`) with `system.service_up`/`_down` and metrics, a site's services up its link; a System section with a Services page; phases S1–S3, five questions (agreed 2026-10-29 on their recommendations); S1 done (v0.25.476): `fcp_cluster::ServiceBeacon` in all fifteen binaries (`fcp:service:<id>`, `fcp_core::service_health`, checks by `Check::tenants`/`database`/`http`, `stopping` on shutdown), `/ready` answering from the required checks (`Drain::set_health`); S2 done (v0.25.477): the admin's record (`service_instances`, migration 127; `fcp-admin/src/services.rs` reconcile and the sweep under `fcp:services:sweep`), `system.service_up`/`_down`, a site's beacons up its heartbeat (`SiteHeartbeat.services`), `/api/v1/system/services` (list, summary, one, forget) for platform administrators, `fcp_service_up`/`fcp_service_check_failing` through `fcp_api_core::MetricsSource`; S3 done (v0.25.478): the admin UI's System section (Services, Cluster — both platform-only — Sites, System Settings, Audit Log, Compliance), the Services page and a service's page with Forget, the dashboard tile, the cluster routes behind `require_platform_admin` — complete
- **docs/PARKING.md**: Parking lots (D8 finding 35, agreed 2026-10-30) — the node-wide 701–720 shared by every tenant and the two-node orbit collision; lots per tenant, each with its own range of digit strings checked against the dial plan, slots claimed cluster-wide per tenant, directed park, dialable slots, timeouts with a fallback, the Parking page with a live board, BLF on slots; four phases, six questions (agreed on their recommendations); phase 1 done (v0.25.483): `fcp_core::parking`, `parking_lots` (migration 128), `/api/v1/parking/lots` under `parking_read`/`parking_manage` with the dial-plan clash check, `*70` and `park_number` through the tenant's lot (else `[pbx.parking]`), slots claimed with `SET NX` on `park:{tenant}:{slot}`, `*71<slot>` within the tenant, `slot` on `call.parked`/`call.unparked`; phase 2 done (v0.25.484): a slot dialed retrieves, a transfer to a slot parks there (486 when taken), the timeout's parker ring shown "Parked <slot>" then the lot's fallback (`TransferRun.fallback`), the lot's music, `call.park_timed_out`, the node's `/parking/calls` and the API's `/api/v1/parking/calls` (list, retrieve-to, hang up) through `CallSupervisor`; phase 3 done (v0.25.485): the Parking page (lots and a live board), slots as presentities lit by `call.parked` (`slot_aors` in the call manager's presence listener), the starters' `park` keys as lamps, `e2e/parking.spec.ts`; phase 4 done (v0.25.486): lots journaled (migration 129) and synced to sites, `parking/lots` refused at a site's admin, a site's parked calls on its heartbeat (`SiteHeartbeat.parked`) and the cloud's board; the SBC on 8 MiB worker stacks — all four phases done
- **docs/DEVICE_LINES.md**: Lines on a device (D8 findings 40 and 42, agreed 2026-10-30) — a device's lines as on a desk phone (own, extension, shared, on behalf; up to ten, each registered with the device's credentials under the "may act as" rule), labels (the admin's, `<extension> - <name>` by default, a user's rename only under the tenant's `lines.users_may_rename`), provisioning's accounts and `/me/softphone`'s `lines` from them, Anvil's many registrations; three phases, four questions (agreed on their recommendations); phase 1a done (v0.25.487): `Device.lines` (migration 130), `fcp_sip::device_lines` resolving and checking each line, "may act as" for an executive's line, a user's names by line in `user_line_labels` under the tenant's `lines.users_may_rename`, `PUT /me/lines/{line}/label`, provisioning accounts and `/me/softphone` `lines` from them; phase 1b done (v0.25.488): `X-FCP-Line` (a line's key: `own`, `extension:…`, `shared:…`, `on_behalf:…`) on every INVITE to a user's phone, MWI per line — FCP's phase 1 complete; phase 2 = Anvil v0.2.24–25; phase 3 (v0.25.489): `barge_in` on `/me/softphone`'s lines, Anvil watching `call-info`, picking up and joining appearances — all three phases done
- **docs/CALL_DIAGNOSTICS.md**: Call diagnostics in the app (D8 findings 43 and 44, agreed 2026-10-30 for captures) — live call stats in Anvil (RTCP, loss, jitter, round trip, an E-model MOS) and a call's quality kept on FCP; captures an admin allows, the user takes, as pcapng with SIP before TLS and the SRTP keys, kept locally and uploaded to FCP under retention and holds; five phases, five questions; phase 1 = Anvil v0.2.27 (RTCP, no forge-media change), phase 2 = Anvil v0.2.28 (the panel, G.107 without concealment); phase 3 done (v0.25.490): `call_quality` (migration 131, one row per phone), `POST /me/calls/{call_id}/quality` from the app's session, `GET /cdrs/{call_id}/quality`, `quality` on `/me/calls`, `call.quality_poor` under 3.1, gone with its record at retention; phase 4 done (v0.25.491): captures — the tenant's `diagnostics` (`capture_allowed`, `capture_max_mb`, `capture_keep_days`), allows by user or device for at most 30 days (`/v1/captures/allows`), `capture` on `/me/softphone`, `POST /me/devices/{id}/captures` (pcapng, 413 over the limit) into `[api.captures]` (a directory or a bucket), `/v1/captures` and audited `/captures/{id}/file`, `device_capture` a legal-hold class, the hourly sweep and a trigger-fed file queue (migration 132); phase 5 = Anvil v0.2.30 (anvil #64): pcapng with SIP before TLS and media outside SRTP (no SRTP secrets block exists), the next call or every call, uploaded at hangup — all five phases done
- **docs/TENANT_SCOPE.md**: Every tenant its own system (D8 findings 48–61, agreed 2026-10-31, every recommendation) — the review found tenancy in the model almost everywhere (trunks, schedules, routing rules, flows, classes of service, LCR, parking, webhooks, sites, locations, tokens), hidden by a picker a signed-in platform administrator cannot use (`TenantContext` prefers the session's home tenant) and seven pages that send no tenant; the real gaps: system prompts only in the default tenant with no override and a cross-tenant play by id, audit rows filed under the actor's tenant, flow traces filtered after paging, `start_flow` unchecked, parking ignoring location, no per-location routing, legacy `admin` tokens; five phases (the picker, prompts, audit, flows/traces/parking/tokens, routing per location); phase 1 done (v0.25.495): `TenantContext` takes a platform administrator's `x-fcp-tenant-id`, the UI sends the picked tenant with every request, `TenantScope` (badge, "pick a tenant") on the seven pages, the Trunks page's platform-fallback card, Parking's Location, the route matrix's third probe; phase 2 done (v0.25.496): system prompts without a tenant (migration 133), seeded once for every tenant, looked up by the call's tenant (its override, then the system prompt, then speech) by name and by id, `POST /prompts/{id}/override` and Restore by deleting the copy, the Prompts page's System, Override and Restore; phase 3 done (v0.25.497): an audit row is the tenant acted on (`audit::acted_on`, `AuditScope`, a `/tenants/:id` route's tenant, none for platform routes), `actor_tenant_id` (migration 134), a tenant's readers seeing a platform administrator's actions on it as `platform administrator`, a platform administrator's `tenant=all|platform|<id>`, the Audit Log page's Tenant column and Rows filter; phase 4a done (v0.25.498): `start_flow` refusing another tenant's flow, flows without a tenant adopted by what names them or assigned (`/flows/untenanted`, `/flows/{id}/assign`), flow traces by tenant in SQL with a total, `*70` in the lot of the parker's location, tokens without a maker held to a tenant administrator, a number `assigned` while bound (migration 135); phase 4b done (v0.25.499): queues by tenant — `fcp_queue::{QueueKey, AgentKey}` keying every queue and agent map, names and extensions unique per tenant (409), every tenant's queues loaded, a queue's legs back to the node naming their tenant (`invite_for_tenant`, `X-FCP-Own-Leg-Tenant`), `can_handle` taking the call's tenant, `*50`/`*51`, DND, stats and agent restore per tenant, and a caller leaving now CANCELs the agent's ring (`cancel_ring`); phase 5 done (v0.25.500): routing rules per location — `routing_rules.location_id` (migration 136), a location's rules tried before the tenant's for its callers alone, the caller's location by device, extension, then line number (`caller_location`), the API's check, filter and 409 on a location's delete, the route preview's `location_id`, the Routing page's column, field and filter — all five phases done
- **docs/DEVELOPER_GUIDE.md**: Building on FCP through its API — a token for an integration, events by webhook, a flow that listens, a call controlled while it is up, errors and paging; each in TypeScript and Python, quoted from the SDKs' `examples/`, which CI runs
- **docs/CALL_CONTROL.md**: Live call control — the seven routes, their bodies and answers, the errors, how a request reaches the node holding the call, how a redirect reaches its flow
- **docs/FLOWS.md**: The Http node (requests, the response as dotted variables, errors), flow versions (kept, read back, what a call runs), the removed `webhook_url`, and menus (the Gather timeout from the prompt's end, retries with `max_attempts`)
- **docs/SPEECH.md**: Speech recognition — the providers and their keys, `[stt]` on a node and `[stt.providers]` on `fcp-tts-server`, the `/v1/transcribe` routes, voicemail transcription and `voicemail.transcribed`, the scripted recognizer tests use
- **docs/PROVISIONING.md**: Provisioning phones — the tenant's URL and key, each vendor's file names and content types, device credentials and networks, the fetch log; templates (which one a phone gets, the Jinja language, every variable, line keys, the starters, `[api.provisioning]`), pending phones and claim, importing by MAC, the registration on the device and resync, the old route removed; zero-touch through Yealink RPS and Poly ZTP, DHCP; firmware (images, targets, pilots, who is behind)
- **docs/SCIM.md**: SCIM provisioning — connecting a provisioner with a `scim_provision` token, what `/scim/v2` supports (filters, paging, PATCH, ETags, errors), how a SCIM user and group map to FCP's, a tenant whose users come from SCIM, and setup guides for Entra ID and Okta
- **docs/VOICEMAIL.md**: Voicemail greetings — standard, personalized (the owner's name spoken once by TTS, or their recorded name spliced in) or their own, the default by name, when the personalized greeting is made again, `*98`'s mailbox options, the one greetings directory the admin, call manager and voicemail server share
- **docs/USERS.md**: Users — one status (active, suspended, locked, deprovisioned) and what each refuses, what suspending does (sessions, registrations, hoteling, events), `enabled` kept for a release, `changed_fields`, `max_users`, listing in SQL; deleting a user — the grace period, restore, a platform admin's purge, and what the purge does to every store; groups — roles that add up, groups following an identity provider's, queues from groups; renaming — the SIP name that stays
- **docs/SSO.md**: Single sign-on — `[api.sso]`, a tenant's OpenID Connect provider (issuer, client, secret sealed), how a sign-in becomes a user (identity by `sub`, link by verified email, just-in-time creation, roles by group), the flow and its checks, the failure reasons, the password policy once a provider exists, a user's identities, back-channel logout, sites, guides for Entra ID, Okta and Google, SAML (the `saml` build, metadata, what is checked)
- **docs/WEBHOOKS.md**: Tenant webhooks — making one on `/api/v1/webhooks`, event filters, where a URL may point, the delivery body and `X-FCP-Signature`, verifying with the SDKs, rotation, test and replay, who delivers, the events server's and the admin's configuration
- **docs/SITE_SETUP.md**: Setting up a site — the customer operator's document: what to get from the cloud (addresses, CA, a one-time token), the site compose project's bring-up and the enrollment's log lines, phones, the site's admin and what it refuses, life without the cloud, the six volumes and what is re-derivable, backup and restore with `COMPOSE_DIR`, upgrade, decommissioning, faults, what the nightly proves
- **docs/NIGHTLY_STACK.md**: The nightly proves the shipped stack (agreed 2026-09-25) — fuzzing in both submodules never run in CI (a musl-built cargo-fuzz targeting musl) and unwatched, fourteen images built nightly and never run, tests sharing one Redis; four phases: fuzzing back and watched, the images pushed to GHCR and run by a `stack` job with the testkit's phone and a live backup/restore, a test's Redis flushed at acquisition, the findings written down; phase 1 (v0.25.265) has both fuzz workflows naming the host target and building before running, and FCP's weekly `submodule-nightlies` job watching them; phase 2 (v0.25.266) publishes the fourteen images to GHCR nightly and runs the stack from them with `compose_live.rs`, a backup and a restore — its first runs found and fixed seven bugs, from the proxy's health check to a first-boot race on the key ring; phase 3 (v0.25.287) empties a test's Redis database when the testkit hands it out; phase 4 (v0.25.288) writes down what the nightly found and fixes it (restore's Postgres race, the site step's `jq`, forge-bfcp's parser) — complete
- **docs/PRODUCTION_STACK.md**: The production-shaped stack (agreed 2026-09-24) — the leftovers ship-and-operate found: five binaries without images, no TLS, encryption at rest off, nested credentials no variable reaches, the routing engine's 500 for a stranger; five phases
- **docs/WEBRTC.md**: Browsers through the WebRTC gateway — join flow, join tokens, gateway configuration, ports
- **docs/GPU.md**: Video on a GPU — the `video-hwaccel` build, `[conference] video_device`, what runs on the device and what falls back, the saturation budget, placement, the NVIDIA image
- **docs/GPU_BOX.md**: The GPU development box for video phase 8 — a Spot L4 on GCP under `deploy/gcp-gpu`, up and down per session
- **docs/TEST_FLOOR.md**: End-to-end tests against the running binaries over real SIP and RTP — the scripted phone in `fcp-testkit`, the seed and binary locator, the five phases
- **docs/CAPACITY_AND_SURVIVABILITY.md**: Capacity and survivability design (agreed 2026-09-20) — the limits the API accepts and the call path ignores, admission on every routing binary, draining a node without dropping calls, the load harness, five phases
- **docs/TENANT_TRUNKS.md**: Tenant trunks design (complete, v0.25.205) — why the API's trunks do not carry calls today, one trunk model, selection per tenant with the platform's trunks as the fallback, inbound tenancy from the trunk, six phases
- **docs/CDR_PIPELINE.md**: The dedicated CDR pipeline design — the record, spool then ship, the processor in `fcp-jobs`, rating off the call path, partitions and rollups, the archive sink, the five phases
- **Phase1 plan.md**: Detailed MVP implementation plan with sprint breakdowns
- **IMPLEMENTATION_DOCS.md**: Index of implementation documentation for completed features
  - **Dial Node B2BUA**: Complete 4-phase implementation (outbound calls, response handling, media bridging)
  - **CDR System**: Automatic call detail record creation with REST API
  - **Webhooks**: General-purpose webhook dispatcher with HMAC signing, retries, and REST management API
- **external/siphon-rs/CLAUDE.md**: Comprehensive SIP stack documentation
- **external/siphon-rs/bins/siphond/README.md**: siphond daemon documentation
- **RFC Implementation Docs**: `external/siphon-rs/RFC_*_IMPLEMENTATION.md`

### RFC Compliance

Core RFCs implemented in siphon-rs:
- **RFC 3261**: SIP base (INVITE, ACK, BYE, CANCEL, REGISTER, OPTIONS)
- **RFC 3262**: PRACK (reliable provisionals)
- **RFC 3263**: DNS (NAPTR/SRV/A/AAAA)
- **RFC 3264**: SDP Offer/Answer
- **RFC 3265/6665**: SUBSCRIBE/NOTIFY
- **RFC 3515**: REFER (call transfer)
- **RFC 3891**: Replaces (attended transfer)
- **RFC 3966**: tel URI
- **RFC 4028**: Session Timers
- **RFC 7616/7617**: Digest authentication

See `external/siphon-rs/CLAUDE.md` for complete RFC compliance details.

## Troubleshooting

### Build Issues

**Missing dependencies:**
```bash
cargo clean
cargo update
cargo build
```

**Workspace resolution:**
- Ensure new crates are added to workspace `members` in root `Cargo.toml`
- Use `path` dependencies between workspace crates

### Runtime Issues

**siphond not responding to SIP messages:**
- Check firewall: `sudo ufw allow 5060/udp`
- Verify bind address: use `0.0.0.0:5060` not `127.0.0.1:5060` for external access
- Check logs for errors: `RUST_LOG=debug cargo run -p siphond`

**Registration failures:**
- Enable auth with `--auth` flag if required
- Verify credentials in users.json
- Check realm matches between client and server

### Testing Issues

**Transaction timeouts:**
- UDP transactions have 32-second default timeout (64*T1)
- TCP/TLS transactions complete immediately after final response
- Check network connectivity with Wireshark

**Dialog management:**
- Verify Call-ID, From tag, and To tag match between requests
- Check CSeq incrementing correctly
- Ensure proper route set construction from Record-Route

## Performance Considerations

### Transport-Aware Optimization

The transaction layer implements RFC 3261 §17.1.2.2 transport-aware timer optimization:
- **UDP**: Full retransmission and wait timers (5-37 seconds per transaction)
- **TCP/TLS**: Zero wait timers (immediate completion after final response)
- **Performance**: TCP/TLS transactions complete **5-37 seconds faster** than UDP

**Guidelines:**
- Prefer TCP/TLS for high-volume deployments
- Use UDP for compatibility with legacy devices
- Monitor transaction metrics via `TransactionMetrics`

### Memory Usage

- Use `Arc` for shared state (transactions, dialogs, registrations)
- `DashMap` provides lock-free concurrent access without RwLock contention
- Connection pooling reduces TCP/TLS handshake overhead

### Concurrency

- All I/O is async via `tokio`
- Transaction state machines run independently
- Use channels for cross-task communication
- Avoid blocking operations in async contexts

## Contact and Support

This is an early-stage project. For questions or contributions:
- Review architecture documents thoroughly before making significant changes
- Follow existing patterns in siphon-rs for new SIP features
- Ensure all changes include tests and documentation
- Run `cargo clippy` and `cargo fmt` before committing
