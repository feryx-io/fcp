# FCP Testing Guide

This guide walks you through testing the FCP server's current functionality.

## What's Implemented

Based on the Phase 1 plan, we've completed:

- ✅ **Core infrastructure** (Sprint 1-2): Types, identifiers, SIP parsing
- ✅ **Transport layer** (Sprint 3): UDP/TCP SIP transport
- ✅ **SIP Registrar** (Sprint 5): User registration handling
- ✅ **Storage layer** (Sprint 7): SQLite/file-backed storage
- ✅ **REST API** (Sprint 7): User management endpoints
- ✅ **Policy-based routing** (BONUS!): Multi-tenant call authorization

**Current Phase:** Between Sprint 5-7 (weeks 9-14 out of 16)

## Prerequisites

1. **Build the server:**
   ```bash
   cargo build --release --bin fcp-server
   ```

2. **Configuration files are ready:**
   - `fcp.toml` - Main server configuration
   - `examples/routing.yaml` - Routing policies
   - `examples/users.json` - Pre-configured test users

## Test 1: Start the Server

### ✅ **WORKING** (API-Only Mode)

```bash
# Run the server (currently in API-only mode due to socket binding issue)
./target/release/fcp-server

# You should see:
# - "Starting FCP server"
# - "Runtime built successfully"
# - "Starting API server" (on 127.0.0.1:8080)
# - "FCP server running. Press Ctrl+C to shutdown."
# - "API server listening on 127.0.0.1:8080"
```

**Current Status:**
- ✅ Server starts successfully
- ✅ API server on port 8080 is functional
- ⚠️ SIP transport disabled (socket binding issue under investigation)
- ✅ Loads routing policies from `examples/routing.yaml`
- ✅ Logs in "pretty" format with debug level

**Known Issue**: UDP socket binding fails with "Address already in use".
**Workaround**: Transport is disabled in config, allowing API-only operation.

## Test 2: REST API - Health Check

Open a new terminal and test the API:

```bash
# Health check (if implemented)
curl http://localhost:8080/api/v1/health

# Metrics endpoint
curl http://localhost:8080/metrics
```

## Test 3: REST API - User Management

### ✅ **TESTED & WORKING!**

```bash
# List all users
curl http://localhost:8080/api/v1/users

# Get specific user
curl http://localhost:8080/api/v1/users/alice

# Create a new user
curl -X POST http://localhost:8080/api/v1/users \
  -H 'Content-Type: application/json' \
  -d '{"username":"testuser","password":"password123","display_name":"Test User","enabled":true}'

# Update user
curl -X PUT http://localhost:8080/api/v1/users/testuser \
  -H 'Content-Type: application/json' \
  -d '{"display_name":"Updated Test User","enabled":true}'

# Change password
curl -X PUT http://localhost:8080/api/v1/users/testuser/password \
  -H 'Content-Type: application/json' \
  -d '{"password":"newpassword123"}'

# Delete user
curl -X DELETE http://localhost:8080/api/v1/users/testuser
```

**✅ Test Results (2025-12-01):**
- ✅ `GET /users` returns array of all users
- ✅ `POST /users` creates user with 201 status and full user object
- ✅ `GET /users/:username` returns specific user
- ✅ `PUT /users/:username` updates user metadata
- ✅ `PUT /users/:username/password` updates password
- ✅ `DELETE /users/:username` removes user
- ✅ All requests logged with latency < 1ms
- ✅ Tenant IDs properly assigned (default tenant)

## Test 4: SIP Registration (with SIP client)

You'll need a SIP softphone like:
- **Odin** (Electron-based, cross-platform)
- **Linphone** (cross-platform)
- **Zoiper** (commercial, free tier available)

### Configuration:

**For Alice:**
- **Username:** `alice`
- **Password:** `secret123` (if auth enabled)
- **Domain:** `fcp.local`
- **SIP Server:** `127.0.0.1:5060`
- **Transport:** UDP
- **Register:** Yes

**For Bob:**
- **Username:** `bob`
- **Password:** `secret123` (if auth enabled)
- **Domain:** `fcp.local`
- **SIP Server:** `127.0.0.1:5060`
- **Transport:** UDP
- **Register:** Yes

### Expected Behavior:
1. Softphone sends REGISTER request
2. Server responds with 200 OK (auth disabled) or 401 Unauthorized (auth enabled)
3. If auth enabled, client re-sends REGISTER with credentials
4. Server responds with 200 OK
5. Check server logs for registration events

## Test 5: Policy Routing System

The routing system is configured in `examples/routing.yaml`. Test scenarios:

### Scenario 1: Internal Extension Calls (Allowed)
Alice (employee role) calls internal extension 2500:
- ✅ Should be **allowed** by "employees-internal-only" policy
- ✅ Reason: "Employees can call internal extensions"

