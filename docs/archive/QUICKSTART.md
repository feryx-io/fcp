# FCP Quick Start Guide

## Prerequisites

- Rust 1.75+ (`rustc --version`)
- Git
- PostgreSQL (production) or use the built-in `FileBackedStorage` for development
- Optional: Redis (for event bus, caching, distributed state)

## Build & Run

```bash
# Clone with submodules
git clone --recursive https://github.com/thevoiceguy/fcp.git
cd fcp

# Build all workspace crates
cargo build --release

# Run the call manager (registrar + B2BUA + PBX features)
cargo run -p fcp-call-manager

# Or run siphond for SIP-level testing
cargo run -p siphond -- --mode registrar
```

## Project Structure

```
fcp/
├── crates/
│   ├── fcp-core/              # Core types, traits, storage abstractions
│   ├── fcp-sip/               # SIP integration layer
│   ├── fcp-b2bua-core/        # B2BUA call engine
│   ├── fcp-pbx/               # PBX feature library (parking, pickup, intercom)
│   ├── fcp-routing/           # Policy-based call routing
│   ├── fcp-flow/              # Programmable call flow engine
│   ├── fcp-voicemail/         # Voicemail service
│   ├── fcp-queue/             # ACD queue service
│   ├── fcp-presence/          # Presence and BLF
│   ├── fcp-media/             # Media session management
│   ├── fcp-events/            # Platform event bus
│   ├── fcp-webhooks/          # Webhook dispatcher
│   ├── fcp-cluster/           # Clustering (SWIM protocol)
│   ├── fcp-sbc-core/          # SBC enforcement library
│   ├── fcp-storage-postgres/  # PostgreSQL backend
│   ├── fcp-storage-redis/     # Redis backend
│   ├── fcp-api-core/          # REST API endpoints
│   ├── fcp-admin/             # Admin HTTP server
│   ├── fcp-admin-ui/          # Admin web UI
│   ├── fcp-cli/               # Terminal UI monitor
│   ├── fcp-call-manager/      # Main PBX binary
│   ├── fcp-sbc/               # SBC binary
│   ├── fcp-ivr/               # IVR binary
│   ├── fcp-voicemail-server/  # Voicemail binary
│   ├── fcp-registrar/         # Standalone registrar binary
│   ├── fcp-events-server/     # Event streaming binary
│   ├── fcp-conference-server/ # Conference bridge binary
│   ├── fcp-media-server/      # Media server binary
│   ├── fcp-webrtc-gateway/    # WebRTC gateway binary
│   ├── fcp-presence-server/   # Presence server binary
│   ├── fcp-api-gateway/       # API gateway binary
│   ├── fcp-jobs/              # Async job worker binary
│   └── siphon-rs/             # SIP stack (git submodule)
├── external/siphon-rs/        # Submodule link
├── tests/                     # Integration tests
└── deploy/                    # Docker and deployment configs
```

## Key Binaries

| Binary | Purpose |
|--------|---------|
| `fcp-call-manager` | Main PBX: registrar + B2BUA + call routing + PBX features |
| `fcp-sbc` | Session Border Controller for network edge |
| `fcp-ivr` | Standalone IVR server with flow engine |
| `fcp-voicemail-server` | Standalone voicemail server |
| `fcp-registrar` | Standalone SIP registrar |
| `fcp-admin` | Admin HTTP API |
| `fcp-events-server` | Event streaming + webhook delivery |
| `siphond` | SIP testing daemon (part of siphon-rs) |

## Run Tests

```bash
# All tests
cargo test --all

# Specific crate
cargo test -p fcp-core

# With output
cargo test -- --nocapture
```

## Check & Lint

```bash
cargo fmt --all
cargo clippy --all
cargo check --all
```

## Key Documents

- **CLAUDE.md** -- Project overview, full crate listing, development guidelines
- **IMPLEMENTATION_DOCS.md** -- Detailed implementation docs for all subsystems
- **ARCHITECTURE-v0.2.md** -- Architecture vision and roadmap
- **docs/STORAGE_ARCHITECTURE.md** -- Storage layer design

---

*Last Updated: 2026-03-04*
