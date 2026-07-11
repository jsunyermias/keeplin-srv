-- Front B: generalize the note-sharing model from fixed roles (editor/viewer) to a
-- capability bitset. (Notebook permissions + the notebook→note cascade land in a
-- follow-up migration.)
--
-- Capability bits (see `permissions.rs`), stored as an INTEGER bitmask:
--     READ        = 1
--     WRITE       = 2   (implies READ)
--     SHARE_READ  = 4   (implies READ)
--     SHARE_WRITE = 8   (implies SHARE_READ + WRITE)
--     MANAGE      = 16  (implies every non-owner bit)
-- The owner is separate and transferable (notes.owner_id) and always has every
-- capability plus ownership transfer + delete.

-- Add the capability bitmask alongside the legacy role, backfilled from it:
--   editor → READ|WRITE = 3, viewer → READ = 1.
ALTER TABLE note_shares
    ADD COLUMN IF NOT EXISTS capabilities INTEGER NOT NULL DEFAULT 1;

UPDATE note_shares SET capabilities = 3 WHERE role = 'editor';
UPDATE note_shares SET capabilities = 1 WHERE role = 'viewer';

-- The legacy CHECK on `role` would reject rows that only carry capabilities, so drop it;
-- `role` stays nullable for backward reads but is no longer authoritative.
ALTER TABLE note_shares DROP CONSTRAINT IF EXISTS note_shares_role_check;
ALTER TABLE note_shares ALTER COLUMN role DROP NOT NULL;
