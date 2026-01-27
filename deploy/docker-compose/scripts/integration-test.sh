#!/bin/bash
# Integration test script - runs all tests in sequence

set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

echo "======================================"
echo "FCP Integration Tests"
echo "======================================"
echo

# Wait for all services to be healthy
echo "Waiting for services to be healthy..."
echo

MAX_WAIT=60
WAIT_TIME=0

while [ $WAIT_TIME -lt $MAX_WAIT ]; do
    ALL_HEALTHY=true

    # Check each service
    for SERVICE in redis fcp-admin fcp-api-gateway fcp-events-server fcp-jobs fcp-registrar; do
        if ! docker compose ps --format json | jq -e ".[] | select(.Service == \"$SERVICE\" and .Health == \"healthy\")" > /dev/null 2>&1; then
            ALL_HEALTHY=false
            break
        fi
    done

    if $ALL_HEALTHY; then
        echo "All services are healthy!"
        break
    fi

    echo "Waiting for services... ($WAIT_TIME/$MAX_WAIT seconds)"
    sleep 5
    WAIT_TIME=$((WAIT_TIME + 5))
done

if [ $WAIT_TIME -ge $MAX_WAIT ]; then
    echo "ERROR: Services did not become healthy in time"
    echo
    echo "Service status:"
    docker compose ps
    exit 1
fi

echo
echo "======================================"
echo "Test 1: fcp-admin"
echo "======================================"
bash "${SCRIPT_DIR}/test-admin.sh"

echo
echo "======================================"
echo "Test 2: fcp-api-gateway"
echo "======================================"
bash "${SCRIPT_DIR}/test-gateway.sh"

echo
echo "======================================"
echo "Test 3: fcp-events-server"
echo "======================================"
bash "${SCRIPT_DIR}/test-events.sh"

echo
echo "======================================"
echo "Test 4: fcp-jobs"
echo "======================================"
bash "${SCRIPT_DIR}/test-jobs.sh"

echo
echo "======================================"
echo "Test 5: fcp-registrar"
echo "======================================"
bash "${SCRIPT_DIR}/test-registrar.sh"

echo
echo "======================================"
echo "All Integration Tests Completed!"
echo "======================================"
echo
echo "Summary:"
docker compose ps
