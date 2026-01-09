# FCP Progress Assessment & Next Steps

**Date:** 2026-01-09
**Phase:** Phase 1 - MVP Foundation (Months 1-4)

---

## Executive Summary

We are approximately **60-70% complete** with Phase 1 MVP. The siphon-rs SIP stack is production-ready and fully RFC-compliant. The FCP integration layer is functional with a working B2BUA. The immediate next steps focus on completing the data plane components and adding the REST API.

---

## Current Status: What's ✅ Complete

### Core Infrastructure (100%)
- ✅ **fcp-core**: Complete with types, identifiers, error handling, config structures
- ✅ **Workspace setup**: Cargo workspace, CI/CD placeholder structure
- ✅ **Submodule**: siphon-rs integrated as git submodule

### SIP Stack - siphon-rs (100%)
The entire siphon-rs stack is production-ready and RFC-compliant:

- ✅ **sip-core**: Complete SIP parser, URI, headers, methods, status codes
- ✅ **sip-parse**: nom-based parser for SIP messages
- ✅ **sip-transport**: UDP/TCP/TLS with connection pooling
- ✅ **sip-transaction**: RFC 3261 transaction state machines with transport-aware optimization
- ✅ **sip-dialog**: Dialog and subscription management
- ✅ **sip-dns**: RFC 3263 DNS resolution (NAPTR→SRV→A/AAAA)
- ✅ **sip-auth**: RFC 7616/7617 Digest authentication (MD5/SHA-256/SHA-512)
- ✅ **sip-registrar**: REGISTER handling with pluggable LocationStore
- ✅ **sip-uac**: User Agent Client with IntegratedUAC
- ✅ **sip-uas**: User Agent Server helpers
- ✅ **sip-proxy**: Proxy primitives
- ✅ **sip-sdp**: SDP support
- ✅ **sip-observe**: Observability (metrics, tracing)
- ✅ **sip-testkit**: Test utilities

**Recent Fix:** CallHandle.dialog now updates automatically on 200 OK (no more "pending" tag)

### FCP Integration Layer (90%)
- ✅ **fcp-sip**: Complete integration between fcp-core and siphon-rs
  - ✅ Type adapters and conversions
  - ✅ TransportManager
  - ✅ Service wrappers (Transaction, Dialog, Registration)
  - ✅ B2BUA implementation with RFC 3261 compliance
  - ✅ SIP handler with INVITE/ACK/BYE/CANCEL/REGISTER/OPTIONS support
  - ✅ To-tag consistency fix (100% RFC 3261 §12.1.2 compliant)
  - ✅ Automatic dialog update from siphon-rs

### Media Layer (40%)
- ✅ **fcp-media**: Basic RTP relay structure
  - ✅ Port allocation
  - ✅ Session management
  - ✅ forge-media integration (RecordingMixer with timestamp tracking)
  - ⚠️  Recording tested but needs production hardening
  - ❌ RTCP support (future)
  - ❌ Transcoding (Phase 4)

### Routing Layer (50%)
- ✅ **fcp-routing**: Basic routing engine structure
  - ✅ Static route definitions
  - ✅ Route matching by prefix
  - ⚠️  Basic gateway failover (needs testing)
  - ❌ LCR with carrier health (Phase 4)
  - ❌ Real-time carrier metrics (Phase 4)

### Runtime & Server (60%)
- ✅ **fcp-runtime**: Basic service coordination
  - ✅ Runtime builder
  - ✅ Lifecycle management
  - ⚠️  Message handlers (partially implemented)
- ✅ **fcp-server**: Main binary
  - ✅ CLI with clap
  - ✅ Configuration loading (TOML + env vars)
  - ✅ Graceful shutdown
  - ⚠️  REST API (partially implemented in src/api)
  - ❌ WebSocket API (Phase 2)

### Testing & Demos (70%)
- ✅ **siphond**: Multi-mode SIP testing daemon
  - ✅ Minimal mode (OPTIONS only)
  - ✅ Full UAS mode
  - ✅ Registrar mode with authentication
  - ✅ Proxy mode
  - ✅ B2BUA mode
- ✅ **test_recording.sh**: RTP recording test script
- ✅ Manual interop testing with Odin (confirmed working)

---

## What's ❌ Missing from Phase 1

### Priority 1: Critical for MVP (Est. 2-3 weeks)

