# FCP Deployment Binaries - Design Document

## Overview

FCP uses a **distributed multi-binary architecture** where each service is an independent, purpose-built binary. This allows independent scaling, security isolation, and resource efficiency.

For small deployments, `fcp-call-manager` serves as the primary entry point, combining registrar + B2BUA + PBX features in a single process. Additional services (voicemail, IVR, SBC, etc.) are deployed as needed.

This document defines the architecture for building independent, purpose-built binaries for each service type.

## Design Principles

- Each service is independently deployable and scalable
- Services communicate via SIP (signaling) and REST APIs (management)
- Shared logic lives in library crates (`fcp-b2bua-core`, `fcp-sbc-core`, `fcp-api-core`)
- Security: edge nodes (SBC) have no database access; admin API is isolated from edge
- Small deployments use `fcp-call-manager` as the primary all-in-one-ish service

## Proposed Architecture

### Deployment Topology Example

```
┌─────────────────────────────────────────────────────────────────┐
│                        INTERNET                                  │
└────────────────────┬────────────────────────────────────────────┘
                     │
          ┌──────────┴──────────┐
          │                     │
     ┌────▼────┐           ┌────▼────┐
     │ fcp-sbc │           │ fcp-sbc │          Edge (DMZ)
     │  :5060  │           │  :5060  │          - No DB access publicly
     └────┬────┘           └────┬────┘          - No admin API publicly
          │                     │               - NAT/Topology hiding
          └──────────┬──────────┘
                     │
       ┌─────────────┼─────────────┐
       │             │             │
  ┌────▼─────────┐ ┌▼──────────┐ ┌▼──────────┐
  │fcp-call-mgr  │ │fcp-call-  │ │fcp-       │   Call Processing
  │ (trunks mode)│ │mgr (PBX)  │ │registrar  │   - Full DB access
  │ LCR/routing  │ │registrar  │ │           │   - Feature-rich
  └────┬─────────┘ └──┬────────┘ └────┬──────┘
       │              │               │
       ├──────────────┴───────────────┤
       │                              │
  ┌────▼────────┐         ┌──────────▼────────┐
  │ fcp-        │         │ fcp-              │   Applications
  │ voicemail   │         │ conference        │   - Specialized
  │ (2 nodes)   │         │                   │   - Independently scalable
  └─────────────┘         └───────────────────┘
       │                          │
  ┌────▼────────┐                 │
  │ fcp-ivr     │                 │           IVR
  │ (DID→flow)  │                 │           - Flow execution
  └─────────────┘                 │           - No registrar
       │                          │
       └────────────┬─────────────┘
                    │
         ┌──────────▼──────────┐
         │   PostgreSQL/Redis   │              Storage Layer
         └─────────────────────┘
```

> **Note:** `fcp-call-manager` runs in two modes via configuration:
> - **PBX mode** (default): `registrar.enabled = true` — registrar + B2BUA for user devices
> - **Trunks mode**: `registrar.enabled = false` + trunks configured — SIP trunk termination, LCR routing, carrier failover (no phone registrations)

## Core Architecture: B2BUA as Shared Foundation

### Why B2BUA is a Library, Not a Service

**B2BUA (Back-to-Back User Agent)** functionality is fundamental to multiple FCP services. Rather than creating a separate B2BUA service, we implement it as a **shared library** (`fcp-b2bua-core`) that multiple services use.

### Services That Use B2BUA Core

```
                    ┌─────────────────────┐
                    │   fcp-b2bua-core    │
                    │   (Shared Library)  │
                    └──────────┬──────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
         ┌────▼────┐      ┌────▼────┐     ┌────▼─────────┐
         │fcp-sbc  │      │fcp-pbx  │     │fcp-call-     │
         │         │      │         │     │manager       │
         │         │      │         │     │(both modes)  │
         └─────────┘      └─────────┘     └──────────────┘
```

### B2BUA Core Responsibilities

1. **Dialog Manipulation**
   - Create back-to-back dialogs (A-leg ↔ B-leg)
   - Maintain dialog state for both legs
   - Handle early/confirmed/terminated states
   - CSeq tracking and validation

2. **Media Control**
   - SDP offer/answer negotiation
   - Media anchoring (RTP proxy mode)
   - Hold/resume (re-INVITE with sendonly/recvonly)
   - Codec transcoding coordination

3. **Topology Hiding**
   - Via header manipulation
   - Record-Route rewriting
   - Contact header manipulation
   - Private IP address masking

