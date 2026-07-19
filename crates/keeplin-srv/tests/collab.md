# `tests/collab.rs` — collaborative channel & hardening tests

Self-contained companion for `crates/keeplin-srv/tests/collab.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand the suite without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context** (compressed for
straightforward tests).

---

## Overview

**Identification** — file-level block: the imports and the `Ws` type alias. Marker
`// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use keeplin_srv::{config::Config, http::router, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
```

**What it does** — End-to-end tests of the `/api/ws` collaborative protocol and the
production-hardening surfaces, driven over a **real WebSocket and real HTTP** against
a real server on a throwaway `#[sqlx::test]` PostgreSQL database. No mocking: the
tests register users, log devices in, open raw `tokio-tungstenite` sockets, and send
hand-built protocol frames (deliberately not importing `protocol.rs`, so a wire-shape
drift breaks these tests). Ops are signed with the login's `device_id` — the vv
actor. Also covers the capability model (Front B), the notebook cascade, the
folder-owner rule, the inbox nil-UUID mapping, probes, metrics auth and the rate
limiter.

**Dependencies** — `tokio_tungstenite`, `futures_util`, `keeplin_srv` (`Config`,
`router`, `AppState`, `bus::spawn`, `Store` via `state.store`), keeplin-core models
(notebook fixtures), `reqwest`, `sqlx`, `serde_json`, `uuid`.

**Used by** — `cargo test`; CI.

**Repeated context** — Protocol contract exercised here: `Join` → `Welcome` (full
snapshot: versioned order + all lines) then `Presence`; `Op` batches are validated
(writer identity, limits, existence), resolved by version vector with the
`(timestamp, writer)` LWW tiebreak, persisted, and fanned out with a monotonic
`server_seq`; errors are per-frame (`Error{code}`) and never close the connection.

---

## Helpers

Compressed entries; each block carries its own marker:

### fn test_config


**Code** — complete and verbatim:

```rust
// md:fn test_config
fn test_config() -> Config {
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

Marker `// md:fn test_config`. The standard test `Config` literal.

### fn spawn_server


**Code** — complete and verbatim:

```rust
// md:fn spawn_server
async fn spawn_server(pool: PgPool) -> SocketAddr {
    spawn_server_with_state(pool).await.0
}
```

Marker `// md:fn spawn_server`. `spawn_server_with_state(pool).await.0` — router
only, no bus.

### fn spawn_instance


**Code** — complete and verbatim:

```rust
// md:fn spawn_instance
async fn spawn_instance(pool: PgPool) -> SocketAddr {
    let state = Arc::new(AppState::new(test_config(), pool));
    keeplin_srv::bus::spawn(state.clone());
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

Marker `// md:fn spawn_instance`. A **bus-enabled** instance (issue #45) — only the
cross-instance test uses it, so the other tests avoid holding a permanent
`PgListener` connection.

### fn spawn_server_with_state


**Code** — complete and verbatim:

```rust
// md:fn spawn_server_with_state
async fn spawn_server_with_state(pool: PgPool) -> (SocketAddr, Arc<AppState>) {
    let state = Arc::new(AppState::new(test_config(), pool));
    let app: Router = router(state.clone());
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
    (addr, state)
}
```

Marker `// md:fn spawn_server_with_state`. Like `spawn_server` but also returns the
`Arc<AppState>`, for tests that poke the store directly (tombstone GC, notebook
fixtures).

### fn user


**Code** — complete and verbatim:

```rust
// md:fn user
async fn user(addr: SocketAddr, email: &str) -> (String, String, String) {
    let client = reqwest::Client::new();
    let reg: Value = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let user_id = reg["user"]["id"].as_str().unwrap().to_string();
    let login: Value = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": email, "password": "password123", "device_name": "test" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (
        user_id,
        login["device_id"].as_str().unwrap().to_string(),
        login["token"].as_str().unwrap().to_string(),
    )
}
```

Marker `// md:fn user`. Register + login; returns `(user_id, device_id, token)` —
ops must be signed with the **device** id.

### fn create_note


**Code** — complete and verbatim:

```rust
// md:fn create_note
async fn create_note(addr: SocketAddr, token: &str, title: &str) -> String {
    let note: Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/notes"))
        .bearer_auth(token)
        .json(&json!({ "title": title }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    note["id"].as_str().unwrap().to_string()
}
```

Marker `// md:fn create_note`. `POST /api/notes`, returns the note id.

### fn share


**Code** — complete and verbatim:

```rust
// md:fn share
async fn share(addr: SocketAddr, token: &str, note_id: &str, email: &str, role: &str) {
    let capabilities = match role {
        "editor" => 3,
        "viewer" => 1,
        other => panic!("unknown test role {other}"),
    };
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/notes/{note_id}/share"))
        .bearer_auth(token)
        .json(&json!({ "user_email": email, "capabilities": capabilities }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}
```

Marker `// md:fn share`. Grant by email with test roles mapped to capability bits:
`editor` = READ|WRITE (3), `viewer` = READ (1).

### fn ws_connect


**Code** — complete and verbatim:

```rust
// md:fn ws_connect
async fn ws_connect(addr: SocketAddr, token: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={token}"))
        .await
        .unwrap();
    ws
}
```

Marker `// md:fn ws_connect`. Raw WS to `ws://…/api/ws?token=…`.

### fn send


**Code** — complete and verbatim:

```rust
// md:fn send
async fn send(ws: &mut Ws, msg: Value) {
    ws.send(Message::Text(msg.to_string())).await.unwrap();
}
```

Marker `// md:fn send`. Send one JSON frame.

### fn recv_until


**Code** — complete and verbatim:

