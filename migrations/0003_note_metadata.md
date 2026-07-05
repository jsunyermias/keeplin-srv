# `0003_note_metadata.sql` — full note metadata

## Purpose

The third schema migration. Extends `notes` with the remaining keeplin-core note fields so the
server stores the **whole note**, not just its title and lines. This is what lets a device restore
everything (notebook membership, todo state) from the server rather than only its collaborative body.

## What it defines

Adds to `notes`:

| Column | Purpose |
|--------|---------|
| `notebook_id UUID` | the notebook the note belongs to (nullable; loose notes have none) |
| `is_todo BOOLEAN` | whether the note is a todo item (default `FALSE`) |
| `todo_due TIMESTAMPTZ` | optional due date |
| `todo_completed TIMESTAMPTZ` | when it was completed (null = open) |

## Notes & gotchas

- All columns are additive and nullable / defaulted, so the migration is safe on a populated table.
- `notebook_id` is intentionally **not** a foreign key: notebooks live in keeplin-core and sync over
  the device relay (`/api/sync`), not in this schema, so the server treats the id as an opaque value.
- These fields are carried on `NotePatch` (`store.md`) and updated through the note metadata endpoints.

## Related files

- `../crates/keeplin-srv/src/store.md` — `NotePatch` / `update_note_meta`.
- `../crates/keeplin-srv/src/http.md` — the note metadata endpoints.
- `0002_collab.md` — the note/line tables these columns extend.
