# `ratelimit.rs` — per-IP token-bucket rate limiter

Self-contained companion for `crates/keeplin-srv/src/ratelimit.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand `ratelimit.rs` without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `ratelimit.rs` carries exactly one marker comment of
the form `// md:<Header> > … > <Block header>`, whose path is the header chain of the
section documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::sync::Mutex;

use crate::state::AppState;
```

**What it does** — A dependency-free per-client-IP token-bucket rate limiter and its
axum middleware. Each source IP gets a bucket of `capacity` tokens that refills at
`capacity` tokens per minute; every request spends one token; an empty bucket yields
`429 Too Many Requests`. This bounds abuse such as a reconnect/login loop from a single
host without adding a dependency. Disabled by default (`RATE_LIMIT_PER_MIN = 0`).

**Behind a reverse proxy** every request carries the proxy's IP, so all clients would
share one bucket — deployments behind a proxy rate-limit **at the proxy** and leave
`RATE_LIMIT_PER_MIN` at `0`.

**Dependencies** — `std` (`HashMap`, `IpAddr`, `Instant`), `axum` (middleware +
`ConnectInfo` extractor), `tokio::sync::Mutex` (async-aware lock), `serde_json` (the
429 body). Internal: `crate::state::AppState` (`state.rs`) — the middleware reads
`state.rate_limiter`.

**Used by** — `state.rs` (`AppState` holds the `RateLimiter`, built in
`AppState::new` from `config.rate_limit_per_min`); `http.rs` layers `rate_limit_mw`
onto the rate-limited sub-router — `/health`, `/ready` and `/version` are mounted
**outside** it so liveness probes and the protocol handshake are never throttled.

**Repeated context** — The 429 produced here is the one HTTP status generated
*outside* `AppError` (`error.rs`), because rate rejection happens before a handler
runs. The middleware needs the peer socket address, so the server (and every
integration test) must serve with
`into_make_service_with_connect_info::<SocketAddr>()`. Limiting is per-instance
in-memory state (rebuildable, like everything in `AppState` outside the `Store`) —
replicas do not share buckets.

---

## Bucket

**Identification** — private struct; marker `// md:Bucket`.

**Code** — complete and verbatim:

```rust
// md:Bucket
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}
```

**What it does** — Token-bucket state for one IP: the current token count (fractional
— refill accrues continuously) and when it was last brought up to date. A bucket is
created full (`capacity` tokens) on first sight of an IP.

**Dependencies** — `std::time::Instant`.

**Used by** — `LimiterState.buckets`, `RateLimiter::{projected_tokens, check}` (this
file).

**Repeated context** — A bucket refilled to capacity is **indistinguishable from a
fresh one** — the fact that makes the idle-bucket sweep behaviour-preserving (see
`fn check`).

---

## LimiterState

**Identification** — private struct; marker `// md:LimiterState`.

**Code** — complete and verbatim:

```rust
// md:LimiterState
struct LimiterState {
    buckets: HashMap<IpAddr, Bucket>,
    last_sweep: Instant,
}
```

**What it does** — The mutable, lock-guarded interior of the limiter: the per-IP
bucket map plus the timestamp of the last idle-bucket sweep. Grouped in one struct so
a single `Mutex` guards both (the sweep reads and mutates the map).

**Dependencies** — `Bucket` (this file), `HashMap`, `IpAddr`, `Instant`.

**Used by** — `RateLimiter.state`, `RateLimiter::{new, check, bucket_count}` (this
file).

