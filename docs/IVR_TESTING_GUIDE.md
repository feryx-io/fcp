# IVR Audio Playback Testing Guide

This guide walks through setting up and testing the Flow Engine with audio playback.

## Prerequisites

- FCP server compiled and ready to run
- PostgreSQL/MySQL/SQLite database configured (for user registration)
- Audio file: `welcome.wav` (created and ready)
- Two SIP clients registered (alice and bob are created by default)

## Quick Start Summary

1. Place audio file at `/var/lib/fcp/audio/welcome.wav`
2. Create API endpoint to configure flow and routing
3. Start FCP server
4. Configure flow via API
5. Call the DID number to test

## Current Implementation Note

⚠️ **Important**: Flows are currently stored in-memory (not database). This means:
- Flows must be configured via API after server starts
- Flows are lost on server restart
- This is an MVP limitation - database storage will be added in Phase 2

## Step 1: Prepare the Audio File

Place your `welcome.wav` file in a known location:

```bash
# Create audio directory
mkdir -p /var/lib/fcp/audio

# Copy the welcome file
cp welcome.wav /var/lib/fcp/audio/welcome.wav

# Set permissions (make sure fcp-server can read it)
chmod 644 /var/lib/fcp/audio/welcome.wav
```

## Step 2: Create the IVR Flow Definition

Create a flow JSON file that plays the welcome message:

```bash
cat > /tmp/welcome-ivr-flow.json <<'EOF'
{
  "id": "welcome-ivr",
  "name": "Welcome IVR",
  "description": "Simple IVR that plays a welcome message",
  "version": 1,
  "enabled": true,
  "initial_node": "answer",
  "nodes": {
    "answer": {
      "id": "answer",
      "name": "Answer Call",
      "type": "answer",
      "next": "play_welcome"
    },
    "play_welcome": {
      "id": "play_welcome",
      "name": "Play Welcome Message",
      "type": "play",
      "url": "/var/lib/fcp/audio/welcome.wav",
      "loop_count": 0,
      "next": "hangup"
    },
    "hangup": {
      "id": "hangup",
      "name": "Hangup",
      "type": "hangup",
      "reason": "normal"
    }
  },
  "variables": {}
}
EOF
```

## Step 3: Insert the Flow into the Database

### Option A: Using SQLite (default)

```bash
# Connect to SQLite database
sqlite3 /var/lib/fcp/fcp.db

# Insert the flow
INSERT INTO flows (
  id,
  name,
  definition,
  enabled,
  version,
  created_at,
  updated_at
) VALUES (
  'welcome-ivr',
  'Welcome IVR',
  readfile('/tmp/welcome-ivr-flow.json'),
  1,
  1,
  datetime('now'),
  datetime('now')
);

# Verify insertion
SELECT id, name, enabled FROM flows;

# Exit
.quit
```

### Option B: Using PostgreSQL

```bash
# Read the JSON file content
FLOW_JSON=$(cat /tmp/welcome-ivr-flow.json)

# Connect to PostgreSQL and insert
psql -h localhost -U fcp_user -d fcp_db <<EOF
INSERT INTO flows (
  id,
  name,
  definition,
  enabled,
  version,
  created_at,
  updated_at
) VALUES (
  'welcome-ivr',
  'Welcome IVR',
  '${FLOW_JSON}'::jsonb,
  true,
  1,
  NOW(),
  NOW()
);

-- Verify
SELECT id, name, enabled FROM flows;
EOF
```

### Option C: Using MySQL

```bash
mysql -u fcp_user -p fcp_db <<'EOF'
INSERT INTO flows (
  id,
  name,
  definition,
  enabled,
  version,
  created_at,
  updated_at
) VALUES (
  'welcome-ivr',
  'Welcome IVR',
  LOAD_FILE('/tmp/welcome-ivr-flow.json'),
  1,
  1,
  NOW(),
  NOW()
);

-- Verify
SELECT id, name, enabled FROM flows;
EOF
```

## Step 4: Configure DID Routing

We need to map a phone number (DID) to the flow. This is currently done at runtime.

First, let's check what DIDs/users are available:

```bash
# Check registered users/DIDs
sqlite3 /var/lib/fcp/fcp.db "SELECT username, display_name FROM users;"
```

Let's say we want calls to `+15551234567` to trigger the IVR.

## Step 5: Update Runtime to Enable Flow Handler

We need to check if the runtime is properly wiring up the FlowCallHandler with MediaManager.

Let me check the current runtime builder:

```bash
# Check runtime builder configuration
cat crates/fcp-runtime/src/builder.rs
```

**Note**: We'll need to update this in the next step if FlowCallHandler isn't properly initialized.

## Step 6: Create a Startup Script with DID Routing

Create a script to start FCP with DID routing configured:

```bash
cat > /tmp/start-fcp-with-ivr.sh <<'EOF'
#!/bin/bash

# Start FCP server
# The DID routing will need to be configured via API or code

# For now, we'll need to add this programmatically
# See Step 7 for the code changes needed

cargo run --release -p fcp-server -- \
  --config /etc/fcp/fcp.toml \
  --log-level info
EOF

chmod +x /tmp/start-fcp-with-ivr.sh
```

