# Phone Number Routing - Current Status and Next Steps

## Problem

You added phone number `+15183217034` but when bob calls it, you get a **480 Temporarily Unavailable** error.

## Root Cause

The call handler (`crates/fcp-sip/src/handler.rs` lines 1000-1025) currently **only looks up destinations in the SIP registrar**. It doesn't check if the destination is a phone number with a line binding.

### Current Flow
```
1. Bob calls +15183217034
2. Handler looks for SIP registration: sip:+15183217034@192.168.1.225
3. Finds no registration (because phones register as sip:alice@domain, not as the DID)
4. Returns 480 Temporarily Unavailable
```

### What Should Happen
```
1. Bob calls +15183217034
2. Handler looks up line binding for phone number +15183217034
3. Line binding shows: phone → alice (user) → alice's devices
4. Handler looks up alice's registered devices in registrar
5. Handler routes call to alice's registered devices
```

## Current Infrastructure Status

### ✅ What's Implemented

1. **Phone Number Management** (`fcp-core/src/phone_number.rs`)
   - PhoneNumber model with E.164 validation
   - Storage: SQLite implementation
   - API: `/api/v1/phone-numbers` (CRUD operations)
   - TUI: Phone numbers view in CLI

2. **Line Bindings** (`fcp-core/src/line_binding.rs`)
   - LineBinding model (maps phone → user → devices)
   - Storage: SQLite implementation (`LineBindingStore` trait)
   - API: `/api/v1/line-bindings` (CRUD operations)
   - Supports binding types: User, IVR, Queue, HuntGroup, ConferenceBridge, etc.
   - Ring strategies: Simultaneous, Sequential, RoundRobin, LongestIdle, Primary

3. **Device Management**
   - Device model with SIP URI tracking
   - Storage and API endpoints
   - TUI: Devices view in CLI

4. **SIP Registrar**
   - Tracks device registrations (sip:alice@domain → Contact URI)
   - Working for device-to-device calls

### ❌ What's Missing

1. **Call Routing Logic** (`crates/fcp-sip/src/handler.rs:1000-1025`)
   - Handler doesn't check line bindings
   - Only does direct registrar lookup
   - Needs logic: phone number → line binding → user → devices → registrar

2. **TUI for Line Bindings**
   - No TUI view to create/manage line bindings
   - Must use API directly

## Testing Basic Routing (Current Workaround)

### Step 1: Get Required IDs

```bash
# Get phone number ID
curl http://localhost:8081/api/v1/phone-numbers | jq '.[] | select(.e164 == "+15183217034") | {id, e164}'

# Get alice's user ID
curl http://localhost:8081/api/v1/users | jq '.[] | select(.username == "alice") | {id, username}'

# Get alice's device ID
curl http://localhost:8081/api/v1/devices | jq '.[] | select(.name == "alice-device") | {id, name}'
```

### Step 2: Create a Line Binding via API

```bash
curl -X POST http://localhost:8081/api/v1/line-bindings \
  -H "Content-Type: application/json" \
  -d '{
    "phone_number_id": "<phone-number-id-from-step1>",
    "owner_user_id": "<alice-user-id-from-step1>",
    "binding_type": "user",
    "tenant_id": "00000000-0000-0000-0000-000000000000",
    "devices": [
      {
        "device_id": "<alice-device-id-from-step1>",
        "priority": 1,
        "line_appearance": 1,
        "shared_line": false,
        "enabled": true
      }
    ],
    "ring_strategy": "simultaneous"
  }'
```

### Step 3: Test (Will Still Fail Until Routing Implemented)

The line binding is now in the database, but the call will still fail with 480 because the handler doesn't know to look up line bindings.

## Implementing Basic Routing

The call handler needs to be updated in `crates/fcp-sip/src/handler.rs` around line 970-1050.

### Pseudocode for New Routing Logic

