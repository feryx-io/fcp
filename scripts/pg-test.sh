#!/usr/bin/env bash
# Run the PostgreSQL integration tests against a throwaway Docker container.
#
#   scripts/pg-test.sh            # run crates/fcp-storage-postgres/tests/integration.rs
#   scripts/pg-test.sh --keep     # leave the container running afterwards
#
# Without Docker, point FCP_TEST_DATABASE_URL at any PostgreSQL superuser
# connection and run:
#   cargo test -p fcp-storage-postgres --test integration
set -euo pipefail

NAME=fcp-pg-test
PORT=${FCP_PG_TEST_PORT:-55432}
IMAGE=${FCP_PG_TEST_IMAGE:-postgres:16}
KEEP=0
[[ "${1:-}" == "--keep" ]] && KEEP=1

if ! command -v docker >/dev/null; then
  echo "docker not found. Set FCP_TEST_DATABASE_URL to a PostgreSQL superuser URL and run:" >&2
  echo "  cargo test -p fcp-storage-postgres --test integration" >&2
  exit 1
fi

if ! docker ps --format '{{.Names}}' | grep -qx "$NAME"; then
  docker run --rm -d --name "$NAME" -p "$PORT:5432" -e POSTGRES_PASSWORD=postgres "$IMAGE" >/dev/null
  for _ in $(seq 1 30); do
    docker exec "$NAME" pg_isready -U postgres >/dev/null 2>&1 && break
    sleep 1
  done
fi

cleanup() { [[ $KEEP -eq 1 ]] || docker rm -f "$NAME" >/dev/null 2>&1 || true; }
trap cleanup EXIT

export FCP_TEST_DATABASE_URL="postgresql://postgres:postgres@localhost:$PORT/postgres"
cargo test -p fcp-storage-postgres --test integration "${@:2}"
