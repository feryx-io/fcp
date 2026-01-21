# Dial Node Implementation Plan - B2BUA with CDR Tracking

## Overview
Implement the Dial node for creating outbound calls (B-leg) and bridging them with incoming calls (A-leg) in a Back-to-Back User Agent (B2BUA) architecture. Both call legs will be tracked in CDR.

## Architecture

### B2BUA Call Flow
```
┌─────────┐          ┌─────────────┐          ┌─────────┐
│ Caller  │          │     FCP     │          │ Callee  │
│ (A-leg) │          │   (B2BUA)   │          │ (B-leg) │
└────┬────┘          └──────┬──────┘          └────┬────┘
     │                      │                      │
     │  INVITE (A-leg)      │                      │
     │─────────────────────>│                      │
     │                      │  INVITE (B-leg)      │
     │                      │─────────────────────>│
     │                      │                      │
     │                      │      180 Ringing     │
     │                      │<─────────────────────│
     │  180 Ringing         │                      │
     │<─────────────────────│                      │
     │                      │                      │
     │                      │      200 OK          │
     │                      │<─────────────────────│
     │  200 OK              │                      │
     │<─────────────────────│                      │
     │                      │                      │
     │  ACK                 │      ACK             │
     │─────────────────────>│─────────────────────>│
     │                      │                      │
     │    RTP (A-leg)       │    RTP (B-leg)       │
     │<════════════════════>│<════════════════════>│
     │       (Bridged RTP - forge-media)           │
     │                      │                      │
     │  BYE                 │      BYE             │
     │─────────────────────>│─────────────────────>│
     │                      │      200 OK          │
     │                      │<─────────────────────│
     │  200 OK              │                      │
     │<─────────────────────│                      │
     │                      │                      │
   (CDR A)               (CDR B)
```

### Component Responsibilities

**FlowEngine (fcp-flow)**:
- Stores dial configuration in context variables
- Returns WaitingForDial execution result
- Passes control to FlowHandler

**FlowHandler (fcp-sip)**:
- Creates outbound SIP call (B-leg)
- Bridges media via MediaManager
- Handles call progress (ringing, answer, failure)
- Transitions to appropriate nodes based on outcome
- Coordinates CDR creation for both legs

**MediaManager (fcp-media)**:
- Creates bridged media session linking A and B participants
- Manages RTP forwarding between legs
- Handles media negotiation for B-leg

**CallManager (fcp-sip)**:
- Tracks both A-leg and B-leg as separate calls
- Creates CDR for each leg when terminated
- Marks B-leg CDR with special metadata (originated_by_dial: true)

## Implementation Steps

### Phase 1: Flow Engine Updates

**File**: `crates/fcp-flow/src/engine.rs`

Add to `FlowExecutionResult` enum:
```rust
/// Waiting for outbound call to complete (Dial node)
WaitingForDial {
    destination: String,
    timeout_secs: u32,
    caller_id: Option<String>,
    max_duration_secs: u32,
    on_busy: Option<String>,
    on_no_answer: Option<String>,
    on_error: Option<String>,
    next: String,
},
```

Update Dial node execution:
```rust
NodeType::Dial {
    destination,
    timeout,
    caller_id,
    max_duration,
    next,
    on_busy,
    on_no_answer,
    on_error,
} => {
    info!(
        call_id = %context.call_id,
        destination = %destination,
        timeout = %timeout,
        "Dialing destination"
    );

    // Interpolate variables in destination
    let interpolated_dest = self.interpolate_variables(destination, context)?;

    // Store dial configuration
    context.set_variable("__dial_destination".to_string(), interpolated_dest.clone());
    context.set_variable("__dial_timeout".to_string(), timeout.to_string());
    if let Some(cid) = caller_id {
        context.set_variable("__dial_caller_id".to_string(), cid.clone());
    }
    context.set_variable("__dial_max_duration".to_string(), max_duration.to_string());

    Ok(FlowExecutionResult::WaitingForDial {
        destination: interpolated_dest,
        timeout_secs: *timeout,
        caller_id: caller_id.clone(),
        max_duration_secs: *max_duration,
        on_busy: on_busy.clone(),
        on_no_answer: on_no_answer.clone(),
        on_error: on_error.clone(),
        next: next.clone(),
    })
}
```

### Phase 2: Outbound Call Creation

