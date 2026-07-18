// md:Overview
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use base64::Engine as _;
use keeplin_srv::{config::Config, crypto::Cipher, http::router, reencrypt, state::AppState};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use tokio::net::TcpListener;
use uuid::Uuid;

// md:fn test_key
fn test_key() -> String {
    base64::engine::general_purpose::STANDARD.encode([9u8; 32])
}

// md:fn test_config
fn test_config(at_rest_key: Option<String>) -> Config {
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
        at_rest_key,
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
async fn spawn_server(pool: PgPool, at_rest_key: Option<String>) -> SocketAddr {
    let state = Arc::new(AppState::new(test_config(at_rest_key), pool));
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

// md:fn seed_note
async fn seed_note(addr: SocketAddr, title: &str, body: &str) -> (String, Uuid) {
    let client = reqwest::Client::new();
    client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "op@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    let login: Value = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({
            "email": "op@example.com",
            "password": "password123",
            "device_name": "seed"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap().to_string();
    let imported: Value = client
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(&token)
        .json(&json!({ "title": title, "body": body }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let note_id: Uuid = imported["note_id"].as_str().unwrap().parse().unwrap();
    (token, note_id)
}

// md:fn raw_values
async fn raw_values(pool: &PgPool) -> (Vec<String>, Vec<String>) {
    let titles = sqlx::query("SELECT title FROM notes ORDER BY title")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get::<String, _>("title"))
        .collect();
    let contents = sqlx::query("SELECT content FROM lines ORDER BY content")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get::<String, _>("content"))
        .collect();
    (titles, contents)
}

// md:fn reencrypts_pre_key_rows_and_server_still_serves_plaintext
#[sqlx::test(migrations = "../../migrations")]
async fn reencrypts_pre_key_rows_and_server_still_serves_plaintext(pool: PgPool) {
    let plain_addr = spawn_server(pool.clone(), None).await;
    let (_, note_id) = seed_note(plain_addr, "Secret title", "line one\nline two").await;

    let (titles, contents) = raw_values(&pool).await;
    assert_eq!(titles, vec!["Secret title"]);
    assert_eq!(contents, vec!["line one", "line two"]);

    let cipher = Cipher::from_key(Some(&test_key())).unwrap();
    let stats = reencrypt::run(
        &pool,
        &cipher,
        &reencrypt::Options {
            dry_run: false,
            batch_size: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(stats.notes_title.rewritten, 1);
    assert_eq!(stats.lines_content.rewritten, 2);

    let (titles, contents) = raw_values(&pool).await;
    for value in titles.iter().chain(contents.iter()) {
        assert!(
            value.starts_with(keeplin_srv::crypto::ENC_PREFIX),
            "row still plaintext after the pass: {value:?}"
        );
    }

    let keyed_addr = spawn_server(pool.clone(), Some(test_key())).await;
    let client = reqwest::Client::new();
    let login: Value = client
        .post(format!("http://{keyed_addr}/api/login"))
        .json(&json!({
            "email": "op@example.com",
            "password": "password123",
            "device_name": "verify"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let note: Value = client
        .get(format!("http://{keyed_addr}/api/notes/{note_id}"))
        .bearer_auth(login["token"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(note["title"], "Secret title");
    assert_eq!(note["body"], "line one\nline two");

    let again = reencrypt::run(&pool, &cipher, &reencrypt::Options::default())
        .await
        .unwrap();
    assert_eq!(again.notes_title.scanned, 0);
    assert_eq!(again.lines_content.scanned, 0);
}

// md:fn dry_run_reports_but_does_not_modify
#[sqlx::test(migrations = "../../migrations")]
async fn dry_run_reports_but_does_not_modify(pool: PgPool) {
    let plain_addr = spawn_server(pool.clone(), None).await;
    seed_note(plain_addr, "Untouched", "alpha\nbeta").await;
    let before = raw_values(&pool).await;

    let cipher = Cipher::from_key(Some(&test_key())).unwrap();
    let stats = reencrypt::run(
        &pool,
        &cipher,
        &reencrypt::Options {
            dry_run: true,
            batch_size: 500,
        },
    )
    .await
    .unwrap();

    assert_eq!(stats.notes_title.scanned, 1);
    assert_eq!(stats.lines_content.scanned, 2);
    assert_eq!(stats.notes_title.rewritten, 0);
    assert_eq!(stats.lines_content.rewritten, 0);

    let after = raw_values(&pool).await;
    assert_eq!(before, after, "--dry-run must not modify any row");
}

// md:fn refuses_to_run_without_a_key
#[sqlx::test(migrations = "../../migrations")]
async fn refuses_to_run_without_a_key(pool: PgPool) {
    let cipher = Cipher::from_key(None).unwrap();
    let result = reencrypt::run(&pool, &cipher, &reencrypt::Options::default()).await;
    assert!(result.is_err(), "a disabled cipher must be an error");
}
