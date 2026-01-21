# Tomorrow's Work Plan - Updated with Decisions

## Decisions Made ✅

1. **CDR Philosophy**: Only winning B-leg creates CDR (not cancelled forks)
2. **Extension Routing**: Implement User/Device/Service targets
3. **Multi-Tenancy**: Needs deeper design discussion (see below)
4. **Next Feature**: Voicemail system

---

## Priority 1: Complete Week 1 CDR Testing (1h 45min)

### 1.1 Fix CDR Creation for Parallel Forking (45 min) 🔥 NEW
**Problem**: Cancelled B-legs may be creating CDRs (need to investigate)

**Solution**: Only create CDR for the winning call leg
- [ ] Review call termination logic in parallel forking
- [ ] Ensure only A-leg creates CDR (not cancelled B-legs)
- [ ] Test: Make parallel fork call, verify only 1 CDR created (not 3)
- [ ] Update CallManager to track "is_forked_leg" flag

**Code Changes Needed**:
```rust
// In handle_parallel_forking:
// Mark cancelled B-legs so they don't create CDRs
for non-winning B-leg:
    mark_as_cancelled_fork(&call_id, b_leg_index);
    cancel().await;

// In terminate_call:
// Skip CDR creation for cancelled fork legs
if call.is_cancelled_fork {
    debug!("Skipping CDR for cancelled parallel fork leg");
    return;
}
```

### 1.2 Verify CDR Creation (30 min)
- [ ] Check CDRs exist in database ✅ (28 found)
- [ ] Verify answered calls have correct data (4 answered)
- [ ] After fork fix: Verify only 1 CDR per parallel fork call
- [ ] Check all CDR fields populated correctly

### 1.3 Test CDR API Endpoints (30 min)
- [ ] GET /api/v1/cdrs (list all)
- [ ] GET /api/v1/cdrs?disposition=answered
- [ ] GET /api/v1/cdrs?limit=5&offset=10
- [ ] GET /api/v1/cdrs/{call_id}
- [ ] Verify timestamp formats (ISO 8601)

---

## Priority 2: Implement Extension Target Types (2 hours)

### 2.1 User Target - Ring All User's Devices (1 hour)
**Use Case**: Extension 5001 → User "alice" → Ring all Alice's devices

**Implementation**:
```rust
ExtensionTarget::User(user_id) => {
    // Get all devices for this user
    let devices = storage.get_devices_by_user(user_id).await?;
    
    // Get registrations for each device
    let mut contacts = Vec::new();
    for device in devices {
        if let Some(registrations) = registrar.get_registrations(&device.sip_uri).await {
            contacts.extend(registrations);
        }
    }
    
    // Parallel fork to all contacts (reuse existing logic)
    parallel_fork(contacts).await
}
```

**Tasks**:
- [ ] Implement User target routing in handler.rs
- [ ] Add storage.get_devices_by_user() method
- [ ] Test: Create extension 5002 → User alice
- [ ] Verify: Calling 5002 rings all Alice's devices (parallel fork)

### 2.2 Device Target - Ring Specific Device (30 min)
**Use Case**: Extension 5100 → Device "Conference Room Phone"

**Implementation**:
```rust
ExtensionTarget::Device(device_id) => {
    // Get specific device
    let device = storage.get_device(device_id).await?;
    
    // Get registrations for this device only
    let contacts = registrar.get_registrations(&device.sip_uri).await?;
    
    // Route to device (no forking needed, single device)
    route_to_contacts(contacts).await
}
```

**Tasks**:
- [ ] Implement Device target routing
- [ ] Test: Create extension 5100 → Specific device
- [ ] Verify: Only that device rings (no forking)

### 2.3 Service Target - Route to IVR/Voicemail/Queue (30 min)
**Use Case**: Extension 500 → Voicemail, 600 → Main Menu IVR

**Implementation** (stub for now):
```rust
ExtensionTarget::Service(service_id) => {
    // Look up service configuration
    let service = storage.get_service(service_id).await?;
    
    match service.service_type {
        ServiceType::Voicemail => route_to_voicemail(service).await,
        ServiceType::IVR => route_to_ivr(service).await,
        ServiceType::Queue => route_to_queue(service).await,
        _ => return_not_implemented()
    }
}
```

