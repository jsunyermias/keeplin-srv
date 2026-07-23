# `0016_resource_note_id.sql` — owning note on resources

## What it does

Adds one column to `resources` plus a supporting index:

```sql
ALTER TABLE resources
    ADD COLUMN IF NOT EXISTS note_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001';
CREATE INDEX IF NOT EXISTS idx_resources_note ON resources(user_id, note_id);
```

It mirrors `keeplin-core`'s `Resource.note_id` (issue #125): every attachment belongs to
**exactly one note**. `note_id` is a plaintext id (like `notebook_id`), never encrypted, so the
server can filter attachments by note without decrypting anything.

## Why the sentinel default

The default is the reserved **system sentinel** `00000000-0000-0000-0000-000000000001`
(`SYSTEM_RESOURCE_NOTE_ID` in the model — the "system resource, not a user note" marker;
`Uuid::nil()` is already the Inbox and is deliberately not reused). Using a valid non-nil
default means:

- Existing rows get a valid value, and Postgres (>= 11) applies `NOT NULL DEFAULT` as a
  metadata-only change — **no table rewrite**. A bare `NOT NULL` would fail on a populated
  table (this is the C4 fix from the issue).
- System resources (vCard contacts / iCal events produced by `interop.rs`) legitimately carry
  the sentinel and stay **out of per-note listings** (a per-note query filters by a real note
  id, which never equals the sentinel).

`note_id` is **immutable** after insert: `upsert_resource_meta` writes it only in the `INSERT`
and never in the `ON CONFLICT DO UPDATE` (attachments are not reparented).

## Forward-only

Like every migration here: never edit it after it has been applied anywhere; correct with a
new migration. `ADD COLUMN IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS` make a re-run
idempotent.

## Related files

- `../crates/keeplin-srv/src/store.rs` — `ResourceMeta` row struct, `upsert_resource_meta`
  (persists `note_id` on insert, immutable on conflict), `list_resources`,
  `list_resources_for_note`, and `cascade_resources_note_deleted` / `_restored`.
- `../crates/keeplin-srv/src/sync.rs` — `materialize` of `ResourceCreate` forwards the whole
  core `Resource` (including `note_id`) to `upsert_resource_meta`.
- `../crates/keeplin-srv/src/http.rs` — `list_resources` accepts an optional `note_id` query
  parameter and delegates to `list_resources_for_note` when present.
