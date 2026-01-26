# FCP Deployment Binaries - Design Document

## Overview

FCP needs to support two deployment models:
1. **All-in-one** (`fcp-server`) - Small business, development, testing
2. **Distributed** (multiple specialized binaries) - Scalable, carrier-grade deployments

This document defines the architecture for building independent, purpose-built binaries for each service type.

## Problem Statement

Currently, `fcp-server` compiles ALL features into a single monolithic binary:
- SBC capabilities
- Registrar
- B2BUA/Call processing
- Voicemail
- IVR/Flow engine
- Media processing
- CDR
- Admin API

**Issues:**
- Cannot scale individual components independently
- Voicemail servers don't need SBC code
- SBC nodes don't need voicemail/IVR code
- Larger binary size and memory footprint
- Security: edge nodes shouldn't have admin API access

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
     │  :5060  │           │  :5060  │          - No DB access
     └────┬────┘           └────┬────┘          - No admin API
          │                     │               - NAT/Topology hiding
          └──────────┬──────────┘
                     │
          ┌──────────▼────────────┐
          │  fcp-session-manager  │             Internal Network
          │   (Call Routing)      │             - Redis for state
          └──────────┬────────────┘             - Event bus pub/sub
                     │
       ┌─────────────┼─────────────┐
       │             │             │
  ┌────▼────┐   ┌───▼───┐    ┌────▼────┐
  │fcp-pbx  │   │fcp-pbx│    │fcp-     │        Call Processing
  │  Node 1 │   │Node 2 │    │registrar│        - Full DB access
  └────┬────┘   └───┬───┘    └────┬────┘        - Feature-rich
       │            │             │
       ├────────────┴─────────────┤
       │                          │
  ┌────▼────────┐         ┌──────▼────────┐
  │ fcp-        │         │ fcp-          │     Applications
  │ voicemail   │         │ conference    │     - Specialized
  │ (2 nodes)   │         │               │     - Independently scalable
  └─────────────┘         └───────────────┘
       │                          │
       └────────────┬─────────────┘
                    │
         ┌──────────▼──────────┐
         │   PostgreSQL/Redis   │              Storage Layer
         └─────────────────────┘
```

## Binary Definitions

### 1. `fcp-sbc` (Session Border Controller)

**Purpose:** Edge security, NAT traversal, topology hiding

**Includes:**
- SIP transport (UDP/TCP/TLS)
- Transaction layer
- Basic authentication (to reject invalid traffic)
- Topology hiding (Via header manipulation)
- NAT traversal (ICE, STUN, TURN)
- Rate limiting
- DOS protection
- Metrics/logging

**Excludes:**
- Voicemail
- IVR/Flow engine
- Conference
- Call detail records (just forwards to session manager)
- Admin API (read-only health check only)
- Database access (optional Redis for rate limiting)

**Configuration:**
```toml
[sbc]
bind_addresses = ["0.0.0.0:5060", "0.0.0.0:5061"]
protocols = ["udp", "tcp", "tls"]
upstream_session_manager = "sip:sm.internal.example.com:5060"
topology_hiding = true
nat_traversal = true
max_calls_per_second = 1000
```

---

### 2. `fcp-session-manager` (Call Routing Brain)

**Purpose:** Centralized call routing and session coordination

**Includes:**
- SIP Proxy logic
- Routing engine (LCR, DID routing)
- Session state management
- Call distribution to PBX nodes
- Dialog tracking
- Redis for distributed state
- Event bus publisher

**Excludes:**
- Media handling
- Voicemail
- IVR
- User registration (forwards to fcp-registrar)
- Admin API (read-only metrics)

**Configuration:**
```toml
[session_manager]
bind_address = "0.0.0.0:5060"
redis_url = "redis://redis.internal:6379"
event_bus = "redis://redis.internal:6379"
pbx_nodes = [
    "sip:pbx1.internal:5060",
    "sip:pbx2.internal:5060"
]
routing_strategy = "round_robin" # or "least_calls", "hash"
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
- Full B2BUA
- Call forking (parallel/sequential)
- Call transfer (blind/attended)
- Call parking/pickup
- Do Not Disturb
- Call forwarding rules
- Extension-to-extension calls
- Integration with voicemail/IVR (via SIP REFER/INVITE)
- Call detail records
- Full database access
- REST API client (for webhooks)

**Excludes:**
- SBC functionality (no edge security)
- Registrar (queries it via API/Redis)

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

### 5. `fcp-voicemail` (Voicemail Server)

**Purpose:** Voicemail storage, retrieval, and MWI

**Includes:**
- Voicemail mailbox management
- Message storage (filesystem or S3)
- MWI (Message Waiting Indicator) via NOTIFY
- Voicemail playback (SIP INVITE)
- Greeting management
- Message transcription (optional)
- Database access for mailbox config

**Excludes:**
- Call routing
- SBC
- Conference
- Admin API (except voicemail-specific endpoints)

**Configuration:**
```toml
[voicemail]
bind_address = "0.0.0.0:5060"
database_url = "postgres://..."
storage_backend = "s3" # or "local"
storage_path = "s3://voicemail-bucket/"
enable_transcription = true
transcription_api = "https://transcribe.api/"
```

