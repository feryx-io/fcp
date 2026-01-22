# Architecture Fixes Plan
**Date**: 2026-01-21
**Priority**: High
**Status**: Planning

## Overview

Three critical architecture issues identified that need immediate attention:

1. **HIGH**: Extension routing ignores tenant context (security issue)
2. **HIGH**: Parallel forking never relays provisional responses (user experience issue)
3. **MEDIUM**: Transcript writer uses blocking I/O on async runtime (performance/stability issue)

---

## Issue #1: Extension Routing Tenant Isolation ⚠️ SECURITY

### Current Behavior

**File**: `crates/fcp-sip/src/handler.rs:1582-1583`

```rust
// Get tenant ID (for now, use default tenant)
let tenant_id = fcp_core::TenantId::nil();
```

**Problem**: All extension lookups use `TenantId::nil()` regardless of which tenant made the call. This breaks multi-tenant isolation:
- Extension 5001 in Tenant A can be dialed from Tenant B
- Cross-tenant call routing bypass
- Security/privacy violation

**Root Causes**:
1. `Call::from_invite()` hardcodes `tenant_id: TenantId::nil()` (call.rs:281)
2. Extension lookup ignores call's tenant_id (handler.rs:1583)

### Solution

**Phase 1: Use Call's tenant_id** (Quick fix - 10 minutes)

In `handler.rs:1582-1583`, replace:
```rust
let tenant_id = fcp_core::TenantId::nil();
```

With:
```rust
// Get tenant ID from the call (already stored when call was created)
let tenant_id = self.call_manager
    .get_call(&call_id)
    .map(|call| call.tenant_id)
    .unwrap_or_else(|| fcp_core::TenantId::nil());
```

**Phase 2: Extract tenant from INVITE** (Proper fix - 1-2 hours)

Modify `Call::from_invite()` in `call.rs` to extract tenant context from:

Option A: **From Domain-based** (simplest)
```rust
// Extract tenant from From header domain
// sip:alice@tenant-abc.fcp.local -> lookup tenant by domain
let tenant_id = extract_tenant_from_domain(request)?
    .unwrap_or_else(|| TenantId::nil());
```

Option B: **Custom Header** (most explicit)
```rust
// X-Tenant-ID: tenant-abc123...
let tenant_id = request.headers().get("X-Tenant-ID")
    .and_then(|h| h.parse::<TenantId>().ok())
    .unwrap_or_else(|| TenantId::nil());
```

Option C: **Authentication-based** (most secure)
```rust
// Extract from P-Asserted-Identity or authenticated From user
// Requires database lookup: user -> tenant_id
let tenant_id = lookup_user_tenant(authenticated_user_id)?
    .unwrap_or_else(|| TenantId::nil());
```

**Recommendation**: Implement Option A first (domain-based), add Option C later for production.

### Testing

```bash
# Create extensions in two different tenants
curl -X POST http://localhost:8081/api/v1/extensions \
  -H "Content-Type: application/json" \
  -d '{"number": "5001", "target": {"type": "user", "id": "user-alice"}, "tenant_id": "tenant-abc"}'

curl -X POST http://localhost:8081/api/v1/extensions \
  -H "Content-Type: application/json" \
  -d '{"number": "5001", "target": {"type": "user", "id": "user-bob"}, "tenant_id": "tenant-xyz"}'

# Test: Dial 5001 from tenant-abc -> should reach Alice
# Test: Dial 5001 from tenant-xyz -> should reach Bob
# Test: Dial 5001 from tenant-abc with spoofed tenant -> should fail
```

### Risk

**HIGH** - Multi-tenant isolation breach. Fix Phase 1 immediately.

---

## Issue #2: Parallel Forking Provisional Response Relay 📞

### Current Behavior

**File**: `crates/fcp-sip/src/handler.rs:2520-2760`

```rust
async fn handle_parallel_forking(...) {
    // Line 2543: Only waits for FINAL responses (200-699)
    match b_leg_handle_clone.await_final().await {
        Ok(response) => { /* ... */ }
    }
}
```

