# FCP REST API – curl quick reference

Base URL (default): `http://127.0.0.1:8080/api/v1`

- Authentication: disabled by default. If `[api].auth_enabled = true`, set `API_TOKEN` in your config and add `-H "Authorization: Bearer $API_TOKEN"` to each request.
- Content type: for POST/PUT send `-H "Content-Type: application/json"`.
- Errors: JSON `{ "error": "Bad Request|Not Found|Conflict|Internal Server Error", "details": "..." }`.

## System
- Health: `curl $BASE/health`
- Status: `curl $BASE/status`

## Users
- List: `curl $BASE/users`
- Get: `curl "$BASE/users/alice"`
- Create:
  ```bash
  curl -X POST $BASE/users \
    -H "Content-Type: application/json" \
    -d '{
      "username": "alice",
      "password": "secret123",
      "display_name": "Alice Test",
      "enabled": true,
      "tenant_id": "00000000-0000-0000-0000-000000000000",
      "media_preferences": {
        "force_media_anchoring": false,
        "enable_dtmf_detection": true,
        "record_all_calls": false,
        "preferred_codecs": ["PCMU", "PCMA"]
      }
    }'
  ```
- Update display name/enabled:
  ```bash
  curl -X PUT $BASE/users/alice \
    -H "Content-Type: application/json" \
    -d '{ "display_name": "Alice Cooper", "enabled": true }'
  ```
- Change password:
  ```bash
  curl -X PUT $BASE/users/alice/password \
    -H "Content-Type: application/json" \
    -d '{ "password": "newpass123" }'
  ```
- Delete: `curl -X DELETE "$BASE/users/alice"`

## Registrations
`aor` and `contact` should be URL-encoded when they contain `@`, `;`, or `>` characters.

- List for AOR: `curl "$BASE/registrations/sip%3Aalice%40192.168.1.81"`
- Delete all for AOR: `curl -X DELETE "$BASE/registrations/sip%3Aalice%40192.168.1.81"`
- Delete specific contact:
  ```bash
  curl -X DELETE "$BASE/registrations/sip%3Aalice%40192.168.1.81/sip%3Aalice%40192.168.1.50%3A5060%3Btransport%3Dudp"
  ```

## Calls & Recording
- List active calls (currently returns an empty list placeholder): `curl $BASE/calls`
- Start recording:
  ```bash
  CALL_ID="call-abc123..."
  curl -X POST "$BASE/calls/$CALL_ID/recording/start" \
    -H "Content-Type: application/json" \
    -d '{ "path": "/tmp/test_recording.wav" }'
  ```
- Stop recording:
  ```bash
  curl -X POST "$BASE/calls/$CALL_ID/recording/stop"
  ```

## Debug (for inspection)
- Summary counts: `curl $BASE/debug/summary`
- Active dialogs: `curl $BASE/debug/dialogs`
- Active subscriptions: `curl $BASE/debug/subscriptions`
- Active calls (SIP/B2BUA state): `curl $BASE/debug/calls`
- Rate limits: `curl $BASE/debug/rate-limits`

## BASE helper
For convenience:
```bash
BASE="http://127.0.0.1:8080/api/v1"
AUTH_HEADER="-H \"Authorization: Bearer $API_TOKEN\"" # if auth enabled
```