**File**: `crates/fcp-sip/src/flow_handler.rs`

Add method:
```rust
async fn handle_dial(
    &self,
    call_id: CallId,  // A-leg call ID
    destination: String,
    timeout_secs: u32,
    caller_id: Option<String>,
    max_duration_secs: u32,
    on_busy: Option<String>,
    on_no_answer: Option<String>,
    on_error: Option<String>,
    next: String,
) -> Result<()>
```

Steps:
1. Get A-leg dialog and call information
2. Create B-leg INVITE using UAC
3. Set From header based on caller_id or A-leg info
4. Set To header from destination
5. Generate new Call-ID for B-leg
6. Create SDP offer for B-leg using MediaManager
7. Send INVITE and handle responses:
   - 180/183: Log ringing
   - 200 OK: Bridge calls (Phase 3)
   - 486/600: Go to on_busy node
   - 408/480/487: Go to on_no_answer node
   - Other 4xx/5xx/6xx: Go to on_error node
8. Handle timeout (408) after timeout_secs

### Phase 3: Media Bridging

**File**: `crates/fcp-media/src/manager.rs`

Add method:
```rust
pub async fn bridge_calls(
    &self,
    a_leg_call_id: &CallId,
    b_leg_call_id: &CallId,
) -> Result<()>
```

Implementation:
1. Get both media sessions
2. Configure forge-media to forward RTP bidirectionally
3. Map ParticipantA of A-leg → ParticipantB of B-leg
4. Map ParticipantA of B-leg → ParticipantB of A-leg
5. Start RTP forwarding

**File**: `crates/fcp-media/src/backend.rs`

Add to trait:
```rust
/// Bridge two calls together for RTP forwarding
async fn bridge_calls(&self, call_a: &CallId, call_b: &CallId) -> Result<()>;

/// Unbridge two calls
async fn unbridge_calls(&self, call_a: &CallId, call_b: &CallId) -> Result<()>;
```

### Phase 4: CDR Tracking for Both Legs

**File**: `crates/fcp-sip/src/call.rs`

Add field:
```rust
pub struct Call {
    // ... existing fields

    /// Parent call ID if this is a B-leg originated by Dial node
    pub originated_by: Option<CallId>,

    /// B-leg call ID if this A-leg originated an outbound call
    pub originated_call: Option<CallId>,
}
```

**File**: `crates/fcp-sip/src/call_manager.rs`

Update CDR creation:
```rust
async fn create_and_insert_cdr(
    storage: &DynStorage,
    call: &Call,
) -> Result<()> {
    // ... existing CDR creation logic

    let mut cdr = CallDetailRecord {
        // ... standard fields
    };

    // Add metadata for Dial node originated calls
    if let Some(parent_call_id) = &call.originated_by {
        cdr.metadata.insert(
            "originated_by_dial".to_string(),
            "true".to_string()
        );
        cdr.metadata.insert(
            "parent_call_id".to_string(),
            parent_call_id.to_string()
        );
    }

    if let Some(child_call_id) = &call.originated_call {
        cdr.metadata.insert(
            "originated_call_id".to_string(),
            child_call_id.to_string()
        );
    }

    storage.insert_cdr(cdr).await?;
    Ok(())
}
```

**FlowHandler Dial Coordination**:
When creating B-leg:
```rust
// Create B-leg call
let b_leg_call_id = CallId::new();

// Link A-leg and B-leg
call_manager.link_calls(a_leg_call_id, b_leg_call_id).await?;

// When either leg terminates, terminate the other
call_manager.set_termination_callback(b_leg_call_id, {
    let manager = call_manager.clone();
    let a_id = a_leg_call_id;
    move || {
        tokio::spawn(async move {
            let _ = manager.terminate_call(a_id, "B-leg terminated").await;
        });
    }
});
```

### Phase 5: Error Handling and Transitions

**Failure Scenarios**:

1. **486 Busy Here**:
   - Go to `on_busy` node if specified
   - Otherwise go to `on_error` or `next`
   - Set variable: `dial_status = "busy"`
   - Create CDR with disposition: Busy

2. **408 Request Timeout / 480 Temporarily Unavailable**:
   - Go to `on_no_answer` node if specified
   - Otherwise go to `on_error` or `next`
   - Set variable: `dial_status = "no_answer"`
   - Create CDR with disposition: NoAnswer

