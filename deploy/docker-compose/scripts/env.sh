#!/bin/bash
# Sourced by the test scripts: the stack's .env, and the URLs and headers
# the scripts call with. Run from anywhere.
COMPOSE_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
set -a
# shellcheck disable=SC1091
. "${COMPOSE_DIR}/.env"
set +a
ADMIN_URL="http://localhost:8080/api/v1"
GATEWAY_URL="http://localhost:8000"
EVENTS_URL="http://localhost:8081"
JOBS_URL="http://localhost:8082"
CALL_MANAGER_URL="http://localhost:8084"
REGISTRAR_URL="http://localhost:8083"
ADMIN_AUTH=(-H "Authorization: Bearer ${FCP_ADMIN_AUTH_TOKEN}")
EVENTS_AUTH=(-H "Authorization: Bearer ${FCP_EVENTS_AUTH_TOKEN}")
CALL_MANAGER_AUTH=(-H "Authorization: Bearer ${FCP_CALL_MANAGER_API_TOKEN}")
REGISTRAR_AUTH=(-H "Authorization: Bearer ${FCP_REGISTRAR_API_TOKEN}")
fail() { echo "FAIL: $*" >&2; exit 1; }
