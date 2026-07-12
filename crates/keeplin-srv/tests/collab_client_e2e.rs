//! End-to-end tests of the **real daemon collaborative client** (keeplin-core's
//! `CollabBackend`, the exact type the daemon mounts) against a real keeplin-srv
//! instance backed by a throwaway PostgreSQL database (`#[sqlx::test]`).
//!
//! `tests/collab.rs` drives the `/api/ws` protocol with hand-built frames;
//! `tests/integration.rs` drives the relay with a raw `DbBackend`. This suite
//! closes the remaining gap: the genuine client stack a daemon runs —
//! `CollabBackend<DbBackend>` — talking to the server over both channels, so the
//! client↔server contract (write-through of edits, and rebuilding a note from
//! the `Welcome` snapshot on a fresh connect) is covered in CI.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use keeplin_core::{
    collab::{CollabBackend, CollabConfig},
    models::Note,
    storage::{db::DbBackend, NoteRepository, StorageBackend},
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
        resource_purge_days: 0,
        db_max_connections: 5,
        db_acquire_timeout_secs: 10,
        db_idle_timeout_secs: 600,
        db_max_lifetime_secs: 1800,
        rate_limit_per_min: 0,
        shutdown_grace_secs: 5,
        log_json: false,
        max_upload_bytes: 100 * 1024 * 1024,
        max_user_storage_bytes: 0,
        max_notes_per_user: 0,
        registration_enabled: true,
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
    reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
}

async fn login(addr: SocketAddr, email: &str, device: &str) -> String {
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

/// Build the exact client stack the daemon mounts in server+collab mode: a
/// `DbBackend` (relay) wrapped in `CollabBackend` (the `/api/ws` line channel),
/// started with itself as the top of the stack.
async fn collab_device(addr: SocketAddr, token: &str) -> Arc<CollabBackend<DbBackend>> {
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
    collab.start(top).await;
    collab
}

/// Poll the server's export endpoint until the materialised body equals `want`,
/// tolerating the transient `404`/empty window before the note's lines exist.
async fn wait_server_body(addr: SocketAddr, token: &str, note_id: Uuid, want: &str) {
    let client = reqwest::Client::new();
    let mut last = String::new();
    for _ in 0..100 {
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

/// Poll a client's local note body until it equals `want`.
async fn wait_local_body(dev: &Arc<CollabBackend<DbBackend>>, note_id: Uuid, want: &str) {
    let mut last = String::new();
    for _ in 0..100 {
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

#[sqlx::test(migrations = "../../migrations")]
async fn collab_client_writes_note_through_to_the_server(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let a = collab_device(addr, &token).await;

    // Create a note through the real client: it POSTs the note, joins the
    // collaborative session and pushes the body as line ops. The server
    // materialises those lines.
    let note = a
        .create_note(Note::new("Title", "hello world"))
        .await
        .unwrap();
    wait_server_body(addr, &token, note.id, "hello world").await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn reconnecting_client_rebuilds_note_from_snapshot(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    // First client writes a note and disconnects.
    let note_id = {
        let a = collab_device(addr, &token).await;
        let note = a
            .create_note(Note::new("Persisted", "durable body"))
            .await
            .unwrap();
        wait_server_body(addr, &token, note.id, "durable body").await;
        note.id
        // `a` is dropped here: its connections close.
    };

    // A fresh client with an empty local database and the same account
    // discovers the note on connect, joins it, and rebuilds the body from the
    // server's Welcome snapshot — the "client DB is a cache" property.
    let b = collab_device(addr, &token).await;
    wait_local_body(&b, note_id, "durable body").await;

    // Having joined cleanly (its mirror settled from the Welcome), an edit from
    // this client converges back on the server.
    let mut edited = b.read_note(note_id).await.unwrap();
    edited.body = "edited after reconnect".into();
    b.update_note(edited).await.unwrap();
    wait_server_body(addr, &token, note_id, "edited after reconnect").await;
}
