#!/bin/bash
# fcp-registrar (the `registrar` profile): health, then status with the token.
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
echo "--- health (no token)"
curl -sf "${REGISTRAR_URL}/health" | jq . || fail "registrar health (is the registrar profile up?)"
echo "--- status (token)"
STATUS=$(curl -sf "${REGISTRAR_AUTH[@]}" "${REGISTRAR_URL}/status") || fail "status (is FCP_REGISTRAR_API_TOKEN right?)"
echo "$STATUS" | jq .
[ "$(echo "$STATUS" | jq -r .uptime_seconds)" != "null" ] || fail "status has no uptime"
echo "All fcp-registrar tests passed!"
