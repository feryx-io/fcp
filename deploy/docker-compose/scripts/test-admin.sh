#!/bin/bash
# fcp-admin: health, and a user's life through the API with the operator token.
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

echo "--- health (no token)"
curl -sf "${ADMIN_URL}/health" | jq . || fail "admin health"

echo "--- tenants"
TENANTS=$(curl -sf "${ADMIN_AUTH[@]}" "${ADMIN_URL}/tenants") || fail "list tenants (is FCP_ADMIN_AUTH_TOKEN right?)"
TENANT_ID=$(echo "$TENANTS" | jq -r '.[0].id')
[ -n "$TENANT_ID" ] && [ "$TENANT_ID" != "null" ] || fail "no tenant"
echo "tenant: $TENANT_ID"

USERNAME="smoke-$(date +%s)"
echo "--- create user $USERNAME"
curl -sf "${ADMIN_AUTH[@]}" -X POST "${ADMIN_URL}/users" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"${USERNAME}\",\"password\":\"smoke-pass-1\",\"display_name\":\"Smoke\",\"tenant_id\":\"${TENANT_ID}\"}" | jq . || fail "create user"

echo "--- read it back"
curl -sf "${ADMIN_AUTH[@]}" "${ADMIN_URL}/users/${USERNAME}" | jq . || fail "get user"

echo "--- delete it"
curl -sf "${ADMIN_AUTH[@]}" -X DELETE "${ADMIN_URL}/users/${USERNAME}" || fail "delete user"
if curl -sf "${ADMIN_AUTH[@]}" "${ADMIN_URL}/users/${USERNAME}" > /dev/null 2>&1; then
  fail "user still there after delete"
fi
echo "All fcp-admin tests passed!"
