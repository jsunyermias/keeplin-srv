# `config.rs` — runtime configuration

## Purpose

Defines `Config`, the process settings read once from environment variables at startup
(`Config::from_env()`). Sensitive values (`JWT_SECRET`, `DATABASE_URL`) never have code
defaults that would be safe to ship; everything else has a sane default via the `env_parse`
helper.

## Configuration / key reference

| Env var | Field | Default | Meaning |
|---------|-------|---------|---------|
| `PORT` | `port` | `3000` | HTTP/WS listen port |
| `DATABASE_URL` | `database_url` | — (required) | PostgreSQL connection string; panics if unset |
| `JWT_SECRET` | `jwt_secret` | dev value | HMAC secret for signing device tokens; **change in production** |
| `TOKEN_TTL_DAYS` | `token_ttl_days` | `365` | Device-token lifetime |
| `CHANGES_RETENTION_DAYS` | `retention_days` | `0` (off) | Prune delivered relay-journal rows older than N days |
| `LINES_GC_DAYS` | `lines_gc_days` | `30` | Compact line tombstones older than N days (design §6.4) |
| `DB_MAX_CONNECTIONS` | `db_max_connections` | `10` | PostgreSQL pool size |
| `DB_ACQUIRE_TIMEOUT_SECS` | `db_acquire_timeout_secs` | `10` | Fail a request instead of blocking forever when the pool is exhausted |
| `DB_IDLE_TIMEOUT_SECS` | `db_idle_timeout_secs` | `600` | Reap idle pooled connections |
| `DB_MAX_LIFETIME_SECS` | `db_max_lifetime_secs` | `1800` | Recycle pooled connections after this age |
| `RATE_LIMIT_PER_MIN` | `rate_limit_per_min` | `0` (off) | Per-client-IP request budget/minute |
| `SHUTDOWN_GRACE_SECS` | `shutdown_grace_secs` | `20` | Drain window before force-exit |
| `LOG_JSON` | `log_json` | `false` | Emit JSON logs (one object/line) |
| `MAX_UPLOAD_BYTES` | `max_upload_bytes` | `104857600` (100 MiB) | Max size of a resource binary upload (`PUT /api/resources/:id/data`); `413` over it |

## Notes & gotchas

- `DATABASE_URL` is the only hard requirement — `from_env` panics if it is missing, on
  purpose (the server cannot run without it).
- Leave `RATE_LIMIT_PER_MIN=0` behind a reverse proxy: every request would carry the proxy's
  IP and share one bucket. Rate-limit at the proxy instead.
- `JWT_SECRET`'s default exists only so `cargo run` works out of the box; rotating it
  invalidates every issued token.

## Related files

- `.env.example` — a copy-paste starting point mirroring these keys.
- `main.md` — how each field is applied at startup.
- `ratelimit.md` / `auth.md` — consumers of the rate-limit and token knobs.
