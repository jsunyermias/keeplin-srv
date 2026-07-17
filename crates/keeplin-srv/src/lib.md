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
| `crypto` | yes | at-rest AES-256-GCM encryption of note title/content (`AT_REST_KEY`, keeplin#110) |
| `error` | yes | `AppError` and its HTTP status mapping |
| `http` | yes | the axum router and every REST handler |
| `mail` | yes | delegated email delivery via the operator's mail webhook (issue #49; no SMTP in keeplin) |
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

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- (no symbols extracted for this file — it contributes only its file node) (EXTRACTED)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- `lib.rs` only declares modules — no logic; every concrete type lives in a sub-module.
- Every public module keeps a companion `.md` (the contractual doc system) and a new module is added to both `lib.rs` and its doc.

## Related files

- `main.rs` — the binary that builds the pool and serves this router.
- `ARCHITECTURE.md` — the one-page mental model this crate fits into.
