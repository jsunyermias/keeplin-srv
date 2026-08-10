// md:Overview
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use keeplin_core::{
    models::{Resource, SYSTEM_RESOURCE_NOTE_ID},
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
        permission_scheme: keeplin_srv::config::PermissionScheme::Strict,
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
    post_note_response(addr, token).await.status().as_u16()
}

// md:fn post_note_response
async fn post_note_response(addr: SocketAddr, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/notes"))
        .bearer_auth(token)
        .json(&json!({ "title": "n" }))
        .send()
        .await
        .unwrap()
}

// md:fn import_note
async fn import_note(addr: SocketAddr, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(token)
        .json(&json!({ "title": "imported", "body": "one\ntwo" }))
        .send()
        .await
        .unwrap()
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
async fn seed_resource(addr: SocketAddr, token: &str, dev: &DbBackend) -> Uuid {
    let resource = dev
        .create_resource(
            Resource::new(
                SYSTEM_RESOURCE_NOTE_ID,
                "f",
                "application/octet-stream",
                "f.bin",
                0,
            ),
            vec![],
        )
        .await
        .unwrap();
    let changes = dev
        .get_changes_since(chrono::DateTime::from_timestamp(0, 0).unwrap())
        .await
        .unwrap();
    dev.send_changes(changes).await.unwrap();
    for _ in 0..50 {
        let resources: Value = reqwest::Client::new()
            .get(format!("http://{addr}/api/resources"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if resources
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["id"] == resource.id.to_string())
        {
            return resource.id;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("resource did not materialize");
}

// md:fn put_blob
async fn put_blob(addr: SocketAddr, token: &str, id: Uuid, len: usize) -> u16 {
    put_blob_response(addr, token, id, len)
        .await
        .status()
        .as_u16()
}

// md:fn put_blob_response
async fn put_blob_response(
    addr: SocketAddr,
    token: &str,
    id: Uuid,
    len: usize,
) -> reqwest::Response {
    reqwest::Client::new()
        .put(format!("http://{addr}/api/resources/{id}/data"))
        .bearer_auth(token)
        .body(vec![7u8; len])
        .send()
        .await
        .unwrap()
}

// md:fn install_quota_barrier
async fn install_quota_barrier(pool: &PgPool, table: &str) {
    sqlx::query(
        "CREATE FUNCTION quota_write_barrier() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(7100003); RETURN NEW; END $$",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(&format!(
        "CREATE TRIGGER quota_write_barrier BEFORE INSERT OR UPDATE ON {table} FOR EACH ROW EXECUTE FUNCTION quota_write_barrier()"
    ))
    .execute(pool)
    .await
    .unwrap();
}

// md:fn wait_for_barrier
async fn wait_for_barrier(pool: &PgPool, expected: i64) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks locks JOIN pg_database database ON database.oid = locks.database WHERE database.datname = current_database() AND locks.locktype = 'advisory' AND NOT locks.granted AND locks.objid = 7100003",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting >= expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

// md:fn wait_for_advisory_waiters
async fn wait_for_advisory_waiters(pool: &PgPool, expected: i64) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks locks JOIN pg_database database ON database.oid = locks.database WHERE database.datname = current_database() AND locks.locktype = 'advisory' AND NOT locks.granted",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting >= expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

// md:fn hold_barrier
async fn hold_barrier(pool: &PgPool) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(7100003)")
        .execute(&mut *blocker)
        .await
        .unwrap();
    blocker
}

// md:fn release_barrier
async fn release_barrier(mut blocker: sqlx::pool::PoolConnection<sqlx::Postgres>) {
    sqlx::query("SELECT pg_advisory_unlock(7100003)")
        .execute(&mut *blocker)
        .await
        .unwrap();
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

// md:fn note_quota_blocks_import_past_the_limit
#[sqlx::test(migrations = "../../migrations")]
async fn note_quota_blocks_import_past_the_limit(pool: PgPool) {
    let addr = spawn(pool, quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;

    assert_eq!(import_note(addr, &token).await.status().as_u16(), 200);
    let create_refusal = post_note_response(addr, &token).await;
    let import_refusal = import_note(addr, &token).await;
    assert_eq!(create_refusal.status().as_u16(), 507);
    assert_eq!(import_refusal.status().as_u16(), 507);
    let create_body = create_refusal.bytes().await.unwrap();
    let import_body = import_refusal.bytes().await.unwrap();
    assert_eq!(create_body, import_body);
    assert_eq!(
        import_body.as_ref(),
        br#"{"error":"note limit reached (1)"}"#
    );
}

// md:fn quota_write_inventory_is_complete
#[test]
fn quota_write_inventory_is_complete() {
    let http = include_str!("../src/http.rs");
    let sync = include_str!("../src/sync.rs");
    let projection = include_str!("../src/projection.rs");
    let store = include_str!("../src/store.rs");

    assert_eq!(http.matches(".create_note(").count(), 1);
    assert_eq!(http.matches(".create_note_on(").count(), 2);
    assert!(http.matches("lock_note_quota").count() >= 2);
    assert_eq!(http.matches(".put_resource_blob(").count(), 1);
    assert_eq!(http.matches(".put_resource_blob_on(").count(), 1);
    assert!(http.contains("lock_blob_quota"));
    assert_eq!(projection.matches(".apply_resource_create(").count(), 1);
    assert!(!sync.contains("lock_blob_quota"));
    assert!(!projection.contains("lock_blob_quota"));
    assert_eq!(
        store.matches("pg_advisory_xact_lock").count(),
        1,
        "all production advisory locks must use the shared domain constructor"
    );
    for domain in [
        "NoteOrder",
        "TagProjection",
        "NoteTagProjection",
        "ResourceProjection",
        "NoteQuota",
        "BlobQuota",
    ] {
        assert!(store.contains(domain), "missing lock domain {domain}");
    }
    assert!(store.contains("count_live_notes_for_user_on"));
    assert!(!http.contains(".count_live_notes_for_user("));
    assert!(!http.contains(".user_blob_bytes_excluding("));
    assert!(!http.contains("run_serializable"));
    let note_quota = store
        .split("// md:impl Store > fn lock_note_quota")
        .nth(1)
        .unwrap()
        .split(&["//", " md:"].concat())
        .next()
        .unwrap();
    let blob_quota = store
        .split("// md:impl Store > fn lock_blob_quota")
        .nth(1)
        .unwrap()
        .split(&["//", " md:"].concat())
        .next()
        .unwrap();
    assert!(note_quota.contains("AdvisoryLockDomain::NoteQuota"));
    assert!(!note_quota.contains("AdvisoryLockDomain::NoteOrder"));
    assert!(blob_quota.contains("AdvisoryLockDomain::BlobQuota"));
    for marker in ["// md:fn create_note", "// md:fn import_note"] {
        let handler = http
            .split(marker)
            .nth(1)
            .unwrap()
            .split(&["//", " md:"].concat())
            .next()
            .unwrap();
        assert!(
            handler.find("lock_note_quota").unwrap()
                < handler.find("count_live_notes_for_user_on").unwrap()
        );
    }
    let blob_handler = http
        .split("// md:fn put_resource_data")
        .nth(1)
        .unwrap()
        .split(&["//", " md:"].concat())
        .next()
        .unwrap();
    assert!(
        blob_handler.find("lock_blob_quota").unwrap()
            < blob_handler.find("user_blob_bytes_excluding_on").unwrap()
    );
}

// md:fn quota_paths_do_not_use_serializable_retry_or_service_unavailable
#[test]
fn quota_paths_do_not_use_serializable_retry_or_service_unavailable() {
    let http = include_str!("../src/http.rs");

    for marker in [
        "// md:fn create_note",
        "// md:fn import_note",
        "// md:fn put_resource_data",
    ] {
        let handler = http
            .split(marker)
            .nth(1)
            .unwrap()
            .split(&["//", " md:"].concat())
            .next()
            .unwrap();
        for forbidden in [
            "serializable(",
            "AppError::ServiceUnavailable",
            "StatusCode::SERVICE_UNAVAILABLE",
            "503",
        ] {
            assert!(
                !handler.contains(forbidden),
                "quota handler {marker} contains forbidden retry/503 token {forbidden}"
            );
        }
    }
}

// md:fn concurrent_blob_quota_writes_serialize_before_the_deciding_read
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_blob_quota_writes_serialize_before_the_deciding_read(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 0)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;
    let left_id = seed_resource(addr, &token, &dev).await;
    let right_id = seed_resource(addr, &token, &dev).await;
    install_quota_barrier(&pool, "resource_blobs").await;
    let blocker = hold_barrier(&pool).await;
    let left_token = token.clone();
    let left = tokio::spawn(async move { put_blob_response(addr, &left_token, left_id, 60).await });
    wait_for_barrier(&pool, 1).await;
    let right_token = token.clone();
    let right =
        tokio::spawn(async move { put_blob_response(addr, &right_token, right_id, 60).await });
    wait_for_advisory_waiters(&pool, 2).await;
    assert!(!right.is_finished());
    release_barrier(blocker).await;
    let left = left.await.unwrap();
    let right = right.await.unwrap();
    let mut statuses = [left.status().as_u16(), right.status().as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 507]);
    let stored: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(octet_length(data)), 0) FROM resource_blobs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, 60);
}

// md:fn concurrent_note_quota_writes_serialize_and_keep_the_refusal_body
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_note_quota_writes_serialize_and_keep_the_refusal_body(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    install_quota_barrier(&pool, "notes").await;
    let blocker = hold_barrier(&pool).await;
    let left_token = token.clone();
    let left = tokio::spawn(async move { post_note_response(addr, &left_token).await });
    wait_for_barrier(&pool, 1).await;
    let right_token = token.clone();
    let right = tokio::spawn(async move { post_note_response(addr, &right_token).await });
    wait_for_advisory_waiters(&pool, 2).await;
    assert!(!right.is_finished());
    release_barrier(blocker).await;
    let responses = [left.await.unwrap(), right.await.unwrap()];
    let mut success = 0;
    let mut contended_refusal = None;
    for response in responses {
        if response.status().as_u16() == 200 {
            success += 1;
        } else {
            assert_eq!(response.status().as_u16(), 507);
            contended_refusal = Some(response.bytes().await.unwrap());
        }
    }
    assert_eq!(success, 1);
    let uncontended = post_note_response(addr, &token).await;
    assert_eq!(uncontended.status().as_u16(), 507);
    assert_eq!(
        contended_refusal.unwrap(),
        uncontended.bytes().await.unwrap()
    );
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 1);
}

// md:fn quota_locks_are_scoped_by_user
#[sqlx::test(migrations = "../../migrations")]
async fn quota_locks_are_scoped_by_user(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let left_token = login(addr, "a@example.com", "dev-a").await;
    let right_token = login(addr, "b@example.com", "dev-b").await;
    install_quota_barrier(&pool, "notes").await;
    let blocker = hold_barrier(&pool).await;
    let left = tokio::spawn(async move { post_note_response(addr, &left_token).await });
    let right = tokio::spawn(async move { post_note_response(addr, &right_token).await });
    wait_for_barrier(&pool, 2).await;
    release_barrier(blocker).await;
    assert_eq!(left.await.unwrap().status().as_u16(), 200);
    assert_eq!(right.await.unwrap().status().as_u16(), 200);
}

// md:fn note_and_blob_quota_locks_use_distinct_domains
#[sqlx::test(migrations = "../../migrations")]
async fn note_and_blob_quota_locks_use_distinct_domains(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(100, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let dev = device(addr, &token).await;
    let resource_id = seed_resource(addr, &token, &dev).await;
    install_quota_barrier(&pool, "notes").await;
    sqlx::query("CREATE TRIGGER blob_quota_write_barrier BEFORE INSERT OR UPDATE ON resource_blobs FOR EACH ROW EXECUTE FUNCTION quota_write_barrier()")
        .execute(&pool)
        .await
        .unwrap();
    let blocker = hold_barrier(&pool).await;
    let note_token = token.clone();
    let note = tokio::spawn(async move { post_note_response(addr, &note_token).await });
    let blob_token = token.clone();
    let blob =
        tokio::spawn(async move { put_blob_response(addr, &blob_token, resource_id, 50).await });
    wait_for_barrier(&pool, 2).await;
    release_barrier(blocker).await;
    assert_eq!(note.await.unwrap().status().as_u16(), 200);
    assert_eq!(blob.await.unwrap().status().as_u16(), 200);
}

// md:fn failed_quota_lock_wait_is_internal_and_retryable
#[sqlx::test(migrations = "../../migrations")]
async fn failed_quota_lock_wait_is_internal_and_retryable(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = 'a@example.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query(
        "SELECT pg_advisory_lock(hashtextextended(concat('note-quota', ':', $1::text), 0))",
    )
    .bind(user_id.to_string())
    .execute(&mut *blocker)
    .await
    .unwrap();
    let max = pool.options().get_max_connections();
    let mut reserved = Vec::new();
    for _ in 1..max {
        reserved.push(pool.acquire().await.unwrap());
    }
    let mut request_connection = reserved.pop().unwrap();
    sqlx::query("SET lock_timeout = '100ms'")
        .execute(&mut *request_connection)
        .await
        .unwrap();
    drop(request_connection);
    let failed = post_note_response(addr, &token).await;
    assert_eq!(failed.status().as_u16(), 500);
    assert_ne!(failed.status().as_u16(), 507);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE owner_id = $1")
        .bind(user_id)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query(
        "SELECT pg_advisory_unlock(hashtextextended(concat('note-quota', ':', $1::text), 0))",
    )
    .bind(user_id.to_string())
    .execute(&mut *blocker)
    .await
    .unwrap();
    drop(reserved);
    drop(blocker);
    assert_eq!(post_note(addr, &token).await, 200);
}

// md:fn failure_between_quota_read_and_write_rolls_back_and_releases_lock
#[sqlx::test(migrations = "../../migrations")]
async fn failure_between_quota_read_and_write_rolls_back_and_releases_lock(pool: PgPool) {
    let addr = spawn(pool.clone(), quota_config(0, 1)).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "dev-a").await;
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = 'a@example.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE FUNCTION fail_quota_note_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected quota write failure'; END $$")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_quota_note_write BEFORE INSERT ON notes FOR EACH ROW EXECUTE FUNCTION fail_quota_note_write()")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(post_note(addr, &token).await, 500);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE owner_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let locks: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_locks locks JOIN pg_database database ON database.oid = locks.database WHERE database.datname = current_database() AND locks.locktype = 'advisory' AND locks.granted")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(locks, 0);
    sqlx::query("DROP TRIGGER fail_quota_note_write ON notes")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(post_note(addr, &token).await, 200);
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

    let a = seed_resource(addr, &token, &dev).await;
    let b = seed_resource(addr, &token, &dev).await;

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

    let ra = seed_resource(addr, &ta, &da).await;
    let rb = seed_resource(addr, &tb, &db).await;

    assert_eq!(put_blob(addr, &ta, ra, 100).await, 200);
    assert_eq!(put_blob(addr, &tb, rb, 100).await, 200);
    let ra2 = seed_resource(addr, &ta, &da).await;
    assert_eq!(put_blob(addr, &ta, ra2, 1).await, 507);
}
