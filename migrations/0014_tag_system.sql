-- Adds the transport-only `system` marker to tags (issue #128). The frontend sets
-- this flag on tags it uses to implement internal features, hidden from the user;
-- the server only stores and returns it. It never interprets the tag title (which
-- arrives already encrypted) and never filters tags by this flag.
--
-- Forward-only: `DEFAULT false` keeps every existing row valid without rewriting the
-- table (Postgres >= 11 stores a non-volatile column default as catalog metadata).
ALTER TABLE tags ADD COLUMN IF NOT EXISTS system BOOLEAN NOT NULL DEFAULT false;
