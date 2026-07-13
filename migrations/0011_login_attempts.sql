-- Login brute-force lockout (production-readiness follow-up to #21/#32).
--
-- One row per email currently accumulating failures. Database-backed (not
-- in-process) so the counter is shared across replicas (#45) and survives a
-- restart. Keyed by the *submitted* (normalized) email whether or not an
-- account exists, so the lockout response is uniform and cannot be used as an
-- account-existence oracle (#32).
CREATE TABLE login_attempts (
    email          TEXT PRIMARY KEY,
    failed_count   INT NOT NULL DEFAULT 0,
    last_failed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    locked_until   TIMESTAMPTZ
);

-- The maintenance loop prunes stale rows (an email that stopped failing) by age.
CREATE INDEX idx_login_attempts_last_failed ON login_attempts (last_failed_at);
