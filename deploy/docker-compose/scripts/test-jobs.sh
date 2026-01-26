#!/bin/bash
# Test script for fcp-jobs

set -e

JOBS_URL="http://localhost:8082"

echo "=== Testing fcp-jobs ==="
echo

# Test 1: Health check
echo "1. Health check..."
curl -s "${JOBS_URL}/health"
echo
echo

# Test 2: Status (before submitting jobs)
echo "2. Status (before jobs)..."
curl -s "${JOBS_URL}/status" | jq '.'
echo

# Test 3: Submit test jobs to Redis
echo "3. Submitting test jobs to Redis queue..."

# Check if redis-cli is available
if ! command -v redis-cli &> /dev/null; then
    echo "Error: redis-cli not found. Installing..."
    sudo apt-get update && sudo apt-get install -y redis-tools
fi

# Submit CDR processing job
echo "  - Submitting CDR processing job (normal priority)..."
JOB_ID="test-cdr-$(date +%s)"
redis-cli -h localhost -p 6379 <<EOF
SET "fcp:jobs:job:job-${JOB_ID}" '{"id":"${JOB_ID}","job_type":"cdr_processing","priority":"normal","status":"pending","retry_count":0,"max_retries":5,"payload":{"call_id":"call-test123","duration_seconds":125},"created_at":$(date +%s),"updated_at":$(date +%s),"retry_at":null,"error":null,"result":null}'
ZADD "fcp:jobs:queue:normal" $(date +%s) "job-${JOB_ID}"
EOF
echo "    Job ID: job-${JOB_ID}"

# Submit webhook delivery job
echo "  - Submitting webhook delivery job (high priority)..."
JOB_ID="test-webhook-$(date +%s)"
redis-cli -h localhost -p 6379 <<EOF
SET "fcp:jobs:job:job-${JOB_ID}" '{"id":"${JOB_ID}","job_type":"webhook_delivery","priority":"high","status":"pending","retry_count":0,"max_retries":5,"payload":{"url":"https://example.com/webhook","payload":{"event":"test"}},"created_at":$(date +%s),"updated_at":$(date +%s),"retry_at":null,"error":null,"result":null}'
ZADD "fcp:jobs:queue:high" $(date +%s) "job-${JOB_ID}"
EOF
echo "    Job ID: job-${JOB_ID}"

# Submit recording processing job
echo "  - Submitting recording processing job (normal priority)..."
JOB_ID="test-recording-$(date +%s)"
redis-cli -h localhost -p 6379 <<EOF
SET "fcp:jobs:job:job-${JOB_ID}" '{"id":"${JOB_ID}","job_type":"recording_processing","priority":"normal","status":"pending","retry_count":0,"max_retries":5,"payload":{"recording_id":"rec-test123","file_path":"/tmp/test.wav"},"created_at":$(date +%s),"updated_at":$(date +%s),"retry_at":null,"error":null,"result":null}'
ZADD "fcp:jobs:queue:normal" $(date +%s) "job-${JOB_ID}"
EOF
echo "    Job ID: job-${JOB_ID}"

echo

# Test 4: Wait for jobs to be processed
echo "4. Waiting 5 seconds for jobs to be processed..."
sleep 5
echo

# Test 5: Status (after submitting jobs)
echo "5. Status (after jobs)..."
curl -s "${JOBS_URL}/status" | jq '.'
echo

# Test 6: Check Redis for completed jobs
echo "6. Checking completed jobs in Redis..."
COMPLETED=$(redis-cli -h localhost -p 6379 SMEMBERS "fcp:jobs:completed" | wc -l)
echo "Completed jobs: $COMPLETED"
echo

# Test 7: Check Redis for failed jobs
echo "7. Checking failed jobs in Redis..."
FAILED=$(redis-cli -h localhost -p 6379 SMEMBERS "fcp:jobs:failed" | wc -l)
echo "Failed jobs: $FAILED"
echo

echo "=== fcp-jobs tests completed ==="
