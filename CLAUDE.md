# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Ferrous Communications Platform (FCP)** is a next-generation, carrier-grade unified communications platform written entirely in Rust. The project aims to be a multi-tenant, API-first alternative to FreeSWITCH/Kamailio/Asterisk with built-in memory safety, modern async I/O, and observability.

The repository contains 37 workspace crates spanning core libraries, application services, storage backends, and standalone binaries, plus two git submodules under `external/`: the siphon-rs SIP stack and the forge-media media stack. Both submodule repositories are public.

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
├── Cargo.toml                    # Workspace root (37 crates)
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
│   ├── fcp-crypto/               # Encryption at rest (AES-256-GCM field cipher, local/AWS KMS key providers)
│   ├── fcp-routing/              # Policy-based call routing engine (re-exports fcp-policy)
│   │
│   │ # ── Application ───────────────────────────────────────
│   ├── fcp-flow/                 # Programmable call flow engine (JSON-based DSL)
│   ├── fcp-voicemail/            # Voicemail service (mailboxes, greetings, MWI)
│   ├── fcp-queue/                # ACD queue service (ring strategies, agent management)
│   ├── fcp-presence/             # Presence and BLF (SUBSCRIBE/NOTIFY, dialog events)
│   ├── fcp-webhooks/             # Webhook dispatcher (HMAC-signed, retry with backoff)
│   ├── fcp-cluster/              # Clustering (SWIM protocol, node membership)
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
│   └── fcp-jobs/                 # Async job worker (CDR processing, webhooks, cleanup)
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

- ✅ `fcp-cluster`: SWIM protocol clustering
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
- E911 integration — block 1 shipped (`docs/E911.md`): detection, dispatchable location, ELIN caller ID, emergency-capable trunks, audit + event, admin API
- Regulatory compliance features — basics shipped (`docs/COMPLIANCE.md`): retention policy + sweep, recording consent, admin audit log, CDR export
- Distributed conferencing (cascaded mixing) — all four phases shipped (registry + placement, mutual-TLS inter-node trunks with cascaded mixing, cross-node control under a shared `cluster.secret`, resilience: registry freshness, trunk liveness, draining, trunk SRTP; `docs/DISTRIBUTED_CONFERENCING.md`). Participant migration off a draining node is not done
- Multi-region support — read replicas shipped (`docs/MULTI_REGION.md`: Postgres primary + per-region streaming replicas, configuration reads routed to a healthy replica; CockroachDB dropped); region-aware cluster membership and event bus next
- Video conferencing

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
- **docs/COMPLIANCE.md**: Retention policy and sweep, recording consent, admin audit log, CDR export
- **docs/DISTRIBUTED_CONFERENCING.md**: Conference registry, placement rule, inter-node trunks (mutual TLS, cascaded mixing) and the frame-clock mixer
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
