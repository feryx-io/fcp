# Media Wiring Progress - Session Checkpoint

**Date**: 2025-12-24
**Status**: Infrastructure Complete, Ready for INVITE Logic

---

## ✅ Completed This Session

### 1. **Call Structure Enhanced**
**Files Modified**: `/home/siphon/fcp/crates/fcp-sip/src/call.rs`

Added media session tracking to Call struct:
```rust
pub struct Call {
    // ... existing fields
    /// Media session ID (if media is anchored on server)
    pub media_session_id: Option<fcp_core::CallId>,
    /// Whether media is anchored for this call
    pub media_anchored: bool,
    // ...
}
```

### 2. **SipMessageHandler Enhanced**
**Files Modified**: `/home/siphon/fcp/crates/fcp-sip/src/handler.rs`

Added media integration to handler:
```rust
pub struct SipMessageHandler {
    // ... existing fields
    /// Media manager for RTP handling
    media_manager: Option<Arc<fcp_media::MediaManager>>,
    /// Media configuration
    media_config: Option<fcp_core::config::MediaConfig>,
    // ...
}
```

Updated constructors to accept media_manager and media_config parameters.

### 3. **RuntimeBuilder Updated**
**Files Modified**: `/home/siphon/fcp/crates/fcp-runtime/src/builder.rs`

- Updated `create_transport_listener()` to accept `media_manager`
- Pass media_manager and config.media to SipMessageHandler::new()
- Handler now has access to media infrastructure

### 4. **Dependency Resolution**
**Files Modified**: `/home/siphon/fcp/crates/fcp-media/Cargo.toml`, `/home/siphon/fcp/crates/fcp-sip/Cargo.toml`

- Fixed cyclic dependency (fcp-media ↔ fcp-sip)
- Removed unnecessary fcp-sip dependency from fcp-media
- Added fcp-media dependency to fcp-sip
- **All compilation clean** ✅

---

## 🎯 Current Architecture

### Data Flow
```
Runtime
  └─> MediaManager (initialized)
        └─> Passed to SipMessageHandler
              └─> Available in handle_invite()
                    └─> Can call media decision logic
                          └─> Can create media sessions
```

### Handler Has Access To
- ✅ `self.media_manager: Option<Arc<MediaManager>>`
- ✅ `self.media_config: Option<MediaConfig>`
- ✅ `fcp_media::extract_codecs_from_sdp()`
- ✅ `fcp_media::should_anchor_media()`
- ✅ `fcp_media::MediaDecisionContext`

### Call Object Tracks
- ✅ `call.media_session_id: Option<CallId>`
- ✅ `call.media_anchored: bool`

---

## 📋 Next Immediate Step

### Wire handle_invite() to Use Media Decision Logic

**Location**: `/home/siphon/fcp/crates/fcp-sip/src/handler.rs:611` (`handle_invite` function)

**Pseudocode**:
```rust
async fn handle_invite(&self, request: &Request, handle: &ServerTransactionHandle, peer: SocketAddr) -> Result<()> {
    // ... existing routing checks ...

    // NEW: Check if media manager is available
    if let (Some(media_manager), Some(media_config)) = (&self.media_manager, &self.media_config) {
        // Extract codecs from A-leg SDP
        let a_leg_codecs = if !request.body.is_empty() {
            let sdp = String::from_utf8_lossy(&request.body);
            fcp_media::extract_codecs_from_sdp(&sdp)
        } else {
            Vec::new()
        };

        // Create decision context
        let ctx = fcp_media::MediaDecisionContext::new(media_config.enabled)
            .with_a_leg_codecs(a_leg_codecs)
            .with_transcoding_enabled(media_config.enable_transcoding)
            .with_recording_enabled(media_config.enable_recording)
            .with_dtmf_enabled(media_config.enable_dtmf);

        // Make decision
        match fcp_media::should_anchor_media(&ctx) {
            fcp_media::MediaAnchoringDecision::PassThrough { reason } => {
                debug!(call_id = %call_id, reason = %reason, "Using pass-through mode");
                // Continue with existing behavior (no changes)
            }
            fcp_media::MediaAnchoringDecision::Anchored { reason, requires_transcoding } => {
                info!(call_id = %call_id, reason = %reason, transcoding = requires_transcoding, "Anchoring media");

                // Create media session
                let media_session = media_manager.create_session_default(
                    fcp_call_id,  // Need to convert from sip CallId
                    dialog_id,
                ).await?;

                // Update call object
                self.call_manager.update_call(&call_id, |call| {
                    call.media_session_id = Some(fcp_call_id);
                    call.media_anchored = true;
                });

                // TODO: Rewrite SDP to point to server's RTP ports
                // (Next task after this)
            }
        }
    }

    // ... continue with existing INVITE forwarding logic ...
}
```

