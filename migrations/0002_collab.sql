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