**Repeated context** — Unbounded per-key maps on hot paths are a leak hazard
(issue #33: under IP churn the map grew forever); pairing the map with `last_sweep`
is the fix's bookkeeping.

---

## RateLimiter

**Identification** — public struct; marker `// md:RateLimiter`.

**Code** — complete and verbatim:

```rust
// md:RateLimiter
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    state: Mutex<LimiterState>,
}
```

**What it does** — The shared limiter: one bucket per seen IP, refilling
`refill_per_sec` (= `capacity / 60`) tokens per second up to `capacity`.
`capacity == 0` means **disabled**: every check passes without touching the lock.
Lives in `AppState`; interior mutability (`tokio::sync::Mutex`) so handlers share it
behind `Arc<AppState>` without `&mut`.

**Dependencies** — `LimiterState` (this file), `tokio::sync::Mutex`.

**Used by** — `state.rs::AppState` (field `rate_limiter`); `rate_limit_mw` (this
file) calls `check` per request.

**Repeated context** — Crate concurrency convention: all mutable in-memory state in
`AppState` uses interior locking; a `tokio` (not `std`) mutex because the critical
section sits on an async path.

---

## SWEEP_INTERVAL

**Identification** — private const; marker `// md:SWEEP_INTERVAL`.

**Code** — complete and verbatim:

```rust
// md:SWEEP_INTERVAL
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
```

**What it does** — How often idle buckets are swept out of the map. A bucket refills
fully within `capacity / refill_per_sec` seconds — 60 s by construction (capacity
refills per minute) — so anything the sweep removes has been idle at least that long
and is indistinguishable from the fresh full bucket it would be recreated as.

**Dependencies** — none.

**Used by** — `RateLimiter::check` (the lazily-triggered sweep).

**Repeated context** — Issue #33: the sweep exists to bound the map under IP churn
(IPv6/mobile rotation, or an attacker spraying source addresses).

---

## impl RateLimiter

**Identification** — inherent impl block; marker `// md:impl RateLimiter`. Contains
`fn new`, `fn enabled`, `fn projected_tokens`, `fn check`, `fn bucket_count` (next
sections).

**Code** — container: members documented as sub-blocks below: fn new, fn enabled, fn projected_tokens, fn check, fn bucket_count.

**What it does** — Construction and the token-spending logic.

**Dependencies** — `RateLimiter` (this file).

**Used by** — see the method sections.

**Repeated context** — none beyond the methods' own (below).

### fn new

**Identification** — associated function; marker `// md:impl RateLimiter > fn new`.

**Code** — complete and verbatim:

```rust
    // md:impl RateLimiter > fn new
    pub fn new(per_min: u32) -> Self {
        Self {
            capacity: per_min as f64,
            refill_per_sec: per_min as f64 / 60.0,
            state: Mutex::new(LimiterState {
                buckets: HashMap::new(),
                last_sweep: Instant::now(),
            }),
        }
    }
```

**What it does** — Builds the limiter: `per_min` requests per IP per minute (`0`
disables). Sets `capacity = per_min`, `refill_per_sec = per_min / 60`, an empty
bucket map, and `last_sweep = now`.

**Dependencies** — `LimiterState` (this file).

**Used by** — `state.rs::AppState::new` (from `config.rate_limit_per_min`); the unit
tests below.

**Repeated context** — `RATE_LIMIT_PER_MIN = 0` (the default) must make the limiter a
strict no-op — the crate's opt-in convention for operational features.

### fn enabled

**Identification** — method; marker `// md:impl RateLimiter > fn enabled`.

**Code** — complete and verbatim:

```rust
    // md:impl RateLimiter > fn enabled
    pub fn enabled(&self) -> bool {
        self.capacity > 0.0
    }
```

**What it does** — `capacity > 0`. The fast-path guard `check` uses to skip the lock
entirely when limiting is disabled.

**Dependencies** — none.

**Used by** — `check` (this file); `http.rs::metrics` reports it.

**Repeated context** — none.

### fn projected_tokens

**Identification** — private method; marker
`// md:impl RateLimiter > fn projected_tokens`.

**Code** — complete and verbatim:

```rust
    // md:impl RateLimiter > fn projected_tokens
    fn projected_tokens(&self, bucket: &Bucket, now: Instant) -> f64 {
        let elapsed = now
            .saturating_duration_since(bucket.last_refill)
            .as_secs_f64();
        (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity)
    }
```

**What it does** — The tokens `bucket` would hold at `now` after refilling for the
elapsed time since `last_refill`, capped at capacity. Uses
`saturating_duration_since` so a caller-supplied `now` earlier than `last_refill`
(possible in tests) yields zero elapsed rather than a panic.

**Dependencies** — `Bucket` (this file).

**Used by** — `check` (both for the spend and inside the sweep predicate, where the
same formula is inlined over the retained closure).

**Repeated context** — Continuous (fractional) refill instead of windowed counters:
smooth behaviour at the boundary, no thundering-herd reset each minute.

### fn check

**Identification** — public async method; marker `// md:impl RateLimiter > fn check`.

**Code** — complete and verbatim:

```rust
    // md:impl RateLimiter > fn check
    pub async fn check(&self, ip: IpAddr, now: Instant) -> bool {
        if !self.enabled() {
            return true;
        }
        let mut state = self.state.lock().await;
        let bucket = state.buckets.entry(ip).or_insert(Bucket {
            tokens: self.capacity,
            last_refill: now,
        });
        bucket.tokens = self.projected_tokens(bucket, now);
        bucket.last_refill = now;
        let allowed = if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        };

        if now.saturating_duration_since(state.last_sweep) >= SWEEP_INTERVAL {
            let cap = self.capacity;
            let rate = self.refill_per_sec;
            state.buckets.retain(|_, b| {
                let elapsed = now.saturating_duration_since(b.last_refill).as_secs_f64();
                (b.tokens + elapsed * rate) < cap
            });
            state.last_sweep = now;
        }
        allowed
    }
```

**What it does** — Try to spend one token for `ip` at time `now`; `true` = allowed.
Disabled → always `true`, no lock taken. Otherwise, under the mutex: get-or-create
the IP's bucket (created full), bring it up to date (`projected_tokens`, stamp
`last_refill = now`), then spend one token if ≥ 1 is available, else deny.
`now` is **passed in** rather than read inside so the unit tests are deterministic.

