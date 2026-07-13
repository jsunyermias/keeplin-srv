# 0012 — email flows: verification + password reset (issue #49)

Adds `users.email_verified_at` and the `email_tokens` table backing the two
email flows.

## Delegated delivery — keeplin is not a mail client

The server never speaks SMTP. With `MAIL_WEBHOOK_URL` configured it POSTs a
JSON payload — `{ kind, to, display_name, token, expires_at }` — to the
operator's own mail service, which composes and sends the actual message
(optionally authenticated with `MAIL_WEBHOOK_TOKEN` as a bearer). Without a
webhook, the request endpoints answer `501 Not Implemented`, making the
deferral explicit instead of silently dropping mail.

## Token model

- 32 random bytes, base64url — sent to the webhook once, never stored.
- Only the SHA-256 hex of the token is stored (`token_hash UNIQUE`), so a
  database dump cannot be replayed into a takeover.
- Single-use and expiring: consumption is one atomic
  `UPDATE … SET used_at = now() WHERE used_at IS NULL AND expires_at > now()`,
  so a token races itself safely across replicas (#45).
- `ON DELETE CASCADE` from `users`: deleting an account destroys its tokens.

## Flows

- **verify_email** — issued on registration (when the webhook is configured)
  or on demand via `POST /api/account/verify/request`; confirmed (unauthenticated)
  via `POST /api/account/verify/confirm { token }`, which stamps
  `email_verified_at`. With `EMAIL_VERIFICATION_REQUIRED=true`, login refuses
  unverified accounts with `403`.
- **password_reset** — requested (unauthenticated) via
  `POST /api/account/reset/request { email }`, which answers a uniform `200`
  whether or not the account exists (no oracle, #32); confirmed via
  `POST /api/account/reset/confirm { token, new_password }`, which sets the
  password, **revokes every device** (sign out everywhere), and clears the
  login-lockout counter.
