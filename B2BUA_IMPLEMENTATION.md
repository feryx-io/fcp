# B2BUA Implementation Progress

## Overview

Implementing Back-to-Back User Agent (B2BUA) functionality for the FCP platform.
This enables the server to act as an intermediary for SIP calls, providing call
control, routing, and policy enforcement.

## What is a B2BUA?

A B2BUA sits between two SIP endpoints and terminates both call legs:
- **A-leg**: Call from the originator (caller)
- **B-leg**: Call to the destination (callee)

The B2BUA:
1. Receives INVITE from caller
2. Checks routing policies
3. Sends new INVITE to callee
4. Relays responses between parties
5. Maintains call state for both legs
6. Handles call termination (BYE)

## Implementation Status

### ✅ Sprint 6.1: Core Call Models (COMPLETE)

**File**: `crates/fcp-sip/src/call.rs` (305 lines)

**Implemented:**
- `CallId` - Unique UUID-based call identifier
- `CallState` - Enum for call lifecycle states:
  - Initiated, RoutingPending, LookingUpDestination
  - Inviting, Proceeding, Established
  - Terminating, Terminated, Failed
- `CallDirection` - Inbound/Outbound
- `CallLeg` - Represents one side of the call:
  - Dialog reference
  - Remote party URI & display name
  - Contact address
  - SDP body
  - Creation timestamp
- `Call` - Main call structure:
  - Both call legs (A-leg, B-leg)
  - Current state
  - Original INVITE
  - Timestamps (created, established, ended)
  - Duration calculation
  - Failure reason tracking

**Helper Functions:**
- `parse_name_addr()` - Parses `"Display Name" <sip:uri>`
- `CallLeg::from_invite()` - Creates leg from INVITE
- `Call::from_invite()` - Creates call from inbound INVITE

**Tests:**
- Call ID uniqueness
- State terminal checks
- Name-addr parsing (3 test cases)

### ✅ Sprint 6.2: Call Manager Service (COMPLETE)

**File**: `crates/fcp-sip/src/call_manager.rs` (285 lines)

**Implemented:**
- `CallManager` - Thread-safe call storage using DashMap
- **CRUD Operations:**
  - `create_call()` - Store new call
  - `get_call()` - Retrieve by ID
  - `update_call()` - Modify with closure
  - `terminate_call()` - End call gracefully
- **Query Methods:**
  - `active_calls()` - Get non-terminal calls
  - `all_calls()` - Get all calls
  - `find_by_dialog_call_id()` - Lookup by SIP Call-ID header
  - `active_call_count()` / `total_call_count()` - Statistics
- **Maintenance:**
  - `cleanup_old_calls()` - Remove old terminated calls
  - `stats()` - Get CallStats structure
- `CallStats` - Aggregate statistics:
  - Total, active, established, terminated, failed counts

**Features:**
- Concurrent access with DashMap
- Structured logging (tracing)
- Clean separation of concerns
- Memory leak prevention (cleanup)

**Tests:**
- Create & retrieve calls
- Update call state
- Terminate & removal
- Active vs terminated filtering
- Statistics accuracy

### ⏳ Sprint 6.3: INVITE Handling (IN PROGRESS)

**Goal**: Handle inbound INVITE requests and create calls

**Tasks:**
1. ✅ Add CallManager to SipMessageHandler
2. Add INVITE handler method:
   - Extract caller/callee from headers
   - Lookup caller's tenant (for routing)
   - Evaluate routing policy
   - Create Call object
   - Store in CallManager
   - Send 100 Trying
   - Lookup destination endpoint
   - Create outbound INVITE
   - Forward to destination

**Routing Integration:**
- Call `routing_service.evaluate_call()` before forwarding
- Deny if policy disallows
- Send 403 Forbidden to caller if denied

**Response Handling:**
- 180 Ringing → forward to caller
- 200 OK → forward to caller, establish call
- 4xx/5xx → forward to caller, fail call

### 🔲 Sprint 6.4: ACK Handling (PENDING)

**Goal**: Handle ACK to complete call establishment

**Tasks:**
- Receive ACK from caller (for 200 OK)
- Find call by dialog Call-ID
- Mark call as Established
- Send ACK to callee (if not already sent)
- Update call timestamps

### 🔲 Sprint 6.5: BYE Handling (PENDING)

**Goal**: Handle call termination

**Tasks:**
- Receive BYE from either leg
- Find call by dialog Call-ID
- Send BYE to other leg
- Send 200 OK responses
- Terminate call in CallManager
- Log call duration & stats

### 🔲 Sprint 6.6: SDP Integration (PENDING)

