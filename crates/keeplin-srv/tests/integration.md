# `tests/integration.rs` — device relay tests (real `DbBackend`)

Self-contained companion for `crates/keeplin-srv/tests/integration.rs`. It documents
**every code block of the source file, in source order** — a reader with only this file
must be able to understand the suite without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context** (compressed for
straightforward tests).

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Json, Router};
use keeplin_core::{
    models::Note,
    storage::{db::DbBackend, NoteRepository, NotebookRepository, SyncBackend},
};
use keeplin_srv::{config::Config, http::router, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;
```

**What it does** — End-to-end tests of the `/api/sync` device relay driven by the
**real client** — keeplin-core's `DbBackend` speaking the genuine wire protocol (the
`auth` handshake sent on construction, `send_changes` envelopes, `receive_changes`
draining) — through a real keeplin-srv on a throwaway `#[sqlx::test]` PostgreSQL
database. Mirrors keeplin-core's own `ws_sync.rs` suite but against the production
relay, adding what the toy relay lacked: authentication, persistence, offline
catch-up. Also covers the REST account surface, history endpoints (issue #27), the
email flows (issue #49), the login lockout, email normalisation (issue #43), the
body-size cap (issue #44), at-rest encryption (keeplin#110) and pagination
(issue #29).

**Dependencies** — keeplin-core (`DbBackend`, models, repository/sync traits),
`keeplin_srv` (`Config`, `router`, `AppState`, `bus::spawn`), `axum` (the mock mail
webhook), `reqwest`, `sqlx`, `tempfile`, `serde_json`, `uuid`, `chrono`.

**Used by** — `cargo test`; CI.

**Repeated context** — Coverage split: the collaborative channel is covered by
`tests/collab.rs`; keeplin-core's internal `DbBackend` behaviour is tested in its own
crate — these tests exercise the **relay**, using `DbBackend` as a faithful client.

---

## Helpers

Each helper is one block with its own marker; compressed entries:

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

Marker `// md:fn test_config`. The standard test `Config` literal. **Used by**
`spawn_server`, `spawn_instance` and the tests that tweak a knob before
`spawn_server_with_config`.

### fn spawn_server


**Code** — complete and verbatim:

```rust
// md:fn spawn_server
async fn spawn_server(pool: PgPool) -> SocketAddr {
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

Marker `// md:fn spawn_server`. Real router on an ephemeral port with
`ConnectInfo`; no bus (single instance).

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

Marker `// md:fn spawn_instance`. Same, **plus `bus::spawn`** — a bus-enabled
instance for the cross-instance relay test (issue #45).

### fn register


**Code** — complete and verbatim:

```rust
// md:fn register
async fn register(addr: SocketAddr, email: &str) {
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}
```

Marker `// md:fn register`. REST registration over real HTTP (asserts 200).

### fn login


**Code** — complete and verbatim:

```rust
// md:fn login
async fn login(addr: SocketAddr, email: &str, device_name: &str) -> String {
    let body: Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": email, "password": "password123", "device_name": device_name }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["token"].as_str().unwrap().to_string()
}
```

Marker `// md:fn login`. REST login returning the device sync token.

### fn device


**Code** — complete and verbatim:

```rust
// md:fn device
async fn device(addr: SocketAddr, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, &format!("ws://{addr}/api/sync"), token)
        .await
        .unwrap()
}
```

Marker `// md:fn device`. A server-mode `DbBackend` (the real keeplin client) on a
leaked temp SQLite file, pointed at `ws://addr/api/sync` — its constructor performs
the `auth` handshake.

### fn epoch


**Code** — complete and verbatim:

```rust
// md:fn epoch
fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(0, 0).unwrap()
}
```

Marker `// md:fn epoch`. Unix epoch — the "everything" bound for
`get_changes_since`.

### fn push


**Code** — complete and verbatim:

```rust
// md:fn push
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
}
```

Marker `// md:fn push`. Sends every local change of a device to the relay (no
grace sleep — tests that need persistence add their own waits/polls).

### fn sync_until


**Code** — complete and verbatim:

```rust
// md:fn sync_until
async fn sync_until(dev: &DbBackend, id: Uuid, want_body: Option<&str>) -> bool {
    for _ in 0..50 {
        let remote = dev.receive_changes().await.unwrap();
        for change in remote {
            dev.apply_change(change).await.unwrap();
        }
        if let Ok(note) = dev.read_note(id).await {
            match want_body {
                None => return true,
                Some(body) if note.body == body => return true,
                Some(_) => {}
            }
        }
    }
    false
}
```

Marker `// md:fn sync_until`.
`async fn sync_until(dev, id, want_body: Option<&str>) -> bool` — up to 50 rounds of
`receive_changes` (each drains ~100 ms) + `apply_change`, until note `id` exists
and (when given) its body matches. The workhorse convergence poll; also used
negatively (isolation tests assert it returns `false`).

---

## Relay tests

### fn note_syncs_live_between_two_devices


**Code** — complete and verbatim:

```rust
// md:fn note_syncs_live_between_two_devices
#[sqlx::test(migrations = "../../migrations")]
async fn note_syncs_live_between_two_devices(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;
    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;

    let note = Note::new("Shared", "over the wire");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;

    assert!(
        sync_until(&b, id, None).await,
        "device B must receive A's note through the relay"
    );
    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Shared");
    assert_eq!(read.body, "over the wire");
}
```

Marker `// md:fn note_syncs_live_between_two_devices`. A creates + pushes a note;
device B (same account) receives it live through the relay with title and body
intact.

### fn relay_batch_propagates_across_instances


**Code** — complete and verbatim:

```rust
// md:fn relay_batch_propagates_across_instances
#[sqlx::test(migrations = "../../migrations")]
async fn relay_batch_propagates_across_instances(pool: PgPool) {
    let addr_a = spawn_instance(pool.clone()).await;
    let addr_b = spawn_instance(pool.clone()).await;
    register(addr_a, "a@example.com").await;
    let a = device(addr_a, &login(addr_a, "a@example.com", "laptop").await).await;
    let b = device(addr_b, &login(addr_b, "a@example.com", "phone").await).await;

    let note = Note::new("Cross", "over two instances");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;

    assert!(
        sync_until(&b, id, None).await,
        "device B on the other instance must receive A's note live"
    );
}
```

Marker `// md:fn relay_batch_propagates_across_instances`. Two **bus-enabled**
instances, A's device on one and B's on the other: a batch pushed to instance A is
delivered **live** to B on instance B via the `sync_batch` NOTIFY wake (issue #45)
— not just on reconnect.

### fn update_propagates_and_converges


**Code** — complete and verbatim:

```rust
// md:fn update_propagates_and_converges
#[sqlx::test(migrations = "../../migrations")]
async fn update_propagates_and_converges(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;
    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;

    let mut note = Note::new("v1", "body v1");
    let id = note.id;
    a.create_note(note.clone()).await.unwrap();
    push(&a).await;
    assert!(sync_until(&b, id, None).await, "B must receive the create");

    note.title = "v2".to_string();
    note.body = "body v2".to_string();
    note.updated_at = chrono::Utc::now();
    a.update_note(note).await.unwrap();
    push(&a).await;

    assert!(
        sync_until(&b, id, Some("body v2")).await,
        "B must converge to A's update"
    );
}
```

Marker `// md:fn update_propagates_and_converges`. After the create syncs, A's
update (new title/body/timestamp) converges on B (`sync_until` with the v2 body).

### fn device_connecting_later_receives_backlog


**Code** — complete and verbatim:

```rust
// md:fn device_connecting_later_receives_backlog
#[sqlx::test(migrations = "../../migrations")]
async fn device_connecting_later_receives_backlog(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Persisted", "written while B did not exist");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;
    assert!(
        sync_until(&b, id, None).await,
        "a device connecting later must receive the persisted backlog"
    );
}
```

Marker `// md:fn device_connecting_later_receives_backlog`. A pushes; only then
does B log in and connect: the note arrives from the **journal** (offline
catch-up), not live fan-out.

### fn users_do_not_see_each_others_changes


**Code** — complete and verbatim:

```rust
// md:fn users_do_not_see_each_others_changes
#[sqlx::test(migrations = "../../migrations")]
async fn users_do_not_see_each_others_changes(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;
    let b = device(addr, &login(addr, "b@example.com", "laptop").await).await;

    let note = Note::new("Private", "user A only");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;

    assert!(
        !sync_until(&b, id, None).await,
        "user B must never receive user A's changes"
    );
}
```

Marker `// md:fn users_do_not_see_each_others_changes`. Two different accounts:
B never receives A's changes (per-user journal/fan-out isolation) —
`sync_until` must come back `false`.

### fn duplicate_batches_are_deduplicated


**Code** — complete and verbatim:

```rust
// md:fn duplicate_batches_are_deduplicated
#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_batches_are_deduplicated(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Once", "sent twice");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;
    assert!(sync_until(&b, id, None).await, "B must receive the note");
    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Once");
}
```

Marker `// md:fn duplicate_batches_are_deduplicated`. A pushes its identical local
journal twice (two envelopes); B still converges to exactly one note — journal
dedup by `(user, batch_id, index)` plus the client's idempotent `apply_change`.

### fn sender_never_receives_its_own_changes_back


**Code** — complete and verbatim:

```rust
// md:fn sender_never_receives_its_own_changes_back
#[sqlx::test(migrations = "../../migrations")]
async fn sender_never_receives_its_own_changes_back(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Echo?", "should not come back");
    a.create_note(note).await.unwrap();
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let echoed = a.receive_changes().await.unwrap();
    assert!(
        echoed.is_empty(),
        "the relay must never echo a device's own changes back: {echoed:?}"
    );
}
```

Marker `// md:fn sender_never_receives_its_own_changes_back`. After pushing, A's
`receive_changes` drains empty — echo suppression by origin device id.

### fn invalid_token_gets_no_data


**Code** — complete and verbatim:

```rust
// md:fn invalid_token_gets_no_data
#[sqlx::test(migrations = "../../migrations")]
async fn invalid_token_gets_no_data(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Secret", "authenticated only");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let intruder = device(addr, "not-a-valid-jwt").await;
    assert!(
        !sync_until(&intruder, id, None).await,
        "an unauthenticated client must not receive any changes"
    );
}
```

Marker `// md:fn invalid_token_gets_no_data`. A garbage token "connects" (the WS
upgrade succeeds) but the server closes after the failed handshake: no changes ever
arrive.

---

## REST surface tests

### fn register_login_and_device_listing


**Code** — complete and verbatim:

```rust
// md:fn register_login_and_device_listing
#[sqlx::test(migrations = "../../migrations")]
async fn register_login_and_device_listing(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    register(addr, "a@example.com").await;

    let dup = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "a@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 409);

    let token = login(addr, "a@example.com", "laptop").await;

    let second: Value = client
        .post(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .json(&json!({ "device_name": "phone" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(second["token"].as_str().is_some());
    assert_eq!(second["device_name"], "phone");

    let devices: Value = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(devices.as_array().unwrap().len(), 2);

    let bad = client
        .post(format!("http://{addr}/api/login"))
        .json(
            &json!({ "email": "a@example.com", "password": "wrong-password", "device_name": "x" }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);
}
```

Marker `// md:fn register_login_and_device_listing`. Duplicate registration → 409;
`POST /api/devices` with an existing token mints a second device+token; the
listing shows 2; wrong password → 401.

### fn history_endpoints_serve_versions_from_the_server_journal


**Code** — complete and verbatim:

```rust
// md:fn history_endpoints_serve_versions_from_the_server_journal
#[sqlx::test(migrations = "../../migrations")]
async fn history_endpoints_serve_versions_from_the_server_journal(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;
    let a = device(addr, &token).await;
    let device_id = a.get_device_id().await.unwrap();

    let note = a.create_note(Note::new("T", "v1")).await.unwrap();
    let mut edited = note.clone();
    edited.body = "v2".into();
    a.update_note(edited).await.unwrap();
    a.delete_note(note.id).await.unwrap();
    let nb = a
        .create_notebook(keeplin_core::models::Notebook::new("old"))
        .await
        .unwrap();
    let mut renamed = nb.clone();
    renamed.title = "new".into();
    a.update_notebook(renamed).await.unwrap();
    push(&a).await;

    let client = reqwest::Client::new();
    let get = |path: String| {
        let client = client.clone();
        let token = token.clone();
        async move {
            client
                .get(format!("http://{addr}{path}"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    let mut versions = Value::Null;
    for _ in 0..50 {
        versions = get(format!("/api/notes/{}/history", note.id)).await;
        if versions.as_array().is_some_and(|v| v.len() >= 3) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let versions = versions.as_array().unwrap();
    assert_eq!(
        versions.len(),
        3,
        "create + update + delete = three versions"
    );
    assert!(versions[0]["entity"].is_null(), "a delete is a tombstone");
    assert_eq!(versions[1]["entity"]["body"], "v2");
    assert_eq!(versions[2]["entity"]["body"], "v1");
    assert_eq!(versions[0]["device_id"], device_id.as_str());

    let capped = get(format!("/api/notes/{}/history?limit=1", note.id)).await;
    assert_eq!(capped.as_array().unwrap().len(), 1);

    let versions = get(format!("/api/notebooks/{}/history", nb.id)).await;
    let versions = versions.as_array().unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["entity"]["title"], "new");
    assert_eq!(versions[1]["entity"]["title"], "old");

    register(addr, "b@example.com").await;
    let token_b = login(addr, "b@example.com", "phone").await;
    let other: Value = client
        .get(format!("http://{addr}/api/notes/{}/history", note.id))
        .bearer_auth(&token_b)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(other.as_array().unwrap().is_empty());

    let denied = client
        .get(format!("http://{addr}/api/notebooks/{}/history", nb.id))
        .bearer_auth(&token_b)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403, "no access to the notebook → 403");
}
```

Marker `// md:fn history_endpoints_serve_versions_from_the_server_journal`.
A authors note create/edit/delete plus a notebook rename and pushes; polls
`GET /api/notes/:id/history` until the batch lands (journaling is async after
`send_changes`). Asserts: newest-first `[tombstone (entity: null), v2, v1]`, each
stamped with the sync device id; `?limit=1` caps the reply; notebook history
mirrors the shape. Cross-account: the relay-only **note** is private (B's read is
scoped to B's own empty journal → `[]`), while the **materialised** notebook is
access-gated (B → 403).

### fn password_change_and_logout_everywhere


**Code** — complete and verbatim:

```rust
// md:fn password_change_and_logout_everywhere
#[sqlx::test(migrations = "../../migrations")]
async fn password_change_and_logout_everywhere(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    let bad = client
        .post(format!("http://{addr}/api/account/password"))
        .bearer_auth(&token)
        .json(&json!({ "current_password": "wrong-one", "new_password": "newpassword1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);

    let ok = client
        .post(format!("http://{addr}/api/account/password"))
        .bearer_auth(&token)
        .json(&json!({ "current_password": "password123", "new_password": "newpassword1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);

    let old = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": "a@example.com", "password": "password123", "device_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), 401);
    let new = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": "a@example.com", "password": "newpassword1", "device_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(new.status(), 200);

    let logout = client
        .delete(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 200);
    let denied = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401, "revoked token must stop working");
}
```

Marker `// md:fn password_change_and_logout_everywhere`. Issue #31: wrong current
password → 401; correct change works; old password stops logging in, new one
works; `DELETE /api/devices` revokes the caller's own token immediately (next
request → 401).

### fn delete_account_requires_password_and_cascades


**Code** — complete and verbatim:

```rust
// md:fn delete_account_requires_password_and_cascades
#[sqlx::test(migrations = "../../migrations")]
async fn delete_account_requires_password_and_cascades(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    let created = client
        .post(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .json(&json!({ "title": "keep me?" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);

    let bad = client
        .delete(format!("http://{addr}/api/account"))
        .bearer_auth(&token)
        .json(&json!({ "password": "wrong-one" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);
    let still = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(still.status(), 200, "account must survive a failed delete");

    let gone = client
        .delete(format!("http://{addr}/api/account"))
        .bearer_auth(&token)
        .json(&json!({ "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 200);

    let denied = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        denied.status(),
        401,
        "deleted account's token must stop working"
    );

    let reused = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "a@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        reused.status(),
        200,
        "email must be reusable after deletion"
    );
}
```

Marker `// md:fn delete_account_requires_password_and_cascades`. Issue #31: wrong
password → 401 and the account survives; correct password deletes it — the token
dies (device row cascaded), and the email is registrable afresh (user row +
unique email gone).

### fn list_notes_paginates_with_cursor


**Code** — complete and verbatim:

```rust
// md:fn list_notes_paginates_with_cursor
#[sqlx::test(migrations = "../../migrations")]
async fn list_notes_paginates_with_cursor(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    let total = 7;
    for i in 0..total {
        let resp = client
            .post(format!("http://{addr}/api/notes"))
            .bearer_auth(&token)
            .json(&json!({ "title": format!("note {i}") }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut req = client
            .get(format!("http://{addr}/api/notes"))
            .bearer_auth(&token)
            .query(&[("limit", "3")]);
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c)]);
        }
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let next = resp
            .headers()
            .get("x-next-cursor")
            .map(|v| v.to_str().unwrap().to_string());
        let page: Vec<Value> = resp.json().await.unwrap();
        assert!(page.len() <= 3, "page must respect the limit");
        for n in &page {
            seen.push(n["id"].as_str().unwrap().to_string());
        }
        pages += 1;
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
        assert!(pages < 10, "pagination must terminate");
    }

    assert_eq!(pages, 3);
    assert_eq!(seen.len(), total);
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), total, "no note may repeat across pages");

    let all = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(all.headers().get("x-next-cursor").is_none());
    let all_notes: Vec<Value> = all.json().await.unwrap();
    assert_eq!(all_notes.len(), total);

    let bad = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .query(&[("limit", "3"), ("cursor", "not-a-cursor")])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}
```

Marker `// md:fn list_notes_paginates_with_cursor`. Issue #29: 7 notes, pages of
3 following `X-Next-Cursor` → exactly 3 pages, every note exactly once (the id
tiebreaker keeps the keyset walk total under `updated_at` ties); no `limit` →
full list and no header; a garbage cursor → 400.

### fn metrics_render_prometheus_format


**Code** — complete and verbatim:

```rust
// md:fn metrics_render_prometheus_format
#[sqlx::test(migrations = "../../migrations")]
async fn metrics_render_prometheus_format(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    let prom = client
        .get(format!("http://{addr}/api/metrics?format=prometheus"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(prom.status(), 200);
    let ct = prom
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.starts_with("text/plain"), "prometheus format is text");
    let body = prom.text().await.unwrap();
    assert!(body.contains("# TYPE keeplin_users gauge"));
    assert!(
        body.contains("keeplin_users 1"),
        "one registered user: {body}"
    );
    assert!(body.contains("keeplin_collab_sessions"));

    let json_resp: Value = client
        .get(format!("http://{addr}/api/metrics"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(json_resp["users"], 1);
}
```

Marker `// md:fn metrics_render_prometheus_format`. `?format=prometheus` renders
the text exposition (content-type `text/plain`, `# TYPE keeplin_users gauge`,
`keeplin_users 1`); the default stays JSON.

---

## Email-flow tests (issue #49)

### fn spawn_mail_webhook


**Code** — complete and verbatim:

```rust
// md:fn spawn_mail_webhook
async fn spawn_mail_webhook() -> (SocketAddr, Arc<tokio::sync::Mutex<Vec<Value>>>) {
    let inbox: Arc<tokio::sync::Mutex<Vec<Value>>> = Arc::default();
    let captured = inbox.clone();
    let app = Router::new().route(
        "/mail",
        axum::routing::post(move |Json(payload): Json<Value>| {
            let captured = captured.clone();
            async move {
                captured.lock().await.push(payload);
                "ok"
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, inbox)
}
```

Marker `// md:fn spawn_mail_webhook`. Helper: a mock of the operator's mail
webhook — an in-process axum route capturing every posted payload into a shared
`Vec` (the "inbox").

### fn webhook_token


**Code** — complete and verbatim:

```rust
// md:fn webhook_token
async fn webhook_token(inbox: &Arc<tokio::sync::Mutex<Vec<Value>>>, kind: &str) -> String {
    for _ in 0..50 {
        if let Some(t) = inbox
            .lock()
            .await
            .iter()
            .rev()
            .find(|p| p["kind"] == kind)
            .and_then(|p| p["token"].as_str())
        {
            return t.to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("no {kind} payload reached the mail webhook");
}
```

Marker `// md:fn webhook_token`. Helper: poll the captured payloads for the most
recent token of a given `kind` (bounded), panicking if none arrives.

### fn email_verification_and_password_reset_flows


**Code** — complete and verbatim:

```rust
// md:fn email_verification_and_password_reset_flows
#[sqlx::test(migrations = "../../migrations")]
async fn email_verification_and_password_reset_flows(pool: PgPool) {
    let (mail_addr, inbox) = spawn_mail_webhook().await;
    let mut config = test_config();
    config.mail_webhook_url = Some(format!("http://{mail_addr}/mail"));
    config.email_verification_required = true;
    let addr = spawn_server_with_config(pool, config).await;
    let client = reqwest::Client::new();

    register(addr, "a@example.com").await;
    let verify_token = webhook_token(&inbox, "verify_email").await;

    let denied = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": "a@example.com", "password": "password123", "device_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 400, "unverified account must not log in");

    let confirmed = client
        .post(format!("http://{addr}/api/account/verify/confirm"))
        .json(&json!({ "token": verify_token }))
        .send()
        .await
        .unwrap();
    assert_eq!(confirmed.status(), 200);
    let token1 = login(addr, "a@example.com", "laptop").await;

    let requested = client
        .post(format!("http://{addr}/api/account/reset/request"))
        .json(&json!({ "email": "a@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(requested.status(), 200);
    let reset_token = webhook_token(&inbox, "password_reset").await;

    let reset = client
        .post(format!("http://{addr}/api/account/reset/confirm"))
        .json(&json!({ "token": reset_token, "new_password": "brand-new-pass1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(reset.status(), 200);

    let revoked = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(&token1)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 401, "reset must sign out everywhere");

    let old = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": "a@example.com", "password": "password123", "device_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), 401);
    let new = client
        .post(format!("http://{addr}/api/login"))
        .json(
            &json!({ "email": "a@example.com", "password": "brand-new-pass1", "device_name": "x" }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(new.status(), 200);

    let replay = client
        .post(format!("http://{addr}/api/account/reset/confirm"))
        .json(&json!({ "token": reset_token, "new_password": "another-pass-123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 400, "single-use token must not replay");

    let before = inbox.lock().await.len();
    let ghost = client
        .post(format!("http://{addr}/api/account/reset/request"))
        .json(&json!({ "email": "ghost@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ghost.status(), 200, "reset request must be uniform");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        inbox.lock().await.len(),
        before,
        "no mail for unknown email"
    );
}
```

Marker `// md:fn email_verification_and_password_reset_flows`. The full delegated
lifecycle with `EMAIL_VERIFICATION_REQUIRED`: registration fires the verification
mail; unverified login → 400 even with the right password; confirming the token
(unauthenticated — the token is the proof) unlocks login. Reset: request →
uniform 200, webhook receives the token; confirm sets the new password, **revokes
every device** (pre-reset token → 401) and old password → 401 / new → 200; a
consumed token cannot replay (400); an unknown email gets the same uniform 200
with **no** mail sent (no oracle — inbox length unchanged).

### fn email_flows_answer_501_when_unconfigured


**Code** — complete and verbatim:

```rust
// md:fn email_flows_answer_501_when_unconfigured
#[sqlx::test(migrations = "../../migrations")]
async fn email_flows_answer_501_when_unconfigured(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    let reset = client
        .post(format!("http://{addr}/api/account/reset/request"))
        .json(&json!({ "email": "a@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(reset.status(), 501);

    let verify = client
        .post(format!("http://{addr}/api/account/verify/request"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(verify.status(), 501);
}
```

Marker `// md:fn email_flows_answer_501_when_unconfigured`. Without
`MAIL_WEBHOOK_URL`, reset-request and verify-request answer 501 — the explicit
deferral.

---

## Hardening tests

### fn login_lockout_blocks_brute_force


**Code** — complete and verbatim:

```rust
// md:fn login_lockout_blocks_brute_force
#[sqlx::test(migrations = "../../migrations")]
async fn login_lockout_blocks_brute_force(pool: PgPool) {
    let mut config = test_config();
    config.login_max_failures = 3;
    config.login_lockout_secs = 2;
    let addr = spawn_server_with_config(pool, config).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;

    let try_login = |email: &'static str, password: &'static str| {
        let client = client.clone();
        async move {
            client
                .post(format!("http://{addr}/api/login"))
                .json(&json!({ "email": email, "password": password, "device_name": "x" }))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };

    for _ in 0..3 {
        assert_eq!(try_login("a@example.com", "wrong-password").await, 401);
    }
    assert_eq!(
        try_login("a@example.com", "password123").await,
        429,
        "locked account must refuse even the correct password"
    );

    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert_eq!(try_login("a@example.com", "password123").await, 200);
    assert_eq!(try_login("a@example.com", "wrong-password").await, 401);
    assert_eq!(try_login("a@example.com", "password123").await, 200);

    for _ in 0..3 {
        assert_eq!(try_login("ghost@example.com", "whatever123").await, 401);
    }
    assert_eq!(try_login("ghost@example.com", "whatever123").await, 429);
}
```

Marker `// md:fn login_lockout_blocks_brute_force`. `LOGIN_MAX_FAILURES=3`,
2-second lockout: three 401s arm the lock; then even the **correct** password →
429; after expiry the correct password works and clears the counter (a single new
failure is a plain 401 again); an unknown email accumulates identically (same
401s, same 429) — lockout is not an existence oracle (issue #32).

### fn email_is_normalized_and_validated


**Code** — complete and verbatim:

```rust
// md:fn email_is_normalized_and_validated
#[sqlx::test(migrations = "../../migrations")]
async fn email_is_normalized_and_validated(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    let reg = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "  John.Doe@Example.COM ", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(reg.status(), 200);

    let login = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": "john.doe@example.com", "password": "password123", "device_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200, "login must be case-insensitive");

    let dup = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "JOHN.DOE@EXAMPLE.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 409, "case-variant email must collide");

    let bad = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "not-an-email", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}
```

Marker `// md:fn email_is_normalized_and_validated`. Issue #43: register with
mixed case + whitespace; lowercase login works; a case-variant re-registration →
409 (same account); a malformed email → 400.

### fn oversized_note_body_is_refused


**Code** — complete and verbatim:

```rust
// md:fn oversized_note_body_is_refused
#[sqlx::test(migrations = "../../migrations")]
async fn oversized_note_body_is_refused(pool: PgPool) {
    let mut config = test_config();
    config.max_note_body_bytes = 32;
    let addr = spawn_server_with_config(pool, config).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    let small = client
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(&token)
        .json(&json!({ "title": "small", "body": "under the cap" }))
        .send()
        .await
        .unwrap();
    assert_eq!(small.status(), 200);
    let small_id = small.json::<Value>().await.unwrap()["note_id"]
        .as_str()
        .unwrap()
        .to_string();
    let got = client
        .get(format!("http://{addr}/api/notes/{small_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), 200);

    let big_body = "x".repeat(100);
    let big = client
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(&token)
        .json(&json!({ "title": "big", "body": big_body }))
        .send()
        .await
        .unwrap();
    assert_eq!(big.status(), 200);
    let big_id = big.json::<Value>().await.unwrap()["note_id"]
        .as_str()
        .unwrap()
        .to_string();
    let refused = client
        .get(format!("http://{addr}/api/notes/{big_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 413, "oversized body must be 413");
}
```

Marker `// md:fn oversized_note_body_is_refused`. Issue #44: with a 32-byte
`max_note_body_bytes`, a small note reads fine while a 100-char note's read →
413 (the body is measured before being built).

### fn note_content_is_encrypted_at_rest


**Code** — complete and verbatim:

```rust
// md:fn note_content_is_encrypted_at_rest
#[sqlx::test(migrations = "../../migrations")]
async fn note_content_is_encrypted_at_rest(pool: PgPool) {
    let key = format!("{}=", "A".repeat(43));
    let mut config = test_config();
    config.at_rest_key = Some(key);
    let addr = spawn_server_with_config(pool.clone(), config).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    let resp = client
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(&token)
        .json(&json!({ "title": "Secret Title", "body": "line one\nline two" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let note_id = resp.json::<Value>().await.unwrap()["note_id"]
        .as_str()
        .unwrap()
        .to_string();

    let exported: Value = client
        .get(format!("http://{addr}/api/notes/{note_id}/export"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(exported["title"], "Secret Title");
    assert_eq!(exported["body"], "line one\nline two");

    let note_uuid: Uuid = note_id.parse().unwrap();
    let raw_title: String = sqlx::query_scalar("SELECT title FROM notes WHERE id = $1")
        .bind(note_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        raw_title.starts_with("enc:v1:") && !raw_title.contains("Secret"),
        "title must be encrypted at rest, got {raw_title}"
    );
    let raw_lines: Vec<String> = sqlx::query_scalar("SELECT content FROM lines WHERE note_id = $1")
        .bind(note_uuid)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(!raw_lines.is_empty());
    for c in &raw_lines {
        assert!(
            c.starts_with("enc:v1:") && !c.contains("line "),
            "line content must be encrypted at rest, got {c}"
        );
    }
}
```

Marker `// md:fn note_content_is_encrypted_at_rest`. keeplin#110: with
`AT_REST_KEY` set, the API returns plaintext transparently while the raw
`notes.title` / `lines.content` columns hold `enc:v1:` ciphertext containing no
plaintext substrings.

---

## History-visibility tests (issue #27)

### fn spawn_server_with_config


**Code** — complete and verbatim:

```rust
// md:fn spawn_server_with_config
async fn spawn_server_with_config(pool: PgPool, config: Config) -> SocketAddr {
    let state = Arc::new(AppState::new(config, pool));
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

Marker `// md:fn spawn_server_with_config`. Helper: `spawn_server` with a custom
`Config` (used by the lockout/cap/encryption/email/visibility tests).

### fn notebook_history


**Code** — complete and verbatim:

```rust
// md:fn notebook_history
async fn notebook_history(addr: SocketAddr, token: &str, nb: Uuid) -> Vec<Value> {
    reqwest::Client::new()
        .get(format!("http://{addr}/api/notebooks/{nb}/history"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()
        .as_array()
        .cloned()
        .unwrap_or_default()
}
```

Marker `// md:fn notebook_history`. Helper: authenticated
`GET /api/notebooks/:id/history` returning the version array.

### fn notebook_history_is_visible_to_shared_collaborators


**Code** — complete and verbatim:

```rust
// md:fn notebook_history_is_visible_to_shared_collaborators
#[sqlx::test(migrations = "../../migrations")]
async fn notebook_history_is_visible_to_shared_collaborators(pool: PgPool) {
    use keeplin_core::storage::NotebookRepository;
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "dev-a").await).await;
    let ta = login(addr, "a@example.com", "rest-a").await;

    let nb = a
        .create_notebook(keeplin_core::models::Notebook::new("old"))
        .await
        .unwrap();
    let mut renamed = nb.clone();
    renamed.title = "new".into();
    a.update_notebook(renamed).await.unwrap();
    push(&a).await;

    let client = reqwest::Client::new();
    let mut shared = false;
    for _ in 0..50 {
        let code = client
            .post(format!("http://{addr}/api/notebooks/{}/share", nb.id))
            .bearer_auth(&ta)
            .json(&json!({ "user_email": "b@example.com", "capabilities": 1 }))
            .send()
            .await
            .unwrap()
            .status();
        if code == 200 {
            shared = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(shared, "A could share the materialised notebook with B");

    let tb = login(addr, "b@example.com", "dev-b").await;
    let mut versions = Vec::new();
    for _ in 0..50 {
        versions = notebook_history(addr, &tb, nb.id).await;
        if versions.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        versions.len(),
        2,
        "collaborator sees the owner's full history"
    );
    assert_eq!(versions[0]["entity"]["title"], "new");
    assert_eq!(versions[1]["entity"]["title"], "old");
}
```

Marker `// md:fn notebook_history_is_visible_to_shared_collaborators`. Default
(`creation`) policy: A materialises + renames a notebook, shares it with B
(capability 1 = read; the share POST is polled because materialisation is async).
B sees the owner's **two** versions — history is per-entity, not per-user.

### fn history_visibility_since_access_windows_a_collaborator


**Code** — complete and verbatim:

```rust
// md:fn history_visibility_since_access_windows_a_collaborator
#[sqlx::test(migrations = "../../migrations")]
async fn history_visibility_since_access_windows_a_collaborator(pool: PgPool) {
    use keeplin_core::storage::NotebookRepository;
    let mut config = test_config();
    config.history_since_access = true;
    let addr = spawn_server_with_config(pool, config).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "dev-a").await).await;
    let ta = login(addr, "a@example.com", "rest-a").await;
    let client = reqwest::Client::new();

    let nb = a
        .create_notebook(keeplin_core::models::Notebook::new("v1"))
        .await
        .unwrap();
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let code = client
        .post(format!("http://{addr}/api/notebooks/{}/share", nb.id))
        .bearer_auth(&ta)
        .json(&json!({ "user_email": "b@example.com", "capabilities": 1 }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let mut renamed = nb.clone();
    renamed.title = "v2".into();
    renamed.updated_at = chrono::Utc::now();
    a.update_notebook(renamed).await.unwrap();
    push(&a).await;

    let tb = login(addr, "b@example.com", "dev-b").await;
    let mut b_versions = Vec::new();
    for _ in 0..50 {
        b_versions = notebook_history(addr, &tb, nb.id).await;
        if !b_versions.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        !b_versions.is_empty() && b_versions.iter().all(|v| v["entity"]["title"] == "v2"),
        "collaborator sees only post-access versions, got {b_versions:?}"
    );

    push(&a).await;
    let mut a_versions = Vec::new();
    for _ in 0..50 {
        a_versions = notebook_history(addr, &ta, nb.id).await;
        if a_versions.len() > 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        a_versions.len() > 2,
        "the re-push must have journaled duplicate rows (owner sees them all)"
    );
    assert!(
        a_versions.iter().any(|v| v["entity"]["title"] == "v1"),
        "owner keeps seeing the full history, v1 included"
    );

    let b_versions = notebook_history(addr, &tb, nb.id).await;
    assert!(
        !b_versions.is_empty() && b_versions.iter().all(|v| v["entity"]["title"] == "v2"),
        "re-pushing the journal from epoch after the share must not leak \
         pre-access versions to the collaborator, got {b_versions:?}"
    );
}
```

Marker `// md:fn history_visibility_since_access_windows_a_collaborator`. With
`HISTORY_VISIBILITY=access`: v1 pushed **before** the share, v2 after (with a
fresh honest `updated_at`); B sees only v2. Then the **reinstall/re-push
loophole**: A re-pushes its whole journal from epoch — new journal rows, fresh
`received_at`, pre-access causal `updated_at`. The owner's unwindowed view grows
(duplicates included, v1 visible), but B **still** sees only v2 — the window
filters on the payload's own causal timestamp, so re-delivery cannot leak
pre-access versions (the honest-client boundary, `SECURITY.md`).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn_server()` / `spawn_instance()` / `spawn_server_with_config()` — defined here (EXTRACTED)
- `test_config()`, `register()`, `login()`, `device()`, `epoch()`, `push()`, `sync_until()`, `spawn_mail_webhook()`, `webhook_token()`, `notebook_history()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×2; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×3; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_server` | `// md:fn spawn_server` |
| 4 | `fn spawn_instance` | `// md:fn spawn_instance` |
| 5 | `fn register` | `// md:fn register` |
| 6 | `fn login` | `// md:fn login` |
| 7 | `fn device` | `// md:fn device` |
| 8 | `fn epoch` | `// md:fn epoch` |
| 9 | `fn push` | `// md:fn push` |
| 10 | `fn sync_until` | `// md:fn sync_until` |
| 11 | `fn note_syncs_live_between_two_devices` | `// md:fn note_syncs_live_between_two_devices` |
| 12 | `fn relay_batch_propagates_across_instances` | `// md:fn relay_batch_propagates_across_instances` |
| 13 | `fn update_propagates_and_converges` | `// md:fn update_propagates_and_converges` |
| 14 | `fn device_connecting_later_receives_backlog` | `// md:fn device_connecting_later_receives_backlog` |
| 15 | `fn users_do_not_see_each_others_changes` | `// md:fn users_do_not_see_each_others_changes` |
| 16 | `fn duplicate_batches_are_deduplicated` | `// md:fn duplicate_batches_are_deduplicated` |
| 17 | `fn sender_never_receives_its_own_changes_back` | `// md:fn sender_never_receives_its_own_changes_back` |
| 18 | `fn invalid_token_gets_no_data` | `// md:fn invalid_token_gets_no_data` |
| 19 | `fn register_login_and_device_listing` | `// md:fn register_login_and_device_listing` |
| 20 | `fn history_endpoints_serve_versions_from_the_server_journal` | `// md:fn history_endpoints_serve_versions_from_the_server_journal` |
| 21 | `fn password_change_and_logout_everywhere` | `// md:fn password_change_and_logout_everywhere` |
| 22 | `fn delete_account_requires_password_and_cascades` | `// md:fn delete_account_requires_password_and_cascades` |
| 23 | `fn list_notes_paginates_with_cursor` | `// md:fn list_notes_paginates_with_cursor` |
| 24 | `fn metrics_render_prometheus_format` | `// md:fn metrics_render_prometheus_format` |
| 25 | `fn spawn_mail_webhook` | `// md:fn spawn_mail_webhook` |
| 26 | `fn webhook_token` | `// md:fn webhook_token` |
| 27 | `fn email_verification_and_password_reset_flows` | `// md:fn email_verification_and_password_reset_flows` |
| 28 | `fn email_flows_answer_501_when_unconfigured` | `// md:fn email_flows_answer_501_when_unconfigured` |
| 29 | `fn login_lockout_blocks_brute_force` | `// md:fn login_lockout_blocks_brute_force` |
| 30 | `fn email_is_normalized_and_validated` | `// md:fn email_is_normalized_and_validated` |
| 31 | `fn oversized_note_body_is_refused` | `// md:fn oversized_note_body_is_refused` |
| 32 | `fn note_content_is_encrypted_at_rest` | `// md:fn note_content_is_encrypted_at_rest` |
| 33 | `fn spawn_server_with_config` | `// md:fn spawn_server_with_config` |
| 34 | `fn notebook_history` | `// md:fn notebook_history` |
| 35 | `fn notebook_history_is_visible_to_shared_collaborators` | `// md:fn notebook_history_is_visible_to_shared_collaborators` |
| 36 | `fn history_visibility_since_access_windows_a_collaborator` | `// md:fn history_visibility_since_access_windows_a_collaborator` |
