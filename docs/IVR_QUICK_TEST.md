# Quick IVR Audio Playback Test

## Overview

This guide provides the fastest way to test audio playback in a flow. The test IVR flow is **automatically loaded** when running fcp-server in debug mode - no manual configuration needed!

## Prerequisites

- FCP workspace compiled
- Audio file: `welcome.wav` created
- Registered SIP users (alice and bob are auto-created)

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

### Step 2: Build and Run FCP Server in Debug Mode

**Important**: The test IVR flow is automatically loaded **only in debug builds**. This is controlled by `cfg!(debug_assertions)` in the runtime code.

```bash
# Option A: Build and run in debug mode (recommended for testing)
cargo run -p fcp-server

# Option B: Build debug binary explicitly
cargo build -p fcp-server
./target/debug/fcp-server

# Option C: Build with specific profile
cargo build --profile dev -p fcp-server
./target/dev/fcp-server
```

**Note**: Do NOT use `--release` flag, as release builds disable the test flow.

### Step 3: Verify Test Flow Loaded

Watch the server startup logs for these messages:

```
INFO Loading test IVR flow
INFO Creating test IVR flow nodes
INFO Test IVR flow definition created flow_id=welcome-ivr node_count=3
INFO Test IVR flow inserted into flow store
INFO Test IVR DID routing configured - call +15551234567 to test audio playback did=+15551234567 flow_id=welcome-ivr
```

If you see these messages, the test flow is ready!

### Step 4: Make a Test Call

From your SIP client (alice or bob):
1. Dial: `+15551234567`
2. The call should be answered automatically
3. You should hear the welcome.wav file playing
4. The call will hang up after playback completes

### Step 5: Verify Flow Execution in Logs

Watch for these log messages during the call:

```
INFO Starting flow execution for call call_id=call-xxx flow_id=welcome-ivr
INFO Executing flow node call_id=call-xxx node_id=answer node_type=Answer
INFO Executing flow node call_id=call-xxx node_id=play_welcome node_type=Play
INFO Playing audio file call_id=call-xxx url=/var/lib/fcp/audio/welcome.wav
INFO Audio playback started call_id=call-xxx playback_id=playback-xxx
INFO Executing flow node call_id=call-xxx node_id=hangup node_type=Hangup
```

## Troubleshooting

### Test Flow Not Loading

**Problem**: You don't see "Loading test IVR flow" in the logs.

**Solution**: Make sure you're running a debug build:
```bash
# Wrong - release build (test flow disabled)
cargo run --release -p fcp-server

# Correct - debug build (test flow enabled)
cargo run -p fcp-server
```

You can verify the build type in the logs:
- Debug builds show more verbose logging
- Look for "Loading test IVR flow" message at startup

### No Audio Playing

1. **Check file exists and is readable**:
   ```bash
   ls -la /var/lib/fcp/audio/welcome.wav
   stat /var/lib/fcp/audio/welcome.wav
   ```

2. **Check logs for errors**:
   ```bash
   # If using journald
   journalctl -u fcp-server | grep -i "audio\|playback"

   # If using file logging
   grep -i "error\|failed" logs/fcp-server.log | grep -i audio
   ```

3. **Verify media manager initialized**:
   ```bash
   # Look for media manager startup
   grep "Media manager" logs/fcp-server.log
   grep "MediaManager created" logs/fcp-server.log
   ```

### Flow Not Triggering

1. **Check DID routing was configured**:
   ```bash
   # Should show the routing configuration
   grep "Test IVR DID routing configured" logs/fcp-server.log
   grep "+15551234567" logs/fcp-server.log
   ```

2. **Verify flow was loaded**:
   ```bash
   # Should show flow insertion
   grep "welcome-ivr" logs/fcp-server.log
   grep "Test IVR flow inserted" logs/fcp-server.log
   ```

3. **Check call is being received**:
   ```bash
   # Look for INVITE messages
   grep "INVITE" logs/fcp-server.log | tail -5
   grep "Received request" logs/fcp-server.log | grep INVITE
   ```

### Call Connects But No Audio

