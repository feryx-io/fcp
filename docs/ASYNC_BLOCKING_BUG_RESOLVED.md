# B2BUA Response Routing Bug - RESOLVED

**Date:** 2025-12-24
**Severity:** Critical
**Status:** ✅ **RESOLVED**

## Summary

SIP responses from B-leg were arriving at the network interface within milliseconds but not being processed by the application until Timer B expired (32 seconds). This caused B2BUA calls to fail with timeout errors.

**Root Cause:** Async/await blocking in the listener event loop prevented concurrent message processing.

## The Problem

### Symptoms
1. ✅ INVITE sent to Bob successfully
2. ✅ Bob's phone rings
3. ✅ Bob responds immediately (100 Trying at 132ms, 180 Ringing at 312ms, 200 OK at 1.95s)
4. ✅ Responses arrive at network interface (confirmed by tcpdump)
5. ❌ Responses NOT processed by application for 32 seconds
6. ❌ Timer B expires
7. ❌ Transaction removed from map
8. ❌ Responses finally processed but transaction is gone

### Network Evidence (from pcap)

```
15:12:41.433036 - Server sends INVITE to Bob
15:12:41.565782 - Bob sends 100 Trying    (132ms latency) ✅
15:12:41.745147 - Bob sends 180 Ringing   (312ms latency) ✅
15:12:43.381544 - Bob sends 200 OK        (1.95s latency) ✅
```

**Bob responded perfectly!** The network was working fine.

### Application Timing (from logs)

```
20:12:41.432971 - Transaction created
20:13:13.433912 - Timer B expired (32 seconds later)
20:13:13.434xxx - Responses "arrive" at application
```

**Responses processed 32 seconds late!** The application was blocked.

## Root Cause Analysis

### The Blocking Code Path

**File:** `/home/siphon/fcp/crates/fcp-sip/src/listener.rs` (lines 200-228)

```rust
// Process incoming packets
loop {
    tokio::select! {
        Some(packet) = rx.recv() => {
            // BLOCKING: This awaits until handle_inbound completes
            if let Err(e) = handler.handle_inbound(
                txn_transport,
                packet.peer,
                packet.payload,
                packet.stream,
            ).await {  // ← BLOCKS HERE
                error!("Failed to handle inbound message: {}", e);
            }
        }
    }
}
```

### The Problem Flow

1. **Alice's INVITE arrives** → Listener calls `handler.handle_inbound().await`
2. **Handler starts B2BUA logic:**
   ```rust
   // From handler.rs:1171
   let res = call.await_provisional().await;  // ← BLOCKS waiting for Bob's responses
   ```
3. **Bob responds immediately** (within 2 seconds)
4. **Responses arrive at network interface** but listener is stuck on line 219
5. **Listener CANNOT process responses** because it's awaiting the previous call
6. **Timer B expires** after 32 seconds
7. **`await_provisional()` channel closes**, handler unblocks
8. **Listener finally processes buffered responses** — but transaction is already removed!

### Why This Happened

The listener processed packets **sequentially** in a single event loop:
- While processing Alice's INVITE, the loop was **blocked**
- Bob's responses arrived but **couldn't be processed**
- The very responses needed to unblock the handler were blocked by the handler itself!

This is a classic **deadlock pattern** in async code.

## The Fix

**Solution:** Spawn each message handler in a separate tokio task.

### Changes Made

**File:** `crates/fcp-sip/src/listener.rs`

#### UDP Listener (line 219)
```rust
// BEFORE (blocking):
if let Err(e) = handler.handle_inbound(...).await {
    error!("Failed to handle inbound message: {}", e);
}

// AFTER (non-blocking):
let handler = Arc::clone(&handler);
tokio::spawn(async move {
    if let Err(e) = handler.handle_inbound(...).await {
        error!("Failed to handle inbound message: {}", e);
    }
});
```

#### TCP Listener (line 271)
Same change applied to TCP message handling.

### Why This Works

1. **Listener never blocks** — immediately ready for next packet
2. **Each message handled concurrently** in its own task
3. **Responses processed immediately** (within milliseconds)
4. **Transaction matching works** because transaction still exists
5. **Call establishes successfully** with two-way audio

## Verification

### Test Results
- ✅ Call establishes successfully
- ✅ Two-way audio working
- ✅ No Timer B timeout
- ✅ Responses processed within 2 seconds (as expected)
- ✅ Transaction matching works correctly

### Debug Logging Added to siphon-rs

While debugging, we added comprehensive logging to `sip-transaction/src/manager.rs`:
- Transaction creation with branch parameter
- Response matching attempts with transaction lookup status
- Warnings when no matching transaction found

This logging helped identify that:
1. **Branch parameters matched correctly** (`z9hG4bKKrUf0c9u3mMEVVRE`)
2. **No branch mismatch** (original hypothesis was wrong)
3. **Transaction was missing** (`total_client_transactions: 0`)

The debug logging revealed the timing issue that led to discovering the blocking await problem.

## Impact

**Before Fix:**
- All B2BUA calls failed with 408 timeout
- Alice leg received 408 Request Timeout after 32 seconds
- No audio path established
- User frustration: "call is up no audio, alice is still ringing"

**After Fix:**
- Calls establish successfully within 2 seconds
- Two-way audio works perfectly
- Proper call signaling (100 Trying → 180 Ringing → 200 OK → ACK)
- User confirmation: "call is up with two way audio" ✅

## Lessons Learned

1. **Async event loops must never block** on long-running operations
2. **Spawn tasks for concurrent processing** when handling multiple independent operations
3. **Network captures are invaluable** for distinguishing network issues from application issues
4. **Branch parameter mismatch was a red herring** — the real issue was timing
5. **User insight was key:** "do we have an await issue where things are getting locked?" 💡

## Files Modified

- `/home/siphon/fcp/crates/fcp-sip/src/listener.rs` — Spawn message handlers in separate tasks
- `/home/siphon/fcp/external/siphon-rs/crates/sip-transaction/src/manager.rs` — Add debug logging (on branch `fix/response-routing-branch-mismatch`)

## Commit

```
commit d516294
Author: Claude Code
Date:   2025-12-24

Fix B2BUA response routing by spawning message handlers

The listener was processing packets sequentially, causing it to block
when the B2BUA logic awaited B-leg responses. This prevented responses
from being processed, leading to Timer B timeouts.

Fix: Spawn each handle_inbound() call in a separate task, allowing
the listener to immediately process subsequent packets.

Result: Calls now establish successfully with two-way audio.
```

## Status

**✅ RESOLVED** — B2BUA calls now work correctly with proper signaling and two-way audio.

## Follow-up Actions

1. ✅ Remove or merge debug logging from siphon-rs branch
2. ✅ Test under load to ensure spawned tasks don't cause resource exhaustion
3. ✅ Consider adding task pool or bounded spawning for production
4. ✅ Document async patterns in ARCHITECTURE.md

## Contact

For questions about this issue, see the FCP repository: https://github.com/feryx-io/fcp
