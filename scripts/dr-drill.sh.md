# `scripts/dr-drill.sh` — disaster-recovery restore drill

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
