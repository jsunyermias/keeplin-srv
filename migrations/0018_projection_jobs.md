# `0018_projection_jobs.sql` — durable journal projection queue

Creates one lifecycle row for each journal change. The composite foreign key keeps replay input in
`changes`; unfinished and dead-lettered jobs therefore prevent deletion. Workers select eligible
rows with `FOR UPDATE SKIP LOCKED` and retain that row lock through projection and completion.

States are `pending`, `retry`, and `dead_letter`. Successful jobs are deleted. Retry scheduling,
attempt counts, lease ownership, the last error, and timestamps make failures observable and
operator-recoverable. The partial indexes support worker claims and scoped reconciliation.

The migration is forward-only and idempotent. Every new non-null column has a default. Rollback is
operational: drain or reconcile the queue before disabling workers; removing it first restores the
journal/projection loss window.

```sql
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

```
