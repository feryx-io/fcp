# FCP Media Integration - Next Steps

## Current Status

✅ **Completed:**
- Created `fcp-media` crate with proper structure
- Designed abstraction layer for embedded/remote backends
- Implemented MediaBackend trait
- Created configuration types
- Added workspace integration

⚠️ **In Progress:**
- Fixing compilation errors with forge-media API integration

## Issue Discovered

The forge-media API is different from what I initially assumed. Key differences:

**forge-media actual API:**
- Uses `SessionManager` (not `ForgeEngine`)
- Has `CallId`, `ParticipantId` from `forge_core`
- Sessions require participants, not just call_id
- Uses `DashMap` for session storage internally
- Has optional HA, XDP features

**Our assumption:**
- Simple `ForgeEngine::new()` → `create_session()` → `delete_session()`
- Direct string-based identifiers
- Simpler session lifecycle

## Pragmatic Solution for Phase 1

**Option 1: Simplified Wrapper (Recommended for Sprint 6)**

For Phase 1, let's create a minimal wrapper that:
1. Uses forge-media's `SessionManager` directly
2. Adapts FCP types (CallId, DialogId) to forge types
3. Provides simple create/terminate/update operations
4. Defers full abstraction layer to Phase 2

```rust
// Simplified fcp-media for Phase 1
pub struct MediaManager {
    session_manager: Arc<forge_engine::SessionManager>,
    // Simple mapping
    sessions: DashMap<fcp_core::CallId, forge_core::CallId>,
}
```

**Option 2: Full API Exploration (Better but slower)**

Spend time understanding all of forge-media's API and create proper adapters for:
- SessionManager
- MediaSession
- Participants
- Port allocation
- Events

This is the "right" approach but will take longer.

## Recommendation

**For Sprint 6 ("First Call" Milestone):**

Use **Option 1** - Get something working quickly:
1. Create minimal fcp-media wrapper around `forge_engine::SessionManager`
2. Handle basic session lifecycle (create, terminate)
3. Defer SDP negotiation, codecs, etc. to manual testing
4. Focus on completing the call flow end-to-end

**For Phase 2 (After Sprint 6):**

Enhance to **Option 2**:
1. Add proper SDP handling
2. Implement codec negotiation
3. Add recording, DTMF, hold/resume
4. Create RemoteBackend for standalone mode
5. Full test coverage

## Immediate Action Items

1. **Study forge-media examples** (30 min)
   - Look at `external/forge-media/examples/`
   - Look at `external/forge-media/bins/forge-test-daemon/`
   - Understand actual usage patterns

2. **Simplify fcp-media** (2 hours)
   - Remove complex abstraction (save for Phase 2)
   - Create minimal wrapper around SessionManager
   - Focus on: create_session, terminate_session, basic SDP

3. **Get it compiling** (1 hour)
   - Fix all type errors
   - Use forge types directly where needed
   - Add proper forge-core dependencies

4. **Integration test** (2 hours)
   - Create simple test that starts SessionManager
   - Allocate ports
   - Verify compilation and basic functionality

## Updated Timeline

**Week 1 Revised:**
- ✅ Day 1-2: Architecture and planning (DONE)
- 🔄 Day 3: Study forge-media API, simplify wrapper (TODAY)
- ⏳ Day 4: Get fcp-media compiling
- ⏳ Day 5: Basic integration test

**Week 2:**
- SIP + Media integration in fcp-server
- Manual testing with softphones
- Sprint 6 completion

## Files to Study

```bash
# forge-media examples
ls external/forge-media/examples/

# forge-test-daemon (shows real usage)
cat external/forge-media/bins/forge-test-daemon/src/main.rs

# SessionManager API
cat external/forge-media/crates/forge-engine/src/manager.rs

# forge-core types
cat external/forge-media/crates/forge-core/src/types.rs
```

## Decision Point

**Which approach should we take?**

1. **Fast**: Minimal wrapper, get Sprint 6 working, refactor later
2. **Proper**: Full abstraction now, takes longer but cleaner

Your call - I can proceed with either approach!
