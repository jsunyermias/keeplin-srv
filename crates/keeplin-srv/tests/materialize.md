# `tests/materialize.rs` — domain-entity materialisation tests

Self-contained companion for `crates/keeplin-srv/tests/materialize.rs`. It documents
**every code block of the source file, in source order** — a reader with only this file
must be able to understand the suite without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use chrono::{Duration, Utc};
use keeplin_core::{
    models::{Note, NoteTag, Notebook, Resource, Tag},
    storage::{
        db::DbBackend, note_log::VersionVector, NoteRepository, NotebookRepository,
        ResourceRepository, SyncBackend, TagRepository,
    },
};
use keeplin_srv::{config::Config, http::router, state::AppState, store::Store};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;
```

**What it does** — End-to-end tests of the server materialising the keeplin-core
domain entities (notebooks, tags, note↔tag associations, resource metadata +
binaries) that arrive over the `/api/sync` relay, driven by the **real relay client**
(keeplin-core's `DbBackend`) against a real server on a throwaway `#[sqlx::test]`
PostgreSQL database — the "server is the truth, client DB is a cache" model. Plus
store-level tests of deterministic vv convergence, pruning survival, per-user batch
dedup (issue #26), phantom-device pruning (issue #23) and quota/purge hygiene
(issue #24).

Coverage note: this suite drives the **relay-mode** client (`DbBackend` alone),
whose `ResourceCreate` still carries the binary inline — deliberately exercising the
server's backward-compat path. The collab-mode client uploads out-of-band and strips
`data` from the relayed change; that path is covered by
`tests/collab_client_resources_e2e.rs`.

**Dependencies** — keeplin-core (`DbBackend`, models, repository/sync traits,
`VersionVector`), `keeplin_srv` (`Config`, `router`, `AppState`, `Store`),
`reqwest`, `sqlx`, `tempfile`, `chrono`, `serde_json`, `uuid`.

**Used by** — `cargo test`; CI.

**Repeated context** — Materialisation model, restated: `sync.rs::materialize`
parses each relayed `Change`, resolves it by version vector against the stored row
(`store.rs::incoming_wins` = keeplin-core's `note_log::resolve`) under
`SELECT … FOR UPDATE`, and upserts — so the server converges to the same winner as
every client, and the materialised tables (not the journal) are the durable truth.

---

## fn test_config

**Identification** — helper; marker `// md:fn test_config`. The standard test
`Config` literal (open registration, everything optional off).

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

**Dependencies** — `Config`. **Used by** — `spawn_server`. **Repeated context** —
config literals keep the environment out of tests.

---

## fn spawn_server

**Identification** — helper; marker `// md:fn spawn_server`. Boots the real router
on an ephemeral loopback port with `ConnectInfo`, on a spawned task.

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

**Dependencies** — `AppState::new`, `router`. **Used by** — the HTTP-level tests.
**Repeated context** — none.

---

## fn register

**Identification** — helper; marker `// md:fn register`. REST registration over real
HTTP (asserts 200). **Dependencies** — `reqwest`. **Used by** — the HTTP-level
tests. **Repeated context** — none.

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

## fn login

**Identification** — helper; marker `// md:fn login`. REST login returning the
device token. **Dependencies** — `reqwest`. **Used by** — the HTTP-level tests.
**Repeated context** — none.

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

---

## fn device

**Identification** — helper; marker `// md:fn device`. A real relay client
(`DbBackend`) on a leaked temp SQLite file, connected to `ws://…/api/sync`.

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

**Dependencies** — keeplin-core, `tempfile`. **Used by** — the relay-driven tests.
**Repeated context** — relay-mode (no collab wrapper): `ResourceCreate` carries the
binary inline — the backward-compat path this suite covers on purpose.

---

## fn epoch

**Identification** — helper; marker `// md:fn epoch`. The Unix-epoch timestamp used
as the "everything" lower bound for `get_changes_since`.

**Code** — complete and verbatim:

```rust
// md:fn epoch
fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(0, 0).unwrap()
}
```

**Dependencies** — chrono. **Used by** — `push`. **Repeated context** — none.

---

## fn push

**Identification** — helper; marker `// md:fn push`. Sends every local change of a
device to the relay (`get_changes_since(epoch)` → `send_changes`) and sleeps 200 ms
to give the server a moment to materialise the batch.

**Code** — complete and verbatim:

```rust
// md:fn push
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}
```

**Dependencies** — keeplin-core sync API. **Used by** — the relay-driven tests.

**Repeated context** — The sleep is a *convenience*, not a guarantee: assertions
that need a specific materialised artefact (notably the resource-blob tests) poll
with a bounded retry on top of it, because under a busy CI database materialisation
can exceed the grace period.

---

## fn get_json

**Identification** — helper; marker `// md:fn get_json`. Authenticated GET
returning parsed JSON.

**Code** — complete and verbatim:

```rust
// md:fn get_json
async fn get_json(addr: SocketAddr, token: &str, path: &str) -> Value {
    reqwest::Client::new()
        .get(format!("http://{addr}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}
```

**Dependencies** — `reqwest`. **Used by** — the HTTP-level tests.
**Repeated context** — none.

---

## fn notebook_materialises_and_is_served

**Identification** — `#[sqlx::test]`; marker
`// md:fn notebook_materialises_and_is_served`.

**Code** — complete and verbatim:

```rust
// md:fn notebook_materialises_and_is_served
#[sqlx::test(migrations = "../../migrations")]
async fn notebook_materialises_and_is_served(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;

    let notebook = a.create_notebook(Notebook::new("Work")).await.unwrap();
    push(&a).await;

    let notebooks = get_json(addr, &token, "/api/notebooks").await;
    let arr = notebooks.as_array().unwrap();
    assert_eq!(arr.len(), 1, "one notebook materialised");
    assert_eq!(arr[0]["id"], notebook.id.to_string());
    assert_eq!(arr[0]["title"], "Work");
}
```

**What it does** — Create a notebook through the real client, push; `GET
/api/notebooks` lists exactly it (id + title).

**Dependencies / Used by** — the helpers; `cargo test`.

**Repeated context** — The REST read side serves the materialised table for cold
rehydration.

---

## fn tag_and_association_materialise

**Identification** — `#[sqlx::test]`; marker
`// md:fn tag_and_association_materialise`.

**Code** — complete and verbatim:

```rust
// md:fn tag_and_association_materialise
#[sqlx::test(migrations = "../../migrations")]
async fn tag_and_association_materialise(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;

    let note = a.create_note(Note::new("N", "body")).await.unwrap();
    let tag = a.create_tag(Tag::new("urgent")).await.unwrap();
    a.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();
    push(&a).await;

    let tags = get_json(addr, &token, "/api/tags").await;
    assert_eq!(tags.as_array().unwrap().len(), 1);
    assert_eq!(tags[0]["title"], "urgent");

    let note_tags = get_json(addr, &token, &format!("/api/notes/{}/tags", note.id)).await;
    let ids = note_tags.as_array().unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], tag.id.to_string());
}
```

**What it does** — Create note + tag + association, push; `GET /api/tags` lists the
tag and `GET /api/notes/:id/tags` returns the tag id.

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** — the
association is itself a versioned entity (`note_tags`).

---

## fn removing_a_tag_association_tombstones_it

**Identification** — `#[sqlx::test]`; marker
`// md:fn removing_a_tag_association_tombstones_it`.

**Code** — complete and verbatim:

```rust
// md:fn removing_a_tag_association_tombstones_it
#[sqlx::test(migrations = "../../migrations")]
async fn removing_a_tag_association_tombstones_it(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;

    let note = a.create_note(Note::new("N", "body")).await.unwrap();
    let tag = a.create_tag(Tag::new("urgent")).await.unwrap();
    a.add_note_tag(NoteTag {
        note_id: note.id,
        tag_id: tag.id,
    })
    .await
    .unwrap();
    push(&a).await;
    a.remove_note_tag(note.id, tag.id).await.unwrap();
    push(&a).await;

    let note_tags = get_json(addr, &token, &format!("/api/notes/{}/tags", note.id)).await;
    assert!(
        note_tags.as_array().unwrap().is_empty(),
        "association removed"
    );
}
```

**What it does** — After `remove_note_tag` + push, `…/tags` is empty — the
association was tombstoned (soft-delete), not deleted.

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** —
soft-delete keeps the row competing in resolution; the read filters live rows.

---

## fn resource_metadata_and_blob_materialise

**Identification** — `#[sqlx::test]`; marker
`// md:fn resource_metadata_and_blob_materialise`.

**Code** — complete and verbatim:

```rust
// md:fn resource_metadata_and_blob_materialise
#[sqlx::test(migrations = "../../migrations")]
async fn resource_metadata_and_blob_materialise(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;

    let bytes = b"opaque-encrypted-bytes".to_vec();
    let resource = a
        .create_resource(
            Resource::new("photo", "image/png", "photo.png", bytes.len() as u64),
            bytes.clone(),
        )
        .await
        .unwrap();
    push(&a).await;

    let resources = get_json(addr, &token, "/api/resources").await;
    let arr = resources.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], resource.id.to_string());
    assert_eq!(arr[0]["file_name"], "photo.png");

    let client = reqwest::Client::new();
    let mut got: Vec<u8> = Vec::new();
    for _ in 0..100 {
        let resp = client
            .get(format!("http://{addr}/api/resources/{}/data", resource.id))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            got = resp.bytes().await.unwrap().to_vec();
            if got == bytes {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(got.as_slice(), bytes.as_slice());
}
```

**What it does** — Create a resource whose binary travels **inside** the
`ResourceCreate` (relay-mode client, backward-compat path), push. Asserts the
metadata is listed, then **polls** `GET /api/resources/:id/data` (bounded, ~10 s)
until it returns the exact bytes — polling because metadata upsert and blob write
land in sequence during async materialisation, and a fixed post-push sleep is not a
guarantee under CI load.

**Dependencies / Used by** — the helpers; `cargo test`.

**Repeated context** — Backward compatibility: `sync.rs::materialize` stores an
inline `data` payload to `resource_blobs` only when the metadata upsert reports the
incoming version won.

---

## fn streaming_blob_upload_then_download

**Identification** — `#[sqlx::test]`; marker
`// md:fn streaming_blob_upload_then_download`.

**Code** — complete and verbatim:

```rust
// md:fn streaming_blob_upload_then_download
#[sqlx::test(migrations = "../../migrations")]
async fn streaming_blob_upload_then_download(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;

    let resource = a
        .create_resource(
            Resource::new("f", "application/pdf", "f.pdf", 3),
            b"abc".to_vec(),
        )
        .await
        .unwrap();
    push(&a).await;

    let new_bytes = vec![9u8; 4096];
    let client = reqwest::Client::new();
    let mut put_status = 0u16;
    for _ in 0..100 {
        put_status = client
            .put(format!("http://{addr}/api/resources/{}/data", resource.id))
            .bearer_auth(&token)
            .body(new_bytes.clone())
            .send()
            .await
            .unwrap()
            .status()
            .as_u16();
        if put_status == 200 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(put_status, 200);

    let got = client
        .get(format!("http://{addr}/api/resources/{}/data", resource.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(got.as_ref(), new_bytes.as_slice(), "PUT replaced the blob");
}
```

**What it does** — The Option B (out-of-band) path against relay-created metadata:
after push, **poll** `PUT /api/resources/:id/data` (bounded) until it answers 200 —
the PUT 404s while the metadata is still materialising — then `GET` returns exactly
the replaced 4 KiB.

**Dependencies / Used by** — the helpers; `cargo test`.

**Repeated context** — The 404-until-materialised behaviour is the same contract
the real collab client handles with its own upload retry
(`collab_client_resources_e2e.rs`).

---

## fn uploading_to_unknown_resource_is_rejected

**Identification** — `#[sqlx::test]`; marker
`// md:fn uploading_to_unknown_resource_is_rejected`.

**Code** — complete and verbatim:

```rust
// md:fn uploading_to_unknown_resource_is_rejected
#[sqlx::test(migrations = "../../migrations")]
async fn uploading_to_unknown_resource_is_rejected(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    let resp = reqwest::Client::new()
        .put(format!(
            "http://{addr}/api/resources/{}/data",
            Uuid::new_v4()
        ))
        .bearer_auth(&token)
        .body(vec![1u8, 2, 3])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "no metadata → no upload");
}
```

**What it does** — `PUT …/data` for a random id → `404`: no metadata, no upload.

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** — the
metadata row is the authorisation anchor for blob writes.

---

## fn deleting_a_notebook_removes_it_from_listings

**Identification** — `#[sqlx::test]`; marker
`// md:fn deleting_a_notebook_removes_it_from_listings`.

**Code** — complete and verbatim:

```rust
// md:fn deleting_a_notebook_removes_it_from_listings
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_notebook_removes_it_from_listings(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;

    let notebook = a.create_notebook(Notebook::new("Temp")).await.unwrap();
    push(&a).await;
    a.delete_notebook(notebook.id).await.unwrap();
    push(&a).await;

    let notebooks = get_json(addr, &token, "/api/notebooks").await;
    assert!(
        notebooks.as_array().unwrap().is_empty(),
        "deleted notebook is not listed"
    );
}
```

**What it does** — Delete a materialised notebook, push; `GET /api/notebooks` is
empty (tombstoned, filtered from live listings).

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** —
soft-delete + live-row reads.

---

## fn users_do_not_see_each_others_entities

**Identification** — `#[sqlx::test]`; marker
`// md:fn users_do_not_see_each_others_entities`.

**Code** — complete and verbatim:

```rust
// md:fn users_do_not_see_each_others_entities
#[sqlx::test(migrations = "../../migrations")]
async fn users_do_not_see_each_others_entities(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let ta = login(addr, "a@example.com", "dev-a").await;
    let tb = login(addr, "b@example.com", "dev-b").await;
    let a = device(addr, &ta).await;
    a.create_notebook(Notebook::new("A-only")).await.unwrap();
    push(&a).await;

    let b_notebooks = get_json(addr, &tb, "/api/notebooks").await;
    assert!(
        b_notebooks.as_array().unwrap().is_empty(),
        "isolation across users"
    );
}
```

**What it does** — User A materialises a notebook; user B's listing stays empty —
per-user isolation of the materialised entities.

**Dependencies / Used by** — the helpers; `cargo test`. **Repeated context** — all
durable data is user-scoped; sharing is explicit and note/notebook-level only.

---

## fn concurrent_notebook_edits_converge_deterministically

**Identification** — `#[sqlx::test]` (store-level); marker
`// md:fn concurrent_notebook_edits_converge_deterministically`.

**Code** — complete and verbatim:

```rust
// md:fn concurrent_notebook_edits_converge_deterministically
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_notebook_edits_converge_deterministically(pool: PgPool) {
    let store = Store::new(pool.clone());
    let user = store
        .create_user("a@example.com", "hash", "A")
        .await
        .unwrap();

    let id = Uuid::new_v4();
    let base = Utc::now();
    let mut nb_a = Notebook::new("from-a");
    nb_a.id = id;
    nb_a.vv = VersionVector::from([("devA".to_string(), 1)]);
    nb_a.updated_at = base;
    nb_a.last_writer = "devA".into();

    let mut nb_b = Notebook::new("from-b");
    nb_b.id = id;
    nb_b.vv = VersionVector::from([("devB".to_string(), 1)]);
    nb_b.updated_at = base + Duration::seconds(1);
    nb_b.last_writer = "devB".into();

    assert!(store.upsert_notebook(user.id, &nb_a).await.unwrap());
    assert!(store.upsert_notebook(user.id, &nb_b).await.unwrap());
    let winner1 = store.list_notebooks(user.id, None, None).await.unwrap();
    assert_eq!(winner1[0].title, "from-b");

    let store2 = Store::new(pool.clone());
    let id2 = Uuid::new_v4();
    let mut nb_a2 = nb_a.clone();
    nb_a2.id = id2;
    let mut nb_b2 = nb_b.clone();
    nb_b2.id = id2;
    assert!(store2.upsert_notebook(user.id, &nb_b2).await.unwrap());
    assert!(!store2.upsert_notebook(user.id, &nb_a2).await.unwrap());
    let winner2 = store2
        .list_notebooks(user.id, None, None)
        .await
        .unwrap()
        .into_iter()
        .find(|n| n.id == id2)
        .unwrap();
    assert_eq!(winner2.title, "from-b", "order-independent convergence");
}
```

**What it does** — Two concurrent edits to one notebook id (neither vv dominates;
B has the later timestamp): applied in either order, B wins — and in the reverse
order the stale A-write reports "not written" (`upsert_notebook` → `false`).
Deterministic, order-independent convergence at the store level.

**Dependencies** — `Store::{create_user, upsert_notebook, list_notebooks}`.
**Used by** — `cargo test`.

**Repeated context** — Pins `incoming_wins` = vv dominance + `(timestamp, writer)`
LWW tiebreak, identical to every client.

---

## fn materialised_entities_survive_journal_pruning

**Identification** — `#[sqlx::test]`; marker
`// md:fn materialised_entities_survive_journal_pruning`.

**Code** — complete and verbatim:

```rust
// md:fn materialised_entities_survive_journal_pruning
#[sqlx::test(migrations = "../../migrations")]
async fn materialised_entities_survive_journal_pruning(pool: PgPool) {
    let addr = spawn_server(pool.clone()).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;
    a.create_notebook(Notebook::new("Durable")).await.unwrap();
    push(&a).await;

    let store = Store::new(pool.clone());
    let device_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM user_devices")
        .fetch_all(&pool)
        .await
        .unwrap();
    let max_seq: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM changes")
        .fetch_one(&pool)
        .await
        .unwrap();
    for id in device_ids {
        store.advance_cursor(id, max_seq).await.unwrap();
    }
    let pruned = store
        .prune_delivered_changes(Utc::now() + Duration::hours(1))
        .await
        .unwrap();
    assert!(pruned > 0, "journal rows were pruned");

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM changes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0, "journal emptied");

    let notebooks = get_json(addr, &token, "/api/notebooks").await;
    assert_eq!(notebooks.as_array().unwrap().len(), 1);
    assert_eq!(notebooks[0]["title"], "Durable");
}
```

**What it does** — Materialise a notebook, then simulate full delivery (advance
every device cursor to max seq) and prune the **entire** journal
(`prune_delivered_changes` with a future cutoff): rows go, journal is empty, and
`GET /api/notebooks` still serves the notebook — the materialised table, not the
journal, is the truth. This is the safety argument behind pruning (issue #23).

**Dependencies** — the helpers + `Store::{advance_cursor,
prune_delivered_changes}`, raw sqlx. **Used by** — `cargo test`.

**Repeated context** — Journal = delivery buffer + history window; materialised
tables = state. Cold rehydration reads the tables over REST.

---

## fn same_batch_id_across_users_is_not_deduplicated

**Identification** — `#[sqlx::test]` (store-level); marker
`// md:fn same_batch_id_across_users_is_not_deduplicated`.

**Code** — complete and verbatim:

```rust
// md:fn same_batch_id_across_users_is_not_deduplicated
#[sqlx::test(migrations = "../../migrations")]
async fn same_batch_id_across_users_is_not_deduplicated(pool: PgPool) {
    let store = Store::new(pool);
    let a = store.create_user("a@x.com", "h", "A").await.unwrap();
    let b = store.create_user("b@x.com", "h", "B").await.unwrap();
    let da = store.create_device(a.id, "da").await.unwrap();
    let db = store.create_device(b.id, "db").await.unwrap();

    let batch = Uuid::new_v4();
    let payload = vec![serde_json::json!({ "op": "noop" })];

    let sa = store
        .append_changes(a.id, da.id, "da", batch, &payload)
        .await
        .unwrap();
    let sb = store
        .append_changes(b.id, db.id, "db", batch, &payload)
        .await
        .unwrap();
    assert_eq!(sa.len(), 1);
    assert_eq!(
        sb.len(),
        1,
        "the same batch_id for a different user must not be treated as a duplicate"
    );

    let retry = store
        .append_changes(a.id, da.id, "da", batch, &payload)
        .await
        .unwrap();
    assert!(retry.is_empty(), "a user's own batch retry is deduped");
}
```

**What it does** — The same client `batch_id` used by two different users is NOT
deduplicated across accounts (issue #26 — dedup is per user:
`UNIQUE (user_id, batch_id, batch_index)`), while a user's own retry of the same
batch still dedupes to empty.

**Dependencies** — `Store::{create_user, create_device, append_changes}`.
**Used by** — `cargo test`.

**Repeated context** — Pins the issue #26 fix: a cross-user batch-id collision (or
a malicious guess) can no longer suppress another account's changes.

---

## fn a_never_connected_device_does_not_block_pruning

**Identification** — `#[sqlx::test]` (store-level); marker
`// md:fn a_never_connected_device_does_not_block_pruning`.

**Code** — complete and verbatim:

```rust
// md:fn a_never_connected_device_does_not_block_pruning
#[sqlx::test(migrations = "../../migrations")]
async fn a_never_connected_device_does_not_block_pruning(pool: PgPool) {
    let store = Store::new(pool);
    let u = store.create_user("a@x.com", "h", "A").await.unwrap();
    let connected = store.create_device(u.id, "connected").await.unwrap();
    let _phantom = store.create_device(u.id, "phantom").await.unwrap();

    let batch = Uuid::new_v4();
    let seqs = store
        .append_changes(
            u.id,
            connected.id,
            "connected",
            batch,
            &[serde_json::json!({ "op": "noop" })],
        )
        .await
        .unwrap();
    let max = *seqs.last().unwrap();
    store.advance_cursor(connected.id, max).await.unwrap();

    let pruned = store
        .prune_delivered_changes(Utc::now() + Duration::hours(1))
        .await
        .unwrap();
    assert!(
        pruned >= 1,
        "a phantom device must not block pruning of rows every connected device has received"
    );
}
```

**What it does** — One connected device (cursor advanced) plus one phantom device
that never connected (no cursor row): pruning still reclaims the delivered rows —
the phantom does not hold the journal hostage (issue #23).

**Dependencies** — `Store` journal/cursor methods. **Used by** — `cargo test`.

**Repeated context** — Only devices **with a cursor row** participate in the
pruning minimum; a fresh device cold-rehydrates from REST + snapshots rather than
replaying from seq 0.

---

## fn deleted_resource_frees_quota_and_blob_is_purgeable

**Identification** — `#[sqlx::test]` (store-level); marker
`// md:fn deleted_resource_frees_quota_and_blob_is_purgeable`.

**Code** — complete and verbatim:

```rust
// md:fn deleted_resource_frees_quota_and_blob_is_purgeable
#[sqlx::test(migrations = "../../migrations")]
async fn deleted_resource_frees_quota_and_blob_is_purgeable(pool: PgPool) {
    let store = Store::new(pool);
    let u = store.create_user("a@x.com", "h", "A").await.unwrap();

    let mut r = keeplin_core::models::Resource::new("f", "application/octet-stream", "f.bin", 3);
    r.vv = VersionVector::from([("dev".to_string(), 1)]);
    r.last_writer = "dev".into();
    assert!(store.upsert_resource_meta(u.id, &r).await.unwrap());
    store.put_resource_blob(r.id, &[1, 2, 3]).await.unwrap();

    assert_eq!(
        store
            .user_blob_bytes_excluding(u.id, Uuid::nil())
            .await
            .unwrap(),
        3
    );

    let del_vv = VersionVector::from([("dev".to_string(), 2)]);
    assert!(store
        .delete_resource(r.id, Utc::now(), &del_vv, "dev")
        .await
        .unwrap());

    assert_eq!(
        store
            .user_blob_bytes_excluding(u.id, Uuid::nil())
            .await
            .unwrap(),
        0,
        "a soft-deleted resource no longer counts against quota"
    );

    let purged = store
        .purge_deleted_resource_blobs(Utc::now() + Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(purged, 1);
    assert!(
        store.get_resource_blob(r.id).await.unwrap().is_none(),
        "the blob was reclaimed"
    );
}
```

**What it does** — A live 3-byte resource counts 3 against quota; after a
dominating soft-delete it counts 0 (deleting frees quota), and
`purge_deleted_resource_blobs` reclaims the blob while the metadata tombstone
stays (issue #24).

**Dependencies** — `Store` resource/quota/purge methods. **Used by** —
`cargo test`.

**Repeated context** — Blob bytes are reclaimable; convergence metadata is not —
the tombstone must keep competing in resolution.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `test_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_server()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `register()` — defined here (EXTRACTED; file-local)
- `login()` — defined here (EXTRACTED; file-local)
- `device()` — defined here (EXTRACTED; file-local)
- `epoch()` — defined here (EXTRACTED; file-local)
- `push()` — defined here (EXTRACTED; file-local)
- `get_json()` — defined here (EXTRACTED; file-local)
- `notebook_materialises_and_is_served()` — defined here (EXTRACTED; file-local)
- `tag_and_association_materialise()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_server` | `// md:fn spawn_server` |
| 4 | `fn register` | `// md:fn register` |
| 5 | `fn login` | `// md:fn login` |
| 6 | `fn device` | `// md:fn device` |
| 7 | `fn epoch` | `// md:fn epoch` |
| 8 | `fn push` | `// md:fn push` |
| 9 | `fn get_json` | `// md:fn get_json` |
| 10 | `fn notebook_materialises_and_is_served` | `// md:fn notebook_materialises_and_is_served` |
| 11 | `fn tag_and_association_materialise` | `// md:fn tag_and_association_materialise` |
| 12 | `fn removing_a_tag_association_tombstones_it` | `// md:fn removing_a_tag_association_tombstones_it` |
| 13 | `fn resource_metadata_and_blob_materialise` | `// md:fn resource_metadata_and_blob_materialise` |
| 14 | `fn streaming_blob_upload_then_download` | `// md:fn streaming_blob_upload_then_download` |
| 15 | `fn uploading_to_unknown_resource_is_rejected` | `// md:fn uploading_to_unknown_resource_is_rejected` |
| 16 | `fn deleting_a_notebook_removes_it_from_listings` | `// md:fn deleting_a_notebook_removes_it_from_listings` |
| 17 | `fn users_do_not_see_each_others_entities` | `// md:fn users_do_not_see_each_others_entities` |
| 18 | `fn concurrent_notebook_edits_converge_deterministically` | `// md:fn concurrent_notebook_edits_converge_deterministically` |
| 19 | `fn materialised_entities_survive_journal_pruning` | `// md:fn materialised_entities_survive_journal_pruning` |
| 20 | `fn same_batch_id_across_users_is_not_deduplicated` | `// md:fn same_batch_id_across_users_is_not_deduplicated` |
| 21 | `fn a_never_connected_device_does_not_block_pruning` | `// md:fn a_never_connected_device_does_not_block_pruning` |
| 22 | `fn deleted_resource_frees_quota_and_blob_is_purgeable` | `// md:fn deleted_resource_frees_quota_and_blob_is_purgeable` |
