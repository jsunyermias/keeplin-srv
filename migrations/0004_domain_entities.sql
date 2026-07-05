-- Materialise the remaining keeplin-core domain entities on the server.
--
-- Until now notebooks, tags, note↔tag associations and resources travelled the
-- device relay (`/api/sync`) as OPAQUE `Change` payloads: the server journaled
-- and forwarded them but never interpreted them. That makes the client's local
-- database the only source of truth for those entities, and it makes journal
-- pruning unsafe (a wiped or newly-registered device could no longer rebuild
-- them). The design goal is the opposite: in server mode the client database is
-- a CACHE and keeplin-srv is the durable truth.
--
-- These tables let the server resolve those entities by version vector (exactly
-- like `note_log::resolve` on the client, so both converge to the same winner),
-- store the current value, and serve it back over REST for cold rehydration.
-- Titles / file names arrive already encrypted from the client, so the server
-- keeps them as opaque text and never interprets them — same as line content.

-- One row per notebook. Soft-delete + version vector, mirroring keeplin-core's
-- `notebooks` table (the `alias` is the optional link-scoping name).
CREATE TABLE IF NOT EXISTS notebooks (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    alias TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    vv JSONB NOT NULL DEFAULT '{}',
    last_writer TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_notebooks_user ON notebooks(user_id);

-- One row per tag. Same shape as notebooks, without the alias.
CREATE TABLE IF NOT EXISTS tags (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    vv JSONB NOT NULL DEFAULT '{}',
    last_writer TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tags_user ON tags(user_id);

-- Note↔tag association as a versioned present/absent state: an add sets it
-- present (`deleted_at IS NULL`), a remove tombstones it, and a concurrent
-- add-vs-remove converges through the same resolution as any other entity.
CREATE TABLE IF NOT EXISTS note_tags (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    note_id UUID NOT NULL,
    tag_id UUID NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    vv JSONB NOT NULL DEFAULT '{}',
    last_writer TEXT NOT NULL,
    PRIMARY KEY (user_id, note_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_note_tags_note ON note_tags(user_id, note_id);

-- Resource METADATA only. The binary payload lives in `resource_blobs` so the
-- large bytes never sit in this hot table and never ride the relay journal.
CREATE TABLE IF NOT EXISTS resources (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    file_name TEXT NOT NULL,
    size BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    vv JSONB NOT NULL DEFAULT '{}',
    last_writer TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_resources_user ON resources(user_id);

-- The binary payload of a resource, uploaded out-of-band by the streaming
-- endpoint (`PUT /api/resources/:id/data`) rather than carried in a `Change`.
-- Split from `resources` so metadata reads/lists never touch the blob bytes.
CREATE TABLE IF NOT EXISTS resource_blobs (
    resource_id UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    data BYTEA NOT NULL
);
