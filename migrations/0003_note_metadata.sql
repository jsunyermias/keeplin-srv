-- The server stores the whole note, not just its lines: bring the remaining
-- keeplin-core note metadata into the notes table so a device can restore
-- everything from the server.
ALTER TABLE notes ADD COLUMN IF NOT EXISTS notebook_id UUID;
ALTER TABLE notes ADD COLUMN IF NOT EXISTS is_todo BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE notes ADD COLUMN IF NOT EXISTS todo_due TIMESTAMPTZ;
ALTER TABLE notes ADD COLUMN IF NOT EXISTS todo_completed TIMESTAMPTZ;
