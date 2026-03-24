#!/bin/bash
# rebuild.sh — Stop all services, clean build, rebuild release, restart
# Usage: ./scripts/rebuild.sh
#   Skip clean: ./scripts/rebuild.sh --no-clean

set -e

FCP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$FCP_DIR"

CLEAN=true
if [[ "$1" == "--no-clean" ]]; then
  CLEAN=false
  shift
fi

echo "=== FCP Rebuild ==="
echo ""

# 1. Stop all services
echo "[1/4] Stopping services..."
./scripts/start-all.sh stop 2>/dev/null || true
sleep 2
echo ""

# 2. Clean (optional)
if $CLEAN; then
  echo "[2/4] Cleaning build artifacts..."
  cargo clean
  echo ""
else
  echo "[2/4] Skipping clean (--no-clean)"
  echo ""
fi

# 3. Build release
echo "[3/4] Building release..."
BUILD_START=$(date +%s)
cargo build --release 2>&1
BUILD_END=$(date +%s)
BUILD_SECS=$((BUILD_END - BUILD_START))
echo ""
echo "Build completed in ${BUILD_SECS}s"
echo ""

# 4. Start all services
echo "[4/4] Starting services..."
./scripts/start-all.sh start
echo ""
echo "=== Rebuild complete ==="