4. **Call Forking**
   - Parallel forking (ring multiple targets simultaneously)
   - Sequential forking (try targets one by one)
   - Early media handling (180 with SDP)
   - CANCEL propagation

5. **Call Transfer Primitives**
   - REFER handling (blind transfer)
   - Replaces header support (attended transfer)
   - Consultative transfer coordination
   - Transfer loop detection

6. **Mid-Call Modifications**
   - re-INVITE handling (codec change, hold)
   - UPDATE support (RFC 3311)
   - Session refresh (timer-based)
   - Codec renegotiation

### How Different Services Use B2BUA

#### SBC Use Case
```rust
use fcp_b2bua_core::{B2buaSession, TopologyHiding, MediaAnchoring};

// SBC uses B2BUA primarily for:
// 1. Topology hiding (protect internal network)
// 2. Media anchoring (NAT traversal)
// 3. DoS protection (rate limiting, validation)

let mut b2bua = B2buaSession::new(a_leg_dialog);
b2bua.enable_topology_hiding(TopologyHiding::Full); // Hide all internal headers
b2bua.enable_media_anchoring(MediaAnchoring::Always); // Always proxy RTP
b2bua.set_max_forwards(10); // Limit loop potential

let b_leg = b2bua.create_b_leg(upstream_uri).await?;
```

#### PBX Use Case
```rust
use fcp_b2bua_core::{B2buaSession, CallForking, CallTransfer};

// PBX uses B2BUA for:
// 1. Call forking (ring multiple devices)
// 2. Call transfers
// 3. Call parking/pickup
// 4. Feature implementation (hold, mute, etc.)

let mut b2bua = B2buaSession::new(a_leg_dialog);
b2bua.enable_forking(CallForking::Parallel); // Ring all devices

// Ring all devices registered to this extension
let targets = extension.get_device_targets().await?;
let results = b2bua.fork_call(targets).await?;

// Handle blind transfer
let transfer = b2bua.handle_refer(refer_request).await?;
transfer.execute().await?;
```

#### Call Manager — Trunks Mode Use Case
```rust
use fcp_b2bua_core::{B2buaSession, RoutingMode};

// fcp-call-manager in trunks mode (registrar.enabled = false) uses B2BUA for:
// 1. Call routing (LCR, failover)
// 2. Load balancing across PBX nodes
// 3. Carrier failover

let mut b2bua = B2buaSession::new(a_leg_dialog);
b2bua.set_routing_mode(RoutingMode::LeastCost);

// Try carriers in order of cost
for carrier in carriers_by_cost {
    match b2bua.try_carrier(&carrier).await {
        Ok(result) => return Ok(result),
        Err(e) if e.should_failover() => continue,
        Err(e) => return Err(e),
    }
}
```

### B2BUA Core API Design

```rust
/// B2BUA session represents a back-to-back call
pub struct B2buaSession {
    a_leg: Dialog,           // Incoming call leg
    b_legs: Vec<Dialog>,     // Outgoing call leg(s)
    media_proxy: Option<MediaProxy>,
    config: B2buaConfig,
}

impl B2buaSession {
    /// Create new B2BUA session from A-leg dialog
    pub fn new(a_leg: Dialog) -> Self;

    /// Create outbound B-leg
    pub async fn create_b_leg(&mut self, target: Uri) -> Result<Dialog>;

    /// Fork call to multiple targets
    pub async fn fork_call(&mut self, targets: Vec<Uri>) -> Result<ForkingResult>;

    /// Handle mid-call REFER (transfer)
    pub async fn handle_refer(&mut self, refer: Request) -> Result<Transfer>;

    /// Enable/disable topology hiding
    pub fn enable_topology_hiding(&mut self, mode: TopologyHiding);

    /// Enable/disable media anchoring
    pub fn enable_media_anchoring(&mut self, mode: MediaAnchoring);

    /// Handle A-leg re-INVITE (hold, resume, codec change)
    pub async fn handle_a_leg_reinvite(&mut self, invite: Request) -> Result<Response>;

    /// Handle B-leg response
    pub async fn handle_b_leg_response(&mut self, response: Response) -> Result<()>;

    /// Terminate both legs
    pub async fn terminate(&mut self) -> Result<()>;
}

pub enum TopologyHiding {
    None,          // Pass through all headers
    Partial,       // Hide private IPs only
    Full,          // Rewrite all topology headers
}

pub enum MediaAnchoring {
    Never,         // Pass through SDP (proxy mode)
    Always,        // Always anchor media (B2BUA mode)
    Conditional,   // Anchor only for NAT traversal
}
```

