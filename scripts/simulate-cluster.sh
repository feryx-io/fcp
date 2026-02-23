#!/usr/bin/env bash
# Simulate a full FCP cluster by writing heartbeat keys to Redis.
# This keeps heartbeats alive (refreshed every 5s, TTL 15s) until Ctrl+C.
#
# Usage: ./scripts/simulate-cluster.sh

set -e

REDIS_CMD="docker exec fcp-redis redis-cli"
PREFIX="fcp:"
TTL=15
NOW=$(date +%s)

# Node definitions: node_id | node_type | http_addr | sip_addr | capabilities | active_calls | version
NODES=(
  "sbc-edge-01|sbc|10.0.1.10:8085|10.0.1.10:5060|sbc,trunking,topology_hiding|23|0.16.2"
  "events-server-01|events_server|10.0.1.20:8081|10.0.1.20:5060|event_ingest,websocket|0|0.16.2"
  "ivr-01|ivr|10.0.1.30:8086|10.0.1.30:5060|ivr,flow_engine,tts|3|0.16.2"
  "voicemail-01|voicemail_server|10.0.1.40:8087|10.0.1.40:5060|voicemail,mwi|1|0.16.2"
  "presence-01|presence_server|10.0.1.50:8088|10.0.1.50:5060|presence,blf|0|0.16.2"
  "conference-01|conference_server|10.0.1.60:8089|10.0.1.60:5060|conference,mixing|2|0.16.2"
  "media-01|media_server|10.0.1.70:8090|10.0.1.70:5060|rtp_relay,transcoding,recording|8|0.16.2"
  "webrtc-gw-01|webrtc_gateway|10.0.1.80:8091|10.0.1.80:443|webrtc,oap_session|4|0.16.2"
  "registrar-01|registrar|10.0.1.90:8092|10.0.1.90:5060|registrar,location_service|0|0.16.2"
)

write_heartbeats() {
  local now
  now=$(date +%s)
  for entry in "${NODES[@]}"; do
    IFS='|' read -r nid ntype http sip caps calls ver <<< "$entry"
    # Convert comma-separated capabilities to JSON array
    local cap_json
    cap_json=$(echo "$caps" | sed 's/,/","/g')
    local json
    json=$(printf '{"node_id":"%s","node_type":"%s","http_address":"%s","sip_address":"%s","capabilities":["%s"],"active_calls":%s,"started_at":%s,"version":"%s"}' \
      "$nid" "$ntype" "$http" "$sip" "$cap_json" "$calls" "$((now - RANDOM % 86400))" "$ver")
    $REDIS_CMD SET "${PREFIX}cluster:node:${nid}" "$json" EX $TTL > /dev/null
  done
}

echo "Simulating ${#NODES[@]} cluster nodes (+ cm-local-01 already running)..."
echo "Press Ctrl+C to stop."
echo ""

# List nodes being simulated
for entry in "${NODES[@]}"; do
  IFS='|' read -r nid ntype _ _ caps calls _ <<< "$entry"
  printf "  %-25s %-20s calls=%-3s caps=%s\n" "$nid" "$ntype" "$calls" "$caps"
done
echo ""

trap 'echo "Stopping simulation..."; exit 0' INT TERM

while true; do
  write_heartbeats
  echo "[$(date +%H:%M:%S)] Heartbeats refreshed (${#NODES[@]} nodes)"
  sleep 5
done