```rust
async fn route_call(destination_uri: &str) {
    // 1. Check if destination is a phone number (E.164 format)
    if is_e164_number(destination_uri) {
        // 2. Look up line binding for this phone number
        if let Some(line_binding) = storage.get_line_binding_by_phone_number(destination_uri).await {
            match line_binding.binding_type {
                BindingType::User => {
                    // 3. Get devices from line binding
                    let devices = line_binding.enabled_devices();
                    
                    // 4. For each device, look up SIP registration
                    let mut contacts = Vec::new();
                    for device in devices {
                        if let Some(user) = storage.get_user(line_binding.owner_user_id).await {
                            let aor = format!("sip:{}@{}", user.username, domain);
                            if let Some(registrations) = registrar.get_registrations(&aor).await {
                                contacts.extend(registrations);
                            }
                        }
                    }
                    
                    // 5. Route to found contacts
                    if !contacts.is_empty() {
                        return route_to_contacts(contacts);
                    }
                }
                BindingType::Ivr => {
                    // TODO: Route to IVR engine (Phase 2)
                    return error_response(501, "IVR not implemented");
                }
                // ... other binding types
            }
        }
    }
    
    // 6. Fall back to direct registrar lookup (existing behavior)
    registrar.get_registrations(destination_uri).await
}
```

### Required Code Changes

**File: `crates/fcp-sip/src/handler.rs`**

1. Add storage field to `SipMessageHandler`:
```rust
pub struct SipMessageHandler {
    // ... existing fields
    storage: Option<Arc<dyn fcp_core::Storage>>,
}
```

2. Import line binding types:
```rust
use fcp_core::{LineBinding, LineBindingStore, BindingType};
```

3. Replace the routing logic (lines 1000-1025) with phone number detection and line binding lookup.

4. Add E.164 detection helper:
```rust
fn is_e164_number(uri: &str) -> bool {
    // Extract user part from sip:+15183217034@domain
    let user_part = uri.strip_prefix("sip:").unwrap_or(uri)
        .split('@').next().unwrap_or("");
    
    user_part.starts_with('+') && user_part.len() > 7
}
```

## Advanced Features (Phase 2+)

### IVR Support

IVR is a **Phase 2 feature** (see `ARCHITECTURE-v0.2.md`). Not yet implemented.

**What's needed for IVR:**
- IVR script engine (`fcp-ivr` crate)
- DTMF detection
- Audio playback (TTS or pre-recorded)
- IVR flow DSL or webhook integration
- Script storage and management

**Current Status:**
- ✅ `BindingType::Ivr` exists in model
- ✅ Can create line bindings with `binding_type: "ivr"`
- ❌ No IVR engine to handle the call
- ❌ No audio/DTMF support
- ❌ No flow definition system

### Other Advanced Routing Features (Also Phase 2+)

- **Queues** - ACD with agent management
- **Hunt Groups** - Sequential/round-robin routing
- **Conference Bridge** - Multi-party calls
- **Time-based routing** - Business hours, holidays
- **Geographic routing** - Route based on caller location
- **LCR** - Least cost routing based on carrier rates

## Recommended Next Steps

### Priority 1: Implement Basic Routing (Required)
1. Add storage field to call handler
2. Implement E.164 detection
3. Add line binding lookup logic
4. Map devices to registrar lookups
5. Test: Bob → +15183217034 → alice

**Impact:** Phone numbers actually work! This is the foundation for everything else.

### Priority 2: Add TUI for Line Bindings (Quality of Life)
1. Create `crates/fcp-cli/src/tui/views/line_bindings.rs`
2. Add to tab navigation
3. UI for create/edit/delete bindings

**Impact:** Easier to test, better user experience.

### Priority 3: Advanced Features (Phase 2)
- IVR engine
- Queues
- Hunt groups
- etc.

**Impact:** Advanced call routing capabilities.

## Summary

**Current Status:**
- ✅ Phone numbers can be stored
- ✅ Line bindings can be created (via API)
- ❌ Calls to phone numbers don't work (missing routing logic)
- ❌ No IVR support yet (Phase 2 feature)

**Quick Answer to Your Question:**
No, you're not ready for IVR. IVR is a Phase 2 feature that requires significant additional work. But you CAN implement basic phone number routing (phone → user → devices) which will let bob successfully call alice at +15183217034.

**Immediate Next Step:**
Implement the routing logic in `crates/fcp-sip/src/handler.rs` to look up line bindings when the destination is a phone number.

## Files to Reference

- **Routing logic:** `crates/fcp-sip/src/handler.rs:970-1050`
- **Line binding model:** `crates/fcp-core/src/line_binding.rs`
- **Storage trait:** `crates/fcp-core/src/storage.rs` (search for `LineBindingStore`)
- **Storage implementation:** `crates/fcp-storage-sqlite/src/lib.rs:1141+`
- **API endpoints:** `crates/fcp-server/src/api/v1/line_bindings.rs`
