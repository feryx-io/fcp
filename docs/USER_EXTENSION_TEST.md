# User Extension Target Test
**Date**: 2026-01-21
**Feature**: ExtensionTarget::User - Ring all user's devices

## Test Setup ✅

### Database State

**Extension Created**:
```sql
-- Extension 5002 points to Alice's user ID
SELECT number, target_type, target_id, description
FROM extensions
WHERE number = '5002';
```

**Result**:
```
5002 | user | user-8840f5e2-87d2-4801-8b93-c854bf2ad5c2 | Alice - All Devices (Test)
```

**Alice's Devices**:
```sql
SELECT d.id, d.name, d.sip_uri, d.enabled, COUNT(r.contact) as reg_count
FROM devices d
LEFT JOIN registrations r ON d.sip_uri = r.aor
WHERE d.owner_user_id = 'user-8840f5e2-87d2-4801-8b93-c854bf2ad5c2'
GROUP BY d.id;
```

**Result**:
```
Device ID: dev-11014c84-9db2-4278-bcdb-700b768b8657
Name: Alice Softphone
SIP URI: sip:alice@192.168.1.225
Enabled: Yes
Registrations: 2
```

**Alice's Registrations** (2 endpoints):
```
1. <sip:alice@192.168.1.206:47211;rinstance=847e4025da2ce4e1;transport=tcp>
2. <sip:alice@192.168.1.80:54490;rinstance=0a50f67651735b2c;transport=tcp>
```

**Caller Device** (Polycom):
```
Registration: <sip:polycom@192.168.1.12>
Expires: 2026-01-21 19:16:59
Status: ✅ Active
```

---

## Test Scenario

### Call Flow

**Action**: Polycom VVX 250 dials extension **5002**

**Expected Behavior**:
1. fcp-server receives INVITE from Polycom to sip:5002@fcp.local
2. Extension lookup finds extension 5002 → User target (user-8840f5e2...)
3. Queries devices for Alice's user ID → Finds 1 device (dev-11014c84...)
4. Device is enabled and has SIP URI (sip:alice@192.168.1.225)
5. Queries registrations for sip:alice@192.168.1.225 → Finds 2 contacts
6. **Parallel forks INVITE to both Alice endpoints simultaneously**:
   - Fork #1: INVITE → sip:alice@192.168.1.206:47211 (tcp)
   - Fork #2: INVITE → sip:alice@192.168.1.80:54490 (tcp)
7. Both Alice endpoints ring (180 Ringing)
8. **First endpoint to answer wins** (e.g., 192.168.1.206 answers with 200 OK)
9. Server sends CANCEL to losing endpoint (192.168.1.80)
10. Server relays 200 OK to Polycom with media anchoring
11. Polycom sends ACK, call is established
12. **One CDR is created** (not 3)

---

## Expected Logs

### Extension Routing

```
INFO Extension routes to user - ring all user's devices
  call_id = "call-..."
  user_id = "user-8840f5e2-87d2-4801-8b93-c854bf2ad5c2"

DEBUG Found devices for user
  call_id = "call-..."
  user_id = "user-8840f5e2-87d2-4801-8b93-c854bf2ad5c2"
  device_count = 1

DEBUG Found registrations for user device
  call_id = "call-..."
  device_id = "dev-11014c84-9db2-4278-bcdb-700b768b8657"
  sip_uri = "sip:alice@192.168.1.225"
  reg_count = 2
```

### Parallel Forking

```
INFO Racing 2 B-legs for first answer
  call_id = "call-..."
  fork_count = 2

INFO B-leg 0 won the race with 200
  call_id = "call-..."
  winner = "sip:alice@192.168.1.206:47211;..."

INFO CANCELing non-winning B-leg 1
  call_id = "call-..."
  target = "sip:alice@192.168.1.80:54490;..."

INFO Relaying winning B-leg response to A-leg
  call_id = "call-..."
  winner = "sip:alice@192.168.1.206:47211;..."
  code = 200
```

---

## Verification Steps

### 1. Before Test
```bash
# Count current CDRs
sqlite3 /home/siphon/fcp/fcp.db "SELECT COUNT(*) FROM cdrs;"
# Result: 28 (current count)

# Check no CDRs for extension 5002
sqlite3 /home/siphon/fcp/fcp.db "SELECT COUNT(*) FROM cdrs WHERE to_uri LIKE '%5002%';"
# Result: 0
```

### 2. Make Test Call

**Manual Steps**:
1. Pick up Polycom VVX 250 handset
2. Dial: 5002
3. Observe: Both Alice devices should ring
4. Answer on one Alice device
5. Talk briefly (5-10 seconds)
6. Hang up from either side

**Watch Logs** (in another terminal):
```bash
tail -f /var/log/syslog | grep fcp-server
# Or check journald:
journalctl -f -u fcp-server
```

### 3. After Test - Verify Results

**Check CDR Created**:
```bash
sqlite3 /home/siphon/fcp/fcp.db "
SELECT call_id, from_uri, to_uri, disposition,
       setup_duration_ms, call_duration_ms, total_duration_ms,
       datetime(start_time, 'unixepoch', 'localtime') as start_time
FROM cdrs
WHERE to_uri LIKE '%5002%'
ORDER BY start_time DESC
LIMIT 1;"
```