**Complexity**: Medium (2-3 hours)
- Need to convert between sip::CallId and fcp_core::CallId
- Need to extract codecs from SDP
- Need to create MediaDecisionContext
- Need to handle anchored vs pass-through paths
- Need to update Call object with media session info

---

## 🔨 Remaining Tasks

### Task 1: Complete handle_invite() Wiring (CURRENT)
**Estimate**: 2-3 hours
**Status**: Ready to implement
**Blocker**: None

### Task 2: Implement SDP Rewriting
**Estimate**: 1-2 hours
**Status**: Pending Task 1
**Function needed**:
```rust
// In fcp-media/src/sdp.rs
pub fn rewrite_sdp_for_anchoring(
    original_sdp: &str,
    server_ip: &str,
    rtp_port: u16,
) -> String {
    // Replace c=IN IP4 <old-ip> with server IP
    // Replace m=audio <old-port> with RTP port
    // Return modified SDP
}
```

### Task 3: Wire BYE Handling
**Estimate**: 30 minutes
**Status**: Pending Task 1
**Pseudocode**:
```rust
async fn handle_bye(&self, ...) -> Result<()> {
    // ... existing BYE logic ...

    // NEW: Terminate media session if anchored
    if call.media_anchored {
        if let (Some(media_session_id), Some(media_manager)) = (call.media_session_id, &self.media_manager) {
            media_manager.terminate_session(&media_session_id).await?;
            info!(call_id = %call.id, "Terminated media session");
        }
    }

    // ... continue with existing BYE handling ...
}
```

### Task 4: Testing with Softphones
**Estimate**: 2-4 hours
**Status**: Pending Tasks 1-3

**Test Plan**:
1. Pass-through mode (media.enabled = false): Verify existing behavior
2. Anchored mode, same codec: Verify RTP flows through server
3. Anchored mode, transcoding: Verify transcoding works
4. Recording: Verify calls are recorded

---

## 📊 Compilation Status

✅ **fcp-core**: Clean
✅ **fcp-media**: Clean (26 tests passing)
✅ **fcp-sip**: Clean
✅ **fcp-runtime**: Clean
✅ **fcp-server**: Clean

---

## 🔍 Key Design Decisions Made

### 1. Media Manager is Optional
The handler accepts `Option<Arc<MediaManager>>` so it can still work without media support (backward compatible).

### 2. Per-Call Decision
Media anchoring is determined per-call based on actual requirements, not globally forced.

### 3. No Breaking Changes
Existing pass-through behavior is preserved when:
- media.enabled = false in config
- No media_manager provided to handler
- Decision logic returns PassThrough

### 4. Clean Dependency Graph
```
fcp-server
  └─> fcp-runtime
        ├─> fcp-sip
        │     └─> fcp-media
        │           └─> fcp-core
        └─> fcp-media
              └─> fcp-core
```

No cycles, clean compilation.

---

## 💡 Implementation Notes

### ID Conversion Challenge
There are two CallId types:
- `sip_core::CallId` (SIP Call-ID header value: string)
- `fcp_core::CallId` (Internal FCP identifier: UUID with "call-" prefix)

**Solution needed**: Map between them in handle_invite()
```rust
// Create FCP CallId from SIP Call object
let fcp_call_id = fcp_core::CallId::new();  // Generate new
// OR derive from SIP Call-ID somehow
```

### SDP Access
Request body contains SDP as bytes:
```rust
let sdp_string = String::from_utf8_lossy(&request.body);
let codecs = fcp_media::extract_codecs_from_sdp(&sdp_string);
```

### Dialog ID
Need to create or extract DialogId for media session:
```rust
let dialog_id = fcp_core::DialogId::new();  // Or extract from call
```

---

## 🎯 Success Criteria

### Task 1 Complete When:
- [ ] handle_invite() calls media decision logic
- [ ] Media sessions created for anchored calls
- [ ] Call object tracks media_session_id
- [ ] Logs show decision reasoning
- [ ] Code compiles cleanly
- [ ] Pass-through mode still works (no media session created)

### Integration Complete When:
- [ ] Task 1 complete
- [ ] SDP rewriting implemented (Task 2)
- [ ] BYE handling wired (Task 3)
- [ ] Manual testing passed (Task 4)
- [ ] Both modes work: pass-through and anchored

---

## 📝 Next Session Start

**Resume at**: Implementing media decision logic in handle_invite()
**File**: `/home/siphon/fcp/crates/fcp-sip/src/handler.rs`
**Function**: `handle_invite` (line ~611)
**Context**: All infrastructure is ready, just need to wire the logic

**Quick Start**:
```bash
# Open the file
code crates/fcp-sip/src/handler.rs

# Search for: async fn handle_invite
# Add media decision logic after routing checks
# Before forwarding INVITE to B-leg
```

---

**Status**: ✅ Infrastructure 100% Complete
**Blocker**: None
**Estimated Time to First Anchored Call**: 3-4 hours (Tasks 1-2)
**Risk Level**: Low (clean compile, good test coverage)