```rust
// md:fn recv_until
async fn recv_until(ws: &mut Ws, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..50 {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
            .expect("socket closed")
            .expect("socket error");
        if let Message::Text(text) = msg {
            let v: Value = serde_json::from_str(&text).unwrap();
            if pred(&v) {
                return v;
            }
        }
    }
    panic!("gave up waiting for {what}");
}
```

Marker `// md:fn recv_until`. Receive frames until a predicate matches (skipping
presence chatter and other noise), panicking after a bounded wait — the suite's
convergence primitive on the socket side.

### fn join


**Code** — complete and verbatim:

```rust
// md:fn join
fn join(note_id: &str) -> Value {
    json!({ "type": "Join", "note_id": note_id })
}
```

Marker `// md:fn join`. Frame builder for `Join`.

### fn insert_op


**Code** — complete and verbatim:

```rust
// md:fn insert_op
#[allow(clippy::too_many_arguments)]
fn insert_op(
    note_id: &str,
    line_id: &str,
    after: Option<&str>,
    content: &str,
    writer: &str,
    counter: u64,
    ts: &str,
) -> Value {
    json!({
        "type": "Op",
        "note_id": note_id,
        "ops": [{
            "op": "Insert",
            "after_line_id": after,
            "line_id": line_id,
            "content": content,
            "vv": { writer: counter },
            "last_writer": writer,
            "updated_at": ts,
        }],
    })
}
```

Marker `// md:fn insert_op`. Frame builder for a single-op `Insert` envelope (vv,
writer, timestamp explicit).

### fn update_op


**Code** — complete and verbatim:

```rust
// md:fn update_op
fn update_op(
    note_id: &str,
    line_id: &str,
    content: &str,
    vv: Value,
    writer: &str,
    ts: &str,
) -> Value {
    json!({
        "type": "Op",
        "note_id": note_id,
        "ops": [{
            "op": "Update",
            "line_id": line_id,
            "content": content,
            "vv": vv,
            "last_writer": writer,
            "updated_at": ts,
        }],
    })
}
```

Marker `// md:fn update_op`. Frame builder for a single-op `Update` envelope.

### fn export_body


**Code** — complete and verbatim:

```rust
// md:fn export_body
async fn export_body(addr: SocketAddr, token: &str, note_id: &str) -> String {
    let v: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes/{note_id}/export"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["body"].as_str().unwrap().to_string()
}
```

Marker `// md:fn export_body`. `GET /api/notes/:id/export` → the materialised body.

### fn wait_export


**Code** — complete and verbatim:

```rust
// md:fn wait_export
async fn wait_export(addr: SocketAddr, token: &str, note_id: &str, expected: &str) {
    let mut last = String::new();
    for _ in 0..50 {
        last = export_body(addr, token, note_id).await;
        if last == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("body never converged to {expected:?}; last seen: {last:?}");
}
```

Marker `// md:fn wait_export`. Poll the export until it equals the expected body
(~5 s), panicking with the last seen value — ops apply asynchronously to the HTTP
surface, and fixed sleeps are exactly what flakes on slow CI runners.

### Timestamps


**Code** — complete and verbatim:

```rust
// md:Timestamps
const T1: &str = "2026-01-01T10:00:00Z";
const T2: &str = "2026-01-01T10:00:01Z";
const T3: &str = "2026-01-01T10:00:02Z";
```

Marker `// md:Timestamps`. `T1`/`T2`/`T3`: three fixed, ordered RFC3339 timestamps
used as deterministic op times.

---

## Protocol tests

### fn join_receives_welcome_snapshot


**Code** — complete and verbatim:

```rust
// md:fn join_receives_welcome_snapshot
#[sqlx::test(migrations = "../../migrations")]
async fn join_receives_welcome_snapshot(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Nota").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;

    let welcome = recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;
    assert_eq!(welcome["note_id"].as_str().unwrap(), note_id);
    assert_eq!(welcome["snapshot"]["order"].as_array().unwrap().len(), 0);
    assert_eq!(welcome["snapshot"]["lines"].as_array().unwrap().len(), 0);

    let presence = recv_until(&mut ws, "Presence", |v| v["type"] == "Presence").await;
    assert_eq!(presence["users"].as_array().unwrap().len(), 1);
}
```

Marker `// md:fn join_receives_welcome_snapshot`. Joining an empty note yields
`Welcome` (empty order/lines) and then a `Presence` list containing yourself.

### fn ops_propagate_between_participants


**Code** — complete and verbatim:

```rust
// md:fn ops_propagate_between_participants
#[sqlx::test(migrations = "../../migrations")]
async fn ops_propagate_between_participants(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Compartida").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "hola desde A", &did_a, 1, T1),
    )
    .await;
    let op_at_b = recv_until(&mut ws_b, "Op at B", |v| v["type"] == "Op").await;
    assert_eq!(op_at_b["user_id"].as_str().unwrap(), uid_a);
    assert!(op_at_b["server_seq"].as_u64().unwrap() >= 1);
    let received = &op_at_b["ops"][0];
    assert_eq!(received["op"], "Insert");
    assert_eq!(received["content"], "hola desde A");

    send(
        &mut ws_b,
        update_op(
            &note_id,
            &line_id,
            "editada por B",
            json!({ did_a.clone(): 1, did_b.clone(): 1 }),
            &did_b,
            T2,
        ),
    )
    .await;
    let op_at_a = recv_until(&mut ws_a, "Op at A", |v| v["type"] == "Op").await;
    assert_eq!(op_at_a["ops"][0]["op"], "Update");
    assert_eq!(op_at_a["ops"][0]["content"], "editada por B");

    assert_eq!(export_body(addr, &token_a, &note_id).await, "editada por B");
}
```

