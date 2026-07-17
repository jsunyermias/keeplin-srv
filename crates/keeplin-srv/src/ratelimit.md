# `ratelimit.rs` — per-IP token-bucket rate limiter

## Purpose

A dependency-free request rate limiter and its axum middleware. Each source IP gets a token
bucket that refills at a configured rate; an empty bucket yields `429 Too Many Requests`. It
bounds abuse such as a reconnect/login loop from a single host. Disabled by default.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `RateLimiter` | struct | one bucket per seen IP; lives in `AppState` |
| `Bucket` | struct (private) | `tokens` + `last_refill` for one IP |

## Public API

| Function | Description |
|----------|-------------|
| `RateLimiter::new(per_min)` | `per_min` requests/IP/minute; `0` disables (every check passes) |
| `RateLimiter::check(ip, now) -> bool` | spend one token; refill for elapsed time first; `true` = allowed |
| `rate_limit_mw(state, ConnectInfo, req, next)` | middleware keyed on the peer socket IP; `429` on empty bucket |

## The token bucket

`capacity = per_min`, refilling `per_min / 60` tokens per second, capped at capacity. On each
request the bucket refills for the elapsed wall-clock time, then spends one token if ≥ 1 is
available. `now` is passed in (not read inside) so the unit tests are deterministic. Buckets
are created lazily on first sight of an IP and never expire (bounded in practice by the IP
space a single server sees; a future improvement could evict idle buckets).

## Notes & gotchas

- **Behind a reverse proxy** every request carries the proxy's IP, so all clients would share
  one bucket. Leave `RATE_LIMIT_PER_MIN=0` and rate-limit at the proxy instead.
- The middleware requires `ConnectInfo<SocketAddr>`, so the server (and the tests) must serve
  with `into_make_service_with_connect_info::<SocketAddr>()`.
- `/health` is mounted outside the rate-limited sub-router (see `http.md`) so liveness probes
  are never throttled.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `RateLimiter` — defined here (EXTRACTED; 1 cross-file edge(s))
- `rate_limit_mw()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `Bucket` — defined here (EXTRACTED; file-local)
- `LimiterState` — defined here (EXTRACTED; file-local)
- `.new()` — defined here (EXTRACTED; file-local)
- `.enabled()` — defined here (EXTRACTED; file-local)
- `.projected_tokens()` — defined here (EXTRACTED; file-local)
- `.check()` — defined here (EXTRACTED; file-local)
- `.bucket_count()` — defined here (EXTRACTED; file-local)
- `ip()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: imports_from×1, references×1; e.g. `AppState`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- `RATE_LIMIT_PER_MIN=0` disables limiting entirely (the middleware must be a strict no-op).
- Limiting is per-client-IP and per-instance; behind a proxy every request shares one IP, so deployments rate-limit at the proxy instead.
- `/health`, `/ready` and `/version` are never rate-limited (liveness probes and the protocol handshake must always pass).

## Related files

- `http.md` — where the middleware is layered onto all routes but `/health`.
- `config.md` — `RATE_LIMIT_PER_MIN`.
- `state.md` — holds the shared `RateLimiter`.

## Memory

Idle buckets are swept out of the map on a periodic pass (every ~60s, triggered lazily on the next `check`): a bucket that has refilled to capacity is indistinguishable from a fresh one, so dropping it is behaviour-preserving and keeps the map bounded under IP churn (issue #33).
