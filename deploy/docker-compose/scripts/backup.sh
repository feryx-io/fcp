#!/bin/bash
# A backup of the compose stack (docs/BACKUP_AND_RESTORE.md §3,
# docs/BACKUP_RESTORE_UPGRADE.md phase 2), into a directory the operator
# names, which must not exist yet or must be empty:
#
#   scripts/backup.sh /backups/fcp-2026-09-24
#
# What it takes, in this order:
#   1. fcp.dump        — pg_dump of the `fcp` database in custom format,
#                        through the `postgres` container (its own pg_dump,
#                        so the client always matches the server)
#   2. <volume>.tar.gz — every stateful volume, from the certs volume with
#                        the master key to each node's recordings and
#                        spool, each a tar of the volume's root, ownership
#                        kept; a volume that does not exist (its service
#                        never ran) is named in the manifest, not an error
#   3. manifest        — the release that took it, the project name, the
#                        KEK id the key ring names, the time, the volumes
#   4. SHA256SUMS      — every file's checksum, `sha256sum -c` verifiable
#
# The dump is one consistent snapshot; the volumes are copied after it,
# live, so a recording still being written at that instant is partial in
# the backup and a file finished after the dump has no row. For a backup
# with neither, `docker compose stop` the services first (not `postgres`)
# — see the document. Redis is not in the backup (docs/BACKUP_AND_RESTORE.md
# §1 says what that loses). Restore with scripts/restore.sh.
#
# Needs: docker (with compose), sha256sum. Reads the stack's .env for the
# project name it may set; the database password is not needed, pg_dump
# runs inside the container over its socket.
set -euo pipefail

# COMPOSE_DIR names another compose project to back up: the site stack's
# directory (docs/SITE_SETUP.md), whose volumes are a subset of the list.
COMPOSE_DIR="${COMPOSE_DIR:-$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )}"
HELPER_IMAGE="${FCP_BACKUP_HELPER_IMAGE:-alpine:3.20}"
# The volumes a restore needs, and the ones it does not: postgres-data is
# the dump, tts-cache and redis-data are re-derivable, prometheus-data and
# grafana-data are the monitoring stack's own. link-data is the site
# stack's link (its identity and cursors); the cloud stack has none.
VOLUMES="fcp-certs call-manager-data admin-data voicemail-data conference-recordings conference-cdr-spool media-recordings media-prompts link-data"

fail() { echo "backup: $*" >&2; exit 1; }
say() { echo "backup: $*"; }

[ $# -eq 1 ] || fail "usage: $0 <directory>"
OUT="$1"
if [ -e "$OUT" ]; then
    [ -d "$OUT" ] || fail "$OUT exists and is not a directory"
    [ -z "$(ls -A "$OUT")" ] || fail "$OUT is not empty; a backup goes in a directory of its own"
else
    mkdir -p "$OUT"
fi
OUT="$( cd "$OUT" && pwd )"
# A run that stops early leaves what it wrote; a rerun refuses the
# directory until it is removed, so say so.
trap '[ $? -eq 0 ] || echo "backup: incomplete; remove $OUT before trying again" >&2' EXIT
command -v docker >/dev/null || fail "docker is not on PATH"
command -v sha256sum >/dev/null || fail "sha256sum is not on PATH"

cd "$COMPOSE_DIR"
if [ -f .env ]; then
    set -a
    # shellcheck disable=SC1091
    . ./.env
    set +a
fi
# The project name decides every volume's real name (<project>_<volume>);
# `docker compose config` prints the one in force, whether it came from the
# directory, .env or the environment.
PROJECT="$(docker compose config 2>/dev/null | sed -n 's/^name: //p' | head -n 1)"
[ -n "$PROJECT" ] || fail "docker compose config did not print the project name; run from a checkout with the compose file"

# The release: the admin binary of the image the stack was built from,
# asked directly, so the manifest names what runs, not what the checkout
# says. A stack never built has no image, and then the checkout's version
# is what a restore will build, and is taken instead.
# The running admin first (the stack is up for a backup), else a fresh
# container of its image; the checkout is found from this script, not from
# COMPOSE_DIR, which may be the site's project a level deeper.
REPO_ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )/../../.." && pwd )"
if VERSION_LINE="$(docker compose exec -T fcp-admin /app/fcp-admin --version 2>/dev/null)" \
    || VERSION_LINE="$(docker compose run --rm --no-deps --no-build -T --entrypoint /app/fcp-admin fcp-admin --version 2>/dev/null)"; then
    STACK_VERSION="${VERSION_LINE##* }"
    VERSION_FROM="image"
else
    STACK_VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO_ROOT/Cargo.toml" | head -n 1)"
    VERSION_FROM="checkout"
fi
[ -n "$STACK_VERSION" ] || fail "could not find the stack's version"

docker compose ps --status running postgres 2>/dev/null | grep -q postgres \
    || fail "the postgres service is not running; a backup needs it up (docker compose up -d postgres)"

TAKEN_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
say "into $OUT (project $PROJECT, release $STACK_VERSION from the $VERSION_FROM)"

say "dumping the database"
docker compose exec -T postgres pg_dump -U fcp -d fcp -Fc --no-owner --no-privileges > "$OUT/fcp.dump"
[ -s "$OUT/fcp.dump" ] || fail "pg_dump wrote nothing"

# The key ring's active KEK id: what the master key in fcp-certs must
# fingerprint to, and what a restore checks before it starts anything.
# Empty when no service has run with encryption on (docs/ENCRYPTION.md).
KEK_ID="$(docker compose exec -T postgres psql -U fcp -d fcp -tA \
    -c "SELECT kek_id FROM data_encryption_keys WHERE active ORDER BY created_at DESC LIMIT 1" 2>/dev/null || true)"
KEK_ID="${KEK_ID//[$'\r\n']/}"

TAKEN=""
ABSENT=""
for vol in $VOLUMES; do
    name="$(docker volume ls --format '{{.Name}}' \
        --filter "label=com.docker.compose.project=$PROJECT" \
        --filter "label=com.docker.compose.volume=$vol")"
    if [ -z "$name" ]; then
        say "volume $vol does not exist (its service has not run); skipped"
        ABSENT="$ABSENT $vol"
        continue
    fi
    say "copying volume $vol ($name)"
    docker run --rm -v "$name:/src:ro" -v "$OUT:/out" "$HELPER_IMAGE" \
        tar -C /src -czf "/out/$vol.tar.gz" .
    TAKEN="$TAKEN $vol"
done
case "$TAKEN" in
    *" fcp-certs"*) ;;
    *) fail "the fcp-certs volume (the master key) was not found; a backup without it is a backup of secrets nobody can open" ;;
esac

cat > "$OUT/manifest" <<MANIFEST
fcp_backup = 1
taken_at = $TAKEN_AT
stack_version = $STACK_VERSION
stack_version_from = $VERSION_FROM
project = $PROJECT
database = fcp.dump
kek_id = $KEK_ID
volumes =${TAKEN}
volumes_absent =${ABSENT}
MANIFEST

( cd "$OUT" && sha256sum fcp.dump manifest *.tar.gz > SHA256SUMS )
say "done: $(ls -1 "$OUT" | wc -l) files, $(du -sh "$OUT" | cut -f1), KEK ${KEK_ID:-none}"
