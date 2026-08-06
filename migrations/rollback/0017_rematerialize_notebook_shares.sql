INSERT INTO note_shares (note_id, user_id, capabilities)
SELECT n.id, nbs.user_id, nbs.capabilities
FROM notes n
JOIN notebook_shares nbs ON nbs.notebook_id = n.notebook_id
WHERE n.deleted_at IS NULL
ON CONFLICT (note_id, user_id) DO UPDATE
SET capabilities = note_shares.capabilities | EXCLUDED.capabilities;
