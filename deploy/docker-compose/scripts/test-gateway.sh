#!/bin/bash
# Test script for fcp-api-gateway

set -e

GATEWAY_URL="http://localhost:8000"

echo "=== Testing fcp-api-gateway ==="
echo

# Test 1: Gateway health check
echo "1. Gateway health check..."
curl -s "${GATEWAY_URL}/health" | jq '.'
echo

# Test 2: Gateway status
echo "2. Gateway status..."
curl -s "${GATEWAY_URL}/gateway/status" | jq '.'
echo

# Test 3: Proxied request (should hit fcp-admin through gateway)
echo "3. Proxied request to admin (through gateway)..."
curl -s "${GATEWAY_URL}/api/v1/health" | jq '.'
echo

# Test 4: Cache test - First request (should be cache MISS)
echo "4a. First request (cache MISS)..."
curl -i -s "${GATEWAY_URL}/api/v1/tenants" | grep -E "HTTP/|X-Cache"
echo

# Test 5: Cache test - Second request (should be cache HIT)
echo "4b. Second request (cache HIT)..."
sleep 1
curl -i -s "${GATEWAY_URL}/api/v1/tenants" | grep -E "HTTP/|X-Cache"
echo
echo

# Test 6: Rate limiting test
echo "5. Rate limiting test (sending 10 requests quickly)..."
for i in {1..10}; do
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" "${GATEWAY_URL}/api/v1/health")
  echo "Request $i: HTTP $STATUS"
  if [ "$STATUS" = "429" ]; then
    echo "Rate limited!"
    break
  fi
done
echo

echo "=== fcp-api-gateway tests completed ==="
