//! End-to-end tests of the domain-entity materialisation added on top of the
//! relay: notebooks, tags, note↔tag associations and resource metadata arrive
//! over `/api/sync` (driven by the real keeplin-core `DbBackend`) and the server
//! turns them into durable, queryable, version-vector-resolved state — the
//! "server is the truth, client DB is a cache" model. Backed by a throwaway
//! Postgres database (`#[sqlx::test]`).

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

fn test_config() -> Config {
    Config {
        port: 0,
        database_url: String::new(),
        jwt_secret: "test-secret".into(),
        token_ttl_days: 1,
        retention_days: 0,
        lines_gc_days: 0,
        db_max_connections: 5,
        db_acquire_timeout_secs: 10,
        db_idle_timeout_secs: 600,
        db_max_lifetime_secs: 1800,
        rate_limit_per_min: 0,
        shutdown_grace_secs: 5,
        log_json: false,
        max_upload_bytes: 100 * 1024 * 1024,
    }
}

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

async fn register(addr: SocketAddr, email: &str) {
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

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

async fn device(addr: SocketAddr, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, &format!("ws://{addr}/api/sync"), token)
        .await
        .unwrap()
}

fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(0, 0).unwrap()
}

/// Push every local change of `dev` to the relay, giving the server a moment to
/// materialise the batch.
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

/// Authenticated GET returning parsed JSON.
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

// ── Materialisation over the relay ───────────────────────────────────────────

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

    // The binary travelled inside the ResourceCreate (current client) and the
    // server stored it in resource_blobs; the download endpoint returns it.
    let got = reqwest::Client::new()
        .get(format!("http://{addr}/api/resources/{}/data", resource.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(got.as_ref(), bytes.as_slice());
}

#[sqlx::test(migrations = "../../migrations")]
async fn streaming_blob_upload_then_download(pool: PgPool) {
    // The Option B path: metadata exists (via the relay), then the binary is
    // PUT out-of-band and read back.
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
    let put = reqwest::Client::new()
        .put(format!("http://{addr}/api/resources/{}/data", resource.id))
        .bearer_auth(&token)
        .body(new_bytes.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 200);

    let got = reqwest::Client::new()
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

// ── Version-vector resolution (store level, deterministic) ───────────────────

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
    // Concurrent (neither vv dominates), later timestamp → B wins the tiebreak.
    nb_b.updated_at = base + Duration::seconds(1);
    nb_b.last_writer = "devB".into();

    // Apply in one order…
    assert!(store.upsert_notebook(user.id, &nb_a).await.unwrap());
    assert!(store.upsert_notebook(user.id, &nb_b).await.unwrap());
    let winner1 = store.list_notebooks(user.id).await.unwrap();
    assert_eq!(winner1[0].title, "from-b");

    // …and the reverse order converges to the same winner (b still wins; the
    // stale a-write is ignored).
    let store2 = Store::new(pool.clone());
    let id2 = Uuid::new_v4();
    let mut nb_a2 = nb_a.clone();
    nb_a2.id = id2;
    let mut nb_b2 = nb_b.clone();
    nb_b2.id = id2;
    assert!(store2.upsert_notebook(user.id, &nb_b2).await.unwrap());
    assert!(!store2.upsert_notebook(user.id, &nb_a2).await.unwrap()); // a loses → not written
    let winner2 = store2
        .list_notebooks(user.id)
        .await
        .unwrap()
        .into_iter()
        .find(|n| n.id == id2)
        .unwrap();
    assert_eq!(winner2.title, "from-b", "order-independent convergence");
}

#[sqlx::test(migrations = "../../migrations")]
async fn materialised_entities_survive_journal_pruning(pool: PgPool) {
    let addr = spawn_server(pool.clone()).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;
    a.create_notebook(Notebook::new("Durable")).await.unwrap();
    push(&a).await;

    // Simulate delivery to every device, then prune the whole journal.
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

    // The materialised notebook is still served — the table is the truth, not
    // the (now-pruned) journal.
    let notebooks = get_json(addr, &token, "/api/notebooks").await;
    assert_eq!(notebooks.as_array().unwrap().len(), 1);
    assert_eq!(notebooks[0]["title"], "Durable");
}
