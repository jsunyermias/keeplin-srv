# `0017_direct_note_shares.sql` — audit ambiguous materialized note grants

## Complete migration

```sql
DO $migration$
DECLARE
    row_record RECORD;
BEGIN
    FOR row_record IN
        SELECT ns.note_id, ns.user_id, ns.capabilities,
               CASE
                   WHEN n.notebook_id IS NULL THEN 'inbox'
                   ELSE 'matches_containing_notebook_share'
               END AS attribution
        FROM note_shares ns
        JOIN notes n ON n.id = ns.note_id
        WHERE n.notebook_id IS NULL
           OR EXISTS (
               SELECT 1
               FROM notebook_shares nbs
               WHERE nbs.notebook_id = n.notebook_id
                 AND nbs.user_id = ns.user_id
                 AND nbs.capabilities = ns.capabilities
           )
        ORDER BY ns.note_id, ns.user_id
    LOOP
        RAISE NOTICE 'unattributable note share: note_id=%, user_id=%, capabilities=%, kind=%',
            row_record.note_id,
            row_record.user_id,
            row_record.capabilities,
            row_record.attribution;
    END LOOP;
END
$migration$;

```

## What it does

Audits legacy `note_shares` rows whose provenance is ambiguous under ADR 0001: Inbox grants and grants identical to a containing notebook grant. It emits a deterministic notice for operator review and deliberately preserves every row because destructive attribution cannot be proven from the old schema.

## Dependencies and invariants

- `note_shares(note_id, user_id, capabilities)` — supplies candidate direct grants; expects existing rows to remain unchanged.
- `notes(id, notebook_id)` — distinguishes Inbox and contained notes; expects `NULL` to mean Inbox.
- `notebook_shares(notebook_id, user_id, capabilities)` — detects exact legacy parent/child matches; expects equality to identify ambiguity, not ownership.
- `RAISE NOTICE` — exposes each ambiguous row without aborting migration; expects operators to retain migration logs when reconciliation is required.

The migration is forward-only, read-only, and safe to run repeatedly: it creates no schema or data changes. Runtime authorization now computes notebook inheritance while `note_shares` remains exclusively direct grants.

## Recovery

No rollback is needed for the audit itself because it mutates nothing. The companion rollback script is an explicit emergency compatibility operation that can rematerialize parent grants for an older server; it is not part of normal forward migration.