1. **Complete REST API** (fcp-server/src/api) - **5 days**
   - [ ] GET /v1/calls - List active calls
   - [ ] GET /v1/calls/:id - Get call details
   - [ ] POST /v1/calls - Originate call
   - [ ] DELETE /v1/calls/:id - Hangup call
   - [ ] GET /v1/registrations - List registrations
   - [ ] DELETE /v1/registrations/:aor - Deregister
   - [ ] GET /v1/health - Health check
   - [ ] GET /metrics - Prometheus metrics

2. **Storage Layer** (fcp-storage-sqlite) - **3 days**
   - [ ] SQLite database setup
   - [ ] Migrations
   - [ ] CDR storage
   - [ ] Registration persistence
   - [ ] Configuration storage

3. **CLI Tool** (fcp-cli) - **2 days**
   - [ ] Server control (start, stop, status)
   - [ ] Call management
   - [ ] Registration management
   - [ ] Configuration management

4. **Docker Deployment** - **2 days**
   - [ ] Dockerfile for fcp-server
   - [ ] Health checks
   - [ ] Volume management
   - [ ] docker-compose.yml example

5. **Documentation** - **3 days**
   - [ ] README with getting started
   - [ ] API documentation
   - [ ] Configuration reference
   - [ ] Deployment guide

### Priority 2: Nice to Have (Can defer to Phase 2)

6. **Enhanced Monitoring**
   - [ ] Structured logging improvements
   - [ ] Additional Prometheus metrics
   - [ ] Tracing with OpenTelemetry

7. **Enhanced Media**
   - [ ] RTCP support
   - [ ] Media quality monitoring
   - [ ] Packet loss concealment

8. **Enhanced Routing**
   - [ ] Number normalization
   - [ ] Destination rewrite rules
   - [ ] Least cost routing (basic)

---

## Phase 1 vs Architecture Alignment

### ✅ On Track Components

| Component | Architecture Plan | Current Status | Notes |
|-----------|------------------|----------------|-------|
| sip-core | Complete SIP stack | ✅ Done | Production-ready |
| sip-transport | UDP/TCP/TLS | ✅ Done | All transports working |
| sip-transaction | State machines | ✅ Done | RFC 3261 compliant |
| sip-dialog | Dialog management | ✅ Done | With PRACK support |
| fcp-core | Foundation types | ✅ Done | All identifiers, errors |
| fcp-sip | Integration layer | ✅ Done | B2BUA working |
| fcp-media | RTP relay | ⚠️ Partial | Basic relay works |
| fcp-runtime | Coordination | ⚠️ Partial | Basic lifecycle |

### ❌ Gap Components (Planned for Later Phases)

| Component | Phase | Status | Priority |
|-----------|-------|--------|----------|
| fcp-event-bus | Phase 2 | Not started | Medium |
| fcp-flow-engine | Phase 2 | Not started | High |
| fcp-ivr | Phase 2 | Not started | Medium |
| fcp-webhook-dispatcher | Phase 2 | Not started | High |
| fcp-queue | Phase 3 | Not started | Medium |
| fcp-voicemail | Phase 3 | Not started | Medium |
| fcp-cluster | Phase 3 | Not started | Medium |
| fcp-routing-engine (LCR) | Phase 4 | Not started | Low |
| fcp-transcoder | Phase 4 | Not started | Low |

---

## Immediate Next Steps (Next 2-4 Weeks)

### Week 1-2: Complete MVP Core

**Goal:** Production-ready single-node PBX

#### Task 1: Complete REST API (5 days)
```rust
// Endpoints needed:
GET    /v1/calls                 - List active calls
POST   /v1/calls                 - Originate new call
GET    /v1/calls/:id             - Get call details
DELETE /v1/calls/:id             - Hangup call

GET    /v1/registrations         - List registrations
DELETE /v1/registrations/:aor    - Deregister

GET    /v1/health                - Health check
GET    /metrics                  - Prometheus metrics
```

**Implementation Steps:**
1. Create `crates/fcp-api` with axum routes
2. Implement CallsApi struct with access to B2BUA
3. Add JSON serialization for Call, Registration types
4. Wire into fcp-server main.rs
5. Add OpenAPI/Swagger docs

#### Task 2: Add Storage Layer (3 days)
```bash
# Create new crate
cargo new --lib crates/fcp-storage-sqlite

# Tables needed:
- calls (CDR)
- registrations
- config
- routes
```

