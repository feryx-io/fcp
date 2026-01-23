# Ferrous Communications Platform (FCP)

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.74%2B-orange.svg)](https://www.rust-lang.org/)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()

> A next-generation, carrier-grade unified communications platform written entirely in Rust.

FCP is a multi-tenant, API-first alternative to FreeSWITCH, Kamailio, and Asterisk, designed with memory safety, modern async I/O, and observability built-in from day one.

---

## 🚀 Features

### Phase 1 (Current - MVP)

- ✅ **Production-Grade SIP Stack** (siphon-rs)
  - RFC 3261 compliant (INVITE, BYE, CANCEL, ACK, REGISTER, OPTIONS)
  - RFC 3262 PRACK support (reliable provisionals)
  - RFC 3263 DNS resolution (NAPTR/SRV/A/AAAA)
  - RFC 7616/7617 Digest authentication (MD5/SHA-256/SHA-512)
  - Multi-transport: UDP, TCP, TLS, WebSocket
  - Dialog and subscription management
  - tel URI support (RFC 3966)

- ✅ **B2BUA (Back-to-Back User Agent)**
  - Parallel forking with proper provisional handling
  - Early media support (183 Session Progress)
  - Media anchoring and bridging
  - Call transfer support (REFER, Replaces)

- ✅ **Multi-Tenant Support**
  - Domain-based tenant isolation
  - Tenant-scoped routing and extensions
  - Per-tenant configuration

- ✅ **Call Routing**
  - Extension-based routing
  - DID routing to extensions
  - Policy-based routing engine
  - Line binding management

- ✅ **Voicemail System**
  - Mailbox management with custom greetings
  - Message recording and retrieval
  - Status tracking (new/heard/saved/deleted)
  - REST API for mailbox management
  - Flow engine integration

- ✅ **Flow Engine (Call Flows)**
  - JSON-based flow definitions
  - Twilio Studio-inspired DSL
  - Nodes: Answer, Play, Say, Gather, Dial, Voicemail, Branch, HTTP, etc.
  - Variable interpolation
  - Webhook support

- ✅ **REST API**
  - User management
  - Device management
  - Extension management
  - Phone number management
  - Line binding management
  - CDR (Call Detail Records)
  - Flow management
  - Voicemail management
  - Real-time event streaming (SSE)

- ✅ **Storage Backends**
  - SQLite (default for single-node)
  - PostgreSQL
  - MySQL
  - MongoDB

- ✅ **Observability**
  - Prometheus metrics export
  - Structured logging (tracing)
  - Real-time TUI (Terminal UI)
  - Debug API endpoints

- ✅ **Media Handling**
  - RTP/RTCP relay
  - DTMF collection
  - Call recording (basic)
  - SDP manipulation

---

## 📦 Quick Start

### Prerequisites

- Rust 1.74 or later
- SQLite (included by default)
- Optional: PostgreSQL, MySQL, or MongoDB

### Installation

```bash
# Clone the repository with submodules
git clone --recursive https://github.com/thevoiceguy/fcp.git
cd fcp

# Build the project
cargo build --release

# Run the server
./target/release/fcp-server
```

### Docker Deployment (Recommended)

```bash
# Using docker-compose (includes PostgreSQL)
docker-compose up -d

# Access the API
curl http://localhost:8080/api/v1/health
```

### Configuration

Create a configuration file at `config/fcp.toml`:

```toml
[server]
bind_address = "0.0.0.0:8080"
log_level = "info"
log_format = "json"

[sip]
bind_address = "0.0.0.0:5060"
external_ip = "192.168.1.100"
domain = "pbx.example.com"

[storage]
backend = "sqlite"
database_path = "/var/lib/fcp/fcp.db"

[media]
rtp_port_range = "10000-20000"
recording_dir = "/var/lib/fcp/recordings"

[voicemail]
enabled = true
recordings_dir = "/var/lib/fcp/voicemail"
max_message_duration = 300
```

Run with custom config:

```bash
./target/release/fcp-server --config config/fcp.toml
```

---

## 🏗️ Architecture

FCP uses a layered architecture:

```
┌─────────────────────────────────────────────────────┐
│                   REST API Layer                    │
│         (User, Device, Extension Management)         │
└─────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────┐
│               Application Layer                      │
│    (Flow Engine, IVR, Voicemail, Routing)           │
└─────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────┐
│                SIP Signaling Layer                   │
│      (Registrar, B2BUA, Proxy, Call Manager)        │
└─────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────┐
│                  Media Layer                         │
│         (RTP Engine, Recording, DTMF)                │
└─────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────┐
│                 Storage Layer                        │
│     (SQLite, PostgreSQL, MySQL, MongoDB)            │
└─────────────────────────────────────────────────────┘
```

### Key Components

- **fcp-core**: Core types, identifiers, traits, error handling
- **fcp-sip**: SIP integration layer (bridges fcp-core and siphon-rs)
- **fcp-routing**: Policy-based routing engine
- **fcp-media**: Media session management, RTP relay
- **fcp-flow**: Flow engine for programmable call flows
- **fcp-voicemail**: Voicemail service implementation
- **fcp-server**: Main binary with REST API
- **fcp-cli**: Terminal UI for monitoring
- **siphon-rs**: Production-grade SIP stack (submodule)

---

## 📚 API Documentation

### Authentication

Currently, the API uses basic authentication (production OAuth2/JWT coming in Phase 2).

### Core Endpoints

#### Users
```bash
# List users
GET /api/v1/users

# Create user
POST /api/v1/users
{
  "username": "alice",
  "display_name": "Alice Smith",
  "email": "alice@example.com",
  "password": "secure_password"
}

# Get user
GET /api/v1/users/alice

# Update user
PUT /api/v1/users/alice

# Delete user
DELETE /api/v1/users/alice
```

#### Extensions
```bash
# List extensions
GET /api/v1/extensions

# Create extension
POST /api/v1/extensions
{
  "number": "1001",
  "display_name": "Alice Desk",
  "extension_type": "user"
}

# Get extension
GET /api/v1/extensions/1001
```

#### Voicemail
```bash
# List mailboxes
GET /api/v1/voicemail/mailboxes

# Create mailbox
POST /api/v1/voicemail/mailboxes
{
  "tenant_id": "tenant-xxx",
  "user_id": "user-xxx",
  "line_binding_id": "line-xxx",
  "extension": "1001",
  "pin": "1234"
}

# List messages for mailbox
GET /api/v1/voicemail/mailboxes/{id}/messages

# Get message
GET /api/v1/voicemail/messages/{id}

# Update message status
PUT /api/v1/voicemail/messages/{id}
{
  "status": "heard"
}

# Delete message
DELETE /api/v1/voicemail/messages/{id}
```

#### Call Detail Records (CDR)
```bash
# List CDRs
GET /api/v1/cdrs?limit=100&offset=0

# Get specific CDR
GET /api/v1/cdrs/{call_id}
```

#### Flows
```bash
# List flows
GET /api/v1/flows

# Create flow
POST /api/v1/flows
{
  "name": "Main IVR",
  "initial_node": "answer",
  "nodes": {
    "answer": {
      "id": "answer",
      "type": "answer",
      "next": "play_greeting"
    },
    "play_greeting": {
      "id": "play_greeting",
      "type": "play",
      "url": "/audio/greeting.wav",
      "next": "gather"
    }
  }
}
```

For complete API documentation, see [API.md](docs/API.md).

---

## 🧪 Testing

### Run All Tests

```bash
# Run unit and integration tests
cargo test --all

# Run with output
cargo test --all -- --nocapture

# Run specific crate tests
cargo test -p fcp-core
cargo test -p fcp-sip
```

### SIP Testing with siphond

```bash
# Start test registrar
cargo run -p siphond -- --mode registrar

# Start B2BUA mode
cargo run -p siphond -- --mode b2bua
```

### Load Testing

```bash
# Coming soon - SIPp scenarios for load testing
```

---

## 🔒 Security

### Current Security Features

- Digest authentication (RFC 7616/7617)
- TLS transport support
- Multi-tenant isolation
- Input validation and sanitization

### Security Roadmap

- [ ] Rate limiting on API endpoints
- [ ] OAuth2/JWT authentication
- [ ] RBAC (Role-Based Access Control)
- [ ] STIR/SHAKEN for caller ID authentication
- [ ] Encryption at rest (KMS integration)

### Reporting Security Issues

Please report security vulnerabilities to security@example.com (private disclosure).

---

## 📊 Monitoring

### Prometheus Metrics

FCP exports Prometheus metrics at `/api/v1/metrics`:

- Call statistics (active calls, total calls, failed calls)
- SIP transaction metrics
- Registration counts
- Media session statistics
- HTTP request metrics

### Grafana Dashboards

Import the pre-built dashboards from `deploy/grafana/` directory.

### Logs

Structured JSON logging with tracing spans:

```bash
# Set log level
export RUST_LOG=info

# Pretty format for development
export FCP_LOG_FORMAT=pretty

# JSON format for production
export FCP_LOG_FORMAT=json
```

---

## 🛠️ Development

### Project Structure

```
fcp/
├── crates/               # Rust crates
│   ├── fcp-core/        # Core types and traits
│   ├── fcp-sip/         # SIP integration
│   ├── fcp-routing/     # Routing engine
│   ├── fcp-media/       # Media handling
│   ├── fcp-flow/        # Flow engine
│   ├── fcp-voicemail/   # Voicemail service
│   ├── fcp-server/      # Main server binary
│   ├── fcp-cli/         # Terminal UI
│   └── fcp-storage-*/   # Storage backends
├── external/
│   └── siphon-rs/       # SIP stack (submodule)
├── config/              # Configuration examples
├── deploy/              # Deployment files
│   ├── docker/
│   └── kubernetes/
├── docs/                # Documentation
└── tests/               # Integration tests
```

### Building from Source

```bash
# Debug build (faster compilation)
cargo build

# Release build (optimized)
cargo build --release

# Build specific crate
cargo build -p fcp-server

# Watch and rebuild on changes
cargo watch -x "build -p fcp-server"
```

### Code Style

```bash
# Format code
cargo fmt --all

# Lint with clippy
cargo clippy --all -- -D warnings

# Check without building
cargo check --all
```

### Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines.

---

## 🗺️ Roadmap

### Phase 1: MVP (Current - 95% Complete)
- ✅ Core SIP functionality
- ✅ B2BUA with parallel forking
- ✅ Multi-tenant support
- ✅ REST API
- ✅ Voicemail system
- ✅ Flow engine
- 🚧 Docker deployment
- 🚧 Documentation

### Phase 2: Programmable Voice (Planned - Q2 2026)
- [ ] Enhanced flow engine (more nodes)
- [ ] Call recording APIs
- [ ] Webhook dispatcher
- [ ] WebSocket API (bidirectional)
- [ ] Web admin UI
- [ ] PostgreSQL optimizations

### Phase 3: Applications & Clustering (Planned - Q3 2026)
- [ ] Queue/ACD service
- [ ] IVR voicemail menu
- [ ] Presence server
- [ ] Clustering with SWIM
- [ ] Redis for distributed state
- [ ] WebRTC gateway
- [ ] gRPC API

### Phase 4: Enterprise Features (Planned - Q4 2026)
- [ ] LCR (Least Cost Routing)
- [ ] Fraud detection
- [ ] Audio transcoding
- [ ] CDR analytics
- [ ] KMS integration

### Phase 5: Carrier Grade (Planned - 2027)
- [ ] Multi-region support
- [ ] STIR/SHAKEN
- [ ] E911 integration
- [ ] Video conferencing
- [ ] Regulatory compliance

---

## 📄 License

FCP is dual-licensed under:

- MIT License ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)

You may choose either license for your use.

---

## 🙏 Acknowledgments

- **siphon-rs**: Production-grade SIP stack developed specifically for FCP
- **Rust Community**: For amazing async runtime and ecosystem
- **Twilio Studio**: Inspiration for flow engine design
- **FreeSWITCH/Asterisk**: Reference implementations for VoIP patterns

---

## 📞 Support

- **Documentation**: https://fcp-docs.example.com
- **Issues**: https://github.com/thevoiceguy/fcp/issues
- **Discussions**: https://github.com/thevoiceguy/fcp/discussions
- **Discord**: https://discord.gg/fcp (community chat)

---

## 🌟 Star History

[![Star History Chart](https://api.star-history.com/svg?repos=thevoiceguy/fcp&type=Date)](https://star-history.com/#thevoiceguy/fcp&Date)

---

**Built with ❤️ and 🦀 Rust**
