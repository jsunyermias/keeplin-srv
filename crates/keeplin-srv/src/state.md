# `state.rs` — shared application state

Self-contained companion for `crates/keeplin-srv/src/state.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able to
understand `state.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `state.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

```rust
use sqlx::{Pool, Postgres};
use uuid::Uuid;

use crate::{
    collab::CollabRegistry, config::Config, ratelimit::RateLimiter, store::Store, sync::SyncHub,
};
```

**What it does** — This module defines `AppState`, the single value every axum handler,
middleware and background task holds (always as `Arc<AppState>`). It bundles the process
configuration, the data-access layer, the live in-memory registries for both WebSocket
surfaces (collaborative channel and device relay), the rate limiter, the per-process
instance identity, and the mailer. Nothing else lives here.

**Dependencies** — `sqlx` (external crate): `Pool<Postgres>`, the bounded connection pool
built in `main.rs`. `uuid` (external crate): the `instance_id`. Internal:
`crate::collab::CollabRegistry` (`collab.rs`), `crate::config::Config` (`config.rs`),
`crate::ratelimit::RateLimiter` (`ratelimit.rs`), `crate::store::Store` (`store.rs`),
`crate::sync::SyncHub` (`sync.rs`); plus `crate::crypto::Cipher` (`crypto.rs`) and
`crate::mail::Mailer` (`mail.rs`) referenced by path inside the constructor.

**Used by** — every subsystem: `http.rs` (all handlers extract `State<Arc<AppState>>`),
`auth.rs` (`auth_mw`), `collab.rs`, `sync.rs`, `bus.rs`, `ratelimit.rs`, `main.rs`
(builds it at boot and hands it to the maintenance loop), and every integration test
under `tests/` (they call `AppState::new` against a throwaway pool).

**Repeated context** — Concurrency model of the process: `AppState` is constructed once
and shared immutably behind `Arc`; all mutable in-memory state (`SyncHub`,
`CollabRegistry`, `RateLimiter`) uses interior locking, so no handler ever needs `&mut`.
All **durable** state lives in PostgreSQL behind the `Store`; everything held here in
memory is per-instance and rebuildable, which is what allows multi-replica deployments
coordinated only by the Postgres `LISTEN/NOTIFY` bus (`bus.rs`, issue #45).

---

## AppState

**Identification** — struct; marker `// md:AppState`.

```rust
pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub hub: SyncHub,
    pub collab: CollabRegistry,
    pub rate_limiter: RateLimiter,
    pub instance_id: Uuid,
    pub mailer: crate::mail::Mailer,
}
```

**What it does** — The shared handler context. Field by field:

- `config: Config` — the process settings loaded from environment variables at startup
  (`config.rs`); read-only after boot.
- `store: Store` — the single PostgreSQL data-access layer (`store.rs`); every SQL
  statement in the server goes through it. Carries the at-rest `Cipher` internally so
  encryption/decryption of `notes.title` / `lines.content` happens at this one choke
  point.
- `hub: SyncHub` — per-user fan-out registry for the device sync relay (`GET /api/sync`,
  `sync.rs`): live relay connections per user, woken when a journal batch lands.
- `collab: CollabRegistry` — per-note collaborative sessions (`GET /api/ws`,
  `collab.rs`): live subscribers, per-note server sequence, presence.
- `rate_limiter: RateLimiter` — per-IP token-bucket request limiter (`ratelimit.rs`);
  a no-op when `RATE_LIMIT_PER_MIN = 0`.
- `instance_id: Uuid` — identity of this server process, minted fresh (UUIDv4) at every
  startup. Stamped on collab fan-out events and presence rows so an instance can tell
  its own writes apart from a sibling's over the cross-instance bus (issue #45): bus
  handlers skip notifications whose origin is their own `instance_id` because the origin
  always delivers locally and synchronously.
