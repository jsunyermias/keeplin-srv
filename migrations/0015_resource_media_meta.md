# `0015_resource_media_meta.sql` — plaintext media metadata on resources

## What it does

Adds three nullable columns to `resources`:

```sql
ALTER TABLE resources ADD COLUMN IF NOT EXISTS duration_ms BIGINT;
ALTER TABLE resources ADD COLUMN IF NOT EXISTS width INTEGER;
ALTER TABLE resources ADD COLUMN IF NOT EXISTS height INTEGER;
```

They mirror `keeplin-core`'s `Resource.duration_ms` (audio/video length) and
`Resource.dimensions` (`(width, height)` for images) (issue #129).

## Why

Same rationale as the existing `size` column: these are media **metadata**, not content, so
the server may hold them in plaintext (unlike the encrypted `title`/`file_name` and the blob).
A frontend can then read an attachment's duration or dimensions from a metadata list without
downloading or decrypting the payload. The server never computes or validates the values — the
producer of the attachment supplies them, and a non-media attachment leaves all three `NULL`.
`width`/`height` are materialised as two columns but travel together (both-or-neither, since
the model side is a single `Option<(u32, u32)>`).

## Forward-only

Like every migration here: never edit it after it has been applied anywhere; correct with a
new migration. `ADD COLUMN IF NOT EXISTS` makes a re-run idempotent, and the columns are
nullable so existing rows stay valid with no rewrite.

## Related files

- `../crates/keeplin-srv/src/store.rs` — `ResourceMeta` row struct, `upsert_resource_meta`,
  `list_resources` persist and return the columns.
- `../crates/keeplin-srv/src/sync.rs` — `materialize` of `ResourceCreate` forwards the whole
  core `Resource` (including the media metadata) to `upsert_resource_meta`.
