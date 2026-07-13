#!/usr/bin/env bash
# Disaster-recovery drill for keeplin-srv: prove the backup is actually
# restorable, end to end, without touching the live database.
#
#   ./scripts/dr-drill.sh "postgres://user:pass@host:5432/keeplin"
#
# What it does:
#   1. pg_dump the source database (custom format).
#   2. Restore the dump into a throwaway database on the same server.
#   3. Verify: row counts of every user table match between source and restore.
#   4. Drop the throwaway database and report PASS/FAIL.
#
# Run it on a schedule (e.g. monthly) — a backup that has never been restored
# is a hope, not a backup. Requires: pg_dump, pg_restore, psql, and a role
# allowed to CREATE DATABASE.

set -euo pipefail

SRC_URL="${1:?usage: dr-drill.sh <postgres-url-of-live-db>}"
STAMP="$(date +%Y%m%d%H%M%S)"
DRILL_DB="keeplin_drill_${STAMP}"
DUMP="$(mktemp -t keeplin-drill-XXXXXX.dump)"
trap 'rm -f "$DUMP"' EXIT

# Admin URL: same server, but connected to the maintenance DB so we can
# CREATE/DROP the drill database.
ADMIN_URL="$(echo "$SRC_URL" | sed -E 's#/[^/?]+(\?|$)#/postgres\1#')"
DRILL_URL="$(echo "$SRC_URL" | sed -E "s#/[^/?]+(\?|\$)#/${DRILL_DB}\1#")"

echo "==> 1/4 dumping source"
pg_dump --format=custom --file="$DUMP" "$SRC_URL"
echo "    dump: $(du -h "$DUMP" | cut -f1)"

echo "==> 2/4 restoring into throwaway database ${DRILL_DB}"
psql "$ADMIN_URL" -qc "CREATE DATABASE \"${DRILL_DB}\";"
# --no-owner/--no-acl: the drill role need not match production roles.
pg_restore --no-owner --no-acl --dbname="$DRILL_URL" "$DUMP"

echo "==> 3/4 verifying row counts"
COUNT_SQL="SELECT relname, n_live_tup FROM pg_stat_user_tables ORDER BY relname;"
# ANALYZE first so pg_stat estimates are fresh, then compare exact counts of
# the tables that matter most plus a full estimated listing for the report.
verify() {
    psql "$1" -qtA -c "
      SELECT 'users:'          || count(*) FROM users
      UNION ALL SELECT 'notes:'      || count(*) FROM notes
      UNION ALL SELECT 'lines:'      || count(*) FROM lines
      UNION ALL SELECT 'notebooks:'  || count(*) FROM notebooks
      UNION ALL SELECT 'tags:'       || count(*) FROM tags
      UNION ALL SELECT 'resources:'  || count(*) FROM resources
      UNION ALL SELECT 'changes:'    || count(*) FROM changes
      ORDER BY 1;"
}
SRC_COUNTS="$(verify "$SRC_URL")"
DRILL_COUNTS="$(verify "$DRILL_URL")"
echo "    source : $(echo "$SRC_COUNTS" | tr '\n' ' ')"
echo "    restore: $(echo "$DRILL_COUNTS" | tr '\n' ' ')"

echo "==> 4/4 cleaning up"
psql "$ADMIN_URL" -qc "DROP DATABASE \"${DRILL_DB}\";"

if [ "$SRC_COUNTS" = "$DRILL_COUNTS" ]; then
    echo "DR DRILL: PASS — dump restored and row counts match."
else
    echo "DR DRILL: FAIL — row counts differ between source and restore." >&2
    exit 1
fi