**Problem**:
- Callers never see `180 Ringing` from callees
- No early media support (`183 Session Progress` with SDP)
- No PRACK support (RFC 3262 reliable provisionals)
- Callers hear dead air during ring time (can be 30+ seconds)
- Poor user experience (caller doesn't know if call is progressing)

**User Impact**: "Is my call even working?" - no feedback until answer or timeout.

### Solution

**Approach**: Relay provisional responses from ALL B-legs to A-leg in real-time

```rust
async fn handle_parallel_forking(
    &self,
    request: sip_core::Request,
    a_leg_handle: Arc<ServerTransactionHandle>,
    b_leg_calls: Vec<(String, Arc<OutboundCall>)>,
    call_id: CallId,
) -> Result<()> {
    // Create response relay tasks for each B-leg
    let mut relay_tasks = Vec::new();

    for (target, b_leg_handle) in &b_leg_calls {
        let b_leg_handle = Arc::clone(b_leg_handle);
        let a_leg_handle = Arc::clone(&a_leg_handle);
        let call_id = call_id.clone();
        let target = target.clone();

        // Spawn task to relay ALL responses (provisional + final)
        let relay_task = tokio::spawn(async move {
            let mut sent_provisional = false;

            loop {
                // Wait for next response (provisional or final)
                match b_leg_handle.wait_response().await {
                    Ok(response) => {
                        let code = response.start_line().code();

                        if code >= 100 && code < 200 {
                            // Provisional response - relay to A-leg
                            if !sent_provisional && code >= 180 {
                                // First 180+ Ringing - send to caller
                                info!(
                                    call_id = %call_id,
                                    target = %target,
                                    code = code,
                                    "Relaying provisional response to A-leg"
                                );

                                // Create provisional response for A-leg
                                let provisional = create_provisional_response(
                                    &request,
                                    code,
                                    response.start_line().reason(),
                                );

                                // Send to A-leg (non-blocking)
                                let _ = a_leg_handle.send_provisional(provisional).await;
                                sent_provisional = true;
                            }

                            // Continue waiting for final
                            continue;
                        } else {
                            // Final response (200-699) - return it
                            return Some((code, response, b_leg_handle));
                        }
                    }
                    Err(e) => {
                        warn!(
                            call_id = %call_id,
                            target = %target,
                            error = %e,
                            "B-leg failed"
                        );
                        return None;
                    }
                }
            }
        });

        relay_tasks.push(relay_task);
    }

    // Race for first 2xx final response
    let mut winning_response = None;
    while !relay_tasks.is_empty() {
        let (result, _idx, remaining) = futures::future::select_all(relay_tasks).await;
        relay_tasks = remaining;

        if let Ok(Some((code, response, handle))) = result {
            if code >= 200 && code < 300 {
                // Winner!
                winning_response = Some((response, handle));
                break;
            }
        }
    }

    // CANCEL non-winners
    for (target, b_leg_handle) in &b_leg_calls {
        if Some(Arc::as_ptr(b_leg_handle)) != winning_response.as_ref().map(|(_, h)| Arc::as_ptr(h)) {
            let _ = b_leg_handle.cancel().await;
        }
    }

    // Relay winning 200 OK to A-leg
    if let Some((response, handle)) = winning_response {
        // ... existing code to relay 200 OK ...
    }

    Ok(())
}
```

**Key Changes**:
1. Use `wait_response()` instead of `await_final()` to get ALL responses
2. Relay first `180 Ringing` from any B-leg to A-leg immediately
3. Continue waiting for `200 OK` after sending provisionals
4. Race for first `200 OK` (winner), CANCEL losers
5. Relay winning `200 OK` to A-leg

**Provisional Response Strategy**:
- **First 180 Ringing wins**: Send to A-leg (caller hears ringing immediately)
- **183 Session Progress**: Relay with SDP for early media
- **PRACK (100rel)**: Store in `call.a_leg.reliable_provisionals` for forwarding

### Testing

```bash
# Test 1: Verify 180 Ringing relayed
# - Polycom dials 5001
# - Should hear ringing tone immediately (not dead air)
# - Check Polycom display shows "Ringing..."

# Test 2: Early media (if supported by B-leg)
# - IVR that sends 183 with SDP
# - Caller should hear early media announcement

# Test 3: Multiple endpoints timing
# - Alice has 3 devices
# - Call 5001
# - First device to send 180 -> caller hears ringing
# - First device to answer (200) -> call connects
```

### Risk

**MEDIUM** - Requires careful state management. Test thoroughly with:
- Single B-leg (no forking)
- Multiple B-legs (parallel forking)
- B-leg failure scenarios
- CANCEL race conditions

---

## Issue #3: Transcript Writer Blocking I/O 💾

### Current Behavior

**File**: `crates/fcp-sip/src/handler.rs:443-488`

```rust
// Line 445: Unbounded channel (no backpressure)
let (tx, mut rx) = mpsc::unbounded_channel::<TranscriptEntry>();

tokio::spawn(async move {
    while let Some(entry) = rx.recv().await {
        // Line 455: Blocking I/O on async runtime
        if let Ok(meta) = std::fs::metadata(&path) {  // BLOCKS
            // ...
            let _ = std::fs::rename(&path, new_path);  // BLOCKS
        }

        // Line 470: Blocking I/O
        if let Ok(mut file) = OpenOptions::new().open(&path) {  // BLOCKS
            let _ = writeln!(file, ...);  // BLOCKS
        }
    }
});
```

**Problems**:
1. **Unbounded channel**: Under high call volume, queue can grow unbounded → OOM risk
2. **Blocking I/O on Tokio runtime**: `std::fs` operations block worker threads
   - Slow disk → stalled workers → latency spikes
   - All active calls affected by transcript I/O
3. **No error handling**: Failed writes silently dropped

### Solution

**Option A: Bounded Channel + tokio::fs** (Recommended)

```rust
// Use bounded channel for backpressure
let (tx, mut rx) = mpsc::channel::<TranscriptEntry>(1000);  // Max 1000 pending
let transcript_dir = dir.clone();
let max_bytes = settings.transcript_max_bytes;

tokio::spawn(async move {
    while let Some(entry) = rx.recv().await {
        let filename = format!("{}.log", entry.call_id.replace(':', "_"));
        let path = transcript_dir.join(&filename);

        // Check file size (async)
        if let Some(max) = max_bytes {
            if let Ok(meta) = tokio::fs::metadata(&path).await {
                if meta.len() > max {
                    let ts = chrono::Utc::now().timestamp();
                    let new_path = transcript_dir.join(format!(
                        "{}.{}.log",
                        entry.call_id.replace(':', "_"),
                        ts
                    ));
                    let _ = tokio::fs::rename(&path, new_path).await;
                }
            }
        }

        // Open and write (async)
        match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
        {
            Ok(mut file) => {
                use tokio::io::AsyncWriteExt;

                let entry_ts = chrono::DateTime::<chrono::Utc>::from(entry.timestamp)
                    .to_rfc3339();
                let line = format!(
                    "{} {} {} dialog={} cseq={} peer={} transport={} rr={} route={} body_hash={}\n",
                    entry_ts,
                    entry.direction,
                    entry.method_or_status,
                    entry.dialog_id_str,
                    entry.cseq,
                    entry.peer.as_deref().unwrap_or(""),
                    entry.transport.as_deref().unwrap_or(""),
                    entry.record_route,
                    entry.route,
                    entry.body_hash.as_deref().unwrap_or("")
                );

                if let Err(e) = file.write_all(line.as_bytes()).await {
                    tracing::error!(
                        call_id = %entry.call_id,
                        error = %e,
                        "Failed to write transcript entry"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    call_id = %entry.call_id,
                    path = ?path,
                    error = %e,
                    "Failed to open transcript file"
                );
            }
        }
    }

    tracing::info!("Transcript writer task exiting");
});
```

**Option B: Bounded Channel + spawn_blocking** (Alternative)

```rust
let (tx, mut rx) = mpsc::channel::<TranscriptEntry>(1000);

tokio::spawn(async move {
    while let Some(entry) = rx.recv().await {
        // Run blocking I/O on dedicated thread pool
        if let Err(e) = tokio::task::spawn_blocking(move || {
            let filename = format!("{}.log", entry.call_id.replace(':', "_"));
            let path = transcript_dir.join(&filename);

            // ... blocking std::fs operations ...

        }).await {
            tracing::error!(error = ?e, "Transcript writer task panicked");
        }
    }
});
```

**Recommendation**: Option A (`tokio::fs`) - cleaner async code, better integration.

### Backpressure Handling

When channel is full (1000 entries), two options:

**Option 1: Drop oldest** (lossy, but keeps system stable)
```rust
if let Err(e) = tx.try_send(entry) {
    tracing::warn!(
        call_id = %call_id,
        "Transcript channel full, dropping entry"
    );
}
```

**Option 2: Block briefly** (preserves all data, adds latency)
```rust
tokio::select! {
    result = tx.send(entry) => {
        if let Err(e) = result {
            tracing::error!(error = %e, "Transcript channel closed");
        }
    }
    _ = tokio::time::sleep(Duration::from_millis(100)) => {
        tracing::warn!("Transcript channel full, dropping entry after timeout");
    }
}
```

**Recommendation**: Option 1 (drop oldest) - transcripts are debug/forensics, not critical path.

### Testing

```bash
# Test 1: Normal operation
# - Make 10 calls
# - Verify transcripts written
# - Check no blocking on call handler

# Test 2: High load
# - Make 100 concurrent calls
# - Monitor channel depth
# - Verify no OOM or latency spikes

# Test 3: Slow disk
# - Simulate slow disk (mount with sync option)
# - Make calls
# - Verify handlers not blocked

# Test 4: Full disk
# - Fill disk to 99%
# - Make calls
# - Verify errors logged, calls succeed
```

### Risk

**LOW** - Isolated to transcript feature. Call handling unaffected by failures.

---

## Implementation Priority

### Phase 1: Critical (Today)
1. ✅ **Issue #1 Phase 1**: Use call's tenant_id (10 min)
   - Security fix
   - Zero risk
   - Unblocks testing

### Phase 2: High (This Week)
2. **Issue #3**: Fix transcript writer (1 hour)
   - Low risk (isolated feature)
   - Prevents production stability issues

3. **Issue #2**: Relay provisional responses (2-3 hours)
   - Improves UX significantly
   - Moderate risk (state management)
   - Requires thorough testing

### Phase 3: Production (Next Week)
4. **Issue #1 Phase 2**: Extract tenant from INVITE (2 hours)
   - Complete multi-tenant security
   - Requires decision on extraction method (domain vs header vs auth)

---

## Verification

After all fixes:

```bash
# Tenant isolation
./test_tenant_isolation.sh  # Pass: Extensions scoped per-tenant

# Provisional responses
./test_ringing_relay.sh     # Pass: Caller hears ringing immediately

# Transcript writer
./test_transcript_load.sh   # Pass: No blocking under 100 concurrent calls
```

---

**Created**: 2026-01-21
**Status**: Ready for implementation
**Estimated Time**: Phase 1 (10m) + Phase 2 (4h) + Phase 3 (2h) = ~6-7 hours total
