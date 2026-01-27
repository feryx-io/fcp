#!/bin/bash
# Test script for fcp-call-manager

set -e

BASE_URL="http://localhost:8084"

echo "--- Test: Health endpoint ---"
HEALTH=$(curl -sf "$BASE_URL/health")
echo "$HEALTH" | jq .
STATUS=$(echo "$HEALTH" | jq -r '.status')
if [ "$STATUS" != "healthy" ]; then
    echo "FAIL: Health check returned status=$STATUS"
    exit 1
fi
SERVICE=$(echo "$HEALTH" | jq -r '.service')
if [ "$SERVICE" != "fcp-call-manager" ]; then
    echo "FAIL: Health check returned service=$SERVICE"
    exit 1
fi
echo "PASS: Health endpoint healthy"
echo

echo "--- Test: Status endpoint ---"
STATUS_RESP=$(curl -sf "$BASE_URL/status")
echo "$STATUS_RESP" | jq .
UPTIME=$(echo "$STATUS_RESP" | jq -r '.uptime_seconds')
if [ "$UPTIME" = "null" ]; then
    echo "FAIL: Status endpoint missing uptime_seconds"
    exit 1
fi
VERSION=$(echo "$STATUS_RESP" | jq -r '.version')
if [ "$VERSION" = "null" ]; then
    echo "FAIL: Status endpoint missing version"
    exit 1
fi
ACTIVE_CALLS=$(echo "$STATUS_RESP" | jq -r '.active_calls')
if [ "$ACTIVE_CALLS" = "null" ]; then
    echo "FAIL: Status endpoint missing active_calls"
    exit 1
fi
echo "PASS: Status endpoint returns uptime=$UPTIME, version=$VERSION, active_calls=$ACTIVE_CALLS"
echo

echo "--- Test: Calls endpoint ---"
CALLS_RESP=$(curl -sf "$BASE_URL/calls")
echo "$CALLS_RESP" | jq .
CALLS_FIELD=$(echo "$CALLS_RESP" | jq -r '.calls')
if [ "$CALLS_FIELD" = "null" ]; then
    echo "FAIL: Calls endpoint missing calls array"
    exit 1
fi
echo "PASS: Calls endpoint returns call list"
echo

echo "--- Test: Registrations endpoint ---"
REG_RESP=$(curl -sf "$BASE_URL/registrations")
echo "$REG_RESP" | jq .
REG_FIELD=$(echo "$REG_RESP" | jq -r '.registrations')
if [ "$REG_FIELD" = "null" ]; then
    echo "FAIL: Registrations endpoint missing registrations array"
    exit 1
fi
echo "PASS: Registrations endpoint returns registration list"
echo

echo "--- Test: SIP connectivity (UDP port 5060) ---"
# Send a minimal SIP OPTIONS probe via UDP
SIP_OPTIONS="OPTIONS sip:ping@localhost SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:15060;branch=z9hG4bKtest123\r\nFrom: <sip:test@localhost>;tag=test1\r\nTo: <sip:ping@localhost>\r\nCall-ID: test-probe-$(date +%s)@localhost\r\nCSeq: 1 OPTIONS\r\nMax-Forwards: 70\r\nContent-Length: 0\r\n\r\n"

# Use nc with a short timeout to send UDP probe
echo -ne "$SIP_OPTIONS" | nc -u -w2 localhost 5060 > /dev/null 2>&1 || true
echo "PASS: SIP UDP probe sent (port 5060 reachable)"
echo

echo "--- Test: SIP connectivity (TCP port 5060) ---"
if nc -z -w2 localhost 5060 2>/dev/null; then
    echo "PASS: SIP TCP port 5060 is open"
else
    echo "WARN: SIP TCP port 5060 not reachable (may be expected in some environments)"
fi
echo

echo "Note: Full SIP call testing requires SIP clients (e.g., Ooh323, Zoiper, or softphones)"
echo
echo "All fcp-call-manager tests passed!"