### Benefits of Shared B2BUA Library

1. **Code Reuse**: Single implementation of complex B2BUA logic
2. **Consistency**: All services handle dialogs the same way
3. **Testing**: Test B2BUA logic once, use everywhere
4. **Performance**: No inter-process communication overhead
5. **Flexibility**: Each service configures B2BUA for its needs

---

## Binary Definitions

### 1. `fcp-sbc` (Session Border Controller)

**Purpose:** Edge security, NAT traversal, topology hiding, carrier interconnection

**Includes:**
- **B2BUA core** (from `fcp-b2bua-core` library)
  - Topology hiding (Via/Record-Route manipulation)
  - Media anchoring (optional)
  - Call admission control
- **SIP transport** (UDP/TCP/TLS)
- **Transaction layer**
- **Authentication** (Digest for trunk registration)
- **NAT traversal** (ICE, STUN, TURN integration)
- **Rate limiting** (CPS limits, burst protection)
- **DOS protection** (SYN flood, malformed SIP)
- **Database access** (limited scope):
  - CDR writing (edge CDRs for billing)
  - Whitelist/blacklist (number filtering)
  - SIPREC configuration (which calls to record)
  - Trunk configuration (carrier credentials)
- **SIPREC** (SIP Recording)
  - Recording metadata generation
  - Forward to recording servers
- **Metrics/logging**

**Excludes:**
- Voicemail/IVR/Conference (application logic)
- User registration (not carrier trunks)
- Admin API (read-only health check only)
- Full database access (only CDR, blacklist, SIPREC tables)

**Configuration:**
```toml
[sbc]
bind_addresses = ["0.0.0.0:5060", "0.0.0.0:5061"]
protocols = ["udp", "tcp", "tls"]
upstream_call_manager = "sip:cm.internal.example.com:5060"  # fcp-call-manager in trunks mode

[sbc.topology_hiding]
enabled = true
hide_via = true
hide_record_route = true
local_domain = "sbc.example.com"

[sbc.media]
anchor_media = true  # B2BUA mode
rtp_port_range = "10000-20000"

[sbc.admission_control]
max_calls_per_second = 1000
max_concurrent_calls = 10000

[sbc.security]
enable_whitelist = true
enable_blacklist = true
rate_limit_per_ip = 100  # CPS per source IP

[database]
# Limited schema access: cdrs, blacklist, whitelist, siprec_config, trunks
url = "postgres://sbc_user:password@db:5432/fcp"
# SBC user has INSERT on cdrs, SELECT on blacklist/whitelist/siprec_config/trunks

[siprec]
enabled = true
recording_servers = [
    "sip:recorder1.internal:5060",
    "sip:recorder2.internal:5060"
]
```

---

### 2. `fcp-call-manager` — Trunks Mode (Call Routing Brain)

> **This is NOT a separate binary.** The `fcp-call-manager` binary operates in
> trunks mode when `registrar.enabled = false` and trunks are configured.
> This consolidates what was previously planned as `fcp-session-manager`.

**Purpose:** SIP trunk termination, LCR routing, carrier failover — a call server with no phone registrations, just SIP trunks.

**Includes:**
- B2BUA core (from `fcp-b2bua-core` library)
- SIP trunk registration and health monitoring (from `fcp-sbc-core`)
- Routing engine (LCR, DID routing, failover)
- Session state management
- Dialog tracking
- Event bus publisher
- Media anchoring via forge-media

**Excludes:**
- Registrar (disabled via config)
- Phone/device registration
- PBX features (parking, pickup, hunt groups)

**Configuration:**
```toml
[service]
node_id = "trunk-01"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"

[registrar]
enabled = false  # Disables registrar → trunks mode

[[trunks]]
name = "carrier-primary"
uri = "sip:carrier1.example.com"
transport = "tcp"
auth_username = "fcp"
auth_password = "secret"

[[trunks]]
name = "carrier-backup"
uri = "sip:carrier2.example.com"
transport = "tcp"
priority = 10  # Lower priority = failover

[http]
bind_address = "0.0.0.0:8084"
```

---

### 3. `fcp-registrar` (Registration Server)

**Purpose:** Device and user registration

**Includes:**
- REGISTER handler
- Location store (Redis + DB)
- Authentication (Digest)
- Registration state management
- AOR (Address of Record) management
- Multiple device support per user

**Excludes:**
- Call processing
- Media
- Voicemail/IVR
- Admin API (read-only status)

