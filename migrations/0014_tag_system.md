# `0014_tag_system.sql` — transport-only `system` marker on tags

## What it does

Adds one column to `tags`:

```sql
ALTER TABLE tags ADD COLUMN IF NOT EXISTS system BOOLEAN NOT NULL DEFAULT false;
```

`system` mirrors `keeplin-core`'s `Tag.system` (issue #128). The frontend sets it on tags it
uses to implement internal features (hidden from the user); the server only **stores and
returns** it.

## Why

Frontends implement some features through tags with a reserved title pattern that must stay
hidden from the user. The server cannot detect that pattern — the tag `title` arrives
**already encrypted** (see `0004_domain_entities.sql`, "Titles / file names arrive already
encrypted") — so the marker travels as its own plaintext boolean column, separate from the
encrypted title. The server never interprets the pattern and never filters tags by `system`;
hiding system tags from the user is entirely the frontend's job.

## Forward-only

Like every migration here: never edit it after it has been applied anywhere; correct with a
new migration. `ADD COLUMN IF NOT EXISTS` makes a re-run idempotent in fresh test databases.
`DEFAULT false` keeps every existing row valid, and on Postgres >= 11 a non-volatile default
is stored as catalog metadata, so no table rewrite is needed.

## Related files

- `../crates/keeplin-srv/src/store.rs` — `Tag` row struct, `upsert_tag`, `list_tags` persist
  and return the column.
- `../crates/keeplin-srv/src/sync.rs` — `materialize` of `TagCreate`/`TagUpdate` passes the
  whole core `Tag` (including `system`) to `upsert_tag`.
