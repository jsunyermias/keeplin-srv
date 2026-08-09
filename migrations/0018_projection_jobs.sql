CREATE TABLE IF NOT EXISTS projection_jobs (
    user_id UUID NOT NULL,
    batch_id UUID NOT NULL,
    batch_index INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'retry', 'dead_letter')),
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    leased_by UUID,
    leased_until TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, batch_id, batch_index),
    FOREIGN KEY (user_id, batch_id, batch_index)
        REFERENCES changes (user_id, batch_id, batch_index) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS projection_jobs_claim_idx
    ON projection_jobs (available_at, created_at)
    WHERE state IN ('pending', 'retry');

CREATE INDEX IF NOT EXISTS projection_jobs_user_idx
    ON projection_jobs (user_id, batch_id, batch_index);
