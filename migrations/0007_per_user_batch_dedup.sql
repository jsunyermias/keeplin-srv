-- Scope the relay batch-dedup key to the owning user.
--
-- 0001 declared `changes UNIQUE (batch_id, batch_index)` — global across users. batch_id is a
-- client-generated UUID, so a collision between two users (a client bug that reuses ids, or a
-- leaked/guessed id) makes the second user's insert a silent no-op via
-- `ON CONFLICT ... DO NOTHING`, dropping their changes (issue #26). Dedup is inherently
-- per-user (the journal is per-user), so the key must include user_id.
--
-- Forward-only: drop the old constraint (auto-named by Postgres for the inline UNIQUE in
-- 0001) and add the user-scoped one. `IF EXISTS` keeps this idempotent across environments
-- where the constraint may already have been renamed.
ALTER TABLE changes DROP CONSTRAINT IF EXISTS changes_batch_id_batch_index_key;
ALTER TABLE changes
    ADD CONSTRAINT changes_user_batch_key UNIQUE (user_id, batch_id, batch_index);