- `mailer: crate::mail::Mailer` — delegated email delivery via the operator's mail
  webhook (issue #49); when `MAIL_WEBHOOK_URL` is unset the mailer is disabled and the
  email flows answer `501` (explicit deferral, never silent mail loss).

**Dependencies** — the types of its seven fields: `Config` (`config.rs`), `Store`
(`store.rs`), `SyncHub` (`sync.rs`), `CollabRegistry` (`collab.rs`), `RateLimiter`
(`ratelimit.rs`), `Uuid` (external `uuid` crate), `Mailer` (`mail.rs`).

**Used by** — held as `Arc<AppState>` by: the axum router state (`http.rs::router`),
`auth::auth_mw`, `ratelimit::rate_limit_mw`, every REST handler in `http.rs`, both
WebSocket engines (`collab.rs`, `sync.rs`), the bus listener (`bus.rs::spawn/run`), the
maintenance loop (`main.rs`), and the integration tests (`tests/*.rs`) which build it
directly.

**Repeated context** — The **instance-identity convention** (issue #45): a
multi-replica deployment coordinates over Postgres `LISTEN/NOTIFY`; every notification
payload carries `origin_instance`, and the origin instance ignores its own events since
it already broadcast locally. `instance_id` existing per process (not per deployment) is
what makes that echo-suppression work. The **single-choke-point encryption** convention:
the `Cipher` never leaves the `Store`; handlers cannot accidentally read or write the
wrong form.

---

## impl AppState

**Identification** — inherent impl block; marker `// md:impl AppState`. Contains one
constructor, `fn new` (next section).

**What it does** — Construction only; `AppState` has no other methods, because all
behaviour lives in the subsystems it holds.

**Dependencies** — `AppState` (this file).

**Used by** — see `fn new`.

**Repeated context** — none beyond the constructor's own (below).

### fn new

**Identification** — associated function; marker `// md:impl AppState > fn new`.

```rust
pub fn new(config: Config, pool: Pool<Postgres>) -> Self
```

**What it does** — Builds the complete state from the loaded configuration and an
already-connected PostgreSQL pool:

1. Sizes the `RateLimiter` from `config.rate_limit_per_min` (0 = disabled/no-op).
2. Builds the at-rest `Cipher` from `config.at_rest_key`
   (`crypto::Cipher::from_key`) and **`expect`s** it: a present-but-invalid
   `AT_REST_KEY` is a fatal misconfiguration. `main.rs` validates the same key before
   calling this, so the `expect` never fires in a real boot — it exists so a direct
   caller (a test) cannot construct a state that silently stores plaintext.
3. Builds the `Mailer` from `config.mail_webhook_url` / `config.mail_webhook_token`.
4. Assembles the struct: `Store::with_cipher(pool, cipher)`, `SyncHub::default()`
   (empty), `CollabRegistry::default()` (empty), a fresh `instance_id`
   (`Uuid::new_v4()`).

Infallible in signature; panics only on the invalid-key case described above.

**Dependencies** — `RateLimiter::new` (`ratelimit.rs`), `crypto::Cipher::from_key`
(`crypto.rs`), `mail::Mailer::new` (`mail.rs`), `Store::with_cipher` (`store.rs`),
`SyncHub::default` (`sync.rs`), `CollabRegistry::default` (`collab.rs`),
`Uuid::new_v4` (external).

**Used by** — `main.rs` (the real boot, after validating the key and running
migrations) and every integration test that spins up a server
(`tests/collab_e2e_common/mod.rs`, `tests/integration.rs`, `tests/collab.rs`,
`tests/materialize.rs`, `tests/quotas.rs`, `tests/soak.rs`, `tests/reencrypt.rs`) —
public precisely so tests build the exact same state a real boot would.

**Repeated context** — **Fail-fast configuration**: a present-but-invalid `AT_REST_KEY`
must abort startup, never fall back to plaintext silently (same rule enforced in
`main.rs` and `crypto.rs::Cipher::from_key`). **Per-process identity**: `instance_id`
is minted here on every construction; two replicas of the same deployment always differ.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

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

## Coverage checklist

Every code block of `state.rs`, in source order, each documented above (five points) and
carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `struct AppState` | `// md:AppState` | AppState |
| 3 | `impl AppState` | `// md:impl AppState` | impl AppState |
| 4 | `fn new` | `// md:impl AppState > fn new` | impl AppState › fn new |
