# `0005_permissions.sql` — note capability bitset

## Purpose

The fifth schema migration. Generalises note sharing from the fixed `editor`/`viewer` roles to a
**capability bitset** (Front B), so a grant can carry any combination of read / write / share /
manage rather than one of two presets. Ownership stays separate and transferable
(`notes.owner_id`).

## What it changes

| Change | Why |
|--------|-----|
| `note_shares.capabilities INTEGER NOT NULL DEFAULT 1` | the new grant representation (bitmask; see `permissions.rs`) |
| backfill `capabilities` from `role` | `editor` → `READ\|WRITE` (3), `viewer` → `READ` (1), so existing shares keep their access |
| drop the `role` CHECK constraint, make `role` nullable | `role` is no longer authoritative; new rows carry only `capabilities` |

## Capability bits

`READ=1`, `WRITE=2`, `SHARE_READ=4`, `SHARE_WRITE=8`, `MANAGE=16`, stored **already
normalised** (a higher bit implies the lower ones — see `permissions.md`).

## Not here yet

Notebook permissions and the destructive notebook→note cascade are a follow-up migration; this
one covers notes only.

## Related files

- `../crates/keeplin-srv/src/permissions.rs` — the `Capabilities`/`Access` model this column backs.
- `../crates/keeplin-srv/src/store.rs` — `NoteShare`, `create_or_update_share`, `list_shares`.
- `0002_collab.sql` — the original `note_shares` table this alters.
