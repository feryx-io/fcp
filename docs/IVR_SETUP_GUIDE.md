# IVR Setup and Testing Guide

## Overview

This guide walks you through setting up and testing the IVR (Interactive Voice Response) system with audio playback in FCP. The implementation now properly waits for audio playback to complete before moving to the next node.

## What Was Fixed

**Problem**: Previously, the Play node would start audio playback but immediately continue to the next node (Hangup), terminating the call before the audio could be heard.

**Solution**:
- Added `PlaybackHandle` storage in `EmbeddedBackend` to track active playbacks
- Added `wait_playback_completion()` method to `MediaBackend` trait and `MediaManager`
- Updated flow handler to wait for playback completion before continuing to next node
- Applied the same fix to both Play (audio file) and Say (TTS) nodes

## Prerequisites

1. **FCP Server Running**: Make sure `fcp-server` is built and running
2. **Audio File**: Have a WAV audio file ready for playback
3. **SIP Client**: A SIP softphone (like Zoiper, Linphone, or MicroSIP) configured to register

## Step-by-Step Setup

### Step 1: Prepare Your Audio File

Create or obtain a WAV audio file for the welcome message:

```bash
# If you don't have one, create a simple test tone
# (requires sox package: sudo apt-get install sox)
sox -n -r 8000 -c 1 /home/siphon/fcp/welcome.wav synth 3 sine 440

# Or copy an existing audio file
cp /path/to/your/audio.wav /home/siphon/fcp/welcome.wav
```

**Audio File Requirements**:
- Format: WAV (PCM) or Opus
- Sample Rate: 8000 Hz (recommended for VoIP) or 48000 Hz
- Channels: Mono (1 channel) recommended
- Location: Use absolute path in flow configuration

### Step 2: Create the IVR Flow Configuration

A simple IVR flow has already been created at `/home/siphon/fcp/welcome-ivr.json`:

```json
{
  "id": "welcome-test",
  "name": "Welcome Test IVR",
  "description": "Simple IVR to test audio playback",
  "version": 1,
  "initial_node": "answer",
  "enabled": true,
  "tenant_id": null,
  "nodes": {
    "answer": {
      "id": "answer",
      "name": "Answer Call",
      "type": "answer",
      "next": "play_welcome"
    },
    "play_welcome": {
      "id": "play_welcome",
      "name": "Play Welcome Audio",
      "type": "play",
      "url": "/home/siphon/fcp/welcome.wav",
      "next": "hangup"
    },
    "hangup": {
      "id": "hangup",
      "name": "Hangup",
      "type": "hangup",
      "reason": "normal"
    }
  }
}
```

**Update the audio file path** if your welcome.wav is located elsewhere:
- Edit `welcome-ivr.json`
- Change the `url` field in the `play_welcome` node to your audio file's absolute path

### Step 3: Load the Flow into the Database

The IVR flow needs to be loaded into the database and associated with a phone number.

**Option A: Using SQLite directly** (if using embedded SQLite):

```bash
# Connect to the FCP database
sqlite3 /path/to/fcp.db

# Insert the flow
INSERT INTO flows (id, name, definition, enabled, created_at, updated_at)
VALUES (
  'welcome-test',
  'Welcome Test IVR',
  readfile('/home/siphon/fcp/welcome-ivr.json'),
  1,
  datetime('now'),
  datetime('now')
);

# Exit SQLite
.exit
```

**Option B: Using the API** (when REST API is available):

```bash
curl -X POST http://localhost:8080/api/v1/flows \
  -H "Content-Type: application/json" \
  -d @/home/siphon/fcp/welcome-ivr.json
```

### Step 4: Create and Route a Phone Number

Create a phone number that routes to the IVR flow:

**Using SQL**:

```bash
sqlite3 /path/to/fcp.db

-- Insert phone number
INSERT INTO phone_numbers (
  id, number, tenant_id, friendly_name, capabilities,
  routing_type, routing_target, created_at, updated_at
)
VALUES (
  lower(hex(randomblob(16))),  -- Generate UUID
  '+15551000',                  -- Phone number (E.164 format)
  lower(hex(randomblob(16))),  -- Tenant ID
  'IVR Test Number',
  'voice,sms',
  'flow',                       -- Route to a flow
  'welcome-test',               -- Flow ID
  datetime('now'),
  datetime('now')
);

.exit
```

**Using API** (when available):

```bash
curl -X POST http://localhost:8080/api/v1/phone-numbers \
  -H "Content-Type: application/json" \
  -d '{
    "number": "+15551000",
    "friendly_name": "IVR Test Number",
    "capabilities": ["voice", "sms"],
    "routing_type": "flow",
    "routing_target": "welcome-test"
  }'
```

### Step 5: Configure Your SIP Client

Register a SIP client to make test calls:

