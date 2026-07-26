# `0002_collab.sql` — the collaborative note model

## Complete migration

```sql
-- Collaborative line editing (see the design doc in the PR / README):
-- notes are lists of independently versioned lines; the order of lines is
-- itself a versioned entity; everything soft-deletes with tombstones and
-- resolves through version vectors, exactly like keeplin-core entities.

-- Presence needs a human-readable name per user.
ALTER TABLE users ADD COLUMN IF NOT EXISTS display_name TEXT NOT NULL DEFAULT '';

-- Note metadata. The body does NOT live here: it is materialised from the
-- live lines (joined with '\n') for non-collaborative reads.
CREATE TABLE IF NOT EXISTS notes (
    id UUID PRIMARY KEY,
    title TEXT NOT NULL,
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ
);

-- One row per line. A line is an independent versioned entity with
-- soft-delete: `deleted_at` set means tombstone, kept for convergence.
-- `content` never contains a newline.
CREATE TABLE IF NOT EXISTS lines (
    id UUID PRIMARY KEY,
    note_id UUID NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    vv JSONB NOT NULL DEFAULT '{}',
    last_writer TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lines_note ON lines(note_id);

-- The order of a note's lines — its own versioned entity (NoteLines in the
-- design). `order_json` holds every LineId, tombstoned lines included, until
-- garbage collection.
CREATE TABLE IF NOT EXISTS note_line_order (
    note_id UUID PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    order_json JSONB NOT NULL DEFAULT '[]',
    updated_at TIMESTAMPTZ NOT NULL,
    vv JSONB NOT NULL DEFAULT '{}',
    last_writer TEXT NOT NULL
);

-- Who may enter a note's collaborative session and with which role. The
-- owner is implicit (notes.owner_id); shares hold editor/viewer grants.
CREATE TABLE IF NOT EXISTS note_shares (
    note_id UUID NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('editor', 'viewer')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (note_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_shares_user ON note_shares(user_id);

```

## Purpose

The second schema migration. Adds the tables behind the `/api/ws` collaborative channel: notes,
their independently versioned lines, the versioned line order, and the sharing grants. This is
the on-disk form of the line-based CRDT-ish design (version vectors + tombstones) that
`collab.rs` operates on.

## What it defines

| Object | Purpose |
|--------|---------|
| `users.display_name` (new column) | human-readable name surfaced in presence |
| `notes` | note metadata (title, owner, timestamps, soft-delete); the **body is not stored here** — it is materialised from live lines |
| `lines` | one row per line: `content` (never contains `\n`), `vv`, `last_writer`, soft-delete `deleted_at` |
| `note_line_order` | the note's line order as its own versioned entity (`order_json`, `vv`, `last_writer`) |
| `note_shares` | editor/viewer grants; the owner is implicit via `notes.owner_id` |

Indexes: `idx_lines_note`, `idx_shares_user`.

## The model in one paragraph

A note is a **list of independently versioned lines** plus a **separately versioned order**. Each
line and the order carry a version vector (`vv`) keyed by device id and the `last_writer` device
that produced the current value; concurrent edits resolve deterministically in `collab.rs`, not in
SQL. Deletes are **tombstones** (`deleted_at` set) so replicas converge; `note_line_order.order_json`
keeps every `LineId`, tombstoned ones included, until GC (`0001`-style journal pruning has an
analogue here: `gc_line_tombstones`).

## Notes & gotchas

- `CHECK (role IN ('editor','viewer'))` — the owner role is never a `note_shares` row; it comes
  from `notes.owner_id`.
- `notes` has no `body` column by design: reads join live (`deleted_at IS NULL`) lines with `\n`.
- `vv` and `order_json` are `JSONB`; the server serialises keeplin's version-vector maps and the
  `LineId` list into them.
- `ON DELETE CASCADE` everywhere off `notes`/`users` keeps a hard note/account delete from leaving
  orphan lines, order, or shares.

## Related files

- `../crates/keeplin-srv/src/collab.md` — the session that reads/writes these tables.
- `../crates/keeplin-srv/src/store.md` — the queries (`insert_line`, `set_note_order`, shares, GC).
- `0003_note_metadata.md` — the remaining note fields added next.