**Configuration:**
```toml
[registrar]
bind_address = "0.0.0.0:5060"
database_url = "postgres://..."
redis_url = "redis://redis.internal:6379"
default_expires = 3600
max_contacts = 10
```

---

### 4. `fcp-pbx` (Call Processing Node)

**Purpose:** Feature-rich call handling (B2BUA, transfers, call parking, etc.)

**Includes:**
- **B2BUA core** (from `fcp-b2bua-core` library)
  - Call forking (parallel/sequential)
  - Call transfer (blind/attended/consultative)
  - Call parking/pickup
  - Call recording (on-demand)
  - Media manipulation (hold music, announcements)
- **PBX features**
  - Extension-to-extension calls
  - Do Not Disturb
  - Call forwarding (busy/no-answer/always)
  - Find Me/Follow Me
  - Call screening
  - Call waiting
  - Speed dial
- **Integration** with applications
  - Voicemail (via SIP REFER/INVITE)
  - IVR (via SIP INVITE)
  - Conference (via SIP INVITE)
  - Queue (ACD routing)
- **Call detail records** (detailed CDRs with all call legs)
- **Full database access** (users, extensions, devices, call history)
- **REST API client** (for webhooks, CRM integration)
- **Event bus publisher** (call events for analytics)

**Excludes:**
- SBC functionality (no edge security, no carrier trunks)
- Registrar (queries it via API/Redis for location)
- Media processing (delegates to fcp-media for transcoding)

**Configuration:**
```toml
[pbx]
bind_address = "0.0.0.0:5060"
database_url = "postgres://..."
redis_url = "redis://redis.internal:6379"
voicemail_server = "sip:vm.internal:5060"
conference_server = "sip:conf.internal:5060"
enable_call_recording = true
```

---

### 5. `fcp-ivr` (IVR Server)

**Purpose:** Standalone Interactive Voice Response — accepts SIP INVITEs, matches DIDs to programmed flows, and executes them

**Includes:**
- **Flow engine** (from `fcp-flow`)
  - Play, Say, Gather, Dial, Answer, Hangup nodes
  - DID-to-flow routing table (static config + REST API)
  - Variable interpolation and conditional branching
- **B2BUA core** (from `fcp-b2bua-core` library, for Dial node outbound calls)
- **SIP transport** (UDP/TCP/TLS)
- **Transaction layer**
- **Media manager** (RTP, audio playback, DTMF collection)
- **Event publishing** (flow started/completed/node-executed events)
- **Trunk support** (optional, for Dial node outbound routing)
- **REST API** for flow CRUD and DID route management
- **Health/status endpoints** (active flows, DID routes)

**Excludes:**
- Registrar (no user registration)
- LCR routing (DID-to-flow routing only)
- Voicemail
- Conference
- Admin API (has its own flow route management API)

**Key Difference from fcp-call-manager:**
- `fcp-call-manager` passes `None` for `flow_handler` to `SipMessageHandler`
- `fcp-ivr` passes `Some(flow_handler)` with a fully-wired `FlowCallHandler`

**Configuration:**
```toml
[service]
service_type = "ivr"
node_id = "ivr-01"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"

[http]
bind_address = "0.0.0.0:8086"

[flows]
default_flow_id = "main-menu"
tts_provider = "none"
tts_language = "en-US"
audio_path = "/var/lib/fcp/audio"

[[flows.routes]]
did = "+15551234567"
flow_id = "main-menu"

[[flows.routes]]
did = "+15559876543"
flow_id = "support-queue"

[storage]
backend = "sqlite"
connection_string = "sqlite://data/fcp-ivr.db"
```

---

### 6. `fcp-voicemail` (Voicemail Server)

**Purpose:** Standalone voicemail — accepts SIP INVITEs for configured DIDs/extensions, plays greetings, records messages, and manages MWI

**Includes:**
- **Flow engine** (from `fcp-flow`) with programmatic Answer → Voicemail → Hangup flows
- **VoicemailService** (from `fcp-voicemail`) for greeting playback, recording, and MWI
- **B2BUA core** (from `fcp-b2bua-core` library)
- **SIP transport** (UDP/TCP/TLS)
- **Transaction layer**
- **Media manager** (RTP, audio playback, recording)
- **Event publishing** (VoicemailEvent::MessageRecorded, MwiUpdated, etc.)
- **Trunk support** (optional, for outbound MWI NOTIFY or transfers)
- **REST API** for voicemail route management, mailbox/message CRUD, MWI status
- **Health/status endpoints** (active calls, voicemail routes, mailbox listing)

