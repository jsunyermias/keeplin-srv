-- Email flows: verification + password reset (issue #49).
--
-- keeplin-srv does NOT speak SMTP. When MAIL_WEBHOOK_URL is configured the
-- server generates a single-use token and POSTs a JSON payload to the
-- operator's mail service (the "mail webhook"), which composes and delivers
-- the actual email. Without a webhook the flows answer 501.

-- When the user proved ownership of their email address (NULL = never).
ALTER TABLE users ADD COLUMN email_verified_at TIMESTAMPTZ;

-- Single-use, expiring tokens for both flows. Only the SHA-256 of the token is
-- stored — a database dump cannot be replayed into a password reset.
CREATE TABLE email_tokens (
    id         UUID PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('verify_email', 'password_reset')),
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The maintenance loop prunes expired/used tokens by age.
CREATE INDEX idx_email_tokens_expires ON email_tokens (expires_at);
