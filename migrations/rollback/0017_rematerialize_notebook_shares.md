# `0017_rematerialize_notebook_shares.sql` — emergency legacy grant rematerialization

## Complete rollback

```sql
INSERT INTO note_shares (note_id, user_id, capabilities)
SELECT n.id, nbs.user_id, nbs.capabilities
FROM notes n
JOIN notebook_shares nbs ON nbs.notebook_id = n.notebook_id
WHERE n.deleted_at IS NULL
ON CONFLICT (note_id, user_id) DO UPDATE
SET capabilities = note_shares.capabilities | EXCLUDED.capabilities;

```

## What it does

Provides an operator-invoked compatibility rollback for returning to a server that expects notebook grants to be materialized into each live child note. Existing direct grants are preserved and unioned with inherited capability bits on key conflicts.

## Dependencies and invariants

- `notes(id, notebook_id, deleted_at)` — selects live child notes; expects deleted notes to remain untouched.
- `notebook_shares(notebook_id, user_id, capabilities)` — supplies legacy inherited grants; expects each notebook/principal pair to be unique.
- `note_shares(note_id, user_id, capabilities)` — receives materialized rows; expects its conflict key to be `(note_id, user_id)`.
- PostgreSQL bitwise `|` — preserves direct capability bits while adding inherited bits; expects capability values to remain integer bitmasks.

This operation is idempotent for unchanged source rows, but intentionally loses provenance: after unioning bits, an older server cannot distinguish direct authority from inherited authority. Run it only as part of a coordinated application rollback, after preserving a database backup and the audit output from migration 0017.

## Forward recovery

Re-deploy the ADR 0001 implementation and recompute authorization from current `notebook_shares`; do not delete ambiguous `note_shares` automatically. Operators must reconcile direct-grant intent using the audit evidence.