**Excludes:**
- Registrar (no user registration)
- LCR routing (DID-to-mailbox routing only)
- IVR (no Gather/Branch nodes — voicemail-only flows)
- Conference
- Admin API (has its own voicemail-specific endpoints)

**Key Difference from fcp-ivr:**
- Uses same `FlowCallHandler` pattern but wires in `VoicemailService`
- Programmatically generates Answer → Voicemail → Hangup flows per route (no JSON)
- DID routes map to `mailbox_id` instead of `flow_id`

**Configuration:**
```toml
[service]
service_type = "voicemail"
node_id = "voicemail-01"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"

[http]
bind_address = "0.0.0.0:8087"

[storage]
backend = "sqlite"
connection_string = "sqlite://data/fcp-voicemail.db"

[voicemail]
recordings_dir = "/var/fcp/voicemail/recordings"
greetings_dir = "/var/fcp/voicemail/greetings"
max_message_duration = 300
max_messages_per_mailbox = 100
play_beep = true
default_mailbox_id = "general"

[[voicemail.routes]]
did = "+15551234567"
mailbox_id = "sales"

[[voicemail.routes]]
did = "5000"
mailbox_id = "support"
```

---

### 7. `fcp-conference` (Conference Bridge)

**Purpose:** Multi-party audio conferencing

**Includes:**
- Conference room management
- Audio mixing
- Conference controls (mute, kick, lock)
- DTMF controls
- Conference recording
- Participant management
- Media engine integration

**Excludes:**
- Call routing
- Voicemail
- IVR
- SBC

**Configuration:**
```toml
[conference]
bind_address = "0.0.0.0:5060"
database_url = "postgres://..."
max_conferences = 100
max_participants_per_conference = 50
enable_recording = true
recording_path = "s3://conference-recordings/"
```

---

### 8. `fcp-media` (Media Server)

**Purpose:** Media processing (transcoding, recording, playback, DTMF detection)

**Includes:**
- RTP engine
- Codec transcoding
- Call recording
- Media playback (announcements, music on hold)
- DTMF detection/generation
- Fax support (T.38)

**Excludes:**
- SIP signaling (receives commands via gRPC or SIP)
- Call logic
- Voicemail/IVR logic

**Configuration:**
```toml
[media]
rtp_port_range = "10000-20000"
enable_transcoding = true
supported_codecs = ["PCMU", "PCMA", "G722", "Opus"]
recording_backend = "s3"
recording_path = "s3://call-recordings/"
```

---

### 9. `fcp-admin` (Admin API Server)

**Purpose:** REST API for configuration and management

**Includes:**
- REST API (devices, users, tenants, phone numbers, etc.)
- WebSocket event stream
- Authentication/authorization
- Rate limiting
- Full database access

**Excludes:**
- SIP signaling
- Media
- Call processing

**Configuration:**
```toml
[admin]
bind_address = "0.0.0.0:8080"
database_url = "postgres://..."
redis_url = "redis://redis.internal:6379"
jwt_secret = "..."
enable_swagger = true
```

---

## Implementation Plan

### Phase 1: Extract Core Libraries (Current State → Shared Libs)

**Goal:** Ensure all functionality is in shared library crates

#### 1.1 Audit Current Code
- [x] `fcp-sip` - SIP handling (✓ already a library)
- [x] `fcp-core` - Core types (✓ already a library)
- [x] **CRITICAL:** Extract B2BUA core → `fcp-b2bua-core` library ✅ DONE
  - B2BUA is used by multiple services (SBC, PBX, call-manager)
  - Includes: dialog manipulation, media control, topology hiding primitives
- [x] Extract SBC-specific code → `fcp-sbc-core` library ✅ DONE
- [x] Extract admin API handlers → `fcp-api-core` library ✅ DONE
- [ ] Extract conference logic → `fcp-conference-core` library

#### 1.2 Create Feature Gates
- Add Cargo features to control what gets compiled
- Example `Cargo.toml` features:
```toml
[features]
default = ["all"]
all = ["sbc", "registrar", "pbx", "voicemail", "conference", "media", "admin-api"]
sbc = []
registrar = []
pbx = ["registrar"]  # PBX depends on registrar client
voicemail = []
conference = []
media = []
admin-api = []
```

### Phase 2: Create Binary Crates