## Step 7: Runtime Integration (Code Changes Needed)

Currently, the FlowCallHandler needs to be initialized in the runtime. Let's check if this is already done:

```bash
# Search for FlowCallHandler initialization
rg "FlowCallHandler" crates/fcp-runtime/src/
```

If not found, we need to:

1. Add FlowCallHandler initialization in `RuntimeBuilder`
2. Add DID routing configuration (either via config file or API)
3. Pass MediaManager to FlowCallHandler
4. Pass FlowCallHandler to SipMessageHandler

**I'll check this in the next step and create the necessary changes.**

## Step 8: Test Call Flow

Once everything is configured:

### 8.1 Start FCP Server

```bash
# Start with debug logging to see flow execution
RUST_LOG=debug ./target/release/fcp-server
```

### 8.2 Watch the Logs

In another terminal:

```bash
# Follow logs
tail -f /var/log/fcp/fcp-server.log | grep -E "(flow|playback|Playing)"
```

### 8.3 Make Test Call

From your SIP client (e.g., Linphone, Zoiper):
1. Dial the IVR number: `+15551234567`
2. You should hear the welcome.wav file playing

### 8.4 Expected Log Output

You should see logs like:

```
INFO  Starting flow execution for call call_id=call-xxx flow_id=welcome-ivr
INFO  Executing flow node call_id=call-xxx node_id=answer
INFO  Executing flow node call_id=call-xxx node_id=play_welcome
INFO  Playing audio file call_id=call-xxx url=/var/lib/fcp/audio/welcome.wav
INFO  Audio playback started call_id=call-xxx playback_id=playback-1
INFO  Executing flow node call_id=call-xxx node_id=hangup
```

## Step 9: Troubleshooting

### Flow Not Found

```bash
# Check if flow was inserted
sqlite3 /var/lib/fcp/fcp.db "SELECT * FROM flows WHERE id='welcome-ivr';"
```

### Audio File Not Found

```bash
# Check file exists and is readable
ls -la /var/lib/fcp/audio/welcome.wav
sudo -u fcp cat /var/lib/fcp/audio/welcome.wav > /dev/null && echo "Readable"
```

### No Audio Playing

Check logs for:
- "Failed to open audio file"
- "Failed to start playback"
- Media session errors

### Call Not Routing to Flow

Check:
- DID routing is configured (see Step 4)
- FlowCallHandler is properly initialized
- SipMessageHandler has flow_handler reference

## Step 10: API-Based DID Routing (Future)

Eventually, we'll have REST API endpoints to configure routing:

```bash
# Create DID routing via API (future)
curl -X POST http://localhost:8080/api/v1/flow-routes \
  -H "Content-Type: application/json" \
  -d '{
    "did": "+15551234567",
    "flow_id": "welcome-ivr",
    "enabled": true
  }'

# List routes
curl http://localhost:8080/api/v1/flow-routes

# Update route
curl -X PUT http://localhost:8080/api/v1/flow-routes/+15551234567 \
  -H "Content-Type: application/json" \
  -d '{
    "flow_id": "different-flow"
  }'

# Delete route
curl -X DELETE http://localhost:8080/api/v1/flow-routes/+15551234567
```

## Advanced: Multi-Stage IVR Example

Here's a more complex IVR with DTMF gathering:

```json
{
  "id": "sales-ivr",
  "name": "Sales IVR",
  "version": 1,
  "enabled": true,
  "initial_node": "answer",
  "nodes": {
    "answer": {
      "id": "answer",
      "type": "answer",
      "next": "main_menu"
    },
    "main_menu": {
      "id": "main_menu",
      "type": "gather",
      "prompt": {
        "type": "play",
        "url": "/var/lib/fcp/audio/main-menu.wav"
      },
      "num_digits": 1,
      "timeout": 5,
      "variable": "menu_choice",
      "transitions": {
        "1": "sales",
        "2": "support",
        "9": "operator"
      },
      "default": "invalid_choice"
    },
    "sales": {
      "id": "sales",
      "type": "play",
      "url": "/var/lib/fcp/audio/sales-info.wav",
      "next": "hangup"
    },
    "support": {
      "id": "support",
      "type": "play",
      "url": "/var/lib/fcp/audio/support-info.wav",
      "next": "hangup"
    },
    "operator": {
      "id": "operator",
      "type": "dial",
      "destination": "sip:operator@pbx.local",
      "timeout": 30,
      "next": "hangup"
    },
    "invalid_choice": {
      "id": "invalid_choice",
      "type": "play",
      "url": "/var/lib/fcp/audio/invalid.wav",
      "next": "main_menu"
    },
    "hangup": {
      "id": "hangup",
      "type": "hangup"
    }
  }
}
```

## Next Steps

The main missing piece is runtime initialization. Let me check and update the runtime builder to properly wire everything together.
