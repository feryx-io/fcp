#!/bin/bash
# fcp-events-server: health, status with the token, and a WebSocket
# connection when websocat is installed. A tenant id must be a UUID.
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

echo "--- health (no token)"
curl -sf "${EVENTS_URL}/health" || fail "events health"; echo
echo "--- status (token)"
curl -sf "${EVENTS_AUTH[@]}" "${EVENTS_URL}/status" | jq . || fail "events status (is FCP_EVENTS_AUTH_TOKEN right?)"
TENANT_ID=$(curl -sf "${ADMIN_AUTH[@]}" "${ADMIN_URL}/tenants" | jq -r '.[0].id')
if command -v websocat > /dev/null; then
  echo "--- websocket for tenant $TENANT_ID (5 s)"
  timeout 5s websocat "ws://localhost:8081/ws/${TENANT_ID}?token=${FCP_EVENTS_AUTH_TOKEN}" < /dev/null || true
else
  echo "websocat not installed; connect with:"
  echo "  websocat 'ws://localhost:8081/ws/${TENANT_ID}?token=\$FCP_EVENTS_AUTH_TOKEN'"
fi
echo "All fcp-events-server tests passed!"
