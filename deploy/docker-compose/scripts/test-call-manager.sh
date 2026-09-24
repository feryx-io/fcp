#!/bin/bash
# fcp-call-manager: health, then its operator routes with the token.
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
echo "--- health (no token)"
curl -sf "${CALL_MANAGER_URL}/health" | jq . || fail "call manager health"
echo "--- ready"
curl -sf "${CALL_MANAGER_URL}/ready" | jq . || fail "call manager ready"
echo "--- status (token)"
STATUS=$(curl -sf "${CALL_MANAGER_AUTH[@]}" "${CALL_MANAGER_URL}/status") || fail "status (is FCP_CALL_MANAGER_API_TOKEN right?)"
echo "$STATUS" | jq .
[ "$(echo "$STATUS" | jq -r .uptime_seconds)" != "null" ] || fail "status has no uptime"
echo "--- calls, registrations, cdrs (token)"
curl -sf "${CALL_MANAGER_AUTH[@]}" "${CALL_MANAGER_URL}/calls" | jq . || fail "calls"
curl -sf "${CALL_MANAGER_AUTH[@]}" "${CALL_MANAGER_URL}/registrations" | jq . || fail "registrations"
curl -sf "${CALL_MANAGER_AUTH[@]}" "${CALL_MANAGER_URL}/cdrs?limit=10" | jq . || fail "cdrs"
echo "All fcp-call-manager tests passed!"