**Goal**: Parse and relay SDP for media negotiation

**Tasks:**
- Extract SDP from INVITE body
- Store in CallLeg
- Relay SDP in forwarded INVITE
- Relay SDP answer in 200 OK
- No transcoding (pass-through for now)

### 🔲 Sprint 6.7: Error Handling (PENDING)

**Tasks:**
- Timeout handling (no response from callee)
- CANCEL support (caller cancels before answer)
- Re-INVITE support (call modifications)
- Proper cleanup on errors

### 🔲 Sprint 6.8: Testing (PENDING)

**Test Scenarios:**
1. Basic call setup (INVITE → 200 OK → ACK)
2. Call with routing policy (allow/deny)
3. Call termination (BYE from caller/callee)
4. Failed call (callee rejects)
5. Canceled call (caller sends CANCEL)
6. Multi-tenant calls (cross-tenant denied)

## Architecture

```
┌─────────────┐         ┌──────────────┐         ┌─────────────┐
│   Caller    │         │  FCP B2BUA   │         │   Callee    │
│  (A-leg)    │         │              │         │  (B-leg)    │
└──────┬──────┘         └───────┬──────┘         └──────┬──────┘
       │                        │                       │
       │  INVITE                │                       │
       │───────────────────────>│                       │
       │                        │ Check routing policy  │
       │                        │ Look up destination   │
       │                        │                       │
       │  100 Trying            │  INVITE               │
       │<───────────────────────│──────────────────────>│
       │                        │                       │
       │                        │  180 Ringing          │
       │  180 Ringing           │<──────────────────────│
       │<───────────────────────│                       │
       │                        │                       │
       │                        │  200 OK (SDP)         │
       │  200 OK (SDP)          │<──────────────────────│
       │<───────────────────────│                       │
       │                        │                       │
       │  ACK                   │                       │
       │───────────────────────>│  ACK                  │
       │                        │──────────────────────>│
       │                        │                       │
       │    *** Call Established ***                    │
       │                        │                       │
       │  BYE                   │                       │
       │───────────────────────>│  BYE                  │
       │                        │──────────────────────>│
       │                        │                       │
       │                        │  200 OK               │
       │  200 OK                │<──────────────────────│
       │<───────────────────────│                       │
       │                        │                       │
```

## Key Design Decisions

### 1. Call Storage
- Using `DashMap` for thread-safe concurrent access
- Call indexed by internal `CallId` (UUID)
- Additional lookup by SIP Call-ID header for in-dialog requests

### 2. Call State Management
- State machine with clear transitions
- Terminal states: Terminated, Failed
- Active states checked for call counts

### 3. Dialog Management
- Each leg has its own Dialog (from sip-dialog crate)
- Dialog tracks CSeq, tags, routes
- B2BUA maintains separate dialogs for each leg

### 4. Error Handling
- Graceful failure with reason tracking
- Cleanup of failed calls
- Proper response codes to both parties

### 5. Routing Integration
- Policy check before call setup
- Tenant-aware routing
- Fail-closed security (deny if policy check fails)

## Files Modified/Created

### Created:
- `crates/fcp-sip/src/call.rs` - Call models (305 lines)
- `crates/fcp-sip/src/call_manager.rs` - Manager service (285 lines)

### Modified:
- `crates/fcp-sip/src/lib.rs` - Export call modules
- `crates/fcp-sip/Cargo.toml` - Add dashmap dependency

### Next to Modify:
- `crates/fcp-sip/src/handler.rs` - Add INVITE/ACK/BYE handling
- `crates/fcp-sip/src/service.rs` - Register CallManager as service
- `crates/fcp-runtime/src/builder.rs` - Create CallManager instance

## Testing Strategy

### Unit Tests (✅ Done)
- Call models (call.rs)
- CallManager operations (call_manager.rs)

### Integration Tests (Pending)
- End-to-end call flow
- Routing policy integration
- Multi-tenant isolation
- Error scenarios

### Manual Testing (Pending)
- Two SIP clients (Linphone, Zoiper)
- Place call from Alice to Bob
- Verify call establishment
- Verify call termination
- Check logs for proper flow

## Next Session

**Priority**: Continue with Sprint 6.3 - INVITE handling
1. Add CallManager to handler.rs
2. Implement handle_invite() method
3. Integrate routing policy check
4. Forward INVITE to destination
5. Test basic call flow

**Estimated Time**: 3-4 hours for full INVITE/ACK/BYE implementation

---

*Last Updated: 2025-12-01*
*Status: 30% complete (2 of 8 sprints done)*