Marker `// md:fn ops_propagate_between_participants`. A inserts (B receives the
`Op` with A's **user** id and a `server_seq ≥ 1`); B updates having seen A's write
(vv covering both device components); A receives it; the exported body reflects the
final state.

### fn ops_and_presence_propagate_across_instances


**Code** — complete and verbatim:

```rust
// md:fn ops_and_presence_propagate_across_instances
#[sqlx::test(migrations = "../../migrations")]
async fn ops_and_presence_propagate_across_instances(pool: PgPool) {
    let addr_a = spawn_instance(pool.clone()).await;
    let addr_b = spawn_instance(pool.clone()).await;

    let (uid_a, did_a, token_a) = user(addr_a, "a@example.com").await;
    let (_uid_b, did_b, token_b) = user(addr_a, "b@example.com").await;
    let note_id = create_note(addr_a, &token_a, "Cross-instance").await;
    share(addr_a, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_a = ws_connect(addr_a, &token_a).await;
    let mut ws_b = ws_connect(addr_b, &token_b).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    recv_until(&mut ws_a, "merged presence", |v| {
        v["type"] == "Presence" && v["users"].as_array().map(|u| u.len()) == Some(2)
    })
    .await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "from instance A", &did_a, 1, T1),
    )
    .await;
    let op_at_b = recv_until(&mut ws_b, "Op at B across instances", |v| v["type"] == "Op").await;
    assert_eq!(op_at_b["user_id"].as_str().unwrap(), uid_a);
    assert_eq!(op_at_b["ops"][0]["content"], "from instance A");

    let line_b = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(
            &note_id,
            &line_b,
            Some(&line_id),
            "from instance B",
            &did_b,
            1,
            T2,
        ),
    )
    .await;
    let op_at_a = recv_until(&mut ws_a, "Op at A across instances", |v| v["type"] == "Op").await;
    assert_eq!(op_at_a["ops"][0]["content"], "from instance B");

    wait_export(
        addr_b,
        &token_b,
        &note_id,
        "from instance A\nfrom instance B",
    )
    .await;
}
```

Marker `// md:fn ops_and_presence_propagate_across_instances`. Issue #45: two
bus-enabled instances over one database; A on instance A, B on instance B. Presence
**merges** across replicas (A eventually sees 2 users); ops flow both directions
via the outbox + NOTIFY; both replicas converge on the same materialised body.

### fn concurrent_updates_resolve_deterministically


**Code** — complete and verbatim:

```rust
// md:fn concurrent_updates_resolve_deterministically
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_updates_resolve_deterministically(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Conflicto").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "base", &did_a, 1, T1),
    )
    .await;
    recv_until(&mut ws_b, "insert at B", |v| v["type"] == "Op").await;

    send(
        &mut ws_a,
        update_op(
            &note_id,
            &line_id,
            "versión de A",
            json!({ did_a.clone(): 2 }),
            &did_a,
            T2,
        ),
    )
    .await;
    send(
        &mut ws_b,
        update_op(
            &note_id,
            &line_id,
            "versión de B",
            json!({ did_a.clone(): 1, did_b.clone(): 1 }),
            &did_b,
            T3,
        ),
    )
    .await;

    wait_export(addr, &token_a, &note_id, "versión de B").await;
}
```

Marker `// md:fn concurrent_updates_resolve_deterministically`. Both edit the same
line from the same base (`{A:1}`): neither vector dominates, so the deterministic
`(timestamp, writer)` tiebreak decides — B's later-stamped edit wins on every
replica regardless of processing order.

### fn stale_op_is_ignored


**Code** — complete and verbatim:

```rust
// md:fn stale_op_is_ignored
#[sqlx::test(migrations = "../../migrations")]
async fn stale_op_is_ignored(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token_a, "Stale").await;

    let mut ws = ws_connect(addr, &token_a).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws,
        insert_op(&note_id, &line_id, None, "v1", &did_a, 1, T1),
    )
    .await;
    send(
        &mut ws,
        update_op(
            &note_id,
            &line_id,
            "v2",
            json!({ did_a.clone(): 2 }),
            &did_a,
            T2,
        ),
    )
    .await;
    send(
        &mut ws,
        update_op(
            &note_id,
            &line_id,
            "v1-replay",
            json!({ did_a.clone(): 2 }),
            &did_a,
            T2,
        ),
    )
    .await;

    wait_export(addr, &token_a, &note_id, "v2").await;
}
```

Marker `// md:fn stale_op_is_ignored`. A replay carrying the same vv (writer
component does not advance) can never win; converging to "v2" proves the genuine
update applied and the replay was dropped — idempotent application.

### fn move_reorders_lines


**Code** — complete and verbatim:

```rust
// md:fn move_reorders_lines
#[sqlx::test(migrations = "../../migrations")]
async fn move_reorders_lines(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Orden").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    let l1 = uuid::Uuid::new_v4().to_string();
    let l2 = uuid::Uuid::new_v4().to_string();
    let l3 = uuid::Uuid::new_v4().to_string();
    send(&mut ws, insert_op(&note_id, &l1, None, "uno", &did, 1, T1)).await;
    send(
        &mut ws,
        insert_op(&note_id, &l2, Some(&l1), "dos", &did, 2, T1),
    )
    .await;
    send(
        &mut ws,
        insert_op(&note_id, &l3, Some(&l2), "tres", &did, 3, T1),
    )
    .await;

    send(
        &mut ws,
        json!({
            "type": "Op",
            "note_id": note_id,
            "ops": [{
                "op": "Move",
                "line_ids": [l3],
                "after_line_id": null,
                "vv": { did.clone(): 4 },
                "last_writer": did,
                "updated_at": T2,
            }],
        }),
    )
    .await;

    wait_export(addr, &token, &note_id, "tres\nuno\ndos").await;
}
```

Marker `// md:fn move_reorders_lines`. Three inserts then a `Move` of the last line
to the front (`after_line_id: null`); the export shows the reordered body.

---

## Permission tests

### fn viewer_can_watch_but_not_edit


**Code** — complete and verbatim:

```rust
// md:fn viewer_can_watch_but_not_edit
#[sqlx::test(migrations = "../../migrations")]
async fn viewer_can_watch_but_not_edit(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, _did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Solo lectura").await;
    share(addr, &token_a, &note_id, "b@example.com", "viewer").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome viewer", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &line_id, None, "no debería", &did_b, 1, T1),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "forbidden");
    assert_eq!(export_body(addr, &token_b, &note_id).await, "");
}
```

Marker `// md:fn viewer_can_watch_but_not_edit`. A `viewer` (READ) joins fine but
its `Op` gets `Error{forbidden}` and the body stays empty — the collaborative
channel enforces the same `can_write` gate as REST.

### fn revoking_a_share_stops_edits_mid_session


**Code** — complete and verbatim:

```rust
// md:fn revoking_a_share_stops_edits_mid_session
#[sqlx::test(migrations = "../../migrations")]
async fn revoking_a_share_stops_edits_mid_session(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, _did_a, token_a) = user(addr, "a@example.com").await;
    let (uid_b, did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Colaborativa").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome", |v| v["type"] == "Welcome").await;

    let l1 = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &l1, None, "primera", &did_b, 1, T1),
    )
    .await;
    wait_export(addr, &token_a, &note_id, "primera").await;

    let code = reqwest::Client::new()
        .delete(format!("http://{addr}/api/notes/{note_id}/share/{uid_b}"))
        .bearer_auth(&token_a)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);

    let l2 = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &l2, Some(&l1), "segunda", &did_b, 2, T2),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "forbidden");
}
```

Marker `// md:fn revoking_a_share_stops_edits_mid_session`. Issue #30: B edits
while shared; A revokes the share while B **stays connected**; B's next edit gets
`Error{forbidden}` immediately — access is re-resolved per op batch, never cached
for the connection's life.

### fn outsider_cannot_join


**Code** — complete and verbatim:

```rust
// md:fn outsider_cannot_join
#[sqlx::test(migrations = "../../migrations")]
async fn outsider_cannot_join(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, _did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Privada").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "forbidden");
}
```

Marker `// md:fn outsider_cannot_join`. A non-shared user's `Join` gets
`Error{forbidden}`.

### fn presence_shows_other_participants


**Code** — complete and verbatim:

```rust
// md:fn presence_shows_other_participants
#[sqlx::test(migrations = "../../migrations")]
async fn presence_shows_other_participants(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, _did_a, token_a) = user(addr, "ana@example.com").await;
    let (_uid_b, _did_b, token_b) = user(addr, "bob@example.com").await;
    let note_id = create_note(addr, &token_a, "Presencia").await;
    share(addr, &token_a, &note_id, "bob@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    let presence = recv_until(&mut ws_a, "presence with both", |v| {
        v["type"] == "Presence" && v["users"].as_array().is_some_and(|u| u.len() == 2)
    })
    .await;
    let names: Vec<&str> = presence["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["display_name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"ana") && names.contains(&"bob"),
        "{names:?}"
    );

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        json!({
            "type": "Cursor",
            "note_id": note_id,
            "cursor": { "line_id": line_id, "column": 3 },
        }),
    )
    .await;
    let presence = recv_until(&mut ws_a, "presence with cursor", |v| {
        v["type"] == "Presence"
            && v["users"]
                .as_array()
                .is_some_and(|u| u.iter().any(|p| p["cursor"]["column"] == 3))
    })
    .await;
    assert_eq!(presence["note_id"].as_str().unwrap(), note_id);
}
```

Marker `// md:fn presence_shows_other_participants`. Two joined users: A sees a
presence list with both display names; B's `Cursor` frame shows up attached to B's
entry in A's next presence broadcast (presence is user-scoped; lists are full
replacements).