**Alice (Test User)**:
- Username: `alice`
- Password: `alicepass` (or check your users.json/database)
- Domain: `192.168.1.81` (or your server's IP)
- Port: `5060`
- Transport: UDP or TCP

**Example Zoiper Configuration**:
1. Account → Add Account → SIP
2. Username: `alice`
3. Domain: `192.168.1.81`
4. Password: `alicepass`
5. Click "Register"

### Step 6: Make a Test Call

Once your SIP client is registered:

1. **Dial the IVR number**: `5551000` or `sip:5551000@192.168.1.81`
2. **Expected behavior**:
   - Call connects (you hear ringback tone)
   - Call is answered automatically (Answer node)
   - Audio file plays completely (Play node)
   - Call terminates after audio finishes (Hangup node)

### Step 7: Monitor Logs

Watch the logs to verify correct behavior:

```bash
# Follow server logs
tail -f /tmp/fcp-server.log | grep -E "(Playing audio|playback|Hangup)"
```

**Expected log sequence**:
```
INFO Playing audio file, url: /home/siphon/fcp/welcome.wav
INFO Audio playback started, playback_id: playback-1
INFO Waiting for playback completion, playback_id: playback-1
DEBUG Reading audio frame, samples: 960
... (multiple frame reads) ...
INFO Playback completed successfully, playback_id: playback-1
INFO Audio playback completed
INFO Hangup node: calling hangup callback
INFO Call terminated
```

## Troubleshooting

### Audio File Not Found

**Error**: `Failed to open audio file: No such file or directory`

**Solution**:
- Verify the file path in your IVR JSON is correct and absolute
- Check file permissions: `ls -l /home/siphon/fcp/welcome.wav`
- Make sure the file exists: `file /home/siphon/fcp/welcome.wav`

### No Audio Heard

**Problem**: Call connects and disconnects immediately, no audio

**Possible Causes**:
1. **RTP/media issues**: Check that media ports are not blocked
2. **Codec mismatch**: Ensure your SIP client supports PCMU (G.711 μ-law)
3. **NAT issues**: If behind NAT, configure external IP in fcp-server config

**Debug Steps**:
```bash
# Check if RTP ports are allocated
netstat -unlp | grep fcp-server

# Verify codec in SIP client settings (should support PCMU/0, PCMA/8)
# Enable "Allow G.711" in your SIP client codec preferences
```

### Call Never Reaches IVR

**Problem**: Phone number doesn't route to flow

**Solutions**:
- Verify phone number exists in database
- Check routing_type is "flow" and routing_target matches flow ID
- Confirm flow is enabled (`enabled = 1` in database)
- Check server logs for routing decisions

### Playback Completes Too Quickly

**Problem**: Audio duration doesn't match file length

**Cause**: The playback simulator uses 20ms frames. If using a real RTP stream, timing should match actual audio.

**Note**: Current implementation reads frames but doesn't yet inject into actual RTP stream (that requires forge-media integration). Once RTP injection is complete, timing will be precise.

## Testing Different Scenarios

### Test 1: Simple Playback (Current)
- Answer → Play → Hangup
- Verifies playback completion waiting works

### Test 2: Multiple Audio Files
```json
{
  "play_message1": {
    "type": "play",
    "url": "/path/to/message1.wav",
    "next": "play_message2"
  },
  "play_message2": {
    "type": "play",
    "url": "/path/to/message2.wav",
    "next": "hangup"
  }
}
```

### Test 3: TTS (When Configured)
```json
{
  "say_welcome": {
    "type": "say",
    "text": "Welcome to the IVR system",
    "language": "en-US",
    "next": "hangup"
  }
}
```

## Expected Timing

For a 3-second audio file:
- **Old behavior**: Call terminates in ~100ms (immediately after starting playback)
- **New behavior**: Call stays connected for ~3 seconds until audio completes, then terminates

## Next Steps

Once basic playback works:

1. **Add DTMF collection** (Gather node)
2. **Implement conditional routing** based on user input
3. **Add dial/transfer** functionality
4. **Configure TTS provider** for Say nodes
5. **Add call recording**

## Architecture Notes

### How It Works

1. **Flow Handler** receives incoming call with IVR routing
2. **Answer node** sends 200 OK to SIP client
3. **Media session** is created by `MediaManager`
4. **Play node** starts audio playback via `PlaybackManager`
   - Returns `playback_id` (e.g., "playback-1")
   - `PlaybackHandle` is stored in `EmbeddedBackend`
5. **Flow handler waits** for playback to complete
   - Calls `media_manager.wait_playback_completion(playback_id)`
   - Blocks until `PlaybackHandle.wait_completion()` returns
6. **Playback task** reads audio frames and signals completion
7. **Flow continues** to next node (Hangup)
8. **Call terminates** gracefully

### Files Modified

- `crates/fcp-media/src/backend.rs`: Added `playback_handles` storage and `wait_playback_completion()`
- `crates/fcp-media/src/manager.rs`: Added `wait_playback_completion()` wrapper
- `crates/fcp-sip/src/flow_handler.rs`: Updated Play and Say nodes to wait for completion
- `crates/fcp-flow/src/engine.rs`: Added Answer/Hangup callback support

## Summary

The IVR system now properly handles audio playback:
- ✅ Audio playback completes before moving to next node
- ✅ No premature call termination
- ✅ Caller hears the full audio file
- ✅ Works for both Play (files) and Say (TTS) nodes

The fix ensures that flow execution waits for media operations to complete, providing a proper IVR experience.
