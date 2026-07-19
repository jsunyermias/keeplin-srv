# `tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries

Self-contained companion for `crates/keeplin-srv/tests/collab_e2e_common/mod.rs`. It
documents **every code block of the source file, in source order, with its complete code embedded** — a reader with only
this file must be able to understand the harness without opening anything else, so
project-wide conventions are deliberately re-explained here (hyper-redundancy is
intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the `#![allow(dead_code)]` inner attribute and
imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;

use keeplin_core::{
    collab::{CollabBackend, CollabConfig},
    storage::{db::DbBackend, NoteRepository, StorageBackend},
};
use keeplin_srv::{config::Config, http::router, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;
```

**What it does** — Shared setup for the `collab_client_*_e2e` test binaries. Each e2e
test lives in its **own** integration-test binary (cargo runs test binaries
sequentially), so the real client's background tasks — reconnect loops, the second
`/api/sync` connection — die with the process instead of hammering the shared
`#[sqlx::test]` PostgreSQL harness while the next test runs; that cross-test
interference is what made these tests flaky in one binary (issue #51).
`#![allow(dead_code)]` because each binary uses only a subset of the harness.

**Dependencies** — `keeplin_core` (`CollabBackend`, `CollabConfig`, `DbBackend`,
storage traits), `keeplin_srv` (`Config`, `router`, `AppState`), `axum`, `sqlx`,
`reqwest`, `tempfile`, `tokio`, `serde_json`, `uuid`.

**Used by** — included via `#[path = "collab_e2e_common/mod.rs"] mod common;` by
`collab_client_e2e.rs`, `collab_client_reconnect_e2e.rs`,
`collab_client_resources_e2e.rs`.

**Repeated context** — Harness invariant: every new real-client e2e scenario gets its
**own** `tests/<name>_e2e.rs` binary including this module via `#[path]` — never a
second scenario in an existing binary (issue #51).

---

## fn test_config

**Identification** — pub fn; marker `// md:fn test_config`.

**Code** — complete and verbatim:

```rust
// md:fn test_config
pub fn test_config() -> Config {
    Config {
        port: 0,
        database_url: String::new(),
        jwt_secret: "test-secret".into(),
        token_ttl_days: 1,
        retention_days: 0,
        lines_gc_days: 0,
        resource_purge_days: 0,
        db_max_connections: 5,
        db_acquire_timeout_secs: 10,
        db_idle_timeout_secs: 600,
        db_max_lifetime_secs: 1800,
        rate_limit_per_min: 0,
        shutdown_grace_secs: 5,
        log_json: false,
        max_upload_bytes: 100 * 1024 * 1024,
        max_note_body_bytes: 0,
        max_user_storage_bytes: 0,
        max_notes_per_user: 0,
        registration_enabled: true,
        at_rest_key: None,
        mail_webhook_url: None,
        mail_webhook_token: None,
        email_token_ttl_secs: 3600,
        email_verification_required: false,
        login_max_failures: 0,
        login_lockout_secs: 300,
        history_since_access: false,
    }
}
```

**What it does** — The standard test `Config` literal: registration open, no
quotas/rate limit/at-rest key/mail webhook/lockout, 5-connection pool, ephemeral
port. Built literally (never `from_env`) so the environment cannot leak into test
behaviour.

**Dependencies** — `keeplin_srv::config::Config`. **Used by** — `spawn_server`.

**Repeated context** — Every test suite in the repo has its own `test_config()`
twin; a new `Config` field must be added to all of them (compile error makes the
omission loud).

---

## fn spawn_server

**Identification** — pub async fn; marker `// md:fn spawn_server`.

**Code** — complete and verbatim:

```rust
// md:fn spawn_server
pub async fn spawn_server(pool: PgPool) -> SocketAddr {
    let state = Arc::new(AppState::new(test_config(), pool));
    let app: Router = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}
```

**What it does** — Boots the real router (`AppState::new(test_config(), pool)` →
`router`) on an ephemeral loopback port, served with
`into_make_service_with_connect_info::<SocketAddr>()` (required by the rate-limit
middleware's `ConnectInfo` extractor even when limiting is off), on a spawned task.
Returns the bound address.

**Dependencies** — `test_config` (this file); `AppState::new`, `router`
(`keeplin-srv`); `tokio`. **Used by** — all three e2e binaries.

**Repeated context** — In-process server on a `#[sqlx::test]` throwaway database:
the same pattern every suite uses — no Docker, no external server.

---

## fn register

**Identification** — pub async fn; marker `// md:fn register`. POSTs
`/api/register` over the real HTTP surface (fixed password `password123`).
**Dependencies** — `reqwest`, `serde_json`. **Used by** — all three e2e binaries.
**Repeated context** — none.

**Code** — complete and verbatim:

```rust
// md:fn register
pub async fn register(addr: SocketAddr, email: &str) {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
}
```

## fn login

**Identification** — pub async fn; marker `// md:fn login`. POSTs `/api/login`
with a device name and returns the **device token** string. **Dependencies** —
`reqwest`, `serde_json`. **Used by** — all three e2e binaries.
**Repeated context** — one login = one device = one token (device-as-actor); the
e2e binaries create a second device by calling `login` again with another name.

**Code** — complete and verbatim:

```rust
// md:fn login
pub async fn login(addr: SocketAddr, email: &str, device: &str) -> String {
    let body: Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": email, "password": "password123", "device_name": device }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["token"].as_str().unwrap().to_string()
}
```

---

## fn collab_device

**Identification** — pub async fn; marker `// md:fn collab_device`.

**Code** — complete and verbatim:

```rust
// md:fn collab_device
pub async fn collab_device(addr: SocketAddr, token: &str) -> Arc<CollabBackend<DbBackend>> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dev.db");
    std::mem::forget(dir);
    let db = DbBackend::new(path, &format!("ws://{addr}/api/sync"), token)
        .await
        .unwrap();
    let collab = Arc::new(
        CollabBackend::new(
            db,
            CollabConfig {
                api_url: format!("http://{addr}"),
                ws_url: format!("ws://{addr}/api/ws"),
                token: token.to_string(),
            },
        )
        .unwrap(),
    );
    let top: Arc<dyn StorageBackend> = collab.clone();
    collab.start(top).await.expect("protocol handshake");
    collab
}
```

**What it does** — Builds the **exact client stack the daemon mounts** in
server+collab mode: a `DbBackend` (relay, `ws://…/api/sync`) on a fresh temp SQLite
file, wrapped in `CollabBackend` (`/api/ws` line channel, REST `api_url`), then
`start`ed with itself as the top of the stack. `start` runs the `GET /version`
protocol handshake — this server is the matching keeplin-srv, so it must negotiate
cleanly (`expect("protocol handshake")`). The tempdir is `std::mem::forget`-leaked
so the SQLite file outlives the call.

**Dependencies** — keeplin-core `DbBackend::new`, `CollabBackend::new`/`start`,
`CollabConfig`; `tempfile`. **Used by** — all three e2e binaries.

**Repeated context** — Protocol-compatibility contract: `PROTOCOL_VERSION`
(server `http.rs`) is mirrored by keeplin-core's `compat.rs` and enforced at client
startup; this helper is where a drift between the pinned client and this server
fails loudly in CI.

---

## CONVERGE_TRIES

**Identification** — pub const; marker `// md:CONVERGE_TRIES`.
`pub const CONVERGE_TRIES: usize = 300;`

**Code** — complete and verbatim:

```rust
// md:CONVERGE_TRIES
pub const CONVERGE_TRIES: usize = 300;
```

**What it does** — The convergence-poll bound (~30 s at 100 ms per try). Generous on
purpose: these tests drive the *real* client (its own async connect/reconnect plus
a second `/api/sync` connection), so convergence latency tracks database
throughput; under a busy CI database a tight deadline flakes even though the client
converges fine.

**Dependencies** — none. **Used by** — the wait helpers below and the resources e2e
binary directly.

**Repeated context** — Do not tighten: that reintroduces the CI flake issue #51
work eliminated.

---

## fn wait_server_body

**Identification** — pub async fn; marker `// md:fn wait_server_body`.

**Code** — complete and verbatim:

```rust
// md:fn wait_server_body
pub async fn wait_server_body(addr: SocketAddr, token: &str, note_id: Uuid, want: &str) {
    let client = reqwest::Client::new();
    let mut last = String::new();
    for _ in 0..CONVERGE_TRIES {
        if let Ok(resp) = client
            .get(format!("http://{addr}/api/notes/{note_id}/export"))
            .bearer_auth(token)
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(v) = resp.json::<Value>().await {
                    last = v["body"].as_str().unwrap_or_default().to_string();
                    if last == want {
                        return;
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("server body never became {want:?}; last {last:?}");
}
```

**What it does** — Polls `GET /api/notes/:id/export` (bearer token) until the
materialised body equals `want`, tolerating the transient 404/empty window before
the note's lines exist; panics with the last observed body after `CONVERGE_TRIES`.

**Dependencies** — `reqwest`, `CONVERGE_TRIES`. **Used by** — the write-through and
reconnect binaries.

**Repeated context** — Export returns the server's derived body (live lines joined
with `\n`) — the strongest server-side convergence signal available over REST.

---

## fn wait_local_body

**Identification** — pub async fn; marker `// md:fn wait_local_body`.

**Code** — complete and verbatim:

```rust
// md:fn wait_local_body
pub async fn wait_local_body(dev: &Arc<CollabBackend<DbBackend>>, note_id: Uuid, want: &str) {
    let mut last = String::new();
    for _ in 0..CONVERGE_TRIES {
        if let Ok(note) = dev.read_note(note_id).await {
            last = note.body.clone();
            if last == want {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("local body never became {want:?}; last {last:?}");
}
```

**What it does** — Polls a client's local `read_note(note_id).body` until it equals
`want`; panics with the last observed value after `CONVERGE_TRIES`.

**Dependencies** — keeplin-core `NoteRepository::read_note`, `CONVERGE_TRIES`.
**Used by** — the reconnect binary.

**Repeated context** — none.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `collab_device()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `wait_server_body()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `test_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_server()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `wait_local_body()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `register()` — defined here (EXTRACTED; file-local)
- `login()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/tests/collab_client_e2e.rs` — real daemon client ↔ real server (EXTRACTED: calls×2)
- `crates/keeplin-srv/tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (EXTRACTED: calls×3)
- `crates/keeplin-srv/tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (EXTRACTED: calls×1)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `#![allow(dead_code)]` + imports | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_server` | `// md:fn spawn_server` |
| 4 | `fn register` | `// md:fn register` |
| 5 | `fn login` | `// md:fn login` |
| 6 | `fn collab_device` | `// md:fn collab_device` |
| 7 | `CONVERGE_TRIES` | `// md:CONVERGE_TRIES` |
| 8 | `fn wait_server_body` | `// md:fn wait_server_body` |
| 9 | `fn wait_local_body` | `// md:fn wait_local_body` |
