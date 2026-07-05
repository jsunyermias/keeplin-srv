# `0004_domain_entities.sql` — server-materialised notebooks, tags, resources

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
