#!/bin/bash
# fcp-jobs: health and status (both open).
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
echo "--- health"
curl -sf "${JOBS_URL}/health" || fail "jobs health"; echo
echo "--- status"
curl -sf "${JOBS_URL}/status" | jq . || fail "jobs status"
echo "All fcp-jobs tests passed!"
