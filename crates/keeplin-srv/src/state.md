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

## Related files

- `store.md`, `sync.md`, `collab.md`, `ratelimit.md` — the four subsystems it holds.
- `http.md` — every handler extracts `State<Arc<AppState>>`.