### fn import_then_export_roundtrip


**Code** — complete and verbatim:

```rust
// md:fn import_then_export_roundtrip
#[sqlx::test(migrations = "../../migrations")]
async fn import_then_export_roundtrip(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;

    let body = "# Título\n\nprimera línea\nsegunda línea";
    let imported: Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(&token)
        .json(&json!({ "title": "Importada", "body": body }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let note_id = imported["note_id"].as_str().unwrap();
    assert_eq!(imported["line_count"].as_u64().unwrap(), 4);

    assert_eq!(export_body(addr, &token, note_id).await, body);

    let note: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes/{note_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(note["body"].as_str().unwrap(), body);

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(note_id)).await;
    let welcome = recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;
    assert_eq!(welcome["snapshot"]["order"].as_array().unwrap().len(), 4);
    assert_eq!(welcome["snapshot"]["lines"].as_array().unwrap().len(), 4);
}
```

Marker `// md:fn import_then_export_roundtrip`. Import a 4-line flat body →
export returns it byte-identical; the plain `GET /api/notes/:id` carries the same
materialised body (design §3.4); a `Join` snapshot shows the 4 lines/order entries.

### fn forged_writer_is_rejected


**Code** — complete and verbatim:

```rust
// md:fn forged_writer_is_rejected
#[sqlx::test(migrations = "../../migrations")]
async fn forged_writer_is_rejected(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Firma").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &line_id, None, "suplantación", &did_a, 1, T1),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "bad_writer");
}
```

