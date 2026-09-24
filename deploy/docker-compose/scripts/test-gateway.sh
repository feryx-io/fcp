#!/bin/bash
# fcp-api-gateway: its own health, its backends' health, and a proxied
# call with the admin token (the gateway passes the header through).
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

echo "--- gateway health"
curl -sf "${GATEWAY_URL}/health" | jq . || fail "gateway health"
echo "--- backends"
STATUS=$(curl -sf "${GATEWAY_URL}/gateway/status") || fail "gateway status"
echo "$STATUS" | jq .
[ "$(echo "$STATUS" | jq -r .status)" = "operational" ] || fail "gateway is not operational"
echo "--- proxied admin health"
curl -sf "${GATEWAY_URL}/api/v1/health" | jq . || fail "proxied health"
echo "--- proxied, authenticated, twice (cache)"
for i in 1 2; do
  curl -si "${ADMIN_AUTH[@]}" "${GATEWAY_URL}/api/v1/tenants" | grep -E "^HTTP/|X-Cache" || true
done
echo "All fcp-api-gateway tests passed!"
