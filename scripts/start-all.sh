#!/bin/bash
# start-all.sh — Start all FCP services for single-node deployment
# Usage: ./scripts/start-all.sh
#   Stop: ./scripts/start-all.sh stop
#   Status: ./scripts/start-all.sh status

set -e

FCP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$FCP_DIR"

PUBLIC_IP="${FCP_PUBLIC_IP:-192.168.1.225}"
LOG_DIR="/tmp"

# Service definitions: name:binary:config:extra_args
SERVICES=(
  "events-server:fcp-events-server:--config fcp-events-server-local.toml"
  "admin:fcp-admin:--config fcp-admin-local.toml"
  "call-manager:fcp-call-manager:--config fcp-call-manager-local.toml --public-ip ${PUBLIC_IP}"
  "jobs:fcp-jobs:--config fcp-jobs-local.toml"
  "ivr:fcp-ivr:--config fcp-ivr-local.toml"
  "voicemail:fcp-voicemail:--config fcp-voicemail-local.toml"
  "conference:fcp-conference:--config fcp-conference-local.toml"
  "sbc:fcp-sbc:--bind 0.0.0.0:5076 --health-bind 0.0.0.0:8094 --log-level info"
  "presence:fcp-presence:--config fcp-presence-local.toml"
  "media:fcp-media:--config fcp-media-local.toml"
  "api-gateway:fcp-api-gateway:--config fcp-api-gateway-local.toml"
)

# Health check endpoints
declare -A HEALTH_ENDPOINTS=(
  [events-server]="8081:/health"
  [admin]="8080:/admin/"
  [call-manager]="8084:/health"
  [jobs]="8082:/health"
  [ivr]="8086:/health"
  [voicemail]="8088:/health"
  [conference]="8090:/health"
  [sbc]="8094:/health"
  [presence]="8092:/health"
  [media]="8083:/health"
  [api-gateway]="8000:/health"
)

pid_file() {
  echo "${LOG_DIR}/fcp-${1}.pid"
}

is_running() {
  local pf
  pf="$(pid_file "$1")"
  if [ -f "$pf" ] && kill -0 "$(cat "$pf")" 2>/dev/null; then
    return 0
  fi
  return 1
}

do_status() {
  echo "=== FCP Service Status ==="
  echo ""
  for entry in "${SERVICES[@]}"; do
    IFS=':' read -r name binary args <<< "$entry"
    local pf
    pf="$(pid_file "$name")"
    if is_running "$name"; then
      local pid
      pid=$(cat "$pf")
      printf "  %-20s PID %-8s " "$name" "$pid"

      # Health check
      local ep="${HEALTH_ENDPOINTS[$name]}"
      if [ -n "$ep" ]; then
        IFS=':' read -r port path <<< "$ep"
        local code
        code=$(curl -s -o /dev/null -w "%{http_code}" --max-time 2 "http://localhost:${port}${path}" 2>/dev/null || echo "000")
        if [ "$code" = "200" ]; then
          echo "HEALTHY"
        else
          echo "UP (HTTP $code)"
        fi
      else
        echo "UP"
      fi
    else
      printf "  %-20s %-13s STOPPED\n" "$name" ""
    fi
  done

  echo ""
  echo "=== Cluster Nodes ==="
  redis-cli keys "fcp:cluster:node:*" 2>/dev/null || echo "  Redis not available"
}

kill_orphans() {
  # Kill any FCP processes not tracked by our PID files (e.g. from a previous crashed session)
  local orphans
  orphans=$(pgrep -f "target/release/fcp-" 2>/dev/null || true)
  if [ -n "$orphans" ]; then
    local tracked_pids=""
    for entry in "${SERVICES[@]}"; do
      IFS=':' read -r name binary args <<< "$entry"
      local pf
      pf="$(pid_file "$name")"
      if [ -f "$pf" ]; then
        tracked_pids="$tracked_pids $(cat "$pf")"
      fi
    done
    for pid in $orphans; do
      if ! echo "$tracked_pids" | grep -qw "$pid"; then
        printf "  Killing orphan PID %s (%s)\n" "$pid" "$(ps -p "$pid" -o comm= 2>/dev/null || echo 'unknown')"
        kill -9 "$pid" 2>/dev/null || true
      fi
    done
    sleep 1
  fi
}