#### 2.1 Repository Structure
```
fcp/
├── crates/
│   ├── fcp-core/           # ✓ Already exists
│   ├── fcp-sip/            # ✓ Already exists
│   ├── fcp-storage-*/      # ✓ Already exists
│   │
│   ├── fcp-b2bua-core/     # ✅ DONE: B2BUA library (SHARED by SBC, PBX, call-manager)
│   │                       #      - Dialog manipulation (forking, serial hunting)
│   │                       #      - Media anchoring and manipulation
│   │                       #      - Topology hiding (Via/Record-Route rewriting)
│   │                       #      - Call transfer primitives (REFER handling)
│   │                       #      - Call parking/pickup
│   │                       #      - Mid-call modifications (hold, unhold, reinvite)
│   │
│   ├── fcp-sbc-core/       # ✅ DONE: SBC library (LCR, trunks, admission, HMR)
│   ├── fcp-api-core/       # ✅ DONE: Shared REST API handlers
│   ├── fcp-conference-core/# TODO: Conference library
│
├── crates/  (binaries live alongside libraries)
│   ├── fcp-sbc/            # ✅ DONE: SBC binary
│   ├── fcp-call-manager/   # ✅ DONE: Call server (PBX mode + trunks mode)
│   ├── fcp-registrar/      # ✅ DONE: Registrar binary
│   ├── fcp-pbx/            # ✅ DONE: PBX binary
│   ├── fcp-voicemail-server/ # ✅ DONE: Voicemail binary
│   ├── fcp-ivr/            # ✅ DONE: IVR binary
│   ├── fcp-conference/     # TODO: Conference binary
│   ├── fcp-media/          # Library (embedded in call-manager; standalone TODO)
│   ├── fcp-admin/          # ✅ DONE: Admin API binary
│   ├── fcp-api-gateway/    # ✅ DONE: Reverse proxy + rate limiting
│   ├── fcp-events-server/  # ✅ DONE: Multi-node event aggregation
│   └── fcp-jobs/           # ✅ DONE: Background job processing
│
└── Cargo.toml
```

#### 2.2 Example Binary Implementation

`bins/fcp-sbc/Cargo.toml`:
```toml
[package]
name = "fcp-sbc"
version = "0.10.0"
edition = "2021"

[dependencies]
fcp-core = { path = "../../crates/fcp-core" }
fcp-sip = { path = "../../crates/fcp-sip" }
fcp-sbc-core = { path = "../../crates/fcp-sbc-core" }
fcp-storage-sqlite = { path = "../../crates/fcp-storage-sqlite", optional = true }
fcp-storage-postgres = { path = "../../crates/fcp-storage-postgres", optional = true }

# Minimal dependencies for SBC
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"

# NO voicemail, IVR, conference dependencies

[features]
default = ["storage-redis"]
storage-redis = []
storage-postgres = ["fcp-storage-postgres"]
```

`bins/fcp-sbc/src/main.rs`:
```rust
use clap::Parser;
use fcp_core::*;
use fcp_sip::*;
use fcp_sbc_core::*;

#[derive(Parser)]
struct Cli {
    #[arg(short, long, default_value = "fcp-sbc.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load SBC-specific config
    let config = SbcConfig::load(&cli.config)?;

    // Initialize ONLY what SBC needs
    let sbc = SbcServer::new(config).await?;

    // Run SBC (no voicemail, IVR, admin API)
    sbc.run().await?;

    Ok(())
}
```

### Phase 3: Configuration Management

#### 3.1 Shared Config Structure
```rust
// fcp-core/src/config.rs
pub struct ServiceConfig {
    pub service_type: ServiceType,
    pub node_id: NodeId,
    pub bind_addresses: Vec<SocketAddr>,
    pub storage: StorageConfig,
    pub redis: Option<RedisConfig>,
    pub event_bus: Option<EventBusConfig>,
    pub metrics: MetricsConfig,
    pub logging: LoggingConfig,
}

pub enum ServiceType {
    Sbc(SbcConfig),
    CallManager(CallManagerConfig),  // PBX mode or trunks mode via config
    Registrar(RegistrarConfig),
    Pbx(PbxConfig),
    Voicemail(VoicemailConfig),
    Conference(ConferenceConfig),
    Media(MediaConfig),
    Admin(AdminConfig),
}
```

#### 3.2 Configuration Files
Each binary has its own config file format:

`fcp-sbc.toml`:
```toml
[service]
type = "sbc"
node_id = "sbc-01"

[network]
bind_addresses = ["0.0.0.0:5060", "0.0.0.0:5061"]
protocols = ["udp", "tcp", "tls"]

[upstream]
call_manager = "sip:cm.internal:5060"  # fcp-call-manager in trunks mode

[sbc]
topology_hiding = true
nat_traversal = true
max_cps = 1000

[redis]
url = "redis://redis:6379"

[metrics]
enabled = true
bind_address = "0.0.0.0:9090"
```

`fcp-pbx.toml`:
```toml
[service]
type = "pbx"
node_id = "pbx-01"

[network]
bind_addresses = ["0.0.0.0:5060"]

[database]
url = "postgres://fcp:password@db:5432/fcp"

[redis]
url = "redis://redis:6379"

[pbx]
voicemail_server = "sip:vm.internal:5060"
conference_server = "sip:conf.internal:5060"
enable_call_recording = true

[metrics]
enabled = true
bind_address = "0.0.0.0:9090"
```

### Phase 4: Service Discovery and Communication

#### 4.1 Service Registry (Redis-based)
```rust
// Each service registers itself
pub struct ServiceRegistry {
    redis: RedisClient,
}

impl ServiceRegistry {
    pub async fn register(&self, service: ServiceInfo) -> Result<()> {
        // Store in Redis with TTL
        // Key: "services:{type}:{node_id}"
        // Value: JSON with address, capabilities, health
    }

    pub async fn discover(&self, service_type: ServiceType) -> Result<Vec<ServiceInfo>> {
        // Query Redis for all services of type
        // Return healthy instances
    }

    pub async fn heartbeat(&self, node_id: NodeId) -> Result<()> {
        // Refresh TTL
    }
}
```

#### 4.2 Inter-Service Communication

**SIP-based:**
- SBC → Call Manager (trunks mode): SIP INVITE
- Call Manager → PBX: SIP INVITE
- PBX → Voicemail: SIP INVITE
- PBX → Conference: SIP INVITE

**API-based (REST/gRPC):**
- Any → Registrar: Query AOR location
- Any → Admin API: Configuration queries
- Any → Media Server: Start recording, playback

**Event Bus:**
- All → Event Bus: Publish events
- All ← Event Bus: Subscribe to events

### Phase 5: Deployment Examples

#### 5.1 Docker Compose (Small Business)

For small deployments, `fcp-call-manager` handles registrar + B2BUA + PBX features,
with `fcp-admin` providing the REST management API.

`docker-compose.yml`:
```yaml
version: '3.8'

services:
  fcp-call-manager:
    image: fcp/fcp-call-manager:latest
    network_mode: host
    volumes:
      - ./configs/fcp-call-manager.toml:/app/fcp-call-manager.toml:ro
      - fcp-data:/app/data
    environment:
      - RUST_LOG=info
    depends_on:
      - redis

  fcp-admin:
    image: fcp/fcp-admin:latest
    ports:
      - "8080:8080"
    volumes:
      - ./configs/fcp-admin.toml:/app/fcp-admin.toml:ro
      - fcp-data:/app/data
    environment:
      - RUST_LOG=info
    depends_on:
      - redis

  redis:
    image: redis:7-alpine
    volumes:
      - redis_data:/data

volumes:
  fcp-data:
  redis_data:
```

#### 5.2 Kubernetes (Carrier Grade)

`k8s/sbc-deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: fcp-sbc
spec:
  replicas: 3
  selector:
    matchLabels:
      app: fcp-sbc
  template:
    metadata:
      labels:
        app: fcp-sbc
    spec:
      containers:
      - name: fcp-sbc
        image: fcp/fcp-sbc:latest
        ports:
        - containerPort: 5060
          name: sip
          protocol: UDP
        - containerPort: 5060
          name: sip-tcp
          protocol: TCP
        - containerPort: 9090
          name: metrics
        env:
        - name: FCP_REDIS_URL
          value: "redis://redis-service:6379"
        - name: FCP_CALL_MANAGER
          value: "sip:fcp-call-manager-service:5060"  # trunks mode instance
        resources:
          requests:
            memory: "256Mi"
            cpu: "500m"
          limits:
            memory: "512Mi"
            cpu: "1000m"
        livenessProbe:
          httpGet:
            path: /health
            port: 9090
          initialDelaySeconds: 10
          periodSeconds: 30
```

