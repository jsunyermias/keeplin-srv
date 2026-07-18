# `main.rs` — keeplin-srv entry point

Self-contained companion for `crates/keeplin-srv/src/main.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able to
understand `main.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `main.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the binary's imports. Marker `// md:Overview` at
the top of the file.

```rust
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use keeplin_srv::{config::Config, http::router, state::AppState};
use tracing_subscriber::EnvFilter;
```

**What it does** — The server binary: pure **wiring**, no request-handling logic (that
all lives in the library — `lib.rs` and its modules — which is what lets integration
tests run the full server in-process). Boot sequence: load `.env`, read `Config`,
initialise logging, open a bounded PostgreSQL pool, run migrations, validate
`AT_REST_KEY`, build `AppState`, clean own presence leftovers, spawn the
cross-instance bus and the maintenance loop, then serve the axum router with graceful
shutdown.

**Dependencies** — `tokio` (runtime), `anyhow` (error context), `dotenvy` (.env),
`tracing`/`tracing_subscriber` (logs), `sqlx` (pool + `migrate!`), `axum` (serve),
`chrono` (cutoff arithmetic in retention). Internal: `keeplin_srv::config::Config`,
`http::router`, `state::AppState`, `crypto::Cipher`, `bus::spawn`, and the
`store.rs` maintenance queries.

**Used by** — nobody imports it; it is the `keeplin-srv` binary entry point. (The
sibling binary `src/bin/reencrypt.rs` repeats the same config/pool bootstrap pattern.)

**Repeated context** — Fail-fast startup (crate convention): missing `DATABASE_URL`,
weak `JWT_SECRET` (unless `KEEPLIN_DEV_INSECURE=1`), or a malformed `AT_REST_KEY`
abort the boot — never limp along insecurely. Migrations are **forward-only** SQL
files in `migrations/`, applied idempotently at every boot (`sqlx::migrate!` no-ops
on an already-migrated database).

---

## fn main

**Identification** — `#[tokio::main] async fn main() -> anyhow::Result<()>`; marker
`// md:fn main`.

**What it does** — The boot sequence, in order:

1. `dotenvy::dotenv().ok()` — load `.env` if present (absence is fine).
2. `Config::from_env()` — read every setting (`config.rs`); panics on missing
   `DATABASE_URL` or weak `JWT_SECRET` (issue #19).
3. Logging: `EnvFilter` from `RUST_LOG` plus a `keeplin_srv=info` default;
   **JSON** output when `LOG_JSON=true` (production/aggregation), pretty otherwise.
4. **Bounded pool**: `max_connections` (`DB_MAX_CONNECTIONS`), `acquire_timeout`
   (fail a request fast instead of blocking forever on an exhausted pool),
   `idle_timeout` and `max_lifetime` (reap zombie/stale connections).
5. `sqlx::migrate!("../../migrations")` — apply pending schema migrations.
6. `Cipher::from_key(at_rest_key)` — fail fast on a malformed `AT_REST_KEY` rather
   than deep inside a request (`AppState::new` re-checks with an `expect` that
   therefore never fires).
7. `AppState::new(config, pool)` behind `Arc` — the shared state every handler holds.
8. `store.delete_instance_presence(instance_id)` — clear any presence rows this
   instance left behind on a previous run (issue #45); instance ids are per-process,
   so rows from *other* crashed instances are left to the age-based sweep. A failure
   here is logged and non-fatal.
9. `bus::spawn(state)` — the cross-instance LISTEN/NOTIFY listener (collab ops,
   presence, relay wakes between replicas — issue #45).
10. Spawn `maintenance_loop(state, retention_days, lines_gc_days,
    resource_purge_days)` — must run regardless of the retention knobs because it
    also owns presence heartbeat/sweep and the collab-outbox prune.
11. Bind `0.0.0.0:PORT`, log readiness, and serve with
    `into_make_service_with_connect_info::<SocketAddr>()` — **required** so the
    rate-limit middleware can key on the peer IP (tests do the same) — and
    `with_graceful_shutdown(shutdown_signal(grace))`.

Errors at any step abort the boot with `anyhow` context.

**Dependencies** — `Config::from_env` (`config.rs`), `Cipher::from_key`
(`crypto.rs`), `AppState::new` (`state.rs`), `Store::delete_instance_presence`
(`store.rs`), `bus::spawn` (`bus.rs`), `router` (`http.rs`), `maintenance_loop` /
`shutdown_signal` (this file); `sqlx`, `axum`, `tokio` (external).

**Used by** — the operating system; nothing imports it.

**Repeated context** — The bounded-pool numbers exist to convert saturation into
fast, visible per-request errors (an ops invariant tested by the soak drill). The
`ConnectInfo` requirement is shared verbatim by every test harness that spawns the
router — forgetting it breaks extraction on all requests.

---

## fn shutdown_signal

**Identification** — private async function; marker `// md:fn shutdown_signal`.

```rust
async fn shutdown_signal(grace: u64)
```

**What it does** — Resolves when the process receives `SIGTERM`
(containers/systemd/Kubernetes) or `Ctrl-C` — on non-Unix targets only `Ctrl-C` is
wired and the SIGTERM arm is `pending()`. On resolution: logs the drain start, and
**arms a watchdog** task that sleeps `grace` seconds
(`SHUTDOWN_GRACE_SECS`, default 20) and then `std::process::exit(0)`. Rationale:
axum's graceful shutdown drains in-flight REST requests, but long-lived
collaborative WebSocket connections would otherwise keep the process alive forever;
the watchdog bounds shutdown so rolling restarts are safe.

**Dependencies** — `tokio::signal` (external), `tokio::spawn`/`sleep`, `tracing`.

**Used by** — `fn main` (passed to `with_graceful_shutdown`).

**Repeated context** — The `expect`s on handler installation are boot-time-only
panics (installing signal handlers cannot fail in any environment the server
supports). Exit code 0 on the forced path: a bounded shutdown is a *successful*
shutdown from the orchestrator's point of view.

---

## fn maintenance_loop

**Identification** — private async function; marker `// md:fn maintenance_loop`.

```rust
async fn maintenance_loop(
    state: Arc<AppState>,
    retention_days: u64,
    lines_gc_days: u64,
    resource_purge_days: u64,
)
```

**What it does** — The background upkeep task, running two cadences in one
`tokio::select!` loop:

- **Presence tick, every 60 s** (`PRESENCE_TICK`; consts defined in this block):
  (1) `touch_instance_presence(instance_id)` — heartbeat this instance's own
  presence rows; (2) `sweep_presence(now − PRESENCE_TTL_SECS)` (TTL 150 s) — drop
  rows no live instance is heartbeating, so a crashed instance's ghost presence
  clears promptly (issue #45); (3) `prune_collab_events(now −
  COLLAB_EVENT_TTL_SECS)` (TTL 300 s) — prune the delivered collab outbox. This
  cadence is deliberately much shorter than the hourly retention work.
- **Retention tick, every 3600 s**: delegate to `run_retention` (next section).

Every failure is logged (`warn`) and the loop continues — maintenance must never
kill the server. Never returns.

**Dependencies** — `Store::{touch_instance_presence, sweep_presence,
prune_collab_events}` (`store.rs`), `run_retention` (this file), `tokio::time`,
`chrono`, `tracing`.

**Used by** — spawned once by `fn main`.

**Repeated context** — Presence is ephemeral, per-connection state stored in a
shared table keyed `(note_id, instance_id, conn_id)` (multi-instance model,
issue #45); heartbeat + TTL sweep is its self-healing mechanism. The collab outbox
(`collab_events`) is safe to prune aggressively because a client that missed an op
rebuilds from the next `Welcome` snapshot — the collaborative channel keeps no
replay history.

---

## fn run_retention

**Identification** — private async function; marker `// md:fn run_retention`.

```rust
async fn run_retention(
    state: &Arc<AppState>,
    retention_days: u64,
    lines_gc_days: u64,
    resource_purge_days: u64,
)
```

**What it does** — The hourly retention pass, split out of `maintenance_loop` so the
loop can also run the shorter presence cadence. Four independent, individually
logged, failure-tolerant steps:

1. `retention_days > 0` → `prune_delivered_changes(now − retention_days)`: delete
   relay-journal rows older than the cutoff **and** already delivered to every
   device of the owning user (conservative: an undelivered row is never pruned).
2. `lines_gc_days > 0` → `gc_line_tombstones(now − lines_gc_days)`: compact line
   tombstones soft-deleted long ago (design §6.4) — tombstones must outlive every
   device's last sync, hence the long default window (30 days).
3. `resource_purge_days > 0` → `purge_deleted_resource_blobs(cutoff)`: reclaim the
   binary payloads of long-soft-deleted resources; the metadata tombstone is kept
   (issue #24 — bytes are reclaimable, convergence metadata is not).
4. Unconditionally: `prune_login_attempts(now − 24 h)` (rows a day old are long
   past any lockout window — `LOGIN_LOCKOUT_SECS` defaults to 300 s) and
   `prune_email_tokens(now − 24 h)` (flow tokens expired a day ago are dead weight,
   used or not).

**Dependencies** — `Store::{prune_delivered_changes, gc_line_tombstones,
purge_deleted_resource_blobs, prune_login_attempts, prune_email_tokens}`
(`store.rs`), `chrono`, `tracing`.

**Used by** — `maintenance_loop` (this file) only.

**Repeated context** — **Soft-delete discipline**: entities are tombstoned
(`deleted_at`), never hard-deleted, so deletions replicate; the GC/purge steps here
are the *only* places tombstoned data is physically reclaimed, always behind an
operator-set age window (`0` = never). The journal-prune step interacts with the
per-device delivery cursors (`device_cursors`): a device that never connects can
block pruning for its user (known hazard, issue #23).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `main()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `maintenance_loop()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `run_retention()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `shutdown_signal()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×2; e.g. `AppState`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

Every code block of `main.rs`, in source order, each documented above (five points)
and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `fn main` | `// md:fn main` | fn main |
| 3 | `fn shutdown_signal` | `// md:fn shutdown_signal` | fn shutdown_signal |
| 4 | `fn maintenance_loop` (incl. its consts) | `// md:fn maintenance_loop` | fn maintenance_loop |
| 5 | `fn run_retention` | `// md:fn run_retention` | fn run_retention |
