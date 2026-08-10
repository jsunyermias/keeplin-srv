# `0019_drop_redundant_projection_index.sql` — remove duplicate queue index

This forward-only migration removes `projection_jobs_user_idx`. The queue primary key already
provides the identical `(user_id, batch_id, batch_index)` B-tree, so retaining both indexes adds
write amplification without enabling another access path. `IF EXISTS` keeps repeated migration
runs idempotent.

```sql
DROP INDEX IF EXISTS projection_jobs_user_idx;

```

Rollback may recreate the index, but doing so has no behavioral effect because the primary-key
index remains present throughout.