`k8s/pbx-deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: fcp-pbx
spec:
  replicas: 5  # Scale PBX nodes independently
  selector:
    matchLabels:
      app: fcp-pbx
  template:
    metadata:
      labels:
        app: fcp-pbx
    spec:
      containers:
      - name: fcp-pbx
        image: fcp/fcp-pbx:latest
        env:
        - name: FCP_DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: fcp-db-secret
              key: url
        - name: FCP_REDIS_URL
          value: "redis://redis-service:6379"
        resources:
          requests:
            memory: "512Mi"
            cpu: "1000m"
          limits:
            memory: "2Gi"
            cpu: "2000m"
```

## Benefits of Multi-Binary Architecture

### 1. **Independent Scaling**
- Scale SBCs for high CPS (calls per second)
- Scale PBX nodes for feature processing
- Scale voicemail independently of call volume

### 2. **Security**
- SBC in DMZ with no database access
- Admin API isolated from edge
- Principle of least privilege per service

### 3. **Resource Efficiency**
- Smaller binaries (faster startup)
- Lower memory footprint per service
- Only pay for what you use

### 4. **Operational Flexibility**
- Deploy voicemail in different region for data sovereignty
- Run conference servers on GPU instances
- Mix local and cloud deployments

### 5. **Cost Optimization**
- Use spot instances for stateless services (SBC, media)
- Reserved instances for stateful services (DB, registrar)
- Right-size each service type

### 6. **Fault Isolation**
- Voicemail crash doesn't affect active calls
- Conference issues don't impact registration
- Blast radius limited to service type

## Migration History

### v0.10-0.12: Library Extraction
- Created `fcp-b2bua-core`, `fcp-sbc-core`, `fcp-api-core` shared libraries
- All functionality moved into library crates

### v0.13: First Standalone Binary — `fcp-sbc`
- SBC binary with trunk management, codec filtering, STIR/SHAKEN

### v0.14: Full Multi-Binary Suite
- Added `fcp-registrar`, `fcp-call-manager`, `fcp-pbx`, `fcp-ivr`, `fcp-voicemail-server`
- Added `fcp-admin`, `fcp-api-gateway`, `fcp-events-server`, `fcp-jobs`
- Removed `fcp-server` monolith — `fcp-call-manager` serves as the primary entry point for small deployments

### Next: v1.0 Reference Architecture
- Complete Kubernetes deployment examples
- Service discovery (Redis-based registry)

## Current Status (v0.14.58)

The multi-binary architecture is **fully implemented**:

| Binary | Status |
|--------|--------|
| `fcp-sbc` | ✅ Done — enforcement, HMR, STIR/SHAKEN, SIPREC, LCR, SRTP SDP rewriting |
| `fcp-call-manager` (PBX mode) | ✅ Done — registrar + B2BUA + media + CFNA |
| `fcp-call-manager` (trunks mode) | ✅ Done — trunk routing, LCR, carrier failover |
| `fcp-registrar` | ✅ Done — standalone persistent registrar |
| `fcp-pbx` | ✅ Done — parking, pickup, intercom, hunt groups |
| `fcp-ivr` | ✅ Done — DID-to-flow routing |
| `fcp-voicemail-server` | ✅ Done — DID-to-mailbox routing, greeting management API |
| `fcp-admin` | ✅ Done — REST API without SIP overhead |
| `fcp-api-gateway` | ✅ Done — reverse proxy, rate limiting |
| `fcp-events-server` | ✅ Done — multi-node event aggregation |
| `fcp-jobs` | ✅ Done — background CDR/job processing |
| `fcp-cli` | ✅ Done — CLI + TUI for administration |
| `fcp-conference` | ✅ Done — SIP conference bridge, PIN auth, recording, events |
| `fcp-media` (standalone) | ✅ Done — wraps forge-api, REST media engine |

Library extractions completed:
- ✅ `fcp-b2bua-core` — shared by SBC, call-manager, PBX
- ✅ `fcp-sbc-core` — SBC library (LCR, trunks, admission, HMR)
- ✅ `fcp-api-core` — shared REST API handlers

## Remaining Next Steps

1. **Kubernetes manifests** — deployment examples for distributed mode
2. **Service discovery** — Redis-based registry for dynamic topology

## Questions to Answer

1. **Service Discovery:** Redis vs Consul vs etcd?
2. **Inter-service auth:** mTLS vs shared secret vs JWT?
3. **Configuration management:** Environment vars vs config files vs config server?
4. **Monitoring:** Prometheus federation? Central metrics collector?
5. **Logging:** Centralized (ELK) or distributed?

---

**Status:** Fully Implemented (v0.14.58)
**Remaining:** Kubernetes deployment, service discovery
