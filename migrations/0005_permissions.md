# `0005_permissions.sql` — note capability bitset

## Complete migration

```sql
-- Front B: generalize the note-sharing model from fixed roles (editor/viewer) to a
-- capability bitset. (Notebook permissions + the notebook→note cascade land in a
-- follow-up migration.)
--
-- Capability bits (see `permissions.rs`), stored as an INTEGER bitmask:
--     READ        = 1
--     WRITE       = 2   (implies READ)
--     SHARE_READ  = 4   (implies READ)
--     SHARE_WRITE = 8   (implies SHARE_READ + WRITE)
--     MANAGE      = 16  (implies every non-owner bit)
-- The owner is separate and transferable (notes.owner_id) and always has every
-- capability plus ownership transfer + delete.

-- Add the capability bitmask alongside the legacy role, backfilled from it:
--   editor → READ|WRITE = 3, viewer → READ = 1.
ALTER TABLE note_shares
    ADD COLUMN IF NOT EXISTS capabilities INTEGER NOT NULL DEFAULT 1;

UPDATE note_shares SET capabilities = 3 WHERE role = 'editor';
UPDATE note_shares SET capabilities = 1 WHERE role = 'viewer';

-- The legacy CHECK on `role` would reject rows that only carry capabilities, so drop it;
-- `role` stays nullable for backward reads but is no longer authoritative.
ALTER TABLE note_shares DROP CONSTRAINT IF EXISTS note_shares_role_check;
ALTER TABLE note_shares ALTER COLUMN role DROP NOT NULL;

```

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