**Tasks**:
- [ ] Add Service target stub implementation
- [ ] Return 501 "Service routing not yet implemented"
- [ ] Document for voicemail feature work

---

## Priority 3: Multi-Tenancy Design Discussion (30 min)

### Two Models Identified

#### Model 1: Service Provider (Full Multi-Tenancy)
**Example**: Hosted PBX provider serving 100 different companies
- Complete tenant isolation (data, users, phone numbers)
- Separate domains (company-a.pbx.example.com, company-b.pbx.example.com)
- Tenant ID in every database query
- Billing per tenant

#### Model 2: Enterprise Multi-Location (Organizational Hierarchy)
**Example**: Single bank with 1000 branches
- Single organization, multiple sites/locations
- Shared resources (phone numbers can route between branches)
- Location-based routing (office hours, emergency routing)
- Hierarchical structure: Organization → Region → Branch → Department

### Design Questions to Answer

1. **Do we need both models?**
   - Hybrid: TenantId (organization) + LocationId (site/branch)?
   - Or: Single TenantId with location as metadata?

2. **How does routing work across locations?**
   - Can Branch A call Branch B extension directly?
   - Do extensions need location prefix (Branch A: 1000-1999, Branch B: 2000-2999)?
   - Or use location codes (LA.5000, NY.5000)?

3. **What's shared vs isolated?**
   - Shared: Users can move between locations, global phone numbers
   - Isolated: Each location has own dial plan, device configs

4. **Database schema implications?**
   ```sql
   -- Current:
   users(id, tenant_id, ...)
   
   -- Option A: Add location:
   users(id, tenant_id, location_id, ...)
   
   -- Option B: Hierarchical:
   organizations(id, name)
   locations(id, org_id, parent_location_id, name)
   users(id, org_id, location_id, ...)
   ```

### Proposed Approach (For Discussion)

**Phase 1: Simple Multi-Tenant (Service Provider Model)**
- Add tenant_id to all queries ✅ (already done in storage)
- Add tenant context propagation through SIP calls
- Test with 2 separate tenants

**Phase 2: Location Support (Enterprise Model)**
- Add location_id as secondary grouping
- Implement location-based routing rules
- Support inter-location calling

**Tasks**:
- [ ] Decide: One model or both?
- [ ] Decide: Database schema approach
- [ ] Document tenant/location architecture
- [ ] Create migration plan

---

## Priority 4: Production Readiness (1 hour)

### 4.1 Testing
- [ ] Test Alice → Polycom calls (reverse direction)
- [ ] Test parallel forking with 3 devices
- [ ] Test edge cases (invalid extensions, offline devices)
- [ ] Test number normalization edge cases

### 4.2 Documentation
- [ ] Update WEEK1_TESTING_PROGRESS.md with completion
- [ ] Document parallel forking CDR behavior
- [ ] Document extension target types

### 4.3 Code Quality
- [ ] Run cargo clippy and fix warnings
- [ ] Add inline docs for parallel forking
- [ ] Clean up fcp.db.backup

---

## Priority 5: Voicemail System Design (If Time - 1 hour)

### High-Level Architecture

```
┌─────────────────────────────────────────────┐
│           Voicemail System                  │
├─────────────────────────────────────────────┤
│                                             │
│  ┌──────────────┐      ┌──────────────┐   │
│  │   Voicemail  │      │   Storage    │   │
│  │   Service    │─────▶│   Backend    │   │
│  └──────────────┘      └──────────────┘   │
│         │                                   │
│         ▼                                   │
│  ┌──────────────┐      ┌──────────────┐   │
│  │  Recording   │      │  Mailbox     │   │
│  │  Manager     │      │  Manager     │   │
│  └──────────────┘      └──────────────┘   │
│         │                      │           │
│         ▼                      ▼           │
│  ┌──────────────┐      ┌──────────────┐   │
│  │    Audio     │      │     MWI      │   │
│  │    Files     │      │   (NOTIFY)   │   │
│  └──────────────┘      └──────────────┘   │
└─────────────────────────────────────────────┘
```