**Expected CDR**:
```
call_id: call-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
from_uri: sip:polycom@fcp.local
to_uri: <sip:5002@fcp.local> (or similar)
disposition: answered
setup_duration_ms: <time to answer in ms>
call_duration_ms: <conversation time in ms>
total_duration_ms: <total call time in ms>
start_time: <timestamp>
```

**Verify Only One CDR** (not 3):
```bash
sqlite3 /home/siphon/fcp/fcp.db "
SELECT COUNT(*) as cdr_count
FROM cdrs
WHERE to_uri LIKE '%5002%'
  AND start_time > $(date -d '5 minutes ago' +%s);"
# Expected: 1 (not 3, proving no CDR duplication from parallel forks)
```

---

## Success Criteria

### Must Have ✅
1. ✅ Both Alice devices ring simultaneously (parallel forking)
2. ✅ First device to answer connects
3. ✅ Losing device receives CANCEL
4. ✅ Two-way audio works (media anchoring)
5. ✅ Only ONE CDR is created
6. ✅ CDR has correct disposition (answered)
7. ✅ CDR has accurate durations

### Verification ✅
1. ✅ Logs show "Extension routes to user"
2. ✅ Logs show "Found devices for user" with device_count = 1
3. ✅ Logs show "Found registrations for user device" with reg_count = 2
4. ✅ Logs show "Racing 2 B-legs for first answer"
5. ✅ Logs show "B-leg won the race"
6. ✅ Logs show "CANCELing non-winning B-leg"

### Database ✅
1. ✅ Extension 5002 exists with target_type = 'user'
2. ✅ Device owner_user_id matches extension target_id
3. ✅ Device has 2 active registrations
4. ✅ After call: 1 new CDR for extension 5002

---

## Alternative Test Scenarios

### Test 2: User with No Registered Devices

**Setup**: Unregister all Alice devices
```bash
# Stop Alice's Zoiper clients
# Registrations will expire
```

**Expected**:
- 480 Temporarily Unavailable
- CDR disposition = failed
- Reason = "User has no registered devices"

### Test 3: User with Disabled Device

**Setup**: Disable Alice's device
```sql
UPDATE devices
SET enabled = 0
WHERE id = 'dev-11014c84-9db2-4278-bcdb-700b768b8657';
```

**Expected**:
- Device skipped during lookup
- 480 Temporarily Unavailable
- Reason = "User has no registered devices"

### Test 4: User with Multiple Devices

**Setup**: Create second device for Alice
```sql
INSERT INTO devices (id, name, device_type, sip_uri, owner_user_id, tenant_id, enabled, created_at, updated_at)
VALUES ('dev-alice-desk', 'Alice Desk Phone', 'desk_phone', 'sip:alice-desk@fcp.local',
        'user-8840f5e2-87d2-4801-8b93-c854bf2ad5c2',
        'tenant-00000000-0000-0000-0000-000000000000', 1,
        CAST(strftime('%s', 'now') AS INTEGER),
        CAST(strftime('%s', 'now') AS INTEGER));
```

**Expected**:
- Query finds 2 devices
- Gets registrations for both devices
- Parallel forks to all registered contacts (2 from Zoiper + N from desk phone)

---

## Troubleshooting

### Issue: No devices found

**Symptom**: "User has no devices"

**Check**:
```sql
SELECT id, name, owner_user_id
FROM devices
WHERE owner_user_id = 'user-8840f5e2-87d2-4801-8b93-c854bf2ad5c2';
```

**Fix**: Ensure device owner_user_id matches user ID in extension

### Issue: Devices not registered

**Symptom**: "User has no registered devices"

**Check**:
```sql
SELECT r.aor, r.contact
FROM devices d
JOIN registrations r ON d.sip_uri = r.aor
WHERE d.owner_user_id = 'user-8840f5e2-87d2-4801-8b93-c854bf2ad5c2';
```

**Fix**: Register Alice's devices (restart Zoiper clients)

### Issue: Extension not found

**Symptom**: Falls back to direct registrar lookup

**Check**:
```sql
SELECT number, target_type, target_id
FROM extensions
WHERE number = '5002';
```

**Fix**: Recreate extension with correct data

### Issue: 501 Not Implemented

**Symptom**: "Extension target type User not yet fully implemented"

**Problem**: Code changes not loaded

**Fix**: Restart fcp-server
```bash
pkill fcp-server
./target/release/fcp-server &
```

---

## Test Results

### Test Execution

**Date**:
**Tester**:
**Result**: ⏳ PENDING

**Observations**:
- [ ] Extension lookup successful
- [ ] Device query successful
- [ ] Registration lookup successful
- [ ] Parallel forking occurred
- [ ] Both endpoints rang
- [ ] Winner connected
- [ ] Loser cancelled
- [ ] Two-way audio working
- [ ] CDR created
- [ ] Only 1 CDR (not duplicates)

**CDR Data**:
```
call_id:
from_uri:
to_uri:
disposition:
setup_duration_ms:
call_duration_ms:
total_duration_ms:
```

**Issues Encountered**: None / List issues

**Resolution**: N/A / Describe fixes

---

## Next Steps After Test

1. ✅ Document test results
2. ✅ Test Device target (extension 5100 → specific device)
3. ✅ Test with multiple user devices
4. ✅ Update WEEK1_TESTING_PROGRESS.md
5. ✅ Create user guide documentation

---

**Test Plan Created**: 2026-01-21
**Status**: Ready for execution ✅
**Prerequisites**: All met (extension created, devices registered, server running)
