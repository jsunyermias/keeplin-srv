# `0008_changes_history_index.sql` — per-user history indexes on the change journal

## Purpose

The eighth schema migration. Indexes the `changes` journal so the entity-history endpoints
(issue #37) are index scans instead of full journal scans.

## What it defines

`entity_history` (`GET /api/{notes,notebooks}/:id/history`) filters `changes` by `user_id` and
a JSONB-extracted entity id. Three shapes of id appear in a change payload:

| Index | Expression | Serves |
|-------|-----------|--------|
| `idx_changes_note_hist` | `(user_id, payload -> 'note' ->> 'id')` | note create/update snapshots |
| `idx_changes_notebook_hist` | `(user_id, payload -> 'notebook' ->> 'id')` | notebook create/update snapshots |
| `idx_changes_delete_hist` | `(user_id, payload ->> 'id')` | delete tombstones (top-level id) |

Without a matching index, every history read scans the user's whole journal and evaluates the
JSON extraction per row; these expression indexes turn each lookup into an index scan.

## Trade-off

Three extra index maintenances per journaled change on write. Acceptable: the history feature
needs to scale, and the relay's hot path (append + fan-out) never reads history.

Note: these `(user_id, …)` composite indexes are **superseded by 0009**, which re-scopes
history to per-entity (user-agnostic) and replaces them with expression-only indexes.

## Related files

- `../crates/keeplin-srv/src/store.rs` — `entity_history` / the history query builder.
- `../crates/keeplin-srv/src/http.rs` — the `/history` handlers.
- `0009_changes_entity_index.sql` — drops these and adds the user-agnostic replacements.
