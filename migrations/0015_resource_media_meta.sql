-- Adds plaintext media metadata to resources (issue #129), mirroring keeplin-core's
-- Resource.duration_ms and Resource.dimensions ((width, height)). Like `size`, these are
-- metadata rather than content: the server stores and returns them so a frontend can render
-- an attachment without downloading or decrypting the blob. The server never computes or
-- validates them — the producer fills them in.
--
-- All three are nullable: a non-media attachment (or an existing row) simply has NULL. The
-- model keeps width/height together (both-or-neither); the server materialises them as two
-- columns and reads them back as a pair.
--
-- Forward-only and idempotent.
ALTER TABLE resources ADD COLUMN IF NOT EXISTS duration_ms BIGINT;
ALTER TABLE resources ADD COLUMN IF NOT EXISTS width INTEGER;
ALTER TABLE resources ADD COLUMN IF NOT EXISTS height INTEGER;
