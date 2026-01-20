# Quick IVR Audio Playback Test

## Overview

This guide provides the fastest way to test audio playback in a flow. Since flows are currently in-memory only, we'll configure everything programmatically.

## Step-by-Step Instructions

### Step 1: Prepare Your Audio File

```bash
# Create audio directory
sudo mkdir -p /var/lib/fcp/audio

# Copy your welcome.wav file
sudo cp welcome.wav /var/lib/fcp/audio/welcome.wav

# Make it readable by the fcp-server process
sudo chmod 644 /var/lib/fcp/audio/welcome.wav
```

### Step 2: Create a Test Configuration Script

Create `/tmp/configure-ivr.sh`:

```bash
#!/bin/bash

# This script configures the flow and DID routing via REST API
# Run this AFTER fcp-server has started

API_BASE="http://localhost:8080/api/v1"

# 1. Create a simple welcome flow
echo "Creating welcome flow..."
FLOW_JSON=$(cat <<'EOF'
{
  "id": "welcome-ivr",
  "name": "Welcome IVR",
  "description": "Plays a welcome message",
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
      "name": "Play Welcome",
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
)

# Note: These API endpoints need to be implemented
# For now, this is the intended API design

curl -X POST "${API_BASE}/flows" \
  -H "Content-Type: application/json" \
  -d "${FLOW_JSON}"

# 2. Configure DID routing (map phone number to flow)
echo "Configuring DID routing..."
curl -X POST "${API_BASE}/flow-routes" \
  -H "Content-Type: application/json" \
  -d '{
    "did": "+15551234567",
    "flow_id": "welcome-ivr",
    "enabled": true
  }'

echo "Configuration complete!"
echo "Now call +15551234567 from your SIP client"
```

Make it executable:

```bash
chmod +x /tmp/configure-ivr.sh
```

### Step 3: Start FCP Server

```bash
# Build the server (if not already built)
cargo build --release -p fcp-server

# Start the server
./target/release/fcp-server
```

### Step 4: Configure Flow (Temporary Workaround)

Since the REST API for flow configuration is not yet implemented, here's the workaround:

**Option A: Add to Runtime Directly (Code Change)**

Edit `crates/fcp-runtime/src/builder.rs` and add this after line 431:

```rust
// Load test flow for IVR testing
if cfg!(debug_assertions) {
    info!("Loading test IVR flow");
    load_test_ivr_flow(&flow_store, &flow_handler).await?;
}
```

Then add this function before the closing brace:

```rust
async fn load_test_ivr_flow(
    store: &Arc<fcp_flow::InMemoryFlowStore>,
    handler: &Arc<fcp_sip::flow_handler::FlowCallHandler>,
) -> Result<()> {
    use fcp_flow::{FlowDefinition, FlowNode, NodeType};
    use std::collections::HashMap;

    // Create nodes
    let mut nodes = HashMap::new();

    nodes.insert(
        "answer".to_string(),
        FlowNode {
            id: "answer".to_string(),
            name: Some("Answer Call".to_string()),
            node_type: NodeType::Answer {
                next: "play_welcome".to_string(),
            },
        },
    );

    nodes.insert(
        "play_welcome".to_string(),
        FlowNode {
            id: "play_welcome".to_string(),
            name: Some("Play Welcome".to_string()),
            node_type: NodeType::Play {
                url: "/var/lib/fcp/audio/welcome.wav".to_string(),
                loop_count: 0,
                next: "hangup".to_string(),
                on_error: None,
            },
        },
    );

    nodes.insert(
        "hangup".to_string(),
        FlowNode {
            id: "hangup".to_string(),
            name: Some("Hangup".to_string()),
            node_type: NodeType::Hangup { reason: None },
        },
    );

    // Create flow definition
    let flow = FlowDefinition {
        id: "welcome-ivr".to_string(),
        name: "Welcome IVR".to_string(),
        description: Some("Plays welcome message".to_string()),
        version: 1,
        initial_node: "answer".to_string(),
        nodes,
        variables: HashMap::new(),
        tenant_id: None,
        enabled: true,
        webhook_url: None,
        created_at: None,
        updated_at: None,
    };

    // Insert flow
    store.insert_flow(flow).await?;

    // Configure DID routing: +15551234567 -> welcome-ivr
    handler.add_route("+15551234567".to_string(), "welcome-ivr".to_string()).await;

    info!("Test IVR flow loaded: call +15551234567 to test");

    Ok(())
}
```

