# FCP Media Integration Plan
## forge-media Integration for Phase 1 Sprint 6

**Date**: 2025-12-24
**Status**: Ready to Begin Media Integration
**Target**: Sprint 6 - B2BUA & RTP (Weeks 11-12)

---

## 1. Progress Assessment

### ✅ What's Been Completed

Based on the Phase 1 plan and current codebase state:

**Sprints 1-5 (Weeks 1-10): COMPLETE** ✅

| Sprint | Scope | Status | Evidence |
|--------|-------|--------|----------|
| **Sprint 1** | Foundation (fcp-core, parser) | ✅ | fcp-core with error types, identifiers, config |
| **Sprint 2** | SIP Messages | ✅ | siphon-rs provides complete SIP message support |
| **Sprint 3** | Transport | ✅ | siphon-rs: UDP, TCP, TLS transports |
| **Sprint 4** | Transactions | ✅ | siphon-rs: Full RFC 3261 transaction layer |
| **Sprint 5** | Registrar & Proxy | ✅ | siphon-rs: Registrar, proxy primitives; siphond daemon |

**Additional Work Completed:**
- ✅ fcp-sip integration layer (adapters, service wrappers)
- ✅ fcp-runtime (service coordination, lifecycle)
- ✅ fcp-server (CLI, config loading, graceful shutdown)
- ✅ fcp-routing (declarative policy-based routing)
- ✅ siphond multi-mode daemon (minimal, full-uas, registrar, proxy, b2bua)

### 🎯 Current Position

**We are at Sprint 6: B2BUA & RTP Integration**

According to the Phase 1 plan, Sprint 6 (Weeks 11-12) requires:
- Call control (B2BUA functionality)
- Media relay (RTP forwarding)
- Complete calls between two endpoints with media

### 🚀 forge-media Capabilities (Available Now)

The forge-media submodule provides a **production-ready** media engine with:

**Core Media:**
- ✅ RTP/RTCP/SRTP packet handling
- ✅ Session management with bidirectional audio
- ✅ Codecs: G.711 (µ-law/A-law), G.722, G.729, Opus
- ✅ Automatic transcoding between codecs
- ✅ Jitter buffering

**Advanced Features:**
- ✅ WebRTC support (ICE, DTLS, SRTP)
- ✅ Audio conferencing with mixing
- ✅ Recording (WAV, Opus formats)
- ✅ DTMF detection and generation
- ✅ AI integration (OpenAI Realtime API)
- ✅ Prometheus metrics
- ✅ Security hardening (rate limiting, SSRF protection)

**Performance:**
- 1,000+ concurrent sessions per instance
- <1ms packet forwarding latency (p99)
- <20ms conference mixing latency

---

## 2. Integration Architecture

### 2.1 Component Relationship

```
┌─────────────────────────────────────────────────────────────┐
│                      FCP Platform                            │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌────────────────────────┐      ┌────────────────────────┐ │
│  │    SIGNALING PLANE     │      │     MEDIA PLANE        │ │
│  │   (siphon-rs / FCP)    │◄────►│    (forge-media)       │ │
│  │                        │      │                        │ │
│  │  • SIP Messages        │      │  • RTP/SRTP           │ │
│  │  • Transactions        │      │  • Codecs             │ │
│  │  • Dialogs            │      │  • Transcoding        │ │
│  │  • Registration       │      │  • Mixing             │ │
│  │  • Authentication     │      │  • Recording          │ │
│  │  • Routing            │      │  • DTMF               │ │
│  └────────────────────────┘      └────────────────────────┘ │
│            │                               ▲                 │
│            │                               │                 │
│            └───────────────┬───────────────┘                 │
│                            │                                 │
│                   ┌────────▼────────┐                        │
│                   │   fcp-media     │                        │
│                   │  Integration    │                        │
│                   │     Layer       │                        │
│                   └─────────────────┘                        │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### 2.2 Integration Layer Design

Create a new crate: **`fcp-media`**

**Purpose:**
- Bridge between SIP signaling (siphon-rs) and media handling (forge-media)
- Coordinate call legs and media sessions
- Implement B2BUA logic
- Provide unified API for FCP services

**Key Responsibilities:**
1. **Call Coordination**: Map SIP dialogs to media sessions
2. **SDP Negotiation**: Convert between SIP and forge-media SDP representations
3. **Codec Negotiation**: Select compatible codecs between call legs
4. **Media Lifecycle**: Start/stop/modify media sessions based on SIP state
5. **Event Correlation**: Link SIP and media events by CallId/DialogId

---

## 3. Implementation Plan

### Phase 1: Foundation (Week 1)

#### Task 1.1: Create fcp-media Crate

```bash
cargo new --lib crates/fcp-media
```

**Dependencies:**
```toml
[dependencies]
fcp-core = { path = "../fcp-core" }
fcp-sip = { path = "../fcp-sip" }
forge-media = { path = "../../external/forge-media" }