do_stop() {
  echo "=== Stopping FCP Services ==="
  # Stop in reverse order
  for (( i=${#SERVICES[@]}-1; i>=0; i-- )); do
    IFS=':' read -r name binary args <<< "${SERVICES[$i]}"
    local pf
    pf="$(pid_file "$name")"
    if is_running "$name"; then
      local pid
      pid=$(cat "$pf")
      printf "  Stopping %-20s (PID %s)... " "$name" "$pid"
      kill "$pid" 2>/dev/null
      # Wait up to 5 seconds for graceful shutdown
      for j in $(seq 1 50); do
        if ! kill -0 "$pid" 2>/dev/null; then
          break
        fi
        sleep 0.1
      done
      if kill -0 "$pid" 2>/dev/null; then
        kill -9 "$pid" 2>/dev/null
        echo "killed"
      else
        echo "stopped"
      fi
      rm -f "$pf"
    else
      printf "  %-20s already stopped\n" "$name"
    fi
  done

  # Clean up any orphans
  kill_orphans
}

do_start() {
  echo "=== Starting FCP Services ==="
  echo ""

  # --- Infrastructure ---
  echo "--- Infrastructure ---"
  printf "  PostgreSQL... "
  if docker start fcp-postgres >/dev/null 2>&1; then
    echo "started"
  else
    echo "FAILED (is Docker running?)"
    exit 1
  fi

  printf "  Redis... "
  if redis-cli ping >/dev/null 2>&1; then
    echo "OK"
  else
    echo "FAILED (start Redis first)"
    exit 1
  fi

  sleep 2

  # --- Kill orphan processes from previous sessions ---
  echo ""
  echo "--- Cleanup ---"
  kill_orphans
  rm -f /tmp/fcp-*.pid

  # --- Create data directories ---
  mkdir -p /tmp/fcp/{voicemail/{recordings,greetings},conference/{recordings,announcements},media/{recordings,prompts}}

  # --- Start services in order ---
  # Group 1: Events server (must be first — other services push events to it)
  echo ""
  echo "--- Event Bus ---"
  start_service "events-server"
  sleep 3

  # Group 2: Core services
  echo ""
  echo "--- Core Services ---"
  start_service "admin"
  start_service "call-manager"
  start_service "jobs"
  sleep 2

  # Group 3: Specialty services
  echo ""
  echo "--- Specialty Services ---"
  start_service "ivr"
  start_service "voicemail"
  start_service "conference"
  start_service "sbc"
  start_service "presence"
  start_service "media"
  sleep 2

  # Group 4: API Gateway (proxies to everything above)
  echo ""
  echo "--- API Gateway ---"
  start_service "api-gateway"
  sleep 3

  # --- Health checks ---
  echo ""
  do_status
  echo ""
  echo "Admin UI: http://${PUBLIC_IP}:8080/admin/"
  echo "Logs:     tail -f /tmp/fcp-*.log"
}

start_service() {
  local target_name="$1"
  for entry in "${SERVICES[@]}"; do
    IFS=':' read -r name binary args <<< "$entry"
    if [ "$name" != "$target_name" ]; then
      continue
    fi

    if is_running "$name"; then
      printf "  %-20s already running (PID %s)\n" "$name" "$(cat "$(pid_file "$name")")"
      return
    fi

    local bin_path="./target/release/${binary}"
    if [ ! -x "$bin_path" ]; then
      printf "  %-20s MISSING (run: cargo build --release -p %s)\n" "$name" "$binary"
      return
    fi

    RUST_LOG=info $bin_path $args > "${LOG_DIR}/fcp-${name}.log" 2>&1 &
    local pid=$!
    echo "$pid" > "$(pid_file "$name")"

    # Brief check that it didn't immediately crash
    sleep 0.3
    if kill -0 "$pid" 2>/dev/null; then
      printf "  %-20s started (PID %s)\n" "$name" "$pid"
    else
      printf "  %-20s FAILED — check %s/fcp-%s.log\n" "$name" "$LOG_DIR" "$name"
      rm -f "$(pid_file "$name")"
    fi
    return
  done
  echo "  Unknown service: $target_name"
}

# --- Main ---
case "${1:-start}" in
  start)
    do_start
    ;;
  stop)
    do_stop
    ;;
  restart)
    do_stop
    echo ""
    do_start
    ;;
  status)
    do_status
    ;;
  *)
    echo "Usage: $0 {start|stop|restart|status}"
    exit 1
    ;;
esac
