# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Ferrous Communications Platform (FCP)** is a next-generation, carrier-grade unified communications platform written entirely in Rust. The project aims to be a multi-tenant, API-first alternative to FreeSWITCH/Kamailio/Asterisk with built-in memory safety, modern async I/O, and observability.

The repository contains:
1. **fcp-core**: Foundational types, traits, and utilities shared across all FCP components
2. **siphon-rs**: A complete, production-grade SIP stack (standalone, reusable)

## Build and Test Commands

### Initial Setup (First Time Only)

If you're cloning this repository, you need to initialize the siphon-rs submodule:

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
├── Cargo.toml                    # Workspace root
├── ARCHITECTURE-v0.2.md          # Detailed architecture and roadmap
├── Phase1 plan.md                # MVP implementation plan (16 weeks)
│
├── crates/
│   ├── fcp-core/                 # Core types and traits
│   │   ├── src/
│   │   │   ├── lib.rs            # Module exports
│   │   │   ├── error.rs          # Error types (ParseError, TransactionError, etc.)
│   │   │   ├── identifiers.rs    # Strongly-typed IDs (CallId, SessionId, DialogId, etc.)
│   │   │   ├── config.rs         # Configuration structures (FcpConfig, NodeConfig, etc.)
│   │   │   ├── service.rs        # Service trait and ServiceManager
│   │   │   ├── codec.rs          # Media codec definitions (AudioCodec, VideoCodec)
│   │   │   ├── transport.rs      # Transport trait and abstractions
│   │   │   └── time.rs           # Time utilities
│   │   └── Cargo.toml
│   │
│   ├── fcp-sip/                  # SIP integration layer
│   │   ├── src/
│   │   │   ├── lib.rs            # Module exports
│   │   │   ├── exports.rs        # Re-exports of siphon-rs types
│   │   │   ├── adapters.rs       # Type conversion functions
│   │   │   ├── transport.rs      # TransportManager
│   │   │   └── service.rs        # Service wrappers (TransactionService, DialogService, etc.)
│   │   └── Cargo.toml
│   │
│   └── siphon-rs/                # SIP stack (git submodule)
│       │                         # Repository: https://github.com/thevoiceguy/siphon-rs
│       ├── CLAUDE.md             # siphon-rs specific documentation
│       ├── README.md
│       ├── Cargo.toml            # siphon-rs workspace root
│       │
│       ├── bins/
│       │   └── siphond/          # Multi-mode SIP testing daemon
│       │       ├── README.md     # Comprehensive siphond documentation
│       │       └── src/
│       │           ├── main.rs
│       │           ├── dispatcher.rs
│       │           ├── config.rs
│       │           └── handlers/
│       │
│       └── crates/
│           ├── sip-core/         # SIP message types, headers, URIs
│           ├── sip-parse/        # nom-based SIP parser
│           ├── sip-transport/    # UDP/TCP/TLS transports
│           ├── sip-transaction/  # RFC 3261 transaction state machines
│           ├── sip-dns/          # RFC 3263 DNS resolution
│           ├── sip-dialog/       # Dialog and subscription management
│           ├── sip-auth/         # Digest authentication
│           ├── sip-registrar/    # REGISTER handler
│           ├── sip-uas/          # User Agent Server helpers
│           ├── sip-uac/          # User Agent Client helpers
│           ├── sip-proxy/        # Proxy primitives
│           ├── sip-sdp/          # SDP support
│           ├── sip-observe/      # Observability
│           └── sip-testkit/      # Test utilities
│
├── tests/                        # Integration tests
├── migrations/                   # Database migrations (future)
└── deploy/                       # Deployment configs (future)
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

### Phase 1 (Current - MVP Foundation)

**Focus:** Single-node PBX with basic call control

**Active Work:**
- ✅ fcp-core types and identifiers
- ✅ siphon-rs SIP stack (complete)
- ⏳ Integration with fcp-core abstractions
- ⏳ Single-node runtime

**Next Steps** (see `Phase1 plan.md`):
1. Complete fcp-core service traits and abstractions
2. Build runtime that integrates siphon-rs components
3. Add SQLite storage backend
4. Create REST API for management
5. Docker deployment

### Phase 2 (Planned - Programmable Voice)

**Focus:** CPaaS-like programmability with webhooks

**Key Components to Build:**
- `fcp-flow-engine`: Call flow DSL runtime
- `fcp-ivr`: IVR service using flow engine
- `fcp-webhook-dispatcher`: Outbound webhook delivery
- `fcp-api-websocket`: Real-time event streaming
- Recording support via media pipeline

### Phase 3 (Planned - Clustering)

**Focus:** HA clustering and application suite

**Key Components:**
- `fcp-cluster-membership`: SWIM protocol implementation
- `fcp-cluster-state`: Distributed registration/dialog cache
- `fcp-queue`: ACD queue service
- `fcp-voicemail`: Voicemail system
- `fcp-webrtc`: WebRTC gateway

### Phase 4 (Planned - Enterprise Features)

**Focus:** Production-ready for enterprise

**Key Components:**
- `fcp-routing-engine`: LCR with carrier health
- `fcp-policy-engine`: Rate limiting, fraud detection
- `fcp-transcoder`: Audio transcoding
- `fcp-cdr-processor`: Dedicated CDR pipeline
- Encryption at rest (KMS integration)

### Phase 5 (Planned - Carrier Grade)

**Focus:** Carrier-scale deployment

**Key Components:**
- Multi-region support
- STIR/SHAKEN implementation
- E911 integration
- Video conferencing
- Regulatory compliance features

## Working with the siphon-rs Submodule

The siphon-rs SIP stack is maintained as a separate repository and linked as a git submodule.

### Updating siphon-rs to Latest

```bash
# Update submodule to latest from remote
cd external/siphon-rs
git pull origin main
cd ../..

# Commit the submodule update in FCP
git add external/siphon-rs
git commit -m "Update siphon-rs submodule to latest"
```

### Making Changes to siphon-rs

If you need to modify siphon-rs code:

```bash
cd external/siphon-rs

# Create a branch for your changes
git checkout -b feature/my-sip-feature

# Make changes, test them
cargo test

# Commit to siphon-rs
git add .
git commit -m "Add new SIP feature"

# Push to siphon-rs repository
git push origin feature/my-sip-feature

# Create PR in siphon-rs repo, get it merged

# After merge, update FCP to use the new version
git checkout main
git pull origin main
cd ../..
git add external/siphon-rs
git commit -m "Update siphon-rs submodule with new feature"
```

### Checking Submodule Status

```bash
# See which commit the submodule is at
git submodule status

# See if submodule has uncommitted changes
cd external/siphon-rs
git status
```

### Important Notes

- Always commit and push changes to siphon-rs first, then update the FCP submodule reference
- Never commit changes to siphon-rs code directly in FCP - use the siphon-rs repository workflow
- When pulling FCP changes, update submodules: `git pull && git submodule update --init --recursive`

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
- **Phase1 plan.md**: Detailed MVP implementation plan with sprint breakdowns
- **IMPLEMENTATION_DOCS.md**: Index of implementation documentation for completed features
  - **Dial Node B2BUA**: Complete 4-phase implementation (outbound calls, response handling, media bridging)
  - **CDR System**: Automatic call detail record creation with REST API
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