### Core Features
1. **Record Message**: INVITE → IVR greeting → DTMF menu → Record after beep
2. **Playback Messages**: INVITE → Auth → Message list → DTMF navigation
3. **MWI (Message Waiting Indicator)**: SIP NOTIFY with Message-Summary body
4. **Mailbox Management**: Save, delete, forward messages

### Database Schema
```sql
CREATE TABLE voicemail_boxes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    pin TEXT,  -- Optional PIN for access
    greeting_url TEXT,  -- Custom greeting audio
    enabled BOOLEAN DEFAULT true,
    FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE TABLE voicemail_messages (
    id TEXT PRIMARY KEY,
    mailbox_id TEXT NOT NULL,
    caller_id TEXT,
    caller_name TEXT,
    recording_url TEXT NOT NULL,
    duration_seconds INTEGER,
    is_new BOOLEAN DEFAULT true,
    is_urgent BOOLEAN DEFAULT false,
    received_at INTEGER NOT NULL,
    FOREIGN KEY (mailbox_id) REFERENCES voicemail_boxes(id)
);
```

### IVR Flow Integration
```yaml
# Voicemail deposit flow (called when user doesn't answer)
flow:
  - type: Play
    audio: "vm_greeting.wav"  # "Please leave a message after the beep"
  
  - type: Play
    audio: "beep.wav"
  
  - type: Record
    max_duration: 300  # 5 minutes
    beep: false  # Already played
    terminator: "#"
    on_complete:
      - type: Play
        audio: "vm_saved.wav"  # "Your message has been saved"
      - type: Hangup

# Voicemail retrieval flow (user dials *98 or voicemail extension)
flow:
  - type: Authenticate
    prompt: "vm_enter_pin.wav"
    max_attempts: 3
    on_success:
      - type: PlayMessageList
        mailbox: "${user.mailbox_id}"
```

### Integration Points
1. **Call Routing**: Forward to voicemail on no_answer/busy
2. **Extension Target**: ExtensionTarget::Service → Voicemail
3. **IVR Nodes**: Record, PlayMessage, ManageMailbox nodes
4. **SIP NOTIFY**: Send MWI updates when new message arrives

### Tasks (Design Only for Now)
- [ ] Review IVR node types needed (Record, PlayMessage, etc.)
- [ ] Design database schema for voicemail
- [ ] Design service architecture
- [ ] Create voicemail feature branch plan
- [ ] Document MWI (Message Waiting Indicator) requirements

---

## Tomorrow's Success Criteria

### Minimum (Must Complete)
- ✅ Week 1 CDR testing complete and documented
- ✅ Parallel fork CDR fix (only winner creates CDR)
- ✅ Extension User/Device targets implemented and tested

### Stretch Goals
- ✅ Multi-tenancy architecture decided and documented
- ✅ Service target stub implemented
- ✅ All edge case testing complete

### Nice to Have
- ✅ Voicemail system designed and documented
- ✅ Production deployment guide started

---

## Timeline Estimate

| Task | Time | Priority |
|------|------|----------|
| Fix Parallel Fork CDRs | 45 min | P1 🔥 |
| CDR Verification | 30 min | P1 |
| CDR API Testing | 30 min | P1 |
| **Week 1 Complete** | **1h 45m** | **P1** |
| Extension User Target | 1 hour | P2 |
| Extension Device Target | 30 min | P2 |
| Extension Service Stub | 30 min | P2 |
| Multi-Tenancy Design | 30 min | P3 |
| Production Testing | 1 hour | P3 |
| Voicemail Design | 1 hour | P4 |
| **Total** | **6-7 hours** | |

---

## Files to Create/Modify Tomorrow

### New Files
- `MULTI_TENANCY_ARCHITECTURE.md` - Design doc for tenant/location model
- `VOICEMAIL_DESIGN.md` - Voicemail system architecture
- `crates/fcp-voicemail/` - New crate (stub/design only)

### Modified Files
- `crates/fcp-sip/src/handler.rs` - Extension User/Device targets, parallel fork CDR fix
- `crates/fcp-core/src/storage.rs` - Add get_devices_by_user() method
- `WEEK1_TESTING_PROGRESS.md` - Mark Day 4-5 complete
- `TOMORROW_PLAN.md` - Update with progress
