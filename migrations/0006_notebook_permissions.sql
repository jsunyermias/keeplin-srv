-- Front B (stage 1b): notebook permissions + the destructive notebook→note cascade.
--
-- Notebook shares mirror note shares: a capability bitset per grantee (see
-- `permissions.rs`). The notebook owner is `notebooks.user_id` (its creator), separate and
-- transferable like note ownership.
--
-- The cascade is applied in application code (see `store.rs`), not by a trigger, so it stays
-- explicit and testable: changing a notebook's shares — or moving a note into a notebook —
-- destructively **overwrites** the affected notes' `note_shares` with the notebook's grants.

CREATE TABLE IF NOT EXISTS notebook_shares (
    notebook_id  UUID NOT NULL REFERENCES notebooks(id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    capabilities INTEGER NOT NULL DEFAULT 1,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (notebook_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_notebook_shares_user ON notebook_shares(user_id);
