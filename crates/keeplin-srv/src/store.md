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

## Public API (by area)

**Users**: `create_user`, `get_user_by_email`, `get_user_by_id`.
**Devices**: `create_device`, `get_device`, `list_devices_by_user`, `delete_device` (revokes a
token), `touch_device` (last-seen).
**Relay journal**: `append_changes` (dedupes by `(batch_id, batch_index)`), `changes_after`,
`get_cursor`, `advance_cursor`, `prune_delivered_changes`.
**Notes**: `create_note` (optional client id → `Conflict` on dup), `get_note`,
`list_notes_for_user` (owned + shared), `update_note_meta`, `soft_delete_note`.
**Shares**: `create_or_update_share`, `get_share`, `delete_share`.
**Lines**: `get_line`, `list_lines`, `insert_line`, `update_line`, `soft_delete_line`.
**Line order**: `get_note_order`, `set_note_order`.
**Maintenance / metrics**: `gc_line_tombstones`, `counts`.

## Database schema

Owned by the SQL migrations, documented in `migrations/*.md`:

- `users`, `user_devices` (0001) — accounts and device logins.
- `changes`, `device_cursors` (0001) — the relay journal and per-device watermarks.
- `notes`, `lines`, `note_line_order`, `note_shares` (0002) — the collaborative note model.
- `notes.notebook_id` + to-do columns (0003) — full note metadata the server stores.

## Design notes

- **Version metadata is opaque here**: `vv` columns are `JSONB` mapping device-id → counter;
  `Store` reads/writes them but the *resolution* rule (`note_log::resolve`) lives in `collab.rs`.
  The store is deliberately mechanism-free — it persists, it does not decide who wins.
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
