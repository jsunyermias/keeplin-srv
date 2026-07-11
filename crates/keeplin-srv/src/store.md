# `store.rs` — the PostgreSQL data-access layer

## Purpose

The single place any SQL lives. `Store` wraps a `sqlx::PgPool` and exposes typed async methods
for every entity: users, devices, the collaborative note model (notes, lines, line order,
shares), and the relay journal (changes, delivery cursors), plus maintenance queries. Handlers
and the two WebSocket engines call `Store`; nothing else touches the database.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Store` | struct | holds the `PgPool`; all methods live here |
| `User`, `UserDevice`, `Note`, `NoteShare` | struct (`FromRow`) | row mappings |
| `Line`, `NoteOrder` | struct | a collaborative line and a note's versioned line order |
| `NotePatch` | struct | partial note-metadata update (absent = unchanged, `Some(None)` = clear) |
| `ChangeRow` | struct | one relay-journal row fetched for delivery |
| `Notebook`, `Tag`, `ResourceMeta` | struct (`FromRow`) | REST row mappings for the materialised domain entities |
| `incoming_wins` (fn) | free fn | wraps keeplin-core's `note_log::resolve` so materialisation picks the same winner as clients |

## Public API (by area)

**Users**: `create_user`, `get_user_by_email`, `get_user_by_id`.
**Devices**: `create_device`, `get_device`, `list_devices_by_user`, `delete_device` (revokes a
token), `touch_device` (last-seen).
**Relay journal**: `append_changes` (dedupes by `(batch_id, batch_index)`), `changes_after`,
`get_cursor`, `advance_cursor`, `prune_delivered_changes`.
**Notes**: `create_note` (optional client id → `Conflict` on dup), `get_note`,
`list_notes_for_user` (owned + shared), `update_note_meta`, `soft_delete_note`.
**Shares** (capability bitset, `permissions.md`): `create_or_update_share`, `get_share`, `list_shares`, `delete_share`, `set_note_owner`.
**Notebook permissions**: `notebook_owner`, `set_notebook_owner`, `create_or_update_notebook_share`/`get_notebook_share`/`list_notebook_shares`/`delete_notebook_share`, and the destructive cascade (`cascade_notebook_to_notes`, `apply_notebook_shares_to_note`) that replaces child notes' `note_shares` with the notebook's grants on a notebook-perm change or a note move.
**Lines**: `get_line`, `list_lines`, `insert_line`, `update_line`, `soft_delete_line`.
**Line order**: `get_note_order`, `set_note_order`.
**Domain entities** (materialised from the relay, server = truth): `upsert_notebook` / `delete_notebook`,
`upsert_tag` / `delete_tag`, `upsert_note_tag` (add/remove), `upsert_resource_meta` / `delete_resource`,
`put_resource_blob` / `get_resource_blob` / `resource_owned_by`, and the reads `list_notebooks`,
`list_tags`, `list_resources`, `list_note_tag_ids`. Each write resolves via `incoming_wins` under a
`SELECT … FOR UPDATE` lock.
**Quotas**: `user_blob_bytes_excluding` (total blob bytes minus one resource), `count_live_notes_for_user`.
**Maintenance / metrics**: `gc_line_tombstones`, `counts`.

## Database schema

Owned by the SQL migrations, documented in `migrations/*.md`:

- `users`, `user_devices` (0001) — accounts and device logins.
- `changes`, `device_cursors` (0001) — the relay journal and per-device watermarks.
- `notes`, `lines`, `note_line_order`, `note_shares` (0002) — the collaborative note model.
- `notes.notebook_id` + to-do columns (0003) — full note metadata the server stores.
- `notebooks`, `tags`, `note_tags`, `resources`, `resource_blobs` (0004) — the domain entities the
  server materialises from the relay so it is their source of truth.

## Design notes

- **Resolution**: for the collaborative line/order rows the store is mechanism-free — `vv` columns
  are `JSONB` it reads/writes, and `collab.rs` decides who wins. For the domain entities materialised
  from the relay (notebooks/tags/associations/resources) the store *does* resolve, via `incoming_wins`
  (a thin wrapper over keeplin-core's `note_log::resolve`), under a `SELECT … FOR UPDATE` lock so
  concurrent updates to one entity serialise. Each such id is created on a single device, so the
  not-yet-present branch cannot race another creator.
- `create_note` accepts a client-supplied id so a daemon uploading a local note keeps the same
  id; a duplicate maps to `AppError::Conflict` via the unique-violation branch.
- `update_note_meta` uses `COALESCE`/`CASE` so an absent field is untouched while an explicit
  null clears a nullable column — the semantics `NotePatch` encodes.
- `gc_line_tombstones` deletes long-dead lines **and** drops their ids from each note's
  `order_json`, leaving the order's version metadata untouched (compaction is not an edit).

## Related files

- `migrations/*.md` — the schema these queries assume.
- `collab.md` — the version-vector resolution layered on the line/order rows.
- `sync.md` — the relay journal consumer.
