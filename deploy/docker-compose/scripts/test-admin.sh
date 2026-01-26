#!/bin/bash
# Test script for fcp-admin

set -e

BASE_URL="http://localhost:8080/api/v1"

echo "=== Testing fcp-admin ==="
echo

# Test 1: Health check
echo "1. Health check..."
curl -s "${BASE_URL}/health" | jq '.'
echo

# Test 2: Create tenant
echo "2. Creating tenant..."
TIMESTAMP=$(date +%s)
TENANT_RESPONSE=$(curl -s -X POST "${BASE_URL}/tenants" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"Test Company ${TIMESTAMP}\",
    \"enabled\": true
  }")
echo "$TENANT_RESPONSE" | jq '.'
TENANT_ID=$(echo "$TENANT_RESPONSE" | jq -r '.id')
echo "Created tenant: $TENANT_ID"
echo

# Test 3: List tenants
echo "3. Listing tenants..."
curl -s "${BASE_URL}/tenants" | jq '.'
echo

# Test 4: Get tenant
echo "4. Getting tenant..."
curl -s "${BASE_URL}/tenants/${TENANT_ID}" | jq '.'
echo

# Test 5: Create user
echo "5. Creating user..."
USER_RESPONSE=$(curl -s -X POST "${BASE_URL}/users" \
  -H "Content-Type: application/json" \
  -d "{
    \"tenant_id\": \"${TENANT_ID}\",
    \"username\": \"alice${TIMESTAMP}\",
    \"email\": \"alice${TIMESTAMP}@example.com\",
    \"enabled\": true
  }")
echo "$USER_RESPONSE" | jq '.'
USER_ID=$(echo "$USER_RESPONSE" | jq -r '.id')
echo "Created user: $USER_ID"
echo

# Test 6: List users
echo "6. Listing users..."
curl -s "${BASE_URL}/users" | jq '.'
echo

# Test 7: Get user
echo "7. Getting user..."
curl -s "${BASE_URL}/users/${USER_ID}" | jq '.'
echo

# Test 8: Update user
echo "8. Updating user..."
curl -s -X PUT "${BASE_URL}/users/${USER_ID}" \
  -H "Content-Type: application/json" \
  -d "{
    \"tenant_id\": \"${TENANT_ID}\",
    \"username\": \"alice${TIMESTAMP}\",
    \"email\": \"alice${TIMESTAMP}.updated@example.com\",
    \"enabled\": true
  }" | jq '.'
echo

# Test 9: Delete user
echo "9. Deleting user..."
curl -s -X DELETE "${BASE_URL}/users/${USER_ID}"
echo "User deleted"
echo

# Test 10: Delete tenant
echo "10. Deleting tenant..."
curl -s -X DELETE "${BASE_URL}/tenants/${TENANT_ID}"
echo "Tenant deleted"
echo

echo "=== fcp-admin tests completed ==="
