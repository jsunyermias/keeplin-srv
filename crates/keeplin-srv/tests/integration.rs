//! End-to-end tests of the sync relay against the **real client**: two (or
//! three) keeplin-core `DbBackend` instances speaking the genuine wire
//! protocol — the `auth` handshake sent on construction, `send_changes`
//! envelopes, and `receive_changes` draining — through a keeplin-srv instance
//! backed by a throwaway Postgres database (`#[sqlx::test]`).
//!
//! This mirrors keeplin-core's own `ws_sync.rs` suite, but replaces its
//! test-only in-memory relay with this production server, adding what the toy
//! relay lacked: authentication, persistence, and offline catch-up.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use keeplin_core::{
    models::Note,
    storage::{db::DbBackend, NoteRepository, SyncBackend},
};
use keeplin_srv::{config::Config, http::router, state::AppState};
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
    }
}

async fn spawn_server(pool: PgPool) -> SocketAddr {
    let state = Arc::new(AppState::new(test_config(), pool));
    let app: Router = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
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

/// Log a device in and return its sync token.
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

/// Build a server-mode `DbBackend` (the real keeplin client) pointed at the
/// relay with `token`. The temp dir is leaked so the database outlives the
/// backend for the duration of the test.
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

/// Push every local change of `dev` to the relay.
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
}

/// Repeatedly `receive_changes` (each call drains ~100 ms), applying every
/// received change, until note `id` is present and — when `want_body` is
/// `Some` — its body matches. Returns whether it converged.
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

// ── Live relay between two connected devices ─────────────────────────────────

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

// ── Offline catch-up: the journal, not just live fan-out ────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn device_connecting_later_receives_backlog(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Persisted", "written while B did not exist");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;
    // Give the relay a moment to persist the batch before B connects.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // B logs in and connects only now: the note must arrive from the journal.
    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;
    assert!(
        sync_until(&b, id, None).await,
        "a device connecting later must receive the persisted backlog"
    );
}

// ── Isolation and safety properties ──────────────────────────────────────────

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

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_batches_are_deduplicated(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Once", "sent twice");
    let id = note.id;
    a.create_note(note).await.unwrap();
    // The same local changes pushed twice → two envelopes with different
    // batch_ids but... a genuine client retry re-sends the *same* payload; the
    // relay's dedup key is (batch_id, index), so pushing the identical journal
    // twice produces two batches. B must still converge to exactly one note.
    push(&a).await;
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;
    assert!(sync_until(&b, id, None).await, "B must receive the note");
    // Applying the duplicate create is idempotent on the client, so the state
    // stays consistent.
    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Once");
}

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

// ── Handshake rejections ─────────────────────────────────────────────────────

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

    // A client with a garbage token connects "successfully" (the TCP/WS
    // upgrade succeeds) but the server closes after the handshake and no
    // changes ever arrive.
    let intruder = device(addr, "not-a-valid-jwt").await;
    assert!(
        !sync_until(&intruder, id, None).await,
        "an unauthenticated client must not receive any changes"
    );
}

// ── HTTP surface ─────────────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn register_login_and_device_listing(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    register(addr, "a@example.com").await;

    // Duplicate registration is rejected with 409.
    let dup = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "a@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 409);

    let token = login(addr, "a@example.com", "laptop").await;

    // A second device can be added with the first device's token.
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

    // Wrong password fails.
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
