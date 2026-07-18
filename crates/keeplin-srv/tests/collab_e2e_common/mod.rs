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

// md:fn register
pub async fn register(addr: SocketAddr, email: &str) {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
}

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

// md:CONVERGE_TRIES
pub const CONVERGE_TRIES: usize = 300;

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
