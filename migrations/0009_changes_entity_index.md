# `0009_changes_entity_index.sql` — re-index history per entity (user-agnostic)

## Purpose

The ninth schema migration. Re-scopes entity history from per-user to **per-entity** (issue
#27) and swaps the journal indexes to match.

## What it changes

History is no longer scoped to the requesting user: a note or notebook has **one timeline**
that every user with read access sees, so `entity_history` now matches by entity id **across
all users' journal rows** (authorization happens first, in the HTTP handler). The `(user_id,
<expr>)` composite indexes from 0008 no longer serve this user-agnostic lookup, so they are
replaced with expression-only indexes on the extracted entity id:

| Dropped (0008) | Added (0009) | Expression |
|----------------|--------------|-----------|
| `idx_changes_note_hist` | `idx_changes_note_id` | `(payload -> 'note' ->> 'id')` |
| `idx_changes_notebook_hist` | `idx_changes_notebook_id` | `(payload -> 'notebook' ->> 'id')` |
| `idx_changes_delete_hist` | `idx_changes_top_id` | `(payload ->> 'id')` |

Forward-only and idempotent: `DROP INDEX IF EXISTS` the three 0008 indexes, then
`CREATE INDEX IF NOT EXISTS` the three replacements.

## Related files

- `../crates/keeplin-srv/src/store.rs` — `entity_history`, now keyed by entity id alone.
- `../crates/keeplin-srv/src/http.rs` — the `/history` handlers that authorize read access
  before querying (the authorization the dropped `user_id` key used to imply).
- `0008_changes_history_index.sql` — the per-user indexes this supersedes.