3. **487 Request Terminated (CANCEL)**:
   - User cancelled - go to `on_error`
   - Set variable: `dial_status = "cancelled"`
   - Create CDR with disposition: Cancelled

4. **Other 4xx/5xx/6xx**:
   - Go to `on_error` node if specified
   - Otherwise go to `next`
   - Set variable: `dial_status = "failed"`
   - Set variable: `dial_error_code = "503"`
   - Create CDR with disposition: Failed

5. **200 OK then immediate BYE**:
   - Normal call flow - answered then terminated
   - Go to `next` node
   - Set variable: `dial_status = "answered"`
   - Set variable: `dial_duration = "45"` (seconds)
   - Create CDRs for both legs with disposition: Answered

### Phase 6: Max Duration Handling

If `max_duration` is set and non-zero:
```rust
tokio::spawn(async move {
    tokio::time::sleep(Duration::from_secs(max_duration_secs as u64)).await;

    // Check if call is still active
    if call_manager.is_call_active(b_leg_call_id) {
        info!(
            a_leg = %a_leg_call_id,
            b_leg = %b_leg_call_id,
            "Max duration reached, terminating call"
        );

        // Terminate both legs
        call_manager.terminate_call(b_leg_call_id, "Max duration reached").await;
        call_manager.terminate_call(a_leg_call_id, "Max duration reached").await;
    }
});
```

## CDR Schema

Both A-leg and B-leg CDRs will have:

**A-leg CDR** (incoming call):
```json
{
  "call_id": "call-abc123",
  "sip_call_id": "xyz@caller-device",
  "from_uri": "sip:alice@example.com",
  "to_uri": "sip:+14155551234@fcp.local",
  "start_time": "2026-01-21T10:30:00Z",
  "answer_time": "2026-01-21T10:30:05Z",
  "end_time": "2026-01-21T10:32:30Z",
  "disposition": "answered",
  "metadata": {
    "originated_call_id": "call-def456"
  }
}
```

**B-leg CDR** (outbound call via Dial):
```json
{
  "call_id": "call-def456",
  "sip_call_id": "uvw@fcp.local",
  "from_uri": "sip:+14155551234@fcp.local",
  "to_uri": "sip:bob@remote.com",
  "start_time": "2026-01-21T10:30:02Z",
  "answer_time": "2026-01-21T10:30:05Z",
  "end_time": "2026-01-21T10:32:30Z",
  "disposition": "answered",
  "metadata": {
    "originated_by_dial": "true",
    "parent_call_id": "call-abc123"
  }
}
```

## Testing Plan

### Unit Tests
1. Test Dial node variable interpolation
2. Test FlowExecutionResult::WaitingForDial creation
3. Test CDR linking logic
4. Test max_duration timeout

### Integration Tests
1. **Successful Call**:
   - A calls FCP
   - FCP dials B
   - B answers
   - A and B talk
   - A hangs up
   - Verify 2 CDRs created and linked

2. **Busy Scenario**:
   - A calls FCP
   - FCP dials B
   - B returns 486 Busy
   - Flow goes to on_busy node
   - Verify CDR shows busy disposition

3. **No Answer Scenario**:
   - A calls FCP
   - FCP dials B
   - Timeout after 30 seconds
   - Flow goes to on_no_answer node
   - Verify CDR shows no_answer disposition

4. **Max Duration**:
   - A calls FCP with max_duration=10
   - FCP dials B
   - After 10 seconds, both legs terminated
   - Verify CDRs show answered with ~10 second duration

## MVP Limitations (Future Phases)

- ❌ Sequential Dialing (try multiple destinations)
- ❌ Parallel Dialing (ring multiple destinations simultaneously)
- ❌ Call screening (answer, then play message before bridging)
- ❌ Call recording during Dial
- ❌ DTMF passthrough configuration
- ❌ Early media handling (183 Session Progress)
- ❌ Call transfer during bridged call
- ❌ Hold/Resume during bridged call

## Success Criteria

✅ Dial node creates outbound SIP call
✅ Media is bridged between A-leg and B-leg
✅ Both legs tracked in separate CDRs
✅ CDRs are linked via metadata
✅ Failure scenarios transition to correct nodes
✅ Max duration enforcement works
✅ Clean termination of both legs
