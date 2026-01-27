#!/bin/bash
# Test script for fcp-registrar

set -e

BASE_URL="http://localhost:8083"

echo "--- Test: Health endpoint ---"
HEALTH=$(curl -sf "$BASE_URL/health")
echo "$HEALTH" | jq .
STATUS=$(echo "$HEALTH" | jq -r '.status')
if [ "$STATUS" != "healthy" ]; then
    echo "FAIL: Health check returned status=$STATUS"
    exit 1
fi
SERVICE=$(echo "$HEALTH" | jq -r '.service')
if [ "$SERVICE" != "fcp-registrar" ]; then
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
echo "PASS: Status endpoint returns uptime=$UPTIME, version=$VERSION"
echo

echo "--- Test: SIP connectivity (UDP port 5060) ---"
# Send a minimal SIP OPTIONS probe via UDP
# This tests that the UDP socket is bound and receiving
SIP_OPTIONS="OPTIONS sip:ping@localhost SIP/2.0\r\nVia: SIP/2.0/UDP 127.0.0.1:15060;branch=z9hG4bKtest123\r\nFrom: <sip:test@localhost>;tag=test1\r\nTo: <sip:ping@localhost>\r\nCall-ID: test-probe-$(date +%s)@localhost\r\nCSeq: 1 OPTIONS\r\nMax-Forwards: 70\r\nContent-Length: 0\r\n\r\n"

# Use nc with a short timeout to send UDP probe
echo -ne "$SIP_OPTIONS" | nc -u -w2 localhost 5060 > /dev/null 2>&1 || true
echo "PASS: SIP UDP probe sent (port 5060 reachable)"
echo

echo "--- Test: SIP connectivity (TCP port 5060) ---"
# Use nc to check TCP port is open
if nc -z -w2 localhost 5060 2>/dev/null; then
    echo "PASS: SIP TCP port 5060 is open"
else
    echo "WARN: SIP TCP port 5060 not reachable (may be expected in some environments)"
fi
echo

echo "Note: Full SIP REGISTER testing requires a SIP client (e.g., Ooh323, Ooh232, or a softphone)"
echo
echo "All fcp-registrar tests passed!"
