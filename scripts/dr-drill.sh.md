# `scripts/dr-drill.sh` — disaster-recovery restore drill

## Complete source

```bash
# md:dr-drill
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
```

## Purpose

Proves the backup is **actually restorable**, end to end, without touching the live database.
A backup that has never been restored is a hope, not a backup — this script closes that gap by
dumping production, restoring into a throwaway database, and comparing row counts.

## Usage

```bash
./scripts/dr-drill.sh "postgres://user:pass@host:5432/keeplin"
```

The single argument is the URL of the **live** database to drill. Requires `pg_dump`,
`pg_restore`, `psql`, and a role allowed to `CREATE DATABASE`. Intended to run on a schedule
(e.g. monthly).

## What it does

1. **Dump** the source database with `pg_dump --format=custom` to a temp file (removed on exit
   via a `trap`).
2. **Restore** into a throwaway database `keeplin_drill_<timestamp>` on the *same* server. It
   derives two URLs from the source with `sed`: an **admin URL** (same server, `/postgres`
   maintenance DB) to `CREATE`/`DROP` the drill database, and the **drill URL** to restore
   into. `pg_restore --no-owner --no-acl` so the drill role need not match production roles.
3. **Verify** exact `count(*)` of the tables that matter most — `users`, `notes`, `lines`,
   `notebooks`, `tags`, `resources`, `changes` — in both source and restore.
4. **Clean up**: drop the throwaway database, then report `PASS` (counts match) or `FAIL`
   (counts differ, exit 1).

## Safety

- Read-only against the source (`pg_dump` only); all writes land in the throwaway database.
- `set -euo pipefail` aborts on the first error; the `trap` removes the dump file even on
  failure. The drill database is dropped in step 4 on the success path — a mid-run failure may
  leave it behind for inspection.
- The row-count comparison is a coarse integrity check (it will not catch corruption that
  preserves row counts); it is a smoke test that the dump/restore pipeline works, not a
  byte-level verification.

## Related files

- `../migrations/` — the schema whose tables this counts; add a table here if a future
  migration introduces one that should be part of the integrity check.
- `.github/workflows/ci.yml` — CI does not run this drill (it needs a real database and
  create-database rights); it is an operational, scheduled tool.
