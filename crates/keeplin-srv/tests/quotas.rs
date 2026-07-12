//! Per-user quota enforcement: note count (`POST /api/notes`) and total
//! resource-blob storage (`PUT /api/resources/:id/data`). Backed by a throwaway
//! Postgres database (`#[sqlx::test]`).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use keeplin_core::{
    models::Resource,
    storage::{db::DbBackend, ResourceRepository, SyncBackend},
};
use keeplin_srv::{config::Config, http::router, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

fn quota_config(max_user_storage_bytes: i64, max_notes_per_user: i64) -> Config {
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
        max_user_storage_bytes,
        max_notes_per_user,
        registration_enabled: true,
    }
}

async fn spawn(pool: PgPool, config: Config) -> SocketAddr {
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

async fn post_note(addr: SocketAddr, token: &str) -> u16 {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/notes"))
        .bearer_auth(token)
        .json(&json!({ "title": "n" }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn device(addr: SocketAddr, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, &format!("ws://{addr}/api/sync"), token)
        .await
        .unwrap()
}

/// Create resource metadata (with an empty blob) through the relay and return
/// its id, so the test can then drive its blob size via `PUT`.
async fn seed_resource(dev: &DbBackend) -> Uuid {
    let resource = dev
        .create_resource(
            Resource::new("f", "application/octet-stream", "f.bin", 0),
            vec![],
        )
        .await
        .unwrap();
    let changes = dev
        .get_changes_since(chrono::DateTime::from_timestamp(0, 0).unwrap())
        .await
        .unwrap();
    dev.send_changes(changes).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    resource.id
}

async fn put_blob(addr: SocketAddr, token: &str, id: Uuid, len: usize) -> u16 {
    reqwest::Client::new()
        .put(format!("http://{addr}/api/resources/{id}/data"))
        .bearer_auth(token)
        .body(vec![7u8; len])
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// With `registration_enabled = false`, the open signup endpoint is closed (issue #21):
/// `POST /api/register` returns 403 while login for existing accounts still works.
#[sqlx::test(migrations = "../../migrations")]
async fn registration_can_be_disabled(pool: PgPool) {
    let mut config = quota_config(0, 0);
    config.registration_enabled = false;
    let addr = spawn(pool, config).await;

    let code = reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "a@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(code, 403, "registration must be closed when disabled");
}

#[sqlx::test(migrations = "../../migrations")]
async fn note_quota_blocks_creation_past_the_limit(pool: PgPool) {
    let addr = spawn(pool, quota_config(0, 2)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    assert_eq!(post_note(addr, &token).await, 200);
    assert_eq!(post_note(addr, &token).await, 200);
    assert_eq!(
        post_note(addr, &token).await,
        507,
        "third note is over quota"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn note_quota_disabled_by_default(pool: PgPool) {
    let addr = spawn(pool, quota_config(0, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    for _ in 0..5 {
        assert_eq!(post_note(addr, &token).await, 200);
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn storage_quota_blocks_upload_over_the_limit(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;

    let a = seed_resource(&dev).await;
    let b = seed_resource(&dev).await;

    // 50 bytes into A: within budget.
    assert_eq!(put_blob(addr, &token, a, 50).await, 200);
    // Re-uploading 50 to A must not double-count (overwrite), still within budget.
    assert_eq!(put_blob(addr, &token, a, 50).await, 200);
    // 60 into B: 50 (A) + 60 = 110 > 100 → rejected.
    assert_eq!(put_blob(addr, &token, b, 60).await, 507);
    // 40 into B: 50 (A) + 40 = 90 ≤ 100 → allowed.
    assert_eq!(put_blob(addr, &token, b, 40).await, 200);
}

#[sqlx::test(migrations = "../../migrations")]
async fn storage_quota_isolated_per_user(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 0)).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let ta = login(addr, "a@example.com", "dev-a").await;
    let tb = login(addr, "b@example.com", "dev-b").await;
    let da = device(addr, &ta).await;
    let db = device(addr, &tb).await;

    let ra = seed_resource(&da).await;
    let rb = seed_resource(&db).await;

    // A fills its budget; B is unaffected.
    assert_eq!(put_blob(addr, &ta, ra, 100).await, 200);
    assert_eq!(put_blob(addr, &tb, rb, 100).await, 200);
    // A is now full.
    let ra2 = seed_resource(&da).await;
    assert_eq!(put_blob(addr, &ta, ra2, 1).await, 507);
}
