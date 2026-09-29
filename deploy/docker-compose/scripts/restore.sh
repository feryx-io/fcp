#!/bin/bash
# A restore of the compose stack from a backup.sh directory
# (docs/BACKUP_AND_RESTORE.md §4, docs/BACKUP_RESTORE_UPGRADE.md phase 3),
# into a fresh stack — one whose volumes do not exist, which is what
# `docker compose down -v` leaves:
#
#   docker compose down -v            # drops what is there, if anything
#   scripts/restore.sh /backups/fcp-2026-09-24
#
# In this order, refusing at the first thing wrong and touching nothing
# before it is sure:
#   1. the manifest and SHA256SUMS are read and every file's checksum
#      verified
#   2. no volume of the stack may exist: a restore never merges into a
#      running deployment
#   3. the master key in fcp-certs.tar.gz must fingerprint to the KEK id
#      the manifest's key ring names (a backup whose key and dump were
#      not taken together is refused here, before any service could)
#   4. each volume the manifest lists is created as compose would and
#      filled from its tar, the certs volume first
#   5. postgres is started alone, waited for, and the dump restored
#      into its empty `fcp` database; the key ring it holds must name
#      the same KEK id
#   6. `docker compose up -d --build`, unless --no-up
#
# The service certificates in the certs volume name the address the
# backup's host had (HOST_IP). On a host with another address, remove
# them from the volume after the restore and before `up` — not ca.* and
# not master.key — and the certs service remakes them under the kept CA.
# Redis starts empty (docs/BACKUP_AND_RESTORE.md §1 says what that means).
#
# Needs: docker (with compose), sha256sum, base64. Reads the stack's .env.
set -euo pipefail

# COMPOSE_DIR names another compose project to restore into: the site
# stack's directory (docs/SITE_SETUP.md).
COMPOSE_DIR="${COMPOSE_DIR:-$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )}"
HELPER_IMAGE="${FCP_BACKUP_HELPER_IMAGE:-alpine:3.20}"

fail() { echo "restore: $*" >&2; exit 1; }
say() { echo "restore: $*"; }

UP=1
BACKUP=""
for arg in "$@"; do
    case "$arg" in
        --no-up) UP=0 ;;
        -*) fail "unknown option $arg (usage: $0 <directory> [--no-up])" ;;
        *) [ -z "$BACKUP" ] || fail "usage: $0 <directory> [--no-up]"; BACKUP="$arg" ;;
    esac
done
[ -n "$BACKUP" ] || fail "usage: $0 <directory> [--no-up]"
[ -d "$BACKUP" ] || fail "$BACKUP is not a directory"
BACKUP="$( cd "$BACKUP" && pwd )"
command -v docker >/dev/null || fail "docker is not on PATH"
command -v sha256sum >/dev/null || fail "sha256sum is not on PATH"
command -v base64 >/dev/null || fail "base64 is not on PATH"

# 1. The manifest, and every file as it was written.
[ -f "$BACKUP/manifest" ] || fail "$BACKUP has no manifest; it is not a backup.sh directory"
manifest() { sed -n "s/^$1 = *//p" "$BACKUP/manifest" | head -n 1; }
[ "$(manifest fcp_backup)" = "1" ] || fail "the manifest is not one this script reads (fcp_backup = $(manifest fcp_backup))"
KEK_ID="$(manifest kek_id)"
VOLUMES="$(manifest volumes)"
DATABASE="$(manifest database)"
[ -n "$DATABASE" ] && [ -f "$BACKUP/$DATABASE" ] || fail "the manifest names no database dump that exists"
say "backup of release $(manifest stack_version), taken $(manifest taken_at) from project $(manifest project)"
say "verifying checksums"
( cd "$BACKUP" && sha256sum -c --quiet SHA256SUMS ) || fail "a file does not match SHA256SUMS; this copy of the backup is damaged"
case " $VOLUMES " in
    *" fcp-certs "*) ;;
    *) fail "the manifest lists no fcp-certs volume; a backup without the master key cannot be restored" ;;