Marker `// md:fn forged_writer_is_rejected`. B signs an op with **A's** device id →
`Error{bad_writer}`: `last_writer` must equal the sender's authenticated device
(clients cannot forge edits in someone else's name).

---

## Hardening tests

### fn ws_accepts_authorization_header


**Code** — complete and verbatim:

```rust
// md:fn ws_accepts_authorization_header
#[sqlx::test(migrations = "../../migrations")]
async fn ws_accepts_authorization_header(pool: PgPool) {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Header").await;

    let mut req = format!("ws://{addr}/api/ws").into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome via header auth", |v| {
        v["type"] == "Welcome"
    })
    .await;
}
```

Marker `// md:fn ws_accepts_authorization_header`. Connecting with the token only
in the `Authorization: Bearer` header (no `?token=`) works — the preferred,
log-safe form.

### fn deleting_a_device_revokes_its_token


**Code** — complete and verbatim:

```rust
// md:fn deleting_a_device_revokes_its_token
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_device_revokes_its_token(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let client = reqwest::Client::new();

    let second: Value = client
        .post(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .json(&json!({ "device_name": "stolen-phone" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_token = second["token"].as_str().unwrap();
    let second_id = second["device_id"].as_str().unwrap();

    let ok = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(second_token)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);

    let del = client
        .delete(format!("http://{addr}/api/devices/{second_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);

    let denied = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(second_token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
}
```

Marker `// md:fn deleting_a_device_revokes_its_token`. A second device's token
works until the device is deleted from the first device; then REST answers 401 —
revocation-by-deletion on the REST surface.

### fn deleting_a_device_revokes_its_collab_token


**Code** — complete and verbatim:

```rust
// md:fn deleting_a_device_revokes_its_collab_token
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_device_revokes_its_collab_token(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let client = reqwest::Client::new();

    let second: Value = client
        .post(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .json(&json!({ "device_name": "stolen-phone" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_token = second["token"].as_str().unwrap();
    let second_id = second["device_id"].as_str().unwrap();

    assert!(
        tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={second_token}"))
            .await
            .is_ok(),
        "a live device's token must connect"
    );

    let del = client
        .delete(format!("http://{addr}/api/devices/{second_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);

    assert!(
        tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={second_token}"))
            .await
            .is_err(),
        "a revoked device's token must be rejected on /api/ws"
    );
}
```

Marker `// md:fn deleting_a_device_revokes_its_collab_token`. Issue #20: the same
revocation on `/api/ws` — the revoked device's token, which connected fine before,
is rejected at the WS handshake afterwards.

### fn gc_compacts_old_tombstones


**Code** — complete and verbatim:

```rust
// md:fn gc_compacts_old_tombstones
#[sqlx::test(migrations = "../../migrations")]
async fn gc_compacts_old_tombstones(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (_uid, did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "GC").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    let l1 = uuid::Uuid::new_v4().to_string();
    let l2 = uuid::Uuid::new_v4().to_string();
    send(&mut ws, insert_op(&note_id, &l1, None, "viva", &did, 1, T1)).await;
    send(
        &mut ws,
        insert_op(&note_id, &l2, Some(&l1), "muerta", &did, 2, T1),
    )
    .await;
    send(
        &mut ws,
        json!({
            "type": "Op",
            "note_id": note_id,
            "ops": [{
                "op": "Delete",
                "line_id": l2,
                "deleted_at": T1,
                "vv": { did.clone(): 3 },
                "last_writer": did,
                "updated_at": T2,
            }],
        }),
    )
    .await;
    let note_uuid = note_id.parse().unwrap();
    let mut settled = false;
    for _ in 0..50 {
        let lines = state.store.list_lines(note_uuid).await.unwrap();
        let tombstones = lines.iter().filter(|l| l.deleted_at.is_some()).count();
        if lines.len() == 2 && tombstones == 1 {
            settled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(settled, "delete never landed as a tombstone");

    let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
    let reclaimed = state.store.gc_line_tombstones(cutoff).await.unwrap();
    assert_eq!(reclaimed, 1);

    assert_eq!(export_body(addr, &token, &note_id).await, "viva");
    let lines = state
        .store
        .list_lines(note_id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    let order = state
        .store
        .get_note_order(note_id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(order.order.len(), 1);
}
```

Marker `// md:fn gc_compacts_old_tombstones`. Two lines, one deleted with a
months-old tombstone. The test polls the **store's line set** (2 lines, exactly 1
tombstoned) rather than the exported body before running GC — the body reads
"viva" both before line 2 exists and after the delete, so polling it could race GC
against the not-yet-landed tombstone. `gc_line_tombstones(30 days)` reclaims
exactly 1; the body is unchanged and the id is gone from both `lines` and the
order.

### fn version_endpoint_advertises_capabilities


**Code** — complete and verbatim:

```rust
// md:fn version_endpoint_advertises_capabilities
#[sqlx::test(migrations = "../../migrations")]
async fn version_endpoint_advertises_capabilities(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let v: Value = reqwest::Client::new()
        .get(format!("http://{addr}/version"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["name"], "keeplin-srv");
    assert!(v["protocol_version"].as_u64().unwrap() >= 1);
    let caps = v["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "history"), "advertises history");
}
```

