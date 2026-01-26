#!/bin/bash
# Test script for fcp-events-server

set -e

EVENTS_URL="http://localhost:8081"

echo "=== Testing fcp-events-server ==="
echo

# Test 1: Health check
echo "1. Health check..."
curl -s "${EVENTS_URL}/health"
echo
echo

# Test 2: Status
echo "2. Status..."
curl -s "${EVENTS_URL}/status" | jq '.'
echo

# Test 3: WebSocket connection test (requires websocat or similar)
echo "3. WebSocket connection test..."
echo "Note: WebSocket testing requires 'websocat' tool"
echo "Install with: cargo install websocat"
echo

if command -v websocat &> /dev/null; then
    echo "Testing WebSocket connection to ws://localhost:8081/ws/tenant-test123"
    echo "Sending subscribe message..."

    # Send subscribe message and wait for response
    timeout 5s websocat "ws://localhost:8081/ws/tenant-test123" <<EOF || true
{"type": "subscribe", "filters": [{"event_types": ["call"]}]}
EOF

    echo "WebSocket test completed"
else
    echo "Skipping WebSocket test (websocat not installed)"
    echo
    echo "To test WebSocket manually:"
    echo "1. Install websocat: cargo install websocat"
    echo "2. Connect: websocat ws://localhost:8081/ws/tenant-test123"
    echo "3. Send: {\"type\": \"subscribe\", \"filters\": []}"
fi
echo

echo "=== fcp-events-server tests completed ==="
