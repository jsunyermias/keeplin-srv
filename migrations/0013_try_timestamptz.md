# `0013_try_timestamptz.sql` — safe text→timestamptz cast for the history access window

## What it does

Creates `keeplin_try_timestamptz(text) RETURNS timestamptz`: the value parsed as a
`timestamptz`, or `NULL` if the cast raises (malformed string, `NULL` input propagates as
`NULL`). `STABLE PARALLEL SAFE` plpgsql with an exception handler.

## Why

Closes the `HISTORY_VISIBILITY=access` loophole (see `../crates/keeplin-srv/src/store.md`,
`entity_history`): the collaborator access window is now compared against the **payload's own
causal timestamp** (`payload->'note'/'notebook'->>'updated_at'`, or `payload->>'deleted_at'`
for tombstones) instead of the journal row's `received_at`. Journal re-delivery — a
reinstalled client re-pushing its journal from epoch — creates fresh rows with fresh
`received_at`, which defeated the old filter and leaked pre-access versions to collaborators.

The payload is client-supplied opaque JSON. A bare `(…)::timestamptz` cast inside the history
query would make a single malformed `updated_at` (accidental or malicious) turn **every**
history read for that entity into a 500 — a denial of service on the endpoint. Wrapping the
cast in an exception-handling function degrades a malformed value to `NULL`; the query then
`COALESCE`s to `received_at`, which is exactly the fallback the displayed version timestamp
has always used for old payloads without one.

## Index note

No index change: the history query is driven by the 0009 expression indexes on the extracted
entity id (`idx_changes_note_id` / `idx_changes_notebook_id` / `idx_changes_top_id`), which
remain the selective filter. The timestamp comparison only post-filters the (small) per-entity
row set, so an expression index on `keeplin_try_timestamptz(...)` would buy nothing.

## Forward-only

Like every migration here: never edit it after it has been applied anywhere; correct with a
new migration. `CREATE OR REPLACE` makes a re-run idempotent in fresh test databases.

## Related files

- `../crates/keeplin-srv/src/store.rs` — `entity_history`, the only caller.
- `../crates/keeplin-srv/src/http.rs` — computes the access cutoff passed down.
- `0009_changes_entity_index.sql` — the entity-id indexes that keep the query fast.
- `SECURITY.md` — the honest residual limit: `updated_at` is client-asserted.