**Current Status**: RTP playback integration is in progress.

If you see these logs but don't hear audio:
- "Audio playback started" - Playback manager initialized correctly
- "Playing audio file" - File was found and opened

This means the flow engine and media manager are working, but RTP injection may not be fully wired yet.

**What to check**:
- Media session is established (check SDP in logs)
- RTP ports are open and accessible
- No firewall blocking RTP traffic

## How It Works

### Automatic Test Flow Loading

The test IVR flow is automatically loaded in `crates/fcp-runtime/src/builder.rs` via the `load_test_ivr_flow()` function.

**What it does**:
1. Creates a 3-node flow: Answer → Play → Hangup
2. Configures DID routing: +15551234567 → welcome-ivr flow
3. Only runs in debug builds (controlled by `cfg!(debug_assertions)`)

**Flow Definition**:
```
answer node:
  - Answers the incoming call
  - Next: play_welcome

play_welcome node:
  - Plays /var/lib/fcp/audio/welcome.wav
  - Loop count: 0 (play once)
  - Next: hangup

hangup node:
  - Terminates the call
  - Reason: normal
```

### Debug vs Release Builds

| Build Type | Command | Test Flow | Use Case |
|------------|---------|-----------|----------|
| Debug | `cargo run -p fcp-server` | ✅ Enabled | Development and testing |
| Debug | `cargo build -p fcp-server` | ✅ Enabled | Development and testing |
| Release | `cargo build --release -p fcp-server` | ❌ Disabled | Production deployment |

**Why debug-only?**
- Test flows should not load in production
- `cfg!(debug_assertions)` is a compile-time check
- Release builds have zero overhead from test code

## Next Steps

Once you verify the flow triggers and logs show playback starting:

1. **Verify RTP Integration**: Confirm audio actually plays through to the SIP client
2. **Test with Multiple Calls**: Make several concurrent calls to test stability
3. **Create Custom Flows**: Experiment with different flow configurations
4. **Database Integration**: Migrate from InMemoryFlowStore to DatabaseFlowStore (Phase 2)
5. **REST API**: Implement flow management endpoints (Phase 2)
6. **Flow Editor UI**: Build web-based flow designer (Phase 2)

## Quick Reference

### Test Configuration

| Item | Value |
|------|-------|
| **Test DID** | `+15551234567` |
| **Flow ID** | `welcome-ivr` |
| **Audio File** | `/var/lib/fcp/audio/welcome.wav` |
| **SIP Users** | alice / bob (auto-created) |
| **Build Mode** | Debug only (`cargo run -p fcp-server`) |

### Expected Call Flow

1. User dials `+15551234567` from SIP client
2. FCP server matches DID to flow (welcome-ivr)
3. Flow engine starts execution
4. Answer node: Call is auto-answered
5. Play node: welcome.wav plays
6. Hangup node: Call terminates normally

### Key Log Messages

| Message | Meaning |
|---------|---------|
| `Loading test IVR flow` | Test flow loading started |
| `Test IVR DID routing configured` | DID +15551234567 mapped to flow |
| `Starting flow execution` | Call triggered flow |
| `node_type=Answer` | Call answered |
| `node_type=Play` | Audio playback node executing |
| `Audio playback started` | Media playback initiated |
| `node_type=Hangup` | Call terminating |

### File Locations

```
/var/lib/fcp/audio/welcome.wav    # Audio file (you provide)
crates/fcp-runtime/src/builder.rs # Test flow code (lines 876-970)
docs/IVR_QUICK_TEST.md           # This guide
docs/IVR_TESTING_GUIDE.md        # Comprehensive guide
```

### Quick Commands

```bash
# Place audio file
sudo mkdir -p /var/lib/fcp/audio && sudo cp welcome.wav /var/lib/fcp/audio/

# Run server (debug mode)
cargo run -p fcp-server

# Watch logs (in another terminal)
tail -f logs/fcp-server.log | grep -E "flow|playback|Playing"

# Check if test flow loaded
grep "Test IVR DID routing configured" logs/fcp-server.log
```
