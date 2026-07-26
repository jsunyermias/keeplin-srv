# `0004_domain_entities.sql` — server-materialised notebooks, tags, resources

## Complete migration

```sql
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

```

## Purpose

The fourth schema migration. Gives the server durable, queryable tables for the keeplin-core
domain entities that used to travel the device relay as **opaque** `Change` payloads: notebooks,
tags, note↔tag associations, and resource metadata (plus a table for resource binaries). This is
what makes the server the **source of truth** in server mode — the client database becomes a cache
that can be fully rehydrated from these tables.

## What it defines

| Table | Purpose |
|-------|---------|
| `notebooks` | one row per notebook; soft-delete + `vv`/`last_writer`, mirroring keeplin-core |
| `tags` | one row per tag; same shape without the alias |
| `note_tags` | note↔tag association as a versioned present/absent state (add = live, remove = tombstone) |
| `resources` | resource **metadata** only (title, mime, file name, size); soft-delete + version vector |
| `resource_blobs` | the resource **binary** payload (`BYTEA`), split out so metadata reads never touch the bytes |

Indexes: `idx_notebooks_user`, `idx_tags_user`, `idx_note_tags_note`, `idx_resources_user`.

## How the server uses these

- The relay (`sync.rs`) parses each incoming `Change`; for these entity variants it **resolves by
  version vector** against the stored row (reusing keeplin-core's `note_log::resolve`, so the server
  picks the exact same winner as every client) and upserts. See `../crates/keeplin-srv/src/store.md`.
- Reads are served over REST for cold rehydration and queries (`GET /api/notebooks|tags|resources`,
  `GET /api/notes/:id/tags`). Binaries move over `GET`/`PUT /api/resources/:id/data`.
- Because the current value lives in these tables, the relay **journal can be pruned safely** —
  losing journal history no longer means losing rehydration.

## Notes & gotchas

- `title` / `file_name` arrive **already encrypted** from the client; the server stores them as
  opaque text and never interprets them (same as line content).
- `notebooks`/`tags` deletes on an **unknown** id write a minimal tombstone, so a later stale
  create/update cannot resurrect the entity — matching keeplin-core's `apply_change`.
- `resources` has no `updated_at`; resolution uses `COALESCE(deleted_at, created_at)` as the
  timestamp, exactly as keeplin-core does.
- `resource_blobs.data` is `BYTEA` (Postgres large-value TOAST handles the storage); an upload is
  capped by `MAX_UPLOAD_BYTES`.

## Related files

- `../crates/keeplin-srv/src/store.md` — the upsert/resolve methods and reads.
- `../crates/keeplin-srv/src/sync.md` — the `materialize` hook that dispatches changes here.
- `../crates/keeplin-srv/src/http.md` — the REST read endpoints and blob upload/download.
- `0002_collab.md` — the notes/lines tables these sit alongside.
