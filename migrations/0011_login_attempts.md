# 0011 — login brute-force lockout

Adds `login_attempts`: one row per (normalized) email currently accumulating
failed logins, with a `failed_count`, `last_failed_at`, and an optional
`locked_until`.

## Semantics

- On a **failed** login the row is upserted: the counter increments (or restarts
  at 1 when the previous failure is older than the lockout window). When the
  counter reaches `LOGIN_MAX_FAILURES`, `locked_until` is set
  `LOGIN_LOCKOUT_SECS` into the future.
- A login attempt while `locked_until` is in the future is refused with `429`
  **before** the password is checked.
- On a **successful** login the row is deleted (a legitimate user resets their
  own counter).

## Design notes

- **Database-backed, not in-process**, so the counter is shared across replicas
  (#45) and survives restarts — an attacker cannot dodge the lockout by
  spreading attempts over instances.
- Keyed by the submitted email **whether or not an account exists**, so the
  `429` is uniform and cannot be used as an account-existence oracle (#32).
  The flip side — someone can deliberately lock an email they don't own — is the
  standard trade-off; the lockout is short (default 5 min) and does not reveal
  anything.
- Stale rows are pruned by age from the maintenance loop
  (`idx_login_attempts_last_failed`).
