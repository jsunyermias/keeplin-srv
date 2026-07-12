-- Index the change journal for the history endpoints.
--
-- `entity_history` (GET /api/{notes,notebooks}/:id/history, issue #37) filters `changes` by
-- `user_id` and a JSONB-extracted entity id: `payload->'note'->>'id'` /
-- `payload->'notebook'->>'id'` for create/update snapshots, and the top-level
-- `payload->>'id'` for delete tombstones. Without a matching index every history read scans
-- the user's whole journal and evaluates the JSON extraction per row. These expression
-- indexes make each lookup an index scan.
--
-- The write cost is three extra index maintenances per journaled change; acceptable for the
-- history feature to scale, and the relay's hot path (append + fan-out) does not read history.
CREATE INDEX IF NOT EXISTS idx_changes_note_hist
    ON changes (user_id, (payload -> 'note' ->> 'id'));
CREATE INDEX IF NOT EXISTS idx_changes_notebook_hist
    ON changes (user_id, (payload -> 'notebook' ->> 'id'));
CREATE INDEX IF NOT EXISTS idx_changes_delete_hist
    ON changes (user_id, (payload ->> 'id'));
