//! Shared harness for the real-client end-to-end tests.
//!
//! Each e2e test lives in its **own** integration-test binary (cargo runs test
//! binaries sequentially), so the real client's background tasks — reconnect
//! loops, the second `/api/sync` connection — die with the process instead of
//! hammering the shared `#[sqlx::test]` PostgreSQL harness while the next test
//! runs. That cross-test interference is what made these tests flaky when they
//! shared one binary (issue #51).
#![allow(dead_code)] // each test binary uses a subset of this shared harness

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

pub async fn register(addr: SocketAddr, email: &str) {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
}

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

/// Build the exact client stack the daemon mounts in server+collab mode: a
/// `DbBackend` (relay) wrapped in `CollabBackend` (the `/api/ws` line channel),
/// started with itself as the top of the stack.
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
    collab.start(top).await;
    collab
}

/// How long the convergence polls wait. Generous on purpose: these tests drive
/// the *real* client (its own async connect/reconnect + a second `/api/sync`
/// connection), so convergence latency tracks database throughput. Under a busy
/// CI database a tight deadline flakes even though the client converges fine — so
/// wait ~30s rather than assert an artificial 10s bound.
pub const CONVERGE_TRIES: usize = 300;

/// Poll the server's export endpoint until the materialised body equals `want`,
/// tolerating the transient `404`/empty window before the note's lines exist.
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

/// Poll a client's local note body until it equals `want`.
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
