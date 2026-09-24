#!/bin/bash
# The stack, end to end: wait for every service to be healthy, then run
# each service's test. Run from the compose directory, or anywhere.
set -e
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "${SCRIPT_DIR}/.."

echo "Waiting for the stack to be healthy..."
MAX_WAIT=180
WAITED=0
SERVICES="postgres redis fcp-events-server fcp-admin fcp-api-gateway fcp-jobs fcp-call-manager fcp-conference fcp-webrtc-gw fcp-media"
while [ $WAITED -lt $MAX_WAIT ]; do
  # `docker compose ps --format json` prints one object per line on
  # current Compose and an array on older ones; slurp and flatten reads both.
  HEALTHY=$(docker compose ps --format json 2>/dev/null | jq -rs 'flatten | .[] | select(.Health == "healthy") | .Service')
  MISSING=""
  for s in $SERVICES; do echo "$HEALTHY" | grep -qx "$s" || MISSING="$MISSING $s"; done
  if [ -z "$MISSING" ]; then echo "All services are healthy."; break; fi
  echo "waiting for:$MISSING ($WAITED/$MAX_WAIT s)"
  sleep 5; WAITED=$((WAITED + 5))
done
if [ $WAITED -ge $MAX_WAIT ]; then
  echo "ERROR: services did not become healthy in time"; docker compose ps; exit 1
fi

for t in admin gateway events jobs call-manager; do
  echo; echo "====== $t"; bash "${SCRIPT_DIR}/test-${t}.sh"
done
if docker compose ps --format json 2>/dev/null | jq -rs 'flatten | .[] | .Service' | grep -qx fcp-registrar; then
  echo; echo "====== registrar"; bash "${SCRIPT_DIR}/test-registrar.sh"
fi
echo; echo "All integration tests passed."
