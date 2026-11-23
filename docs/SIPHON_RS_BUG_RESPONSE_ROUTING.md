# siphon-rs Bug Report: Response Routing Failure in B2BUA Scenario

**Date:** 2025-12-24
**Reporter:** FCP Integration Testing
**Severity:** Critical
**Affects:** sip-transaction, sip-uac (integrated UAC)

## Summary

SIP responses (180 Ringing, 200 OK) from UAS are arriving at the UAC's transport layer but are not being delivered to the `ClientTransactionUser` callbacks (`on_provisional`, `on_final`). This causes B2BUA call setup to fail as the A-leg never receives responses from the B-leg.

## Environment

- **FCP Server:** 0.1.1
- **siphon-rs:** git submodule at commit 1efcf7e
- **Network:** Local subnet (192.168.1.0/24), no firewall
- **Softphones:** Zoiper on both ends (Alice: 192.168.1.80, Bob: 192.168.1.206)
- **Server:** 192.168.1.81:5060

## Symptoms

1. ✅ INVITE sent to Bob successfully (tcpdump confirms)
2. ✅ Bob's phone rings
3. ✅ Bob responds with 100 Trying, 180 Ringing, 200 OK (tcpdump confirms arrival)
4. ✅ Responses logged by FCP handler: "SIP message trace, direction: inbound"
5. ✅ `TransactionManager::receive_response()` is called
6. ❌ `InviteTransactionUser::on_provisional()` NEVER called
7. ❌ `InviteTransactionUser::on_final()` NEVER called
8. ❌ Timer B expires after 32 seconds
9. ❌ `InviteTransactionUser::on_terminated()` called with "channel closed"

## tcpdump Evidence

```
14:47:38.671 - Server sends INVITE to Bob
Via: SIP/2.0/UDP 192.168.1.81:5060;branch=z9hG4bKYdUEStJOu2GdzYmZ;rport

14:47:38.804 - Bob responds with 100 Trying
Via: SIP/2.0/UDP 192.168.1.81:5060;branch=z9hG4bKYdUEStJOu2GdzYmZ;rport=5060

14:47:38.906 - Bob responds with 180 Ringing
Via: SIP/2.0/UDP 192.168.1.81:5060;branch=z9hG4bKYdUEStJOu2GdzYmZ;rport=5060

14:47:40.285 - Bob responds with 200 OK
Via: SIP/2.0/UDP 192.168.1.81:5060;branch=z9hG4bKYdUEStJOu2GdzYmZ;rport=5060
```

**Note:** Branch parameter is correctly echoed in all responses: `z9hG4bKYdUEStJOu2GdzYmZ`

## FCP Server Logs

```
INFO sip_uac::integrated: Started INVITE client transaction z9hG4bKsq7AvvjoQQMGhxUT to sip:bob@...
INFO fcp_sip::handler: B-leg INVITE sent, awaiting responses, call_id: 97fede7e...
INFO fcp_sip::handler: Waiting for provisional responses from B-leg...

[32 seconds pass - no callbacks invoked]

INFO fcp_sip::handler: Waiting for final response from B-leg...
ERROR fcp_sip::handler: Failed to receive final response from B-leg, error: Final response channel closed: channel closed
INFO fcp_sip::call_manager: Call terminated, state: Failed
```

## Code Flow Analysis

### Working Flow (Expected)

1. `IntegratedUAC::invite()` creates channels and `InviteTransactionUser`
2. `TransactionManager::start_client_transaction()` stores transaction with key
3. Response arrives → `SipMessageHandler::handle_inbound()` (✅ works)
4. `TransactionManager::receive_response()` called (✅ works)
5. Extract branch from response Via header (❓)
6. Build `TransactionKey` from branch + method (❓)
7. Look up transaction: `self.inner.client.contains_key(&key)` (❌ returns false)
8. Call `dispatch_response()` if found (❌ skipped)
9. `InviteTransactionUser` callbacks invoked (❌ never happens)

### Actual Behavior

Response matching fails at step 7, causing silent drop at `TransactionManager::receive_response()` line 714:

```rust
// Look up and dispatch to client transaction
if self.inner.client.contains_key(&key) {
    self.dispatch_response(&key, response).await;
}
// SILENTLY DROPS response if key not found!
```

## Root Cause Hypothesis

**Branch Parameter Mismatch:** The `TransactionKey` built from the response doesn't match the key used when creating the transaction.

