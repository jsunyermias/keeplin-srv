# `lib.rs` — keeplin-srv library root

## Purpose

Declares the public modules of the `keeplin-srv` crate so both the binary (`main.rs`) and the
integration tests (`tests/`) can build the router and drive the server. It carries no logic —
only `pub mod` declarations, one per source file.

## Module map

| Module | Public | Description |
|--------|--------|-------------|
| `auth` | yes | password hashing, JWT mint/verify, the auth middleware and device-revocation check |
| `bus` | yes | cross-instance coordination over Postgres `LISTEN/NOTIFY` (multi-replica collab/relay, issue #45) |
| `collab` | yes | the collaborative line-editing session engine (`/api/ws`) |
| `config` | yes | `Config` loaded from environment variables |
| `error` | yes | `AppError` and its HTTP status mapping |
| `http` | yes | the axum router and every REST handler |
| `permissions` | yes | `Role` (owner/editor/viewer) and `resolve_role` |
| `protocol` | yes | wire types of the collaborative channel |
| `ratelimit` | yes | per-IP token-bucket rate limiter + middleware |
| `state` | yes | `AppState` shared by every handler |
| `store` | yes | the single PostgreSQL data-access layer |
| `sync` | yes | the device sync relay (`/api/sync`) |

## Design notes

- No re-exports at the crate root: every import names its origin module (`keeplin_srv::store::Store`),
  so a reader always sees where a type comes from.
- The library exposes `router(state)` and `AppState::new(config, pool)` so tests spin up the
  full server against a throwaway database without touching `main.rs`.

## Related files

- `main.rs` — the binary that builds the pool and serves this router.
- `ARCHITECTURE.md` — the one-page mental model this crate fits into.