---

### 6. `fcp-conference` (Conference Bridge)

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

### 7. `fcp-media` (Media Server)

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

### 8. `fcp-server` (All-in-One)

**Purpose:** Small business, development, testing

**Includes:** EVERYTHING from all the above binaries

**Use Cases:**
- Small offices (< 100 users)
- Development environment
- Demo/testing
- Docker Compose single-container deployment

**Configuration:** Superset of all other configs

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

**Goal:** Ensure all functionality is in shared library crates, not in `fcp-server` binary

#### 1.1 Audit Current Code
- [x] `fcp-sip` - SIP handling (✓ already a library)
- [x] `fcp-core` - Core types (✓ already a library)
- [ ] Extract SBC-specific code from `fcp-server` → `fcp-sbc-core` library
- [ ] Extract registrar code → `fcp-registrar-core` library
- [ ] Extract voicemail logic → `fcp-voicemail-core` library
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
│   ├── fcp-sbc-core/       # NEW: SBC library
│   ├── fcp-registrar-core/ # NEW: Registrar library
│   ├── fcp-pbx-core/       # NEW: PBX/B2BUA library
│   ├── fcp-voicemail-core/ # NEW: Voicemail library
│   ├── fcp-conference-core/# NEW: Conference library
│   ├── fcp-media-core/     # NEW: Media library
│   └── fcp-admin-core/     # NEW: Admin API library
│
├── bins/
│   ├── fcp-sbc/            # NEW: SBC binary
│   ├── fcp-session-manager/# NEW: Session manager binary
│   ├── fcp-registrar/      # NEW: Registrar binary
│   ├── fcp-pbx/            # NEW: PBX binary
│   ├── fcp-voicemail/      # NEW: Voicemail binary
│   ├── fcp-conference/     # NEW: Conference binary
│   ├── fcp-media/          # NEW: Media server binary
│   ├── fcp-admin/          # NEW: Admin API binary
│   └── fcp-server/         # EXISTING: All-in-one binary
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
    SessionManager(SessionManagerConfig),
    Registrar(RegistrarConfig),
    Pbx(PbxConfig),
    Voicemail(VoicemailConfig),
    Conference(ConferenceConfig),
    Media(MediaConfig),
    Admin(AdminConfig),
    AllInOne(AllInOneConfig),
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
session_manager = "sip:sm.internal:5060"

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
- SBC → Session Manager: SIP INVITE
- Session Manager → PBX: SIP INVITE
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

`docker-compose.yml`:
```yaml
version: '3.8'

services:
  # All-in-one deployment
  fcp-server:
    image: fcp/fcp-server:latest
    ports:
      - "5060:5060/udp"
      - "5060:5060/tcp"
      - "8080:8080"
    environment:
      - FCP_DATABASE_URL=postgres://fcp:password@db:5432/fcp
      - FCP_REDIS_URL=redis://redis:6379
    volumes:
      - ./fcp.toml:/etc/fcp/fcp.toml
      - ./data:/var/lib/fcp
    depends_on:
      - db
      - redis

  db:
    image: postgres:15
    environment:
      - POSTGRES_DB=fcp
      - POSTGRES_USER=fcp
      - POSTGRES_PASSWORD=password
    volumes:
      - postgres_data:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    volumes:
      - redis_data:/data

volumes:
  postgres_data:
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
        - name: FCP_SESSION_MANAGER
          value: "sip:fcp-session-manager-service:5060"
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

## Migration Path

### Step 1: Keep `fcp-server` Working (v0.10)
- Current monolithic binary continues to work
- All code stays in `crates/`
- No breaking changes

### Step 2: Extract Libraries (v0.11-0.12)
- Create `fcp-*-core` library crates
- Move code from `fcp-server/` to libraries
- Add feature gates
- `fcp-server` depends on all libraries with all features

### Step 3: Create First Binary - `fcp-sbc` (v0.13)
- Implement `bins/fcp-sbc/`
- Test in parallel with `fcp-server`
- Document deployment

### Step 4: Create More Binaries (v0.14-0.16)
- `fcp-registrar`
- `fcp-pbx`
- `fcp-voicemail`
- Each release adds one new binary

### Step 5: Reference Architecture (v1.0)
- Complete Kubernetes deployment examples
- Terraform modules
- Production-ready multi-binary deployment

## Next Steps

1. **Create library crates** for major subsystems
2. **Add feature gates** to control compilation
3. **Extract configuration** into per-service configs
4. **Build first specialized binary** (fcp-sbc)
5. **Test deployment** side-by-side with fcp-server
6. **Document and iterate**

## Questions to Answer

1. **Service Discovery:** Redis vs Consul vs etcd?
2. **Inter-service auth:** mTLS vs shared secret vs JWT?
3. **Configuration management:** Environment vars vs config files vs config server?
4. **Monitoring:** Prometheus federation? Central metrics collector?
5. **Logging:** Centralized (ELK) or distributed?

---

**Status:** Design Phase
**Target:** v0.13+ (after Phase 1 MVP is complete)
**Owner:** Architecture Team
