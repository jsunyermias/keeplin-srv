# `0007_per_user_batch_dedup.sql` — scope batch dedup to the owning user

## Purpose

The seventh schema migration. Fixes a cross-user data-loss bug (issue #26): the relay's
batch-dedup key was global, so one user's insert could be silently swallowed as a duplicate of
another user's.

## What it changes

`0001` declared `changes UNIQUE (batch_id, batch_index)` — a key that is **global across all
users**. `batch_id` is a client-generated UUID, so a collision between two users (a client bug
that reuses ids, or a leaked/guessed id) makes the second user's insert a silent no-op through
the append path's `ON CONFLICT … DO NOTHING`, dropping their changes.

Deduplication is inherently **per-user** (the change journal is per-user), so the uniqueness
key must include `user_id`:

| Before | After |
|--------|-------|
| `UNIQUE (batch_id, batch_index)` (auto-named `changes_batch_id_batch_index_key`) | `UNIQUE (user_id, batch_id, batch_index)` (named `changes_user_batch_key`) |

Forward-only and idempotent: `DROP CONSTRAINT IF EXISTS` on the old inline-UNIQUE name, then
`ADD CONSTRAINT` the user-scoped one. `IF EXISTS` keeps it safe across environments where the
constraint may already have been renamed.

## Related files

- `../crates/keeplin-srv/src/store.rs` — the `append_changes` path whose `ON CONFLICT … DO
  NOTHING` relies on this key; the dedup now discriminates by `(user_id, batch_id,
  batch_index)`, so replays are still idempotent per user but no longer collide across users.
- `0001_initial.sql` — declared the original global `UNIQUE (batch_id, batch_index)`.