### Step 5: Test the IVR

1. **Start FCP Server** (with debug build so test flow loads):
   ```bash
   cargo run -p fcp-server
   ```

2. **Watch the Logs**:
   ```bash
   # In another terminal
   tail -f fcp-server.log | grep -E "(flow|playback|Playing)"
   ```

3. **Make a Test Call**:
   - From your SIP client (alice or bob)
   - Dial: `+15551234567` (or whatever DID you configured)
   - You should hear the welcome.wav file playing

4. **Expected Log Output**:
   ```
   INFO  Test IVR flow loaded: call +15551234567 to test
   INFO  Starting flow execution for call call_id=call-xxx flow_id=welcome-ivr
   INFO  Executing flow node call_id=call-xxx node_id=answer
   INFO  Executing flow node call_id=call-xxx node_id=play_welcome
   INFO  Playing audio file call_id=call-xxx url=/var/lib/fcp/audio/welcome.wav
   INFO  Audio playback started call_id=call-xxx playback_id=playback-1
   ```

## Troubleshooting

### No Audio Playing

1. **Check file exists and is readable**:
   ```bash
   ls -la /var/lib/fcp/audio/welcome.wav
   stat /var/lib/fcp/audio/welcome.wav
   ```

2. **Check logs for errors**:
   ```bash
   grep -i "error\|failed" fcp-server.log | grep -i audio
   ```

3. **Verify media manager initialized**:
   ```bash
   grep "Media manager" fcp-server.log
   ```

### Flow Not Triggering

1. **Check DID routing was configured**:
   ```bash
   grep "flow routing rule" fcp-server.log
   grep "+15551234567" fcp-server.log
   ```

2. **Verify flow was loaded**:
   ```bash
   grep "welcome-ivr" fcp-server.log
   ```

3. **Check call is being received**:
   ```bash
   grep "INVITE" fcp-server.log | tail -5
   ```

### Call Connects But No Audio

This is expected for now because:
- PlaybackManager starts playback
- But actual RTP injection is not yet fully wired
- The logs will show "Audio playback started" but you might not hear anything yet

**Next Steps for Full RTP Integration**:
- Wire PlaybackManager to ForwardingEngine
- Ensure media session is anchored on FCP server
- Test end-to-end with actual audio

## Alternative: Manual Testing Without Code Changes

If you don't want to modify the runtime code, you can:

1. Run fcp-server normally
2. Use a REST client (curl, Postman) to configure flows via API (once implemented)
3. Or wait for the flow management UI (Phase 2)

For now, the code change approach above is the fastest way to test.

## Next Steps

Once you verify the flow triggers and logs show playback starting:

1. Implement full RTP injection (connect PlaybackManager to ForwardingEngine)
2. Add database-backed FlowStore
3. Implement REST API for flow management
4. Add flow editor UI
5. Test with multiple concurrent calls

## Quick Reference

**Default Test Users**:
- Username: `alice`, Password: `Secret123!`
- Username: `bob`, Password: `Secret456!`

**Test DID**: `+15551234567`

**Flow ID**: `welcome-ivr`

**Audio File Path**: `/var/lib/fcp/audio/welcome.wav`

**Expected Behavior**:
1. Call +15551234567
2. Flow starts execution
3. Call is answered
4. Audio file plays (or attempts to play)
5. Call hangs up

**Log Keywords to Watch**:
- "flow routing rule" - DID mapping configured
- "Starting flow execution" - Flow started
- "Playing audio file" - Play node executing
- "Audio playback started" - Playback initiated
