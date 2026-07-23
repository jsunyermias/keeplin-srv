-- Adds note_id to resources (issue #125): every attachment belongs to exactly one note.
-- Mirrors keeplin-core's Resource.note_id, which is a plaintext id (like notebook_id) so the
-- server can filter attachments by note without decrypting anything.
--
-- The column is NOT NULL with a reserved sentinel default (SYSTEM_RESOURCE_NOTE_ID =
-- 00000000-0000-0000-0000-000000000001, the "system resource, no user note" marker). The
-- sentinel default means existing rows get a valid, non-nil value and Postgres (>= 11)
-- applies the NOT NULL DEFAULT without rewriting the table — a bare NOT NULL would fail on a
-- table that already has rows. New user attachments carry their real owning note; system
-- resources (contacts/events from interop) keep the sentinel and stay out of per-note listings.
--
-- The index backs list_resources_for_note (WHERE user_id = $1 AND note_id = $2).
--
-- Forward-only and idempotent.
ALTER TABLE resources
    ADD COLUMN IF NOT EXISTS note_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001';
CREATE INDEX IF NOT EXISTS idx_resources_note ON resources(user_id, note_id);