esac
for vol in $VOLUMES; do
    [ -f "$BACKUP/$vol.tar.gz" ] || fail "the manifest lists $vol but $vol.tar.gz is missing"
done

cd "$COMPOSE_DIR"
if [ -f .env ]; then
    set -a
    # shellcheck disable=SC1091
    . ./.env
    set +a
fi
PROJECT="$(docker compose config 2>/dev/null | sed -n 's/^name: //p' | head -n 1)"
[ -n "$PROJECT" ] || fail "docker compose config did not print the project name; run from a checkout with the compose file"

# 2. A fresh stack: none of its volumes may exist.
EXISTING="$(docker volume ls --format '{{.Name}}' --filter "label=com.docker.compose.project=$PROJECT")"
[ -z "$EXISTING" ] || fail "the stack has volumes ($(echo "$EXISTING" | tr '\n' ' ')); a restore goes into a fresh stack — \`docker compose down -v\` drops them, and everything in them"

# 3. The key in the backup is the key the dump's key ring names.
KEY_B64="$(tar -xzOf "$BACKUP/fcp-certs.tar.gz" ./master.key 2>/dev/null || true)"
[ -n "$KEY_B64" ] || fail "fcp-certs.tar.gz holds no master.key"
KEY_FINGERPRINT="local:$(printf '%s' "$KEY_B64" | tr -d '[:space:]' | base64 -d | sha256sum | cut -c1-16)"
case "$KEK_ID" in
    "") say "the key ring was empty when the backup was taken (encryption had not run); the key restores unchecked" ;;
    local:*)
        [ "$KEY_FINGERPRINT" = "$KEK_ID" ] \
            || fail "the master key in fcp-certs.tar.gz is $KEY_FINGERPRINT but the manifest's key ring names $KEK_ID; this key cannot open this dump's secrets, and no service would start with it"
        say "the master key is the one the key ring names ($KEK_ID)"
        ;;
    *) say "the key ring names $KEK_ID, not a local key; the master key restores unchecked" ;;
esac

# 4. The volumes, as compose names and labels them, filled from their tars.
for vol in $VOLUMES; do
    name="${PROJECT}_${vol}"
    say "restoring volume $vol ($name)"
    docker volume create \
        --label "com.docker.compose.project=$PROJECT" \
        --label "com.docker.compose.volume=$vol" \
        "$name" >/dev/null
    docker run --rm -v "$name:/dst" -v "$BACKUP:/backup:ro" "$HELPER_IMAGE" \
        tar -C /dst -xzf "/backup/$vol.tar.gz"
done

# 5. The database, into postgres started alone.
say "starting postgres"
docker compose up -d --no-build postgres >/dev/null
# Over TCP: on an empty volume the image first runs a temporary server on
# the socket alone for its init scripts, then restarts; a socket check
# passes against that server and the restore then finds it gone.
ready() { docker compose exec -T postgres pg_isready -h 127.0.0.1 -U fcp -d fcp >/dev/null 2>&1; }
for _ in $(seq 1 60); do
    if ready; then break; fi
    sleep 1
done
ready || fail "postgres did not become ready"
say "restoring the database from $DATABASE"
docker compose exec -T postgres pg_restore -U fcp -d fcp --no-owner --no-privileges --exit-on-error < "$BACKUP/$DATABASE" \
    || fail "pg_restore failed; the database is partial — docker compose down -v and start over"
RING="$(docker compose exec -T postgres psql -U fcp -d fcp -tA \
    -c "SELECT kek_id FROM data_encryption_keys WHERE active ORDER BY created_at DESC LIMIT 1" 2>/dev/null || true)"
RING="${RING//[$'\r\n']/}"
[ "$RING" = "$KEK_ID" ] || fail "the restored key ring names KEK '$RING' but the manifest says '$KEK_ID'; the dump and the manifest are not from one backup"

# 6. Up.
if [ "$UP" = 1 ]; then
    say "starting the stack"
    docker compose up -d --build
    say "done; the stack is up from the backup of $(manifest taken_at)"
else
    say "done; the volumes and the database are restored, start with: docker compose up -d --build"
fi
