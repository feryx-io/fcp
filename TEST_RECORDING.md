# Testing Call Recording with Two Registered Endpoints

This guide walks you through testing the call recording feature end-to-end.

## Prerequisites

- Two SIP softphones (or use siphond in UAC mode)
- fcp-server built and ready to run
- curl (for API calls)

## Step 1: Start fcp-server

```bash
cd /home/siphon/fcp
./target/release/fcp-server

# Or with debug logging:
RUST_LOG=info,forge_engine=debug ./target/release/fcp-server
```

The server will start on:
- SIP: `0.0.0.0:5060` (UDP)
- API: `127.0.0.1:8080`

## Step 2: Create Two Test Users

```bash
# Create user alice
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "alice",
    "password": "secret123",
    "display_name": "Alice Test",
    "enabled": true
  }'

# Create user bob
curl -X POST http://localhost:8080/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "bob",
    "password": "secret456",
    "display_name": "Bob Test",
    "enabled": true
  }'

# Verify users were created
curl http://localhost:8080/api/v1/users
```

## Step 3: Configure SIP Softphones

Configure two SIP clients (e.g., Linphone, Zoiper, or MicroSIP):

### Alice's Configuration:
- **Username**: `alice`
- **Password**: `secret123`
- **Domain**: `192.168.1.81` (or your server IP)
- **Proxy**: `192.168.1.81:5060`

### Bob's Configuration:
- **Username**: `bob`
- **Password**: `secret456`
- **Domain**: `192.168.1.81` (or your server IP)
- **Proxy**: `192.168.1.81:5060`

## Step 4: Register Both Endpoints

Register both softphones. You should see registration confirmations in fcp-server logs:

```
INFO Registration successful aor="sip:alice@192.168.1.81"
INFO Registration successful aor="sip:bob@192.168.1.81"
```

Verify registrations via API:
```bash
curl http://localhost:8080/api/v1/registrations/sip:alice@192.168.1.81
curl http://localhost:8080/api/v1/registrations/sip:bob@192.168.1.81
```

## Step 5: Initiate a Call

From Alice's softphone, call Bob:
- Dial: `sip:bob@192.168.1.81` or just `bob`

Bob should receive the call. Answer it from Bob's softphone.

## Step 6: Identify the Call ID

Watch fcp-server logs for the call setup. You'll see log entries with the CallId:

```
INFO Media session created call_id="call-abc123..."
```

Alternatively, query the debug endpoint:
```bash
curl http://localhost:8080/api/v1/debug/calls
```

## Step 7: Start Recording

Once the call is active and you have the Call ID, start recording:

```bash
# Replace CALL_ID with the actual call ID from logs
CALL_ID="call-abc12345-..."

curl -X POST "http://localhost:8080/api/v1/calls/${CALL_ID}/recording/start" \
  -H "Content-Type: application/json" \
  -d '{
    "path": "/tmp/test_recording.wav"
  }'
```

You should see:
```json
{
  "call_id": "call-abc12345-...",
  "message": "Recording started to /tmp/test_recording.wav"
}
```

## Step 8: Have a Conversation

Talk between Alice and Bob for 10-20 seconds. Both sides of the conversation will be recorded.

## Step 9: Stop Recording

```bash
curl -X POST "http://localhost:8080/api/v1/calls/${CALL_ID}/recording/stop"
```

You should see:
```json
{
  "call_id": "call-abc12345-...",
  "message": "Recording stopped"
}
```

## Step 10: Verify the Recording

```bash
# Check the file exists
ls -lh /tmp/test_recording.wav

# Play the recording (Linux with aplay)
aplay /tmp/test_recording.wav

# Or play with ffplay
ffplay /tmp/test_recording.wav

# Or convert to mp3 for easier sharing
ffmpeg -i /tmp/test_recording.wav /tmp/test_recording.mp3
```

## Expected Results

✅ **Recording file created**: File exists at specified path
✅ **File size > 44 bytes**: Valid WAV header present
✅ **Audio playable**: Recording contains conversation from both parties
✅ **Audio quality**: Clear audio without distortion
✅ **Both parties audible**: Alice and Bob's voices both recorded

## Troubleshooting

### No audio in recording
- **Cause**: RTP packets may not be flowing through the server
- **Solution**: Check media anchoring is enabled in config

### Recording file is empty
- **Cause**: Recording stopped immediately
- **Solution**: Ensure call was active when recording started

### API returns "Session not found"
- **Cause**: Wrong Call ID or session already terminated
- **Solution**: Verify Call ID from logs/debug endpoint

### Can't hear both parties
- **Cause**: Media anchoring not working correctly
- **Solution**: Check logs for forwarding errors

## Advanced: Test with siphond

Instead of softphones, you can use siphond for automated testing:

```bash
# Terminal 1: Start fcp-server
./target/release/fcp-server

# Terminal 2: Alice (UAC)
cd external/siphon-rs
cargo run -p siphond -- --mode uac --username alice --password secret123 \
  --target sip:bob@192.168.1.81

# Terminal 3: Bob (UAS)
cargo run -p siphond -- --mode uas --username bob --password secret456
```

## Opus Format Testing

To test Opus format recording (requires opus feature):

```bash
# Build with opus support
cargo build --release --features opus

# Start recording with .opus extension
curl -X POST "http://localhost:8080/api/v1/calls/${CALL_ID}/recording/start" \
  -H "Content-Type: application/json" \
  -d '{
    "path": "/tmp/test_recording.opus"
  }'
```

## Cleanup

```bash
# End the call from either softphone
# Cleanup test recordings
rm /tmp/test_recording.*
```

## API Reference

### Start Recording
```
POST /api/v1/calls/:call_id/recording/start
Content-Type: application/json

{
  "path": "/path/to/recording.wav"  // .wav or .opus
}
```

### Stop Recording
```
POST /api/v1/calls/:call_id/recording/stop
```

### List Active Calls (future)
```
GET /api/v1/calls
```

## Notes

- Recording format is auto-detected from file extension
- `.wav` → PCM format (default)
- `.opus` → Opus format (requires opus feature)
- Recording captures both legs of the conversation
- Samples are recorded at the codec's native sample rate
- Recording starts immediately upon API call
- Recording stops automatically when call ends

