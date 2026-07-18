// md:Overview
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

// md:fn quota_config
fn quota_config(max_user_storage_bytes: i64, max_notes_per_user: i64) -> Config {
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
        max_user_storage_bytes,
        max_notes_per_user,
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

// md:fn spawn
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

// md:fn register
async fn register(addr: SocketAddr, email: &str) {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
}

// md:fn login
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

// md:fn post_note
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

// md:fn device
async fn device(addr: SocketAddr, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, &format!("ws://{addr}/api/sync"), token)
        .await
        .unwrap()
}

// md:fn seed_resource
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

// md:fn put_blob
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

// md:fn registration_can_be_disabled
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

// md:fn note_quota_blocks_creation_past_the_limit
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

// md:fn note_quota_disabled_by_default
#[sqlx::test(migrations = "../../migrations")]
async fn note_quota_disabled_by_default(pool: PgPool) {
    let addr = spawn(pool, quota_config(0, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    for _ in 0..5 {
        assert_eq!(post_note(addr, &token).await, 200);
    }
}

// md:fn storage_quota_blocks_upload_over_the_limit
#[sqlx::test(migrations = "../../migrations")]
async fn storage_quota_blocks_upload_over_the_limit(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;

    let a = seed_resource(&dev).await;
    let b = seed_resource(&dev).await;

    assert_eq!(put_blob(addr, &token, a, 50).await, 200);
    assert_eq!(put_blob(addr, &token, a, 50).await, 200);
    assert_eq!(put_blob(addr, &token, b, 60).await, 507);
    assert_eq!(put_blob(addr, &token, b, 40).await, 200);
}

// md:fn storage_quota_isolated_per_user
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

    assert_eq!(put_blob(addr, &ta, ra, 100).await, 200);
    assert_eq!(put_blob(addr, &tb, rb, 100).await, 200);
    let ra2 = seed_resource(&da).await;
    assert_eq!(put_blob(addr, &ta, ra2, 1).await, 507);
}