Before returning, a lazily-triggered sweep (issue #33): if `SWEEP_INTERVAL` has
elapsed since `last_sweep`, drop every bucket that would be at full capacity at
`now` — a full bucket is identical to the fresh one that would replace it, so
removal is behaviour-preserving; the just-touched bucket has spent a token and is
under capacity, so it survives. This bounds the map under IP churn without a
background task.

**Dependencies** — `enabled`, `projected_tokens`, `Bucket`, `LimiterState`,
`SWEEP_INTERVAL` (this file).

**Used by** — `rate_limit_mw` (this file); the unit tests.

**Repeated context** — Per-IP and per-instance: replicas do not share buckets, and
behind a proxy all traffic shares the proxy's IP (so deployments rate-limit at the
proxy instead — see *Overview*).

### fn bucket_count

**Identification** — `#[cfg(test)]` method; marker
`// md:impl RateLimiter > fn bucket_count`.

**Code** — complete and verbatim:

```rust
    // md:impl RateLimiter > fn bucket_count
    #[cfg(test)]
    async fn bucket_count(&self) -> usize {
        self.state.lock().await.buckets.len()
    }
```

**What it does** — The number of live buckets. Test-only introspection for the sweep
test; compiled out of release builds.

**Dependencies** — `LimiterState` (this file).

**Used by** — `mod tests::idle_buckets_are_swept_after_the_interval`.

**Repeated context** — none.

---

## fn rate_limit_mw

**Identification** — public async function (axum middleware); marker
`// md:fn rate_limit_mw`.

**Code** — complete and verbatim:

```rust
// md:fn rate_limit_mw
pub async fn rate_limit_mw(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    if state.rate_limiter.check(addr.ip(), Instant::now()).await {
        next.run(req).await
    } else {
        (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({ "error": "rate limit exceeded" })),
        )
            .into_response()
    }
}
```

**What it does** — Enforces the limiter on every request of the sub-router it is
layered on, keyed by the peer socket IP (`ConnectInfo`). Allowed → run the inner
handler. Denied → respond `429 Too Many Requests` with body
`{"error": "rate limit exceeded"}` — the same `{"error": …}` shape as `AppError`
responses, though produced here directly because no handler has run yet. A strict
no-op when limiting is disabled.

**Dependencies** — `RateLimiter::check` (this file), `AppState` (`state.rs`),
axum extractors, `serde_json`.

**Used by** — `http.rs::router`, layered onto the rate-limited group (everything
except `/health`, `/ready`, `/version`).

**Repeated context** — `ConnectInfo` requires the server to be built with
`into_make_service_with_connect_info::<SocketAddr>()` — `main.rs` and every
test-spawn helper do this; forgetting it makes every request fail extraction.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`. The
helper and four tests are the subsections below.

**Code** — container: members documented as sub-blocks below: fn ip, fn disabled_always_allows, fn burst_then_throttle_then_refill, fn separate_ips_have_separate_buckets, fn idle_buckets_are_swept_after_the_interval.

**What it does** — Deterministic unit tests of the bucket algebra (time is injected
via the `now` parameter — no sleeping, no wall clock).

**Dependencies** — `super::*`; `std::time::Duration`.

**Used by** — `cargo test` only.

**Repeated context** — These pin the operational contract: disabled = unlimited,
burst-then-throttle-then-refill, per-IP isolation, and bounded memory (issue #33).

### fn ip

**Identification** — test helper; marker `// md:mod tests > fn ip`.
`fn ip() -> IpAddr` — the fixed loopback IP used by single-IP tests.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn ip
    fn ip() -> IpAddr {
        IpAddr::from([127, 0, 0, 1])
    }
```

**What it does / Dependencies / Used by** — trivially returns `127.0.0.1`; used by
`disabled_always_allows` and `burst_then_throttle_then_refill`.

**Repeated context** — none.

### fn disabled_always_allows

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn disabled_always_allows`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn disabled_always_allows
    #[tokio::test]
    async fn disabled_always_allows() {
        let rl = RateLimiter::new(0);
        let now = Instant::now();
        for _ in 0..1000 {
            assert!(rl.check(ip(), now).await);
        }
    }