# Siphon for SIP types
sip-core = { path = "../../external/siphon-rs/crates/sip-core" }
sip-dialog = { path = "../../external/siphon-rs/crates/sip-dialog" }
sip-sdp = { path = "../../external/siphon-rs/crates/sip-sdp" }

# Async
tokio = { workspace = true }
async-trait = "0.1"

# Error handling
anyhow = { workspace = true }
thiserror = { workspace = true }

# Serialization
serde = { workspace = true }
serde_json = { workspace = true }

# Logging
tracing = { workspace = true }

# Collections
dashmap = "6"
```

#### Task 1.2: Define Core Types

**File: `crates/fcp-media/src/lib.rs`**

```rust
//! FCP Media Integration Layer
//!
//! Bridges SIP signaling (siphon-rs) with media handling (forge-media)

pub mod session;
pub mod codec;
pub mod sdp;
pub mod manager;
pub mod error;
pub mod config;

pub use error::{Error, Result};
pub use manager::MediaManager;
pub use session::{MediaSession, MediaSessionConfig};
pub use config::MediaConfig;
```

**File: `crates/fcp-media/src/error.rs`**

```rust
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Media error: {0}")]
    Media(#[from] forge_media::ForgeError),

    #[error("SDP error: {0}")]
    Sdp(String),

    #[error("Codec negotiation failed: {0}")]
    CodecNegotiation(String),

    #[error("Session not found: {0}")]
    SessionNotFound(fcp_core::CallId),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

#### Task 1.3: Define Media Session

**File: `crates/fcp-media/src/session.rs`**

```rust
use fcp_core::{CallId, DialogId, SessionId};
use forge_media::{ForgeEngine, SessionHandle};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Represents a media session tied to a SIP dialog
pub struct MediaSession {
    /// Internal FCP call identifier
    pub call_id: CallId,

    /// SIP dialog identifier
    pub dialog_id: DialogId,

    /// forge-media session identifier
    pub forge_session_id: SessionId,

    /// forge-media session handle
    pub forge_session: Arc<SessionHandle>,

    /// Current state
    pub state: RwLock<MediaSessionState>,

    /// Configuration
    pub config: MediaSessionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaSessionState {
    /// Negotiating SDP
    Negotiating,

    /// Media active
    Active,

    /// Media on hold
    OnHold,

    /// Terminated
    Terminated,
}

#[derive(Debug, Clone)]
pub struct MediaSessionConfig {
    /// Enable transcoding
    pub enable_transcoding: bool,

    /// Enable recording
    pub enable_recording: bool,

    /// Preferred codecs
    pub preferred_codecs: Vec<String>,

    /// DTMF mode
    pub dtmf_mode: DtmfMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtmfMode {
    /// RFC 2833 (RTP events)
    Rfc2833,

    /// In-band (audio detection)
    InBand,

    /// Both methods
    Both,
}

impl Default for MediaSessionConfig {
    fn default() -> Self {
        Self {
            enable_transcoding: true,
            enable_recording: false,
            preferred_codecs: vec![
                "PCMU".to_string(),
                "PCMA".to_string(),
                "opus".to_string(),
            ],
            dtmf_mode: DtmfMode::Rfc2833,
        }
    }
}
```

#### Task 1.4: Create Media Manager

**File: `crates/fcp-media/src/manager.rs`**

```rust
use crate::{Error, Result, MediaSession, MediaSessionConfig};
use fcp_core::{CallId, DialogId};
use forge_media::{ForgeEngine, ForgeConfig};
use dashmap::DashMap;
use std::sync::Arc;
use tracing::{info, debug, error};

/// Manages media sessions for FCP calls
pub struct MediaManager {
    /// forge-media engine
    engine: Arc<ForgeEngine>,

    /// Active media sessions (CallId -> MediaSession)
    sessions: DashMap<CallId, Arc<MediaSession>>,

    /// Configuration
    config: MediaConfig,
}

impl MediaManager {
    /// Create a new media manager
    pub async fn new(config: MediaConfig) -> Result<Self> {
        info!("Initializing media manager");

        // Create forge-media engine
        let forge_config = ForgeConfig {
            rtp_port_range: (config.rtp_port_start, config.rtp_port_end),
            enable_srtp: config.enable_srtp,
            ..Default::default()
        };

        let engine = Arc::new(ForgeEngine::new(forge_config).await?);

        Ok(Self {
            engine,
            sessions: DashMap::new(),
            config,
        })
    }

    /// Create a media session for a call
    pub async fn create_session(
        &self,
        call_id: CallId,
        dialog_id: DialogId,
        config: MediaSessionConfig,
    ) -> Result<Arc<MediaSession>> {
        debug!(?call_id, ?dialog_id, "Creating media session");

        // Check if session already exists
        if self.sessions.contains_key(&call_id) {
            return Err(Error::InvalidState(
                format!("Session already exists for call {}", call_id)
            ));
        }

        // Create forge-media session
        let forge_session = self.engine.create_session(forge_media::CreateSessionRequest {
            call_id: call_id.to_string(),
            ..Default::default()
        }).await?;

        // Create media session
        let session = Arc::new(MediaSession {
            call_id: call_id.clone(),
            dialog_id,
            forge_session_id: forge_session.session_id.clone(),
            forge_session: Arc::new(forge_session),
            state: tokio::sync::RwLock::new(MediaSessionState::Negotiating),
            config,
        });

        // Store session
        self.sessions.insert(call_id.clone(), session.clone());

        info!(?call_id, "Media session created");
        Ok(session)
    }

    /// Get an existing media session
    pub fn get_session(&self, call_id: &CallId) -> Result<Arc<MediaSession>> {
        self.sessions
            .get(call_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| Error::SessionNotFound(call_id.clone()))
    }

    /// Terminate a media session
    pub async fn terminate_session(&self, call_id: &CallId) -> Result<()> {
        debug!(?call_id, "Terminating media session");

        if let Some((_, session)) = self.sessions.remove(call_id) {
            // Update state
            *session.state.write().await = MediaSessionState::Terminated;

            // Clean up forge-media session
            self.engine.delete_session(&session.forge_session_id.to_string()).await?;

            info!(?call_id, "Media session terminated");
        }

        Ok(())
    }

    /// Get statistics for all sessions
    pub async fn get_stats(&self) -> MediaManagerStats {
        MediaManagerStats {
            total_sessions: self.sessions.len(),
            active_sessions: self.sessions.iter()
                .filter(|entry| {
                    matches!(
                        *entry.value().state.blocking_read(),
                        MediaSessionState::Active
                    )
                })
                .count(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaManagerStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
}
```

**File: `crates/fcp-media/src/config.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// RTP port range start
    #[serde(default = "default_rtp_port_start")]
    pub rtp_port_start: u16,

    /// RTP port range end
    #[serde(default = "default_rtp_port_end")]
    pub rtp_port_end: u16,

    /// Enable SRTP
    #[serde(default)]
    pub enable_srtp: bool,

    /// Enable transcoding
    #[serde(default = "default_true")]
    pub enable_transcoding: bool,

    /// Enable recording
    #[serde(default)]
    pub enable_recording: bool,
}

fn default_rtp_port_start() -> u16 { 10000 }
fn default_rtp_port_end() -> u16 { 20000 }
fn default_true() -> bool { true }

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            rtp_port_start: default_rtp_port_start(),
            rtp_port_end: default_rtp_port_end(),
            enable_srtp: false,
            enable_transcoding: true,
            enable_recording: false,
        }
    }
}
```

#### Task 1.5: Update Workspace

**Update `/home/siphon/fcp/Cargo.toml`:**

```toml
[workspace]
members = [
    "crates/fcp-core",
    "crates/fcp-runtime",
    "crates/fcp-sip",
    "crates/fcp-media",  # ADD THIS
    "crates/fcp-server",
    "crates/fcp-routing",
]
```

### Phase 2: SDP Integration (Week 1-2)

#### Task 2.1: SDP Adapter

**File: `crates/fcp-media/src/sdp.rs`**

```rust
use crate::{Error, Result};
use sip_sdp::{SessionDescription, MediaDescription};

/// Convert sip-sdp to forge-media SDP
pub fn convert_sdp_to_forge(
    sdp: &SessionDescription,
) -> Result<forge_media::Sdp> {
    // Extract media information
    let media = sdp.media_descriptions.iter()
        .filter(|m| m.media_type == "audio")
        .map(|m| convert_media_description(m))
        .collect::<Result<Vec<_>>>()?;

    Ok(forge_media::Sdp {
        origin: convert_origin(&sdp.origin),
        media,
        ..Default::default()
    })
}

/// Convert forge-media SDP to sip-sdp
pub fn convert_forge_to_sdp(
    forge_sdp: &forge_media::Sdp,
) -> Result<SessionDescription> {
    // Build SDP from forge representation
    todo!("Implement forge to SIP SDP conversion")
}

fn convert_media_description(
    media: &MediaDescription,
) -> Result<forge_media::MediaLine> {
    // Parse codec information
    let codecs = media.attributes.iter()
        .filter_map(|attr| {
            if attr.name == "rtpmap" {
                parse_rtpmap(attr.value.as_deref()?)
            } else {
                None
            }
        })
        .collect();

    Ok(forge_media::MediaLine {
        media_type: forge_media::MediaType::Audio,
        port: media.port,
        codecs,
        ..Default::default()
    })
}

fn parse_rtpmap(value: &str) -> Option<forge_media::CodecInfo> {
    // Parse "96 opus/48000/2" format
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }

    let payload_type: u8 = parts[0].parse().ok()?;
    let codec_parts: Vec<&str> = parts[1].split('/').collect();

    Some(forge_media::CodecInfo {
        payload_type,
        name: codec_parts[0].to_string(),
        clock_rate: codec_parts.get(1)?.parse().ok()?,
        channels: codec_parts.get(2).and_then(|c| c.parse().ok()).unwrap_or(1),
    })
}
```

#### Task 2.2: Codec Negotiation

**File: `crates/fcp-media/src/codec.rs`**

```rust
use crate::{Error, Result};

/// Negotiate codecs between two SDP offers
pub fn negotiate_codecs(
    offer: &[forge_media::CodecInfo],
    answer: &[forge_media::CodecInfo],
) -> Result<Vec<forge_media::CodecInfo>> {
    let mut negotiated = Vec::new();

    for offered in offer {
        if let Some(matched) = answer.iter()
            .find(|a| codec_match(offered, a))
        {
            negotiated.push(offered.clone());
        }
    }

    if negotiated.is_empty() {
        return Err(Error::CodecNegotiation(
            "No common codecs found".to_string()
        ));
    }

    Ok(negotiated)
}

fn codec_match(a: &forge_media::CodecInfo, b: &forge_media::CodecInfo) -> bool {
    a.name.eq_ignore_ascii_case(&b.name) && a.clock_rate == b.clock_rate
}
```

### Phase 3: B2BUA Implementation (Week 2)

#### Task 3.1: Call Leg Coordination

**File: `crates/fcp-media/src/b2bua.rs`**

```rust
use crate::{MediaManager, MediaSession, Result};
use fcp_core::{CallId, DialogId};
use std::sync::Arc;
use dashmap::DashMap;

/// B2BUA call with two legs (A-leg and B-leg)
pub struct B2buaCall {
    pub call_id: CallId,
    pub leg_a: CallLeg,
    pub leg_b: Option<CallLeg>,
    pub state: B2buaCallState,
}

pub struct CallLeg {
    pub dialog_id: DialogId,
    pub media_session: Arc<MediaSession>,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum B2buaCallState {
    Establishing,
    Active,
    Terminating,
    Terminated,
}

pub struct B2buaManager {
    media_manager: Arc<MediaManager>,
    calls: DashMap<CallId, Arc<B2buaCall>>,
}

impl B2buaManager {
    pub fn new(media_manager: Arc<MediaManager>) -> Self {
        Self {
            media_manager,
            calls: DashMap::new(),
        }
    }

    /// Handle incoming INVITE (create A-leg)
    pub async fn handle_incoming_invite(
        &self,
        call_id: CallId,
        dialog_id: DialogId,
        remote_sdp: String,
    ) -> Result<String> {
        // Create media session for A-leg
        let media_session = self.media_manager.create_session(
            call_id.clone(),
            dialog_id.clone(),
            Default::default(),
        ).await?;

        // Generate local SDP
        let local_sdp = self.generate_sdp(&media_session).await?;

        // Create call with A-leg
        let call = Arc::new(B2buaCall {
            call_id: call_id.clone(),
            leg_a: CallLeg {
                dialog_id,
                media_session,
                local_sdp: Some(local_sdp.clone()),
                remote_sdp: Some(remote_sdp),
            },
            leg_b: None,
            state: B2buaCallState::Establishing,
        });

        self.calls.insert(call_id, call);

        Ok(local_sdp)
    }

    /// Place outgoing call (create B-leg)
    pub async fn place_outgoing_call(
        &self,
        call_id: &CallId,
        target_dialog_id: DialogId,
    ) -> Result<String> {
        // Implementation for B-leg creation
        todo!("Implement B-leg creation")
    }

    /// Connect A-leg and B-leg media
    pub async fn connect_legs(&self, call_id: &CallId) -> Result<()> {
        let call = self.get_call(call_id)?;

        // Get both legs
        let leg_b = call.leg_b.as_ref()
            .ok_or_else(|| Error::InvalidState("B-leg not created".to_string()))?;

        // Connect media sessions in forge-media
        // This creates bidirectional audio path
        todo!("Implement media leg connection in forge-media")
    }

    fn get_call(&self, call_id: &CallId) -> Result<Arc<B2buaCall>> {
        self.calls.get(call_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| Error::SessionNotFound(call_id.clone()))
    }

    async fn generate_sdp(&self, session: &MediaSession) -> Result<String> {
        // Generate SDP for this session
        todo!("Implement SDP generation")
    }
}
```

### Phase 4: Integration with fcp-server (Week 2)

#### Task 4.1: Update fcp-server

**File: `crates/fcp-server/src/main.rs`**

```rust
use fcp_media::{MediaManager, MediaConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ... existing initialization ...

    // Initialize media manager
    let media_config = MediaConfig {
        rtp_port_start: 10000,
        rtp_port_end: 20000,
        enable_transcoding: true,
        ..Default::default()
    };

    let media_manager = MediaManager::new(media_config).await?;

    // Add to runtime
    runtime.add_service("media", Arc::new(media_manager)).await?;

    // ... rest of initialization ...
}
```

---

## 4. Testing Strategy

### Unit Tests

**Test: Codec Negotiation**
```rust
#[tokio::test]
async fn test_codec_negotiation() {
    let offer = vec![
        CodecInfo { name: "PCMU".into(), clock_rate: 8000, .. },
        CodecInfo { name: "opus".into(), clock_rate: 48000, .. },
    ];

    let answer = vec![
        CodecInfo { name: "PCMA".into(), clock_rate: 8000, .. },
        CodecInfo { name: "opus".into(), clock_rate: 48000, .. },
    ];

    let result = negotiate_codecs(&offer, &answer).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "opus");
}
```

### Integration Tests

**Test: Basic Call Flow**
```rust
#[tokio::test]
async fn test_basic_call() {
    // Start FCP server
    let server = start_test_server().await;

    // Register two endpoints
    let alice = register_endpoint(&server, "alice").await;
    let bob = register_endpoint(&server, "bob").await;

    // Alice calls Bob
    let call = alice.invite("sip:bob@test.local").await.unwrap();

    // Bob receives INVITE
    let incoming = bob.wait_for_invite().await.unwrap();

    // Bob answers
    bob.answer(&incoming).await.unwrap();

    // Verify media session created
    let media_session = server.media_manager()
        .get_session(&call.call_id)
        .unwrap();

    assert_eq!(media_session.state.read().await, &MediaSessionState::Active);

    // Hangup
    alice.bye(&call).await.unwrap();
}
```

### Manual Testing with Softphones

**Test Setup:**
```bash
# Terminal 1: Start FCP server
cargo run --bin fcp-server -- --config config/fcp.toml

# Terminal 2: Monitor logs
tail -f /var/log/fcp/fcp-server.log

# Terminal 3: Configure two softphones (Linphone, Zoiper, etc.)
# Phone 1: sip:1001@localhost:5060
# Phone 2: sip:1002@localhost:5060

# Make a call from 1001 to 1002
# Verify:
# - Registration succeeds
# - Call setup (INVITE, 180, 200)
# - Media flows (audio in both directions)
# - Hangup works (BYE)
```

---

## 5. Success Criteria

### Sprint 6 Deliverables

✅ **Milestone: "First Media Call"**

The Sprint 6 deliverable is met when:

1. **Registration**: Two softphones can register to FCP server
2. **Call Setup**: Alice can call Bob (INVITE → 180 Ringing → 200 OK → ACK)
3. **Media Flow**:
   - RTP packets flow in both directions
   - Audio is audible on both sides
   - Codec negotiation works (G.711 or Opus)
4. **Call Teardown**: Either party can hang up (BYE → 200 OK)
5. **Logging**: All SIP and media events are logged with tracing
6. **Metrics**: Basic call metrics available (setup time, duration)

### Acceptance Tests

```bash
# Test 1: G.711 µ-law call
./tests/manual/test_pcmu_call.sh

# Test 2: Opus wideband call
./tests/manual/test_opus_call.sh

# Test 3: Codec transcoding (PCMU → Opus)
./tests/manual/test_transcoding.sh

# Test 4: Call hold and resume
./tests/manual/test_hold.sh

# Test 5: Call transfer (if time permits)
./tests/manual/test_transfer.sh
```

---

## 6. Next Steps (This Week)

### Immediate Actions

1. **Create fcp-media crate** (2 hours)
   ```bash
   cd /home/siphon/fcp
   cargo new --lib crates/fcp-media
   # Update Cargo.toml as shown above
   ```

2. **Implement foundation types** (4 hours)
   - Error types
   - MediaSession
   - MediaManager skeleton

3. **Add forge-media dependency** (1 hour)
   - Verify forge-media builds
   - Test basic engine initialization

4. **SDP integration** (6 hours)
   - SDP conversion functions
   - Codec negotiation logic

5. **B2BUA skeleton** (4 hours)
   - Call leg management
   - Basic call flow handling

### Week 1 Goal

By end of Week 1, we should have:
- ✅ fcp-media crate compiling
- ✅ MediaManager can create/destroy sessions
- ✅ SDP conversion working
- ✅ Unit tests passing

### Week 2 Goal

By end of Week 2 (Sprint 6 complete), we should have:
- ✅ Two softphones can complete a call through FCP
- ✅ Audio flows in both directions
- ✅ Basic metrics exposed
- ✅ Ready for Sprint 7 (Storage & API)

---

## 7. Risk Mitigation

### Known Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| SDP format incompatibility | Medium | High | Use sip-sdp parser, write comprehensive tests |
| Codec negotiation edge cases | Medium | Medium | Start with G.711 only, add others incrementally |
| NAT traversal issues | High | High | Start with local testing, add ICE/STUN later |
| Media timing issues | Low | Medium | Rely on forge-media's proven implementation |

### Debugging Strategy

**Tools:**
- Wireshark: Capture SIP and RTP packets
- sipp: SIP load testing and protocol validation
- rtpdump: RTP packet analysis
- tcpdump: Low-level network debugging

**Log Levels:**
```bash
# Maximum verbosity for debugging
RUST_LOG=fcp_media=trace,forge_media=debug,siphon=debug cargo run
```

---

## 8. Future Enhancements (Post-Sprint 6)

Once Sprint 6 is complete, future work includes:

**Sprint 7-8:**
- Recording integration
- Conference bridge support
- WebRTC gateway

**Phase 2:**
- AI integration (IVR, voice bots)
- Advanced transcoding
- Video support

**Phase 3+:**
- High availability
- Clustering
- Carrier-grade features

---

## Appendix A: forge-media API Reference

### Key forge-media APIs

```rust
// Create engine
let engine = ForgeEngine::new(config).await?;

// Create session
let session = engine.create_session(CreateSessionRequest {
    call_id: "call-123".into(),
    sdp: Some(remote_sdp),
}).await?;

// Get session stats
let stats = engine.get_session_stats(&session_id).await?;

// Delete session
engine.delete_session(&session_id).await?;
```

### forge-media Configuration

```toml
[media]
rtp_port_range = [10000, 20000]
enable_srtp = true
enable_transcoding = true

[codecs]
enabled = ["PCMU", "PCMA", "opus", "G722"]
```

---

## Appendix B: Resources

### Documentation
- [forge-media README](../external/forge-media/README.md)
- [forge-media CLAUDE.MD](../external/forge-media/CLAUDE.MD)
- [siphon-rs CLAUDE.md](../external/siphon-rs/CLAUDE.md)
- [Phase 1 Plan](Phase1%20plan.md)

### Reference Implementations
- forge-media examples: `external/forge-media/examples/`
- siphond daemon: `external/siphon-rs/bins/siphond/`

### Testing Tools
- Linphone (softphone): https://www.linphone.org/
- Zoiper (softphone): https://www.zoiper.com/
- sipp (SIP testing): https://github.com/SIPp/sipp
- Wireshark: https://www.wireshark.org/

---

**Status**: Ready to begin implementation
**Next Milestone**: First media call working
**Target Date**: 2 weeks from start
**Owner**: Development team
