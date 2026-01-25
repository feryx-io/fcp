# Tomorrow's Work Plan - Week 1 Completion + Beyond

## Priority 1: Complete Week 1 CDR Testing (Day 4-5) ✅

### 1.1 Verify CDR Creation (30 min)
**Status**: CDRs ARE being created! Found 28 CDRs in database.
- [x] Check CDRs exist in database ✅ (28 records found)
- [ ] Verify answered calls have correct data (4 answered CDRs found)
- [ ] Verify failed/cancelled calls have correct dispositions (17 failed, 7 cancelled)
- [ ] Check all CDR fields are populated (setup_duration, call_duration, codec, etc.)
- [ ] Identify any missing/null fields that should be populated

**Quick Check Already Done**:
```sql
-- 4 answered, 7 cancelled, 17 failed
-- Sample answered CDR has: setup=11s, call=2.7s, total=13.8s, codec=G722
-- hangup_party is NULL (expected MVP limitation)
```

### 1.2 Test CDR API Endpoints (30 min)
- [ ] Test GET /api/v1/cdrs (list all CDRs)
- [ ] Test GET /api/v1/cdrs?disposition=answered (filter by disposition)
- [ ] Test GET /api/v1/cdrs?limit=5&offset=10 (pagination)
- [ ] Test GET /api/v1/cdrs/{call_id} (get specific CDR)
- [ ] Verify timestamps are in correct format (ISO 8601)
- [ ] Test error cases (invalid call_id, etc.)

### 1.3 CDR Field Verification (30 min)
Create test calls and verify CDR fields:
- [ ] **Answered call**: Polycom → Alice, answer, talk 10s, hangup
  - Verify: disposition=answered, durations, codec, sip_call_id
- [ ] **Busy call**: Call alice while on another call
  - Verify: disposition=busy, sip_status_code=486
- [ ] **No answer call**: Call and don't answer
  - Verify: disposition=no_answer, sip_status_code=480/408
- [ ] **Cancelled call**: Call and CANCEL before answer
  - Verify: disposition=cancelled, sip_status_code=487

### 1.4 Update Documentation (15 min)
- [ ] Update WEEK1_TESTING_PROGRESS.md with Day 4-5 completion
- [ ] Document CDR API endpoints tested
- [ ] Note any CDR fields still TODO (hangup_party, recording_url, cost)

---

## Priority 2: Production Readiness (1-2 hours)

### 2.1 Testing Improvements
- [ ] Test Alice → Polycom calls (reverse direction)
- [ ] Test parallel forking with 3 devices (register 3rd softphone)
- [ ] Test extension dialing edge cases:
  - [ ] Invalid extension (should return 404 or 480)
  - [ ] Extension with no registered devices
  - [ ] Extension pointing to offline user
- [ ] Test number normalization edge cases:
  - [ ] 7-digit numbers (should NOT normalize)
  - [ ] 11-digit with leading 1
  - [ ] Already normalized (+1...)
  - [ ] International numbers (non-NANP)

### 2.2 Configuration Cleanup
- [ ] Review fcp.toml for production settings
- [ ] Document required config values for production
- [ ] Add example configs for common scenarios

### 2.3 Logging & Monitoring
- [ ] Review log output quality for production ops
- [ ] Add metrics for parallel forking (fork count, winner latency)
- [ ] Document how to monitor call routing in production

---

## Priority 3: Bug Fixes & Polish (30-60 min)

### 3.1 Known Issues
- [ ] Check why some CDRs show "cancelled" when calls were answered
  - Investigate: Are these from parallel forking losers?
  - Should cancelled B-legs create separate CDRs?
- [ ] Review failed CDRs (17 found) - why so many failures?
- [ ] Clean up untracked fcp.db.backup file

### 3.2 Code Quality
- [ ] Run cargo clippy and fix warnings
- [ ] Add inline documentation for parallel forking logic
- [ ] Clean up any TODO comments in handler.rs

---

## Priority 4: Next Features (If Time Permits)

### 4.1 Missing CDR Features (from plan)
- [ ] Implement hangup_party detection (track who sent BYE)
- [ ] Add RTP statistics to CDRs (packets, bytes, jitter)
- [ ] Implement `extract_sip_status_code` helper for API

### 4.2 Advanced Routing
- [ ] Implement User target for extensions (ring all user's devices)
- [ ] Implement Device target for extensions (ring specific device)
- [ ] Add call forwarding (forward on busy, no answer, always)

### 4.3 Multi-Tenancy
- [ ] Add tenant context propagation through call flow
- [ ] Test multi-tenant scenarios (2 tenants with same extensions)
- [ ] Verify tenant isolation in database queries

---

## Success Criteria for Tomorrow

**Minimum (Complete Week 1)**:
- ✅ CDR verification complete with documentation
- ✅ CDR API endpoints tested and working
- ✅ All test scenarios documented

**Stretch Goals**:
- ✅ Parallel forking tested with 3+ devices
- ✅ All edge cases tested and documented
- ✅ Production deployment guide started

**Nice to Have**:
- ✅ Hangup party detection implemented
- ✅ Call forwarding basics working
- ✅ Multi-tenant testing complete

---

## Estimated Timeline

| Task | Time | Priority |
|------|------|----------|
| CDR Verification | 30 min | P1 |
| CDR API Testing | 30 min | P1 |
| CDR Field Testing | 30 min | P1 |
| Documentation Update | 15 min | P1 |
| **Week 1 Complete** | **1h 45m** | **P1** |
| Testing Improvements | 45 min | P2 |
| Config/Logging | 30 min | P2 |
| Bug Fixes | 30 min | P3 |
| New Features | 1-2 hours | P4 |
| **Total** | **4-5 hours** | |

---

## Questions to Consider

1. **CDR Philosophy**: Should cancelled B-legs in parallel forking create CDRs?
   - Current: They might be (need to verify)
   - Alternative: Only create CDR for winning call leg

2. **Extension Routing**: Should we implement User/Device/Service targets now?
   - User target = ring all user's devices (like parallel forking)
   - Device target = ring specific device only
   - Service target = route to IVR/voicemail/queue

3. **Multi-Tenancy**: Is this needed for your deployment?
   - Single tenant = simpler, faster to deploy
   - Multi-tenant = more complex, needed for hosting

4. **Next Big Feature**: What's most valuable after Week 1?
   - Voicemail system
   - Call queues (ACD)
   - Call recording
   - WebRTC support
   - IVR enhancements