**Schema:**
```sql
CREATE TABLE calls (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    from_uri TEXT NOT NULL,
    to_uri TEXT NOT NULL,
    start_time INTEGER NOT NULL,
    answer_time INTEGER,
    end_time INTEGER,
    duration_ms INTEGER,
    disposition TEXT,
    hangup_cause TEXT,
    sip_call_id TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE registrations (
    aor TEXT PRIMARY KEY,
    contact_uri TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    user_agent TEXT,
    received_from TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
```

#### Task 3: Build CLI Tool (2 days)
```bash
# Create new crate
cargo new crates/fcp-cli

# Commands:
fcp-cli server start
fcp-cli server stop
fcp-cli server status
fcp-cli calls list
fcp-cli calls hangup <call-id>
fcp-cli registrations list
fcp-cli registrations delete <aor>
```

### Week 3: Docker & Documentation

#### Task 4: Docker Deployment (2 days)
```dockerfile
# Dockerfile
FROM rust:1.80 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin fcp-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/fcp-server /usr/local/bin/
EXPOSE 5060/udp 5060/tcp 8080/tcp 9090/tcp
HEALTHCHECK --interval=30s --timeout=3s CMD curl -f http://localhost:8080/v1/health || exit 1
ENTRYPOINT ["/usr/local/bin/fcp-server"]
```

```yaml
# docker-compose.yml
version: '3.8'
services:
  fcp:
    build: .
    ports:
      - "5060:5060/udp"
      - "5060:5060/tcp"
      - "8080:8080"       # REST API
      - "9090:9090"       # Metrics
    volumes:
      - ./fcp-data:/var/lib/fcp
    environment:
      - FCP_LOG_LEVEL=info
      - FCP_LOCAL_URI=sip:fcp@192.168.1.10
```

#### Task 5: Documentation (3 days)
1. **README.md**: Getting started guide
2. **API.md**: REST API reference
3. **CONFIG.md**: Configuration reference
4. **DEPLOY.md**: Deployment guide
5. Update **CLAUDE.md** with latest progress

### Week 4: Testing & Polish

#### Task 6: Integration Testing
1. Test with multiple softphones (Odin, Linphone, Zoiper)
2. Test with SIP trunk (Telnyx/Twilio)
3. Load testing (100+ concurrent calls)
4. Failure scenarios (network issues, crashes)

#### Task 7: Performance Tuning
1. Profile with cargo-flamegraph
2. Optimize hot paths
3. Memory leak detection
4. Connection pool tuning

---

## Phase 2 Planning (Months 5-8)

### High-Level Goals
1. **Programmable Voice** (Twilio-like)
2. **Webhooks** for event notifications
3. **WebSocket API** for real-time events
4. **Basic IVR** with call flow DSL
5. **Recording** support (production-ready)

### Key Deliverables
- `fcp-event-bus`: Redis-backed event streaming
- `fcp-flow-engine`: Call flow DSL runtime
- `fcp-ivr`: IVR service
- `fcp-webhook-dispatcher`: Outbound webhooks
- `fcp-api-websocket`: Real-time events
- PostgreSQL backend option

---

## Critical Decisions Needed

### Decision 1: Storage Backend Priority
**Options:**
- A) SQLite only for MVP (simpler, faster to market)
- B) PostgreSQL for MVP (better for multi-tenant)
- C) Both with abstraction layer

**Recommendation:** **A - SQLite only** for MVP, add PostgreSQL in Phase 2

**Rationale:**
- Faster to implement
- Single-node deployment is simpler
- Can migrate data later with tools

### Decision 2: REST API Framework
**Current:** Using axum (embedded in fcp-server)

**Options:**
- A) Keep embedded in fcp-server
- B) Separate `fcp-api` crate
- C) Separate service process

**Recommendation:** **B - Separate `fcp-api` crate** but same process

**Rationale:**
- Better code organization
- Reusable for testing
- Still single binary deployment

### Decision 3: Recording Implementation
**Current:** forge-media integration working but not production-ready

**Options:**
- A) Complete forge-media integration
- B) Use external recorder (Asterisk, FreeSWITCH)
- C) Build custom RTP recorder

**Recommendation:** **A - Complete forge-media** in Phase 2

**Rationale:**
- Already integrated
- Rust-native (memory safety)
- Flexible architecture

### Decision 4: Event Bus Timing
**Question:** When to implement event bus?