Possible causes:
1. **Branch extraction issue:** `Self::extract_branch(&response.headers)` returns different value than expected
2. **Via header parsing:** Top Via may have additional parameters affecting branch extraction
3. **Case sensitivity:** Branch comparison might be case-sensitive when it should be case-insensitive
4. **Transaction storage timing:** Transaction might be removed or not yet stored when response arrives

## Investigation Needed

### 1. Add Debug Logging

In `sip-transaction/src/manager.rs`, `receive_response()` method (line 693):

```rust
pub async fn receive_response(&self, response: Response) {
    // Extract branch from Via header
    let branch = match Self::extract_branch(&response.headers) {
        Some(b) => b,
        None => {
            debug!("Response has no branch in Via header");  // ADD THIS
            return;
        }
    };

    // Extract method from CSeq header
    let method = match Self::extract_cseq_method(&response.headers) {
        Some(m) => m,
        None => {
            debug!("Response has no method in CSeq header");  // ADD THIS
            return;
        }
    };

    // Build transaction key (is_server=false for client transactions)
    let key = TransactionKey {
        branch,
        method,
        is_server: false,
    };

    debug!(             // ADD THIS ENTIRE BLOCK
        "Attempting to match response to transaction",
        branch = %key.branch,
        method = ?key.method,
        has_transaction = self.inner.client.contains_key(&key),
        total_client_transactions = self.inner.client.len()
    );

    // Look up and dispatch to client transaction
    if self.inner.client.contains_key(&key) {
        self.dispatch_response(&key, response).await;
    } else {
        warn!(          // ADD THIS
            "No matching transaction for response",
            branch = %key.branch,
            method = ?key.method,
            status = response.start.code
        );
    }
}
```

### 2. Log Transaction Key at Creation

In `sip-transaction/src/manager.rs`, `start_client_transaction()` method (line 634):

```rust
pub async fn start_client_transaction(
    &self,
    request: Request,
    ctx: TransportContext,
    tu: Arc<dyn ClientTransactionUser>,
) -> Result<TransactionKey> {
    let key = TransactionKey::from_request(&request, false)
        .ok_or_else(|| anyhow!("No branch in top Via"))?;

    info!(          // ADD THIS
        "Starting client transaction",
        branch = %key.branch,
        method = ?key.method,
        call_id = ?request.headers.get("Call-ID")
    );

    // ... rest of method
}
```

### 3. Compare Branch Values

Add test to print raw branch extraction:

```rust
// In extract_branch() or add temporary debug code
fn extract_branch(headers: &Headers) -> Option<SmolStr> {
    let via = headers.get("Via")?;
    let branch = branch_from_via(via.as_str())?;
    debug!("Extracted branch from Via: '{}' (raw Via: '{}')", branch, via);
    Some(SmolStr::new(branch.to_owned()))
}
```

## Workaround

None currently. The transaction layer response routing must be fixed for B2BUA operation.

## Suggested Fix

After identifying the exact mismatch via debug logging:

1. **If branch extraction differs:** Fix `branch_from_via()` parser
2. **If timing issue:** Ensure transaction is stored before any responses can arrive
3. **If case sensitivity:** Make branch comparison case-insensitive per RFC 3261
4. **If Via parameter pollution:** Strip parameters before comparison

## Files Involved

- `/home/siphon/fcp/external/siphon-rs/crates/sip-transaction/src/manager.rs` - Response routing logic
- `/home/siphon/fcp/external/siphon-rs/crates/sip-transaction/src/lib.rs` - Branch extraction (`branch_from_via()`)
- `/home/siphon/fcp/external/siphon-rs/crates/sip-uac/src/integrated.rs` - `InviteTransactionUser` callbacks
- `/home/siphon/fcp/crates/fcp-sip/src/handler.rs` - Response forwarding to TransactionManager

## Testing

### Reproduce

1. Configure FCP with `local_ip = "192.168.1.81"` in fcp.toml
2. Register two softphones (Alice, Bob)
3. Alice calls Bob
4. Observe logs: INVITE sent, responses arrive, but callbacks never invoked
5. Timer B expires after 32 seconds

### Verify Fix

After applying fix, should see:
- `InviteTransactionUser::on_provisional()` called for 180 Ringing
- `InviteTransactionUser::on_final()` called for 200 OK
- ACK sent to Bob
- Media path established
- No Timer B timeout

## Next Steps

1. Add debug logging as described above
2. Capture branch values at both creation and response matching
3. Identify exact mismatch
4. Fix branch extraction/comparison logic
5. Test with same scenario
6. Add regression test to siphon-rs test suite

## Contact

For questions or updates, see FCP repository: https://github.com/feryx-io/fcp
