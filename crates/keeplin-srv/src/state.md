# `state.rs` — shared application state

## Purpose

Defines `AppState`, the single value every axum handler and middleware holds (as
`Arc<AppState>`). It bundles the configuration, the data-access layer, the live in-memory
registries for both WebSocket surfaces, and the rate limiter.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `AppState` | struct | shared handler context, held as `Arc<AppState>` |

Fields:

| Field | Type | Purpose |
|-------|------|---------|
| `config` | `Config` | process settings (see `config.md`) |
| `store` | `Store` | the PostgreSQL data-access layer |
| `hub` | `SyncHub` | per-user fan-out for the device relay (`/api/sync`) |
| `collab` | `CollabRegistry` | per-note collaborative sessions (`/api/ws`) |
| `rate_limiter` | `RateLimiter` | per-IP request limiter (no-op when disabled) |

## Public API

| Function | Description |
|----------|-------------|
| `AppState::new(config, pool)` | builds the store from the pool, empty hub/registry, and a rate limiter sized from `config.rate_limit_per_min` |

## Design notes

- State is constructed once and shared immutably behind `Arc`; all mutable in-memory state
  (`SyncHub`, `CollabRegistry`, `RateLimiter`) uses interior locking, so no handler needs a
  `&mut`.
- `AppState::new` is public so integration tests build the exact same state a real boot would,
  against a throwaway pool.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `AppState` — defined here (EXTRACTED; 78 cross-file edge(s))
- `.new()` — defined here (EXTRACTED; 1 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: references×1; e.g. `CollabRegistry`)
- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×2; e.g. `Config`)
- `crates/keeplin-srv/src/mail.rs` — delegated email delivery (mail webhook) (EXTRACTED: references×1; e.g. `Mailer`)
- `crates/keeplin-srv/src/ratelimit.rs` — per-IP token-bucket rate limiter (EXTRACTED: references×1; e.g. `RateLimiter`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×1; e.g. `Store`)
- `crates/keeplin-srv/src/sync.rs` — the device sync relay (EXTRACTED: references×1; e.g. `SyncHub`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/auth.rs` — passwords, tokens, and the auth middleware (EXTRACTED: references×1; e.g. `auth_mw()`)
- `crates/keeplin-srv/src/bus.rs` — cross-instance coordination (issue #45) (EXTRACTED: imports_from×1, references×5; e.g. `bus.rs`, `spawn()`, `run()`)
- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: references×10; e.g. `touch_presence()`, `clear_presence()`, `announce_presence()`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: references×43; e.g. `router()`, `ready()`, `metrics()`)
- `crates/keeplin-srv/src/main.rs` — keeplin-srv entry point (EXTRACTED: references×2; e.g. `maintenance_loop()`, `run_retention()`)
- `crates/keeplin-srv/src/ratelimit.rs` — per-IP token-bucket rate limiter (EXTRACTED: imports_from×1, references×1; e.g. `ratelimit.rs`, `rate_limit_mw()`)
- `crates/keeplin-srv/src/sync.rs` — the device sync relay (EXTRACTED: references×7; e.g. `authenticate()`, `deliver_backlog()`, `handle_incoming()`)
- `crates/keeplin-srv/tests/collab.rs` — collaborative channel & hardening tests (EXTRACTED: references×1; e.g. `spawn_server_with_state()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- `AppState` is built exactly once per instance; `instance_id` is minted per process so bus events and presence rows can be told apart across replicas.
- The `Cipher` is validated at construction — an invalid `AT_REST_KEY` must abort startup, never fall back to plaintext silently.
- All durable state lives in the `Store`/PostgreSQL; everything else in `AppState` (hub, collab registry, rate limiter) is per-instance and rebuildable.

## Related files

- `store.md`, `sync.md`, `collab.md`, `ratelimit.md` — the four subsystems it holds.
- `http.md` — every handler extracts `State<Arc<AppState>>`.