```

**What it does** — `RateLimiter::new(0)` allows 1000 checks at one instant: disabled
means strict no-op.

**Dependencies** — `RateLimiter`, `ip` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins the `RATE_LIMIT_PER_MIN=0` contract.

### fn burst_then_throttle_then_refill

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn burst_then_throttle_then_refill`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn burst_then_throttle_then_refill
    #[tokio::test]
    async fn burst_then_throttle_then_refill() {
        let rl = RateLimiter::new(60);
        let t0 = Instant::now();
        for _ in 0..60 {
            assert!(rl.check(ip(), t0).await);
        }
        assert!(!rl.check(ip(), t0).await);
        let t1 = t0 + Duration::from_secs(1);
        assert!(rl.check(ip(), t1).await);
        assert!(!rl.check(ip(), t1).await);
    }
```

**What it does** — With 60/min (1 token/s, burst 60): the full burst passes at one
instant; the 61st is denied; one second later exactly one more passes, the next is
denied — verifying capacity, denial, and continuous refill.

**Dependencies** — `RateLimiter`, `ip` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins the token-bucket shape (burst capacity + steady refill).

### fn separate_ips_have_separate_buckets

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn separate_ips_have_separate_buckets`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn separate_ips_have_separate_buckets
    #[tokio::test]
    async fn separate_ips_have_separate_buckets() {
        let rl = RateLimiter::new(1);
        let now = Instant::now();
        let a = IpAddr::from([10, 0, 0, 1]);
        let b = IpAddr::from([10, 0, 0, 2]);
        assert!(rl.check(a, now).await);
        assert!(!rl.check(a, now).await);
        assert!(rl.check(b, now).await);
    }
```

**What it does** — Exhausting IP `a`'s bucket leaves IP `b` unaffected.

**Dependencies** — `RateLimiter` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins per-IP isolation.

### fn idle_buckets_are_swept_after_the_interval

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn idle_buckets_are_swept_after_the_interval`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn idle_buckets_are_swept_after_the_interval
    #[tokio::test]
    async fn idle_buckets_are_swept_after_the_interval() {
        let rl = RateLimiter::new(60);
        let t0 = Instant::now();
        for i in 0..100u32 {
            let ip = IpAddr::from([10, 0, (i >> 8) as u8, i as u8]);
            assert!(rl.check(ip, t0).await);
        }
        assert_eq!(rl.bucket_count().await, 100);

        let later = t0 + Duration::from_secs(120);
        let active = IpAddr::from([10, 0, 0, 0]);
        rl.check(active, later).await;
        assert_eq!(rl.bucket_count().await, 1);
    }
```

**What it does** — 100 distinct IPs create 100 buckets; 120 s later (past both the
sweep interval and the refill window) one request from an active IP triggers the
sweep and only that just-touched bucket survives — memory stays bounded under IP
churn (issue #33).

**Dependencies** — `RateLimiter`, `bucket_count` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins the issue #33 fix.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

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

## Coverage checklist

Every code block of `ratelimit.rs`, in source order, each documented above (five
points) and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `struct Bucket` | `// md:Bucket` | Bucket |
| 3 | `struct LimiterState` | `// md:LimiterState` | LimiterState |
| 4 | `struct RateLimiter` | `// md:RateLimiter` | RateLimiter |
| 5 | `SWEEP_INTERVAL` | `// md:SWEEP_INTERVAL` | SWEEP_INTERVAL |
| 6 | `impl RateLimiter` | `// md:impl RateLimiter` | impl RateLimiter |
| 7 | `fn new` | `// md:impl RateLimiter > fn new` | impl RateLimiter › fn new |
| 8 | `fn enabled` | `// md:impl RateLimiter > fn enabled` | impl RateLimiter › fn enabled |
| 9 | `fn projected_tokens` | `// md:impl RateLimiter > fn projected_tokens` | impl RateLimiter › fn projected_tokens |
| 10 | `fn check` | `// md:impl RateLimiter > fn check` | impl RateLimiter › fn check |
| 11 | `fn bucket_count` | `// md:impl RateLimiter > fn bucket_count` | impl RateLimiter › fn bucket_count |
| 12 | `fn rate_limit_mw` | `// md:fn rate_limit_mw` | fn rate_limit_mw |
| 13 | `mod tests` | `// md:mod tests` | mod tests |
| 14 | `fn ip` | `// md:mod tests > fn ip` | mod tests › fn ip |
| 15 | `fn disabled_always_allows` | `// md:mod tests > fn disabled_always_allows` | mod tests › fn disabled_always_allows |
| 16 | `fn burst_then_throttle_then_refill` | `// md:mod tests > fn burst_then_throttle_then_refill` | mod tests › fn burst_then_throttle_then_refill |
| 17 | `fn separate_ips_have_separate_buckets` | `// md:mod tests > fn separate_ips_have_separate_buckets` | mod tests › fn separate_ips_have_separate_buckets |
| 18 | `fn idle_buckets_are_swept_after_the_interval` | `// md:mod tests > fn idle_buckets_are_swept_after_the_interval` | mod tests › fn idle_buckets_are_swept_after_the_interval |
