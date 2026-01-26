#!/bin/bash
# Test script for fcp-admin (MVP single-tenant mode)

set -e

BASE_URL="http://localhost:8080/api/v1"

echo "=== Testing fcp-admin (MVP Single-Tenant Mode) ==="
echo

# Test 1: Health check
echo "1. Health check..."
curl -s "${BASE_URL}/health" | jq '.'
echo

# Test 2: List tenants (MVP returns only default tenant)
echo "2. Listing tenants..."
TENANT_RESPONSE=$(curl -s "${BASE_URL}/tenants")
echo "$TENANT_RESPONSE" | jq '.'
TENANT_ID=$(echo "$TENANT_RESPONSE" | jq -r '.[0].id')
# Extract UUID without "tenant-" prefix for API calls
TENANT_UUID="${TENANT_ID#tenant-}"
echo "Using default tenant ID: $TENANT_ID (UUID: $TENANT_UUID)"
echo

# Test 3: Get default tenant
echo "3. Getting default tenant..."
curl -s "${BASE_URL}/tenants/${TENANT_ID}" | jq '.'
echo

# NOTE: Tenant creation/deletion not supported in MVP single-tenant mode

# Test 4: Create user
echo "4. Creating user..."
UNIQUE_ID=$(cat /proc/sys/kernel/random/uuid | cut -d'-' -f1)
USER_RESPONSE=$(curl -s -X POST "${BASE_URL}/users" \
  -H "Content-Type: application/json" \
  -d "{
    \"tenant_id\": \"${TENANT_UUID}\",
    \"username\": \"alice${UNIQUE_ID}\",
    \"password\": \"TestPass123!\",
    \"email\": \"alice${UNIQUE_ID}@example.com\",
    \"enabled\": true
  }")
echo "$USER_RESPONSE" | jq '.'
USER_ID=$(echo "$USER_RESPONSE" | jq -r '.id')
echo "Created user: $USER_ID"
echo

# Test 5: List users
echo "5. Listing users..."
curl -s "${BASE_URL}/users" | jq '.'
echo

# Test 6: Get user
echo "6. Getting user..."
curl -s "${BASE_URL}/users/${USER_ID}" | jq '.'
echo

# Test 7: Update user
echo "7. Updating user..."
curl -s -X PUT "${BASE_URL}/users/${USER_ID}" \
  -H "Content-Type: application/json" \
  -d "{
    \"tenant_id\": \"${TENANT_UUID}\",
    \"username\": \"alice${UNIQUE_ID}\",
    \"password\": \"TestPass123!\",
    \"email\": \"alice${UNIQUE_ID}.updated@example.com\",
    \"enabled\": true
  }" | jq '.'
echo

# Test 8: Delete user
echo "8. Deleting user..."
curl -s -X DELETE "${BASE_URL}/users/${USER_ID}"
echo "User deleted"
echo

# Test 9: Verify user deleted
echo "9. Verifying user deleted..."
DELETE_CHECK=$(curl -s "${BASE_URL}/users/${USER_ID}")
echo "$DELETE_CHECK" | jq '.'
echo

echo "=== fcp-admin tests completed (MVP mode) ==="
echo "Note: Multi-tenant operations (create/update/delete tenants) are not supported in MVP mode"
