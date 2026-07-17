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
| `JWT_SECRET` | `jwt_secret` | — (required) | HMAC secret for signing device tokens. The server **refuses to start** if it is unset, empty, shorter than 16 chars, or the known dev placeholder (issue #19); set `KEEPLIN_DEV_INSECURE=1` to allow a weak/placeholder secret for local dev only |
| `KEEPLIN_DEV_INSECURE` | — | `false` | `1`/`true` downgrades the `JWT_SECRET` strength check to a warning (local dev only — tokens become forgeable) |
| `TOKEN_TTL_DAYS` | `token_ttl_days` | `365` | Device-token lifetime |
| `CHANGES_RETENTION_DAYS` | `retention_days` | `0` (off) | Prune delivered relay-journal rows older than N days |
| `LINES_GC_DAYS` | `lines_gc_days` | `30` | Compact line tombstones older than N days (design §6.4) |
| `RESOURCE_PURGE_DAYS` | `resource_purge_days` | `0` (off) | Reclaim the blob bytes of resources soft-deleted more than N days ago; the metadata tombstone is kept (issue #24) |
| `DB_MAX_CONNECTIONS` | `db_max_connections` | `10` | PostgreSQL pool size |
| `DB_ACQUIRE_TIMEOUT_SECS` | `db_acquire_timeout_secs` | `10` | Fail a request instead of blocking forever when the pool is exhausted |
| `DB_IDLE_TIMEOUT_SECS` | `db_idle_timeout_secs` | `600` | Reap idle pooled connections |
| `DB_MAX_LIFETIME_SECS` | `db_max_lifetime_secs` | `1800` | Recycle pooled connections after this age |
| `RATE_LIMIT_PER_MIN` | `rate_limit_per_min` | `0` (off) | Per-client-IP request budget/minute |
| `SHUTDOWN_GRACE_SECS` | `shutdown_grace_secs` | `20` | Drain window before force-exit |
| `LOG_JSON` | `log_json` | `false` | Emit JSON logs (one object/line) |
| `MAX_UPLOAD_BYTES` | `max_upload_bytes` | `104857600` (100 MiB) | Max size of a resource binary upload (`PUT /api/resources/:id/data`); `413` over it |
| `MAX_USER_STORAGE_BYTES` | `max_user_storage_bytes` | `0` (off) | Total resource-blob bytes per user; a blob upload over it → `507` |
| `MAX_NOTES_PER_USER` | `max_notes_per_user` | `0` (off) | Max live notes a user may own; creating past it → `507` |
| `REGISTRATION_ENABLED` | `registration_enabled` | `true` | When `false`, `POST /api/register` returns `403` — close open signups on a private deployment (issue #21) |
| `HISTORY_VISIBILITY` | `history_since_access` | `creation` | `creation`: everyone with read access sees an entity's full history; `access`: a **collaborator** sees only versions from when they were granted access (the owner always sees all) — issue #27 |

## Notes & gotchas

- `DATABASE_URL` and `JWT_SECRET` are hard requirements — `from_env` panics if either is
  missing (or if `JWT_SECRET` is weak), on purpose: a guessable signing key lets anyone forge
  a token for any user, so the server must not run without a real secret. Use
  `KEEPLIN_DEV_INSECURE=1` only for local `cargo run`.
- Leave `RATE_LIMIT_PER_MIN=0` behind a reverse proxy: every request would carry the proxy's
  IP and share one bucket. Rate-limit at the proxy instead.
- Rotating `JWT_SECRET` invalidates every issued token (all devices must log in again).

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Config` — defined here (EXTRACTED; 11 cross-file edge(s))
- `env_parse()` — defined here (EXTRACTED; file-local)
- `dev_insecure()` — defined here (EXTRACTED; file-local)
- `is_weak_secret()` — defined here (EXTRACTED; file-local)
- `resolve_jwt_secret()` — defined here (EXTRACTED; file-local)
- `.from_env()` — defined here (EXTRACTED; file-local)
- `weak_secrets_are_rejected()` — defined here (EXTRACTED; file-local)
- `a_strong_secret_is_accepted()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×2; e.g. `AppState`, `.new()`)
- `crates/keeplin-srv/tests/collab.rs` — collaborative channel & hardening tests (EXTRACTED: references×1; e.g. `test_config()`)
- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: references×1; e.g. `test_config()`)
- `crates/keeplin-srv/tests/integration.rs` — device relay tests (real `DbBackend`) (EXTRACTED: references×2; e.g. `spawn_server_with_config()`, `test_config()`)
- `crates/keeplin-srv/tests/materialize.rs` — domain-entity materialisation tests (EXTRACTED: references×1; e.g. `test_config()`)
- `crates/keeplin-srv/tests/quotas.rs` — per-user quota enforcement tests (EXTRACTED: references×2; e.g. `quota_config()`, `spawn()`)
- `crates/keeplin-srv/tests/reencrypt.rs` — re-encrypt pass tests (EXTRACTED: references×1; e.g. `test_config()`)
- `crates/keeplin-srv/tests/soak.rs` — multi-instance collaborative soak/load drill (EXTRACTED: references×1; e.g. `test_config()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Every knob comes from the environment with a backward-compatible default; a fresh deployment must run with only `DATABASE_URL` + `JWT_SECRET` set.
- A weak/placeholder `JWT_SECRET` aborts startup unless `KEEPLIN_DEV_INSECURE=1` explicitly opts into insecure local dev.
- `0` disables every optional limit/retention knob; `HISTORY_VISIBILITY` maps `access` → `history_since_access = true`, anything else → full history.

## Related files

- `.env.example` — a copy-paste starting point mirroring these keys.
- `main.md` — how each field is applied at startup.
- `ratelimit.md` / `auth.md` — consumers of the rate-limit and token knobs.
