-- Cross-instance collaboration bus (issue #45): make the collaborative channel
-- and the device relay work correctly when the server runs more than one
-- replica. Instances coordinate over Postgres LISTEN/NOTIFY (no new infra).

-- Outbox of applied collaborative op batches. Every applied batch is written
-- here and the row's `seq` is the note's fan-out sequence number — a single
-- BIGSERIAL is globally (hence per-note) monotonic, so it replaces the old
-- in-process per-session counter that collided across instances. Listeners on
-- every instance read the row and deliver it to their local subscribers; the
-- origin instance suppresses the echo to the connection that authored it.
CREATE TABLE collab_events (
    seq             BIGSERIAL PRIMARY KEY,
    note_id         UUID NOT NULL,
    origin_instance UUID NOT NULL,
    origin_conn     BIGINT NOT NULL,
    user_id         UUID NOT NULL,
    ops             JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The outbox is a short-lived delivery buffer, not durable history (the lines
-- and order are the source of truth); it is pruned by age in the maintenance
-- loop, so index the prune predicate.
CREATE INDEX idx_collab_events_created ON collab_events (created_at);

-- Live presence per note, contributed by every instance. Each connected
-- subscriber owns one row keyed by (note_id, instance_id, conn_id); on any
-- change an instance rewrites its rows and notifies, and every instance rebuilds
-- the merged presence list for its local subscribers. `updated_at` is heartbeat
-- touched so rows left behind by a crashed instance are swept by age.
CREATE TABLE collab_presence (
    note_id      UUID NOT NULL,
    instance_id  UUID NOT NULL,
    conn_id      BIGINT NOT NULL,
    user_id      UUID NOT NULL,
    display_name TEXT NOT NULL,
    cursor       JSONB,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (note_id, instance_id, conn_id)
);

CREATE INDEX idx_collab_presence_note ON collab_presence (note_id);
CREATE INDEX idx_collab_presence_instance ON collab_presence (instance_id);
CREATE INDEX idx_collab_presence_updated ON collab_presence (updated_at);