**Options:**
- A) Phase 1 (now)
- B) Phase 2 (with webhooks)
- C) Phase 3 (with clustering)

**Recommendation:** **B - Phase 2** with webhooks

**Rationale:**
- Not needed for basic PBX
- Webhooks are the first consumer
- Can use simpler in-memory events for MVP

---

## Risk Assessment & Mitigation

### High Risk Items

1. **Risk:** Media quality issues in production
   - **Impact:** Poor call quality, dropped audio
   - **Mitigation:** Extensive testing, add RTCP monitoring
   - **Timeline:** Address in Phase 1.5 (after MVP)

2. **Risk:** NAT traversal failures
   - **Impact:** One-way audio, failed calls
   - **Mitigation:** Implement proper rport/received handling (already done), add STUN
   - **Timeline:** STUN in Phase 2

3. **Risk:** Database performance at scale
   - **Impact:** Slow API responses, CDR bottleneck
   - **Mitigation:** Use proper indexes, async writes, connection pooling
   - **Timeline:** Phase 1 (include in storage layer)

### Medium Risk Items

4. **Risk:** Memory leaks in long-running deployment
   - **Impact:** Server crashes, requires restarts
   - **Mitigation:** Extensive leak testing, valgrind, production monitoring
   - **Timeline:** Phase 1 testing week

5. **Risk:** Transaction timeout issues
   - **Impact:** Hung calls, zombie dialogs
   - **Mitigation:** Proper cleanup on timeout, dialog reaping
   - **Timeline:** Already implemented, needs testing

---

## Success Metrics

### MVP Completion Criteria (Phase 1)

#### Functional Requirements
- [ ] Register 10+ endpoints simultaneously
- [ ] Complete 100+ concurrent calls
- [ ] Zero-downtime for established calls during load
- [ ] Proper CDR for all calls
- [ ] REST API response time < 100ms
- [ ] Prometheus metrics exporting

#### Quality Requirements
- [ ] Zero memory leaks in 24-hour test
- [ ] < 1% call failure rate
- [ ] MOS > 4.0 for media quality
- [ ] 100% RFC 3261 compliance (already achieved)

#### Documentation Requirements
- [ ] README with quick start (< 5 minutes to first call)
- [ ] API documentation with examples
- [ ] Deployment guide (Docker + bare metal)
- [ ] Troubleshooting guide

---

## Resource Requirements

### Development Time Estimate
- **Remaining Phase 1 work:** 15-20 days (3-4 weeks)
- **Phase 2 planning:** 2-3 days
- **Total to MVP:** ~4 weeks

### Infrastructure Needed
- **Development:**
  - 2-3 softphones for testing
  - SIP trunk account (Telnyx/Twilio)
  - Docker host for testing

- **CI/CD:**
  - GitHub Actions (current)
  - Docker registry
  - Test infrastructure

---

## Recommendation: Prioritized Roadmap

### Next 4 Weeks (Complete Phase 1)

**Week 1:** REST API + Storage
- Days 1-3: Complete REST API endpoints
- Days 4-5: SQLite storage implementation

**Week 2:** CLI + Testing
- Days 1-2: CLI tool
- Days 3-5: Integration testing

**Week 3:** Docker + Documentation
- Days 1-2: Docker deployment
- Days 3-5: Documentation

**Week 4:** Polish + Release
- Days 1-2: Performance tuning
- Days 3-4: Bug fixes from testing
- Day 5: Release v0.1.0

### Weeks 5-8 (Start Phase 2)

**Focus:** Programmable Voice
- Week 5-6: Event bus + Webhooks
- Week 7-8: Call flow engine (basic)

---

## Conclusion

We are **70% complete with Phase 1 MVP**. The core SIP stack is production-ready and RFC-compliant. The immediate focus should be:

1. **Complete REST API** (highest priority)
2. **Add storage layer** (enables CDR, persistence)
3. **Build CLI tool** (usability)
4. **Docker deployment** (easier adoption)
5. **Documentation** (critical for users)

With 3-4 focused weeks, we can complete Phase 1 and have a production-ready single-node PBX that can compete with FreeSWITCH/Asterisk in core functionality while providing better memory safety, modern async I/O, and clean architecture for future phases.

The architecture from ARCHITECTURE-v0.2.md remains sound. We're on track, just need to complete the data plane and add the management layer (API + CLI + storage).

**Recommendation:** Execute the 4-week plan above to complete MVP, then reassess before starting Phase 2.