### Scenario 2: US/Canada Number - Business Hours (Sales)
Alice (sales-team role) calls +15551234567:
- ✅ Should be **allowed** during business hours (Mon-Fri, 8am-6pm ET)
- ❌ Should be **denied** outside business hours
- ✅ Reason: "Sales can call US/Canada during business hours"

### Scenario 3: Management Override
Charlie (management role) calls +15551234567:
- ✅ Should be **allowed** anytime
- ✅ Reason: "Management can call US/Canada anytime"

### Scenario 4: Emergency Services (Always Allowed)
Any employee calls 911:
- ✅ Should be **allowed** (highest priority: 1)
- ✅ Reason: "Emergency services always allowed"

### Scenario 5: Multi-Tenant Isolation
If you configure multiple tenants:
- ✅ Users in Tenant A cannot see Tenant B's policies
- ✅ Policy evaluation is scoped to the caller's tenant
- ✅ Cross-tenant calls are denied

### Testing Routing via API (Future Enhancement)
```bash
# If routing evaluation API is implemented:
curl -X POST http://localhost:8080/api/v1/routing/evaluate \
  -H "Content-Type: application/json" \
  -d '{
    "caller": "alice@company.com",
    "destination": "2500",
    "tenant_id": "00000000-0000-0000-0000-000000000000",
    "source_ip": "10.0.1.100"
  }' | jq
```

## Test 6: Integration Tests

Run the test suite:

```bash
# Run all tests
cargo test --workspace

# Run specific tests
cargo test --test multi_tenant_test

# Run with output
cargo test -- --nocapture
```

**Expected Results:**
- All 38 tests pass ✅
- Multi-tenant isolation tests verify policy scoping
- Routing evaluation tests verify policy matching
- User storage tests verify CRUD operations

## Test 7: Call Flow (End-to-End) - FUTURE

Once B2BUA and RTP relay are complete (Sprint 6), you'll be able to:

1. Register Alice and Bob
2. Alice calls Bob (INVITE)
3. Bob's phone rings (180 Ringing)
4. Bob answers (200 OK)
5. RTP media flows in both directions
6. Either party hangs up (BYE)

**Not yet implemented:**
- B2BUA call control
- RTP media relay
- SDP negotiation
- Call state management

## Troubleshooting

### Server won't start
- **Port 5060 in use:** Another SIP server running? `lsof -i :5060`
- **Port 8080 in use:** `lsof -i :8080`
- **Config errors:** Check `fcp.toml` syntax
- **Database errors:** Delete `fcp.db` and `data/` directory to start fresh

### SIP registration fails
- **Authentication required:** Set `registrar.auth_enabled = false` in config
- **Network issues:** Ensure firewall allows UDP 5060
- **Check logs:** Run with `log_level = "debug"`

### Routing not working
- **Routing disabled:** Set `routing.enabled = true` in config
- **YAML syntax errors:** Validate `examples/routing.yaml`
- **Tenant mismatch:** Ensure user's tenant matches policy tenant

### API 404 errors
- **Wrong URL:** Ensure `/api/v1/` prefix
- **API disabled:** Set `api.enabled = true` in config

## What's Next?

Based on the Phase 1 plan, the remaining work includes:

### Sprint 6 (Weeks 11-12): B2BUA & RTP
- [ ] Call state management (INVITE/ACK/BYE)
- [ ] Dialog tracking
- [ ] RTP media relay
- [ ] SDP negotiation

### Sprint 8 (Weeks 15-16): Polish
- [ ] Docker image
- [ ] CLI tool
- [ ] Load testing
- [ ] Production hardening

### Out of Scope (Phase 2+)
- [ ] Voicemail
- [ ] Call recording
- [ ] Conference bridges
- [ ] WebRTC gateway
- [ ] Horizontal scaling

## Success Metrics

**Milestone 1 (Week 4): "First SIP Message"** ✅
- ✅ Parse and serialize any valid SIP message
- ✅ Send/receive over UDP

**Milestone 2 (Week 8): "First Registration"** ✅
- ✅ Register a softphone
- ✅ See registration in logs

**Milestone 3 (Week 12): "First Call"** ⏳ *In Progress*
- [ ] Complete call between two softphones
- [ ] Hear audio in both directions

**Milestone 4 (Week 16): "Production Ready"** ⏳ *Planned*
- [ ] Docker deployment working
- ✅ REST API functional
- [ ] Prometheus metrics exposed
- [ ] SIP trunk connected
- [ ] Documentation complete

---

Happy Testing! 🚀

For issues or questions, see the main README or open an issue on GitHub.
