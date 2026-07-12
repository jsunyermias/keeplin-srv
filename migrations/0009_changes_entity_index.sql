-- Re-index the change journal for **per-entity** history (issue #27).
--
-- History is no longer scoped to the requesting user: a note/notebook has one timeline that
-- every user with read access sees, so `entity_history` now matches by entity id across all
-- users' journal rows (authorization happens in the HTTP handler first). The 0008 composite
-- indexes were keyed `(user_id, <expr>)` and no longer serve the user-agnostic lookup, so
-- replace them with expression-only indexes on the extracted entity id.
DROP INDEX IF EXISTS idx_changes_note_hist;
DROP INDEX IF EXISTS idx_changes_notebook_hist;
DROP INDEX IF EXISTS idx_changes_delete_hist;

CREATE INDEX IF NOT EXISTS idx_changes_note_id
    ON changes ((payload -> 'note' ->> 'id'));
CREATE INDEX IF NOT EXISTS idx_changes_notebook_id
    ON changes ((payload -> 'notebook' ->> 'id'));
CREATE INDEX IF NOT EXISTS idx_changes_top_id
    ON changes ((payload ->> 'id'));