Marker `// md:fn version_endpoint_advertises_capabilities`. `GET /version`
(unauthenticated): name, `protocol_version ≥ 1`, and the `history` capability
present (issues #39/#114).

### fn health_and_readiness_probes


**Code** — complete and verbatim:

```rust
// md:fn health_and_readiness_probes
#[sqlx::test(migrations = "../../migrations")]
async fn health_and_readiness_probes(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    let health = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    assert_eq!(health.text().await.unwrap(), "ok");

    let ready = client
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);
    assert_eq!(ready.text().await.unwrap(), "ready");
}
```

Marker `// md:fn health_and_readiness_probes`. `/health` → `200 ok` (liveness
stub); `/ready` → `200 ready` with the database up (real DB round-trip,
issue #36). Both unauthenticated, never rate-limited.

### fn metrics_reports_counts


**Code** — complete and verbatim:

```rust
// md:fn metrics_reports_counts
#[sqlx::test(migrations = "../../migrations")]
async fn metrics_reports_counts(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    create_note(addr, &token, "Contada").await;

    let anon = reqwest::Client::new()
        .get(format!("http://{addr}/api/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401, "metrics must not be world-readable");

    let m: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/metrics"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(m["users"].as_i64().unwrap(), 1);
    assert_eq!(m["notes"].as_i64().unwrap(), 1);
    assert!(m["collab_sessions"].as_i64().is_some());
}
```

Marker `// md:fn metrics_reports_counts`. Issue #22: anonymous `/api/metrics` →
401; authenticated → correct `users`/`notes` counts plus the live collab gauges.

### fn spawn_rate_limited


**Code** — complete and verbatim:

```rust
// md:fn spawn_rate_limited
async fn spawn_rate_limited(pool: PgPool, per_min: u32) -> SocketAddr {
    let mut cfg = test_config();
    cfg.rate_limit_per_min = per_min;
    let state = Arc::new(AppState::new(cfg, pool));
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

Marker `// md:fn spawn_rate_limited`. Helper: a server with
`rate_limit_per_min = N`.

### fn rate_limit_throttles_and_spares_health


**Code** — complete and verbatim:

```rust
// md:fn rate_limit_throttles_and_spares_health
#[sqlx::test(migrations = "../../migrations")]
async fn rate_limit_throttles_and_spares_health(pool: PgPool) {
    let addr = spawn_rate_limited(pool, 10).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let client = reqwest::Client::new();

    let mut got_ok = false;
    let mut got_throttled = false;
    for _ in 0..40 {
        let code = client
            .get(format!("http://{addr}/api/metrics"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status();
        match code.as_u16() {
            200 => got_ok = true,
            429 => {
                got_throttled = true;
                break;
            }
            other => panic!("unexpected status {other}"),
        }
    }
    assert!(
        got_ok,
        "authenticated requests succeed before the budget is spent"
    );
    assert!(
        got_throttled,
        "burst past the budget must be throttled with 429"
    );

    for _ in 0..10 {
        let code = client
            .get(format!("http://{addr}/health"))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(code, 200);
    }
}
```

Marker `// md:fn rate_limit_throttles_and_spares_health`. Budget 10/min: hammering
an authenticated route yields some 200s then a 429 (the limiter short-circuits
before the handler); `/health` never throttles (orchestrator probes must always
pass).

---

## Capability-model tests (Front B)

### fn share_caps


**Code** — complete and verbatim:

```rust
// md:fn share_caps
async fn share_caps(addr: SocketAddr, token: &str, note_id: &str, email: &str, caps: i32) -> u16 {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/notes/{note_id}/share"))
        .bearer_auth(token)
        .json(&json!({ "user_email": email, "capabilities": caps }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}
```

Marker `// md:fn share_caps`. Helper: grant with an explicit capability bitmask,
returning the HTTP status.

### fn note_status


**Code** — complete and verbatim:

```rust
// md:fn note_status
async fn note_status(addr: SocketAddr, token: &str, note_id: &str, method: &str) -> u16 {
    let http = reqwest::Client::new();
    let url = format!("http://{addr}/api/notes/{note_id}");
    let req = match method {
        "GET" => http.get(url),
        "PATCH" => http.patch(url).json(&json!({ "title": "x" })),
        "DELETE" => http.delete(url),
        _ => unreachable!(),
    };
    req.bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}
```

Marker `// md:fn note_status`. Helper: GET/PATCH/DELETE a note, returning the HTTP
status.

### fn capability_grants_enforce_hierarchy_and_escalation


**Code** — complete and verbatim:

```rust
// md:fn capability_grants_enforce_hierarchy_and_escalation
#[sqlx::test(migrations = "../../migrations")]
async fn capability_grants_enforce_hierarchy_and_escalation(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_b, _db, token_b) = user(addr, "b@example.com").await;
    let (_c, _dc, _token_c) = user(addr, "c@example.com").await;
    let note_id = create_note(addr, &token_a, "N").await;

    assert_eq!(
        share_caps(addr, &token_a, &note_id, "b@example.com", 1).await,
        200
    );
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 200);
    assert_eq!(note_status(addr, &token_b, &note_id, "PATCH").await, 403);
    assert_eq!(
        share_caps(addr, &token_b, &note_id, "c@example.com", 1).await,
        403,
        "read-only grantee has no share_write"
    );

    assert_eq!(
        share_caps(addr, &token_a, &note_id, "b@example.com", 8).await,
        200
    );
    assert_eq!(
        share_caps(addr, &token_b, &note_id, "c@example.com", 3).await,
        200,
        "B holds write, so it may grant read+write"
    );
    assert_eq!(
        share_caps(addr, &token_b, &note_id, "c@example.com", 16).await,
        403,
        "B lacks manage, so it cannot grant manage"
    );
}
```

Marker `// md:fn capability_grants_enforce_hierarchy_and_escalation`. READ-only B:
can GET, cannot PATCH, cannot share (no `share_write`). Upgraded to `SHARE_WRITE`
(normalises to read|write|share_read|share_write = 15, **not** manage): B can grant
C read+write (within its own caps) but granting `MANAGE` → 403 — **no privilege
escalation**: a grant is capped to the granter's own capabilities.

### fn ownership_transfer_moves_delete_rights


**Code** — complete and verbatim:

```rust
// md:fn ownership_transfer_moves_delete_rights
#[sqlx::test(migrations = "../../migrations")]
async fn ownership_transfer_moves_delete_rights(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_b, _db, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "N").await;

    let code = reqwest::Client::new()
        .post(format!("http://{addr}/api/notes/{note_id}/transfer"))
        .bearer_auth(&token_a)
        .json(&json!({ "user_email": "b@example.com" }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(code, 200);

    assert_eq!(note_status(addr, &token_a, &note_id, "DELETE").await, 403);
    assert_eq!(note_status(addr, &token_b, &note_id, "DELETE").await, 200);
}
```

Marker `// md:fn ownership_transfer_moves_delete_rights`. After
`POST …/transfer` to B: A (no implicit residual access) cannot DELETE (403); B, the
new owner, can (200) — delete/transfer are owner-only powers that no capability bit
confers.

### fn move_note


**Code** — complete and verbatim:

```rust
// md:fn move_note
async fn move_note(addr: SocketAddr, token: &str, note_id: &str, notebook_id: &str) {
    let code = reqwest::Client::new()
        .patch(format!("http://{addr}/api/notes/{note_id}"))
        .bearer_auth(token)
        .json(&json!({ "notebook_id": notebook_id }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);
}
```

Marker `// md:fn move_note`. Helper: PATCH `notebook_id`, asserting 200.

### fn notebook_share_cascades_to_child_notes


**Code** — complete and verbatim:

```rust
// md:fn notebook_share_cascades_to_child_notes
#[sqlx::test(migrations = "../../migrations")]
async fn notebook_share_cascades_to_child_notes(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();

    let note_id = create_note(addr, &token_a, "N").await;
    move_note(addr, &token_a, &note_id, &nb_id).await;
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 403);

    let code = reqwest::Client::new()
        .post(format!("http://{addr}/api/notebooks/{nb_id}/share"))
        .bearer_auth(&token_a)
        .json(&json!({ "user_email": "b@example.com", "capabilities": 1 }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);
    assert_eq!(
        note_status(addr, &token_b, &note_id, "GET").await,
        200,
        "notebook share cascaded read onto the note"
    );
    assert_eq!(note_status(addr, &token_b, &note_id, "PATCH").await, 403);

    let code = reqwest::Client::new()
        .delete(format!("http://{addr}/api/notebooks/{nb_id}/share/{uid_b}"))
        .bearer_auth(&token_a)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 403);
}
```

Marker `// md:fn notebook_share_cascades_to_child_notes`. A materialised notebook
(seeded via `store.upsert_notebook`) with a note moved in: B has no access; sharing
the **notebook** (read) cascades read onto the child note (GET 200, PATCH still
403); revoking the notebook share re-cascades and B loses access — the destructive
cascade in both directions.

### fn notebook_share_caps


**Code** — complete and verbatim:

```rust
// md:fn notebook_share_caps
async fn notebook_share_caps(
    addr: SocketAddr,
    token: &str,
    notebook_id: &str,
    email: &str,
    caps: i32,
) -> u16 {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/notebooks/{notebook_id}/share"))
        .bearer_auth(token)
        .json(&json!({ "user_email": email, "capabilities": caps }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}
```

Marker `// md:fn notebook_share_caps`. Helper returning the status of a notebook
grant with explicit capabilities.

### fn move_note_status


**Code** — complete and verbatim:

```rust
// md:fn move_note_status
async fn move_note_status(addr: SocketAddr, token: &str, note_id: &str, notebook_id: &str) -> u16 {
    reqwest::Client::new()
        .patch(format!("http://{addr}/api/notes/{note_id}"))
        .bearer_auth(token)
        .json(&json!({ "notebook_id": notebook_id }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}
```

Marker `// md:fn move_note_status`. Helper returning the status of a note-move
PATCH.

### fn note_move_requires_write_on_destination_notebook


**Code** — complete and verbatim:

```rust
// md:fn note_move_requires_write_on_destination_notebook
#[sqlx::test(migrations = "../../migrations")]
async fn note_move_requires_write_on_destination_notebook(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();
    let note_id = create_note(addr, &token_b, "N").await;

    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        403
    );
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 1).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        403
    );
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 2).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &uuid::Uuid::new_v4().to_string()).await,
        404
    );
}
```

Marker `// md:fn note_move_requires_write_on_destination_notebook`. Issue #13:
moving B's note into A's notebook is 403 with no destination access **and** with
only read; write on the destination allows it (200); an unknown destination is 404
— consent on both sides, because the move adopts the destination's grants
(disclosure + share replacement).

### fn notebook_owner_can_manage_child_notes_they_do_not_own


**Code** — complete and verbatim:

```rust
// md:fn notebook_owner_can_manage_child_notes_they_do_not_own
#[sqlx::test(migrations = "../../migrations")]
async fn notebook_owner_can_manage_child_notes_they_do_not_own(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();

    let note_id = create_note(addr, &token_b, "N").await;
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 2).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        200
    );

    assert_eq!(note_status(addr, &token_a, &note_id, "GET").await, 200);
    assert_eq!(note_status(addr, &token_a, &note_id, "PATCH").await, 200);
    let notes: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        notes
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == note_id.as_str()),
        "notebook owner sees child notes in their listing"
    );
    assert_eq!(note_status(addr, &token_a, &note_id, "DELETE").await, 403);
    assert_eq!(note_status(addr, &token_b, &note_id, "DELETE").await, 200);
}
```

Marker `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own`.
Issue #15 (folder-owner model): B's note filed in A's notebook — A holds no
`note_shares` row (the cascade copies only `notebook_shares`), yet as notebook
owner A can GET/PATCH the child note and sees it in `GET /api/notes`; but DELETE
stays with B (403 for A, 200 for B) — implicit `manage`, never ownership.

### fn nil_notebook_id_patch_means_inbox_and_keeps_shares


**Code** — complete and verbatim:

```rust
// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares
#[sqlx::test(migrations = "../../migrations")]
async fn nil_notebook_id_patch_means_inbox_and_keeps_shares(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_b, _db, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "N").await;
    share(addr, &token_a, &note_id, "b@example.com", "viewer").await;

    let response = reqwest::Client::new()
        .patch(format!("http://{addr}/api/notes/{note_id}"))
        .bearer_auth(&token_a)
        .json(&json!({
            "title": "renamed",
            "notebook_id": "00000000-0000-0000-0000-000000000000",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "nil uuid is the Inbox, not a 404");
    let note: Value = response.json().await.unwrap();
    assert!(note["notebook_id"].is_null(), "stored as NULL (the Inbox)");
    assert_eq!(note["title"], "renamed");
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 200);
}
```

Marker `// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares`.
keeplin-core models the inbox as the nil UUID; the server as `NULL`. A PATCH with
the nil UUID is a move **to the inbox**: 200 (not a 404 destination check),
stored `notebook_id` is null, and **no destructive cascade ran** — the
collaborator's share survives.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn_server()`, `spawn_instance()`, `spawn_server_with_state()`, `spawn_rate_limited()` — defined here (EXTRACTED)
- the helper fns and every test fn — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×3; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports + `type Ws` | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_server` | `// md:fn spawn_server` |
| 4 | `fn spawn_instance` | `// md:fn spawn_instance` |
| 5 | `fn spawn_server_with_state` | `// md:fn spawn_server_with_state` |
| 6 | `fn user` | `// md:fn user` |
| 7 | `fn create_note` | `// md:fn create_note` |
| 8 | `fn share` | `// md:fn share` |
| 9 | `fn ws_connect` | `// md:fn ws_connect` |
| 10 | `fn send` | `// md:fn send` |
| 11 | `fn recv_until` | `// md:fn recv_until` |
| 12 | `fn join` | `// md:fn join` |
| 13 | `fn insert_op` | `// md:fn insert_op` |
| 14 | `fn update_op` | `// md:fn update_op` |
| 15 | `fn export_body` | `// md:fn export_body` |
| 16 | `fn wait_export` | `// md:fn wait_export` |
| 17 | `T1`/`T2`/`T3` | `// md:Timestamps` |
| 18 | `fn join_receives_welcome_snapshot` | `// md:fn join_receives_welcome_snapshot` |
| 19 | `fn ops_propagate_between_participants` | `// md:fn ops_propagate_between_participants` |
| 20 | `fn ops_and_presence_propagate_across_instances` | `// md:fn ops_and_presence_propagate_across_instances` |
| 21 | `fn concurrent_updates_resolve_deterministically` | `// md:fn concurrent_updates_resolve_deterministically` |
| 22 | `fn stale_op_is_ignored` | `// md:fn stale_op_is_ignored` |
| 23 | `fn move_reorders_lines` | `// md:fn move_reorders_lines` |
| 24 | `fn viewer_can_watch_but_not_edit` | `// md:fn viewer_can_watch_but_not_edit` |
| 25 | `fn revoking_a_share_stops_edits_mid_session` | `// md:fn revoking_a_share_stops_edits_mid_session` |
| 26 | `fn outsider_cannot_join` | `// md:fn outsider_cannot_join` |
| 27 | `fn presence_shows_other_participants` | `// md:fn presence_shows_other_participants` |
| 28 | `fn import_then_export_roundtrip` | `// md:fn import_then_export_roundtrip` |
| 29 | `fn forged_writer_is_rejected` | `// md:fn forged_writer_is_rejected` |
| 30 | `fn ws_accepts_authorization_header` | `// md:fn ws_accepts_authorization_header` |
| 31 | `fn deleting_a_device_revokes_its_token` | `// md:fn deleting_a_device_revokes_its_token` |
| 32 | `fn deleting_a_device_revokes_its_collab_token` | `// md:fn deleting_a_device_revokes_its_collab_token` |
| 33 | `fn gc_compacts_old_tombstones` | `// md:fn gc_compacts_old_tombstones` |
| 34 | `fn version_endpoint_advertises_capabilities` | `// md:fn version_endpoint_advertises_capabilities` |
| 35 | `fn health_and_readiness_probes` | `// md:fn health_and_readiness_probes` |
| 36 | `fn metrics_reports_counts` | `// md:fn metrics_reports_counts` |
| 37 | `fn spawn_rate_limited` | `// md:fn spawn_rate_limited` |
| 38 | `fn rate_limit_throttles_and_spares_health` | `// md:fn rate_limit_throttles_and_spares_health` |
| 39 | `fn share_caps` | `// md:fn share_caps` |
| 40 | `fn note_status` | `// md:fn note_status` |
| 41 | `fn capability_grants_enforce_hierarchy_and_escalation` | `// md:fn capability_grants_enforce_hierarchy_and_escalation` |
| 42 | `fn ownership_transfer_moves_delete_rights` | `// md:fn ownership_transfer_moves_delete_rights` |
| 43 | `fn move_note` | `// md:fn move_note` |
| 44 | `fn notebook_share_cascades_to_child_notes` | `// md:fn notebook_share_cascades_to_child_notes` |
| 45 | `fn notebook_share_caps` | `// md:fn notebook_share_caps` |
| 46 | `fn move_note_status` | `// md:fn move_note_status` |
| 47 | `fn note_move_requires_write_on_destination_notebook` | `// md:fn note_move_requires_write_on_destination_notebook` |
| 48 | `fn notebook_owner_can_manage_child_notes_they_do_not_own` | `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own` |
| 49 | `fn nil_notebook_id_patch_means_inbox_and_keeps_shares` | `// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares` |
