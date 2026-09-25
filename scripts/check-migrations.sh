#!/bin/bash
# The migration discipline for a rolling upgrade (docs/BACKUP_AND_RESTORE.md
# §5, docs/BACKUP_RESTORE_UPGRADE.md Q5). Old and new nodes share one
# schema while an upgrade runs, so a migration may add but must not take
# away what the previous release still reads. A new migration file that
# drops a table, view or column, renames one, or changes a column's type
# fails this check unless it says why it is safe:
#
#   -- fcp: contraction: <what it removes>, read by nothing since vX.Y.Z
#
# naming the release whose nodes no longer read it — a contraction ships
# one release after the code stopped reading, never with it. Migrations
# up to 077 predate the rule and are not checked (three of them contract:
# 047, 058, 059).
#
#   scripts/check-migrations.sh              # every migration above 077
#   scripts/check-migrations.sh a.sql b.sql  # these files
#   scripts/check-migrations.sh --self-test  # the check against a drop and an add
#
# Not a contraction, and not flagged: DROP INDEX, DROP TRIGGER, DROP
# FUNCTION, DROP POLICY, DROP CONSTRAINT, DROP NOT NULL, DROP DEFAULT —
# an old binary reads through none of them. CI runs this in the
# packaging job.
set -euo pipefail

ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
MIGRATIONS="$ROOT/crates/fcp-storage-postgres/migrations"
FIRST_CHECKED=78
MARKER='-- fcp: contraction:.*v[0-9]+\.[0-9]+\.[0-9]+'
# What contracts: a table, view or column going, a rename, a type change —
# on one line with its ALTER TABLE or as a continuation line under it.
CONTRACTION='^[[:space:]]*(DROP[[:space:]]+(TABLE|VIEW|MATERIALIZED[[:space:]]+VIEW)[[:space:]]|(ALTER[[:space:]]+TABLE[[:space:]]+[^[:space:]]+[[:space:]]+)?(DROP[[:space:]]+(COLUMN[[:space:]]+)?(IF[[:space:]]+EXISTS[[:space:]]+)?[a-z_"]+[[:space:]]*[,;]?[[:space:]]*(--.*)?$|RENAME[[:space:]]|ALTER[[:space:]]+(COLUMN[[:space:]]+)?[a-z_"]+[[:space:]]+(SET[[:space:]]+DATA[[:space:]]+)?TYPE[[:space:]]))'

check() {
    local file="$1"
    local hits
    hits="$(grep -inE "$CONTRACTION" "$file" | grep -ivE 'DROP[[:space:]]+(INDEX|TRIGGER|FUNCTION|POLICY|CONSTRAINT|NOT[[:space:]]+NULL|DEFAULT)' || true)"
    [ -n "$hits" ] || return 0
    if grep -qiE -e "$MARKER" "$file"; then
        echo "ok: $(basename "$file") contracts and says so: $(grep -iE -e "$MARKER" "$file" | head -n 1 | sed 's/^[[:space:]]*//')"
        return 0
    fi
    echo "FAIL: $(basename "$file") removes or reshapes what the previous release may still read, and does not say it is safe:" >&2
    echo "$hits" | sed 's/^/    /' >&2
    echo "    A contraction ships one release after the code stopped reading it, marked:" >&2
    echo "    -- fcp: contraction: <what it removes>, read by nothing since vX.Y.Z" >&2
    return 1
}

SELF_TEST_DIR=""
self_test() {
    local dir
    dir="$(mktemp -d)"
    SELF_TEST_DIR="$dir"
    trap 'rm -rf "$SELF_TEST_DIR"' EXIT
    printf 'ALTER TABLE users ADD COLUMN nickname TEXT;\nCREATE INDEX IF NOT EXISTS users_nickname ON users (nickname);\nDROP INDEX IF EXISTS users_old;\n' > "$dir/078_add.sql"
    printf 'ALTER TABLE users DROP COLUMN nickname;\n' > "$dir/079_drop.sql"
    printf -- '-- fcp: contraction: users.nickname, read by nothing since v0.25.300\nALTER TABLE users DROP COLUMN nickname;\n' > "$dir/080_drop_marked.sql"
    printf 'ALTER TABLE users RENAME COLUMN nickname TO handle;\n' > "$dir/081_rename.sql"
    printf 'ALTER TABLE cdrs ALTER COLUMN cost TYPE NUMERIC(18,6);\n' > "$dir/082_type.sql"
    printf 'DROP TABLE IF EXISTS legacy_things;\n' > "$dir/083_drop_table.sql"
    printf 'ALTER TABLE cdrs\n    ALTER COLUMN cost TYPE NUMERIC(18,6) USING round(cost::numeric, 6),\n    ALTER COLUMN fee TYPE NUMERIC(18,6);\n' > "$dir/084_type_continued.sql"
    printf 'ALTER TABLE users\n    ADD COLUMN handle TEXT,\n    DROP COLUMN nickname;\n' > "$dir/085_drop_continued.sql"
    printf 'ALTER TABLE users DROP CONSTRAINT IF EXISTS users_nickname_key;\nALTER TABLE users ALTER COLUMN nickname DROP NOT NULL;\nALTER TABLE users ALTER COLUMN nickname DROP DEFAULT;\n' > "$dir/086_loosen.sql"
    check "$dir/078_add.sql" || { echo "self-test: an add must pass" >&2; return 1; }
    check "$dir/080_drop_marked.sql" >/dev/null || { echo "self-test: a marked contraction must pass" >&2; return 1; }
    check "$dir/086_loosen.sql" || { echo "self-test: dropping a constraint, a NOT NULL or a default must pass" >&2; return 1; }
    for f in 079_drop 081_rename 082_type 083_drop_table 084_type_continued 085_drop_continued; do
        if check "$dir/$f.sql" 2>/dev/null; then
            echo "self-test: $f must fail" >&2
            return 1
        fi
    done
    echo "self-test: an add, a loosened constraint and a marked contraction pass; a dropped column, a rename, a type change (on its line or continued) and a dropped table fail"
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
    exit $?
fi

FILES=("$@")
if [ ${#FILES[@]} -eq 0 ]; then
    for f in "$MIGRATIONS"/*.sql; do
        n="$(basename "$f" | sed -n 's/^\([0-9]\{3\}\)_.*/\1/p')"
        [ -n "$n" ] || continue
        [ "$((10#$n))" -ge "$FIRST_CHECKED" ] && FILES+=("$f")
    done
fi
if [ ${#FILES[@]} -eq 0 ]; then
    echo "no migration above $(printf '%03d' $((FIRST_CHECKED - 1))) to check"
    exit 0
fi
status=0
for f in "${FILES[@]}"; do
    check "$f" || status=1
done
exit $status
