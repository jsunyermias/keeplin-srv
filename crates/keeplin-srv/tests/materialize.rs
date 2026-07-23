// md:Overview
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use chrono::{Duration, Utc};
use keeplin_core::{
    models::{Note, NoteTag, Notebook, Resource, Tag, SYSTEM_RESOURCE_NOTE_ID},
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

// md:fn device
async fn device(addr: SocketAddr, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, &format!("ws://{addr}/api/sync"), token)
        .await
        .unwrap()
}

// md:fn epoch
fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(0, 0).unwrap()
}

// md:fn push
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

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
            Resource::new(
                SYSTEM_RESOURCE_NOTE_ID,
                "photo",
                "image/png",
                "photo.png",
                bytes.len() as u64,
            ),
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

// md:fn streaming_blob_upload_then_download
#[sqlx::test(migrations = "../../migrations")]
async fn streaming_blob_upload_then_download(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = device(addr, &token).await;

    let resource = a
        .create_resource(
            Resource::new(SYSTEM_RESOURCE_NOTE_ID, "f", "application/pdf", "f.pdf", 3),
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

// md:fn deleted_resource_frees_quota_and_blob_is_purgeable
#[sqlx::test(migrations = "../../migrations")]
async fn deleted_resource_frees_quota_and_blob_is_purgeable(pool: PgPool) {
    let store = Store::new(pool);
    let u = store.create_user("a@x.com", "h", "A").await.unwrap();

    let mut r = keeplin_core::models::Resource::new(
        SYSTEM_RESOURCE_NOTE_ID,
        "f",
        "application/octet-stream",
        "f.bin",
        3,
    );
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

// md:fn store_note_delete_cascades_to_attachments_and_restore_recovers_dragged
#[sqlx::test(migrations = "../../migrations")]
async fn store_note_delete_cascades_to_attachments_and_restore_recovers_dragged(pool: PgPool) {
    let store = Store::new(pool.clone());
    let user = store
        .create_user("a@example.com", "hash", "A")
        .await
        .unwrap();
    let note = store.create_note(None, "N", user.id).await.unwrap();

    let r1 = Resource::new(note.id, "a", "text/plain", "a.txt", 1);
    let r2 = Resource::new(note.id, "b", "text/plain", "b.txt", 1);
    let r3 = Resource::new(note.id, "c", "text/plain", "c.txt", 1);
    for r in [&r1, &r2, &r3] {
        store.upsert_resource_meta(user.id, r).await.unwrap();
    }

    let before = store
        .list_resources_for_note(user.id, note.id, None, None)
        .await
        .unwrap();
    assert_eq!(
        before.len(),
        3,
        "three attachments present before any delete"
    );

    let r3_ts = Utc::now() + Duration::seconds(60);
    let mut r3_vv = r3.vv.clone();
    keeplin_core::storage::note_log::increment(&mut r3_vv, "test-device");
    let r3_deleted = store
        .delete_resource(r3.id, r3_ts, &r3_vv, "test-device")
        .await
        .unwrap();
    assert!(r3_deleted, "r3 direct delete must win");
    let after_r3 = store
        .list_resources_for_note(user.id, note.id, None, None)
        .await
        .unwrap();
    assert_eq!(after_r3.len(), 2, "r3 gone after its direct delete");

    let deleted = store.soft_delete_note(note.id).await.unwrap().unwrap();
    let note_ts = deleted.deleted_at.unwrap();

    let live = store
        .list_resources_for_note(user.id, note.id, None, None)
        .await
        .unwrap();
    assert!(
        live.is_empty(),
        "every attachment is soft-deleted after the note delete"
    );

    let revived = Store::cascade_resources_note_restored(&pool, note.id, note_ts)
        .await
        .unwrap();
    assert_eq!(
        revived, 2,
        "only the two attachments the note dragged are revived"
    );

    let ids: Vec<_> = store
        .list_resources_for_note(user.id, note.id, None, None)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(
        ids,
        vec![r1.id, r2.id],
        "restore recovers r1 and r2 in created_at order; the directly-deleted r3 stays deleted"
    );
}
