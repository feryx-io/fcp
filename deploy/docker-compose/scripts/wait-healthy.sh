#!/bin/bash
# Wait until every service of the stack that has a health check reports
# healthy and every other one is running (`certs` runs once and exits,
# and is not waited for), or fail after the given seconds with what
# `docker compose ps` shows (docs/NIGHTLY_STACK.md phase 2). Run from
# anywhere; needs docker (with compose).
#
#   scripts/wait-healthy.sh 300
set -euo pipefail

COMPOSE_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
DEADLINE=$(( $(date +%s) + ${1:-300} ))
cd "$COMPOSE_DIR"

while :; do
    # One line per container: service, state, health (empty without a
    # health check). Ready is healthy when it has one, running when not.
    not_ready="$(docker compose ps -a --format '{{.Service}} {{.State}} {{.Health}}' 2>/dev/null \
        | awk '$1 != "certs" && !($3 == "healthy" || ($3 == "" && $2 == "running")) { print "    " $1 ": " $2 ($3 != "" ? " (" $3 ")" : "") }')"
    if [ -z "$not_ready" ]; then
        echo "wait-healthy: every service is up"
        docker compose ps
        exit 0
    fi
    if [ "$(date +%s)" -ge "$DEADLINE" ]; then
        echo "wait-healthy: not every service came up in time:" >&2
        echo "$not_ready" >&2
        docker compose ps >&2
        exit 1
    fi
    sleep 3
done
