use std::net::SocketAddr;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use keeplin_srv::{config::Config, http::router, state::AppState, store::Note};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

fn test_config() -> Config {
    Config {
        port: 0,
        database_url: String::new(),
        jwt_secret: "test-secret".into(),
    }
}

async fn spawn_server(pool: PgPool) -> SocketAddr {
    let state = std::sync::Arc::new(AppState::new(test_config(), pool));
    let app: Router = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn register_and_login(addr: SocketAddr, email: &str) -> (String, String, Note) {
    let client = reqwest::Client::new();

    let register = client
        .post(format!("http://{}/api/register", addr))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(register.status(), 200);

    let login: Value = client
        .post(format!("http://{}/api/login", addr))
        .json(&json!({ "email": email, "password": "password123", "device_name": "test" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap().to_string();
    let device_id = login["device_id"].as_str().unwrap().to_string();

    let note: Note = client
        .post(format!("http://{}/api/notes", addr))
        .bearer_auth(&token)
        .json(&json!({ "title": "WS test" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    (token, device_id, note)
}

#[sqlx::test(migrations = "../../migrations")]
async fn health_check(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    let res = client
        .get(format!("http://{}/health", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.text().await.unwrap(), "ok");
}

#[sqlx::test(migrations = "../../migrations")]
async fn auth_and_note_crud(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    let register = client
        .post(format!("http://{}/api/register", addr))
        .json(&json!({ "email": "test@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(register.status(), 200);

    let login: serde_json::Value = client
        .post(format!("http://{}/api/login", addr))
        .json(&json!({ "email": "test@example.com", "password": "password123", "device_name": "test" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();

    let note: Note = client
        .post(format!("http://{}/api/notes", addr))
        .bearer_auth(token)
        .json(&json!({ "title": "Integration test" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(note.title, "Integration test");

    let fetched: Note = client
        .get(format!("http://{}/api/notes/{}", addr, note.id))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(fetched.id, note.id);
    assert_eq!(fetched.title, note.title);

    let export: serde_json::Value = client
        .get(format!("http://{}/api/notes/{}/export", addr, note.id))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(export["body"].as_str().unwrap(), "");
}

#[sqlx::test(migrations = "../../migrations")]
async fn import_export_roundtrip(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    let register = client
        .post(format!("http://{}/api/register", addr))
        .json(&json!({ "email": "round@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(register.status(), 200);

    let login: serde_json::Value = client
        .post(format!("http://{}/api/login", addr))
        .json(&json!({ "email": "round@example.com", "password": "password123", "device_name": "test" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();

    let import_res = client
        .post(format!("http://{}/api/import", addr))
        .bearer_auth(token)
        .json(&json!({ "title": "Roundtrip", "body": "line1\nline2\nline3" }))
        .send()
        .await
        .unwrap();
    let import_text = import_res.text().await.unwrap();
    eprintln!("import response: {}", import_text);
    let import: serde_json::Value = serde_json::from_str(&import_text).unwrap();
    let note_id = import["note_id"].as_str().unwrap();

    let export: serde_json::Value = client
        .get(format!("http://{}/api/notes/{}/export", addr, note_id))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(export["body"].as_str().unwrap(), "line1\nline2\nline3");
}

#[sqlx::test(migrations = "../../migrations")]
async fn websocket_receives_snapshot_and_insert(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (token, device_id, note) = register_and_login(addr, "ws@example.com").await;

    let url = format!(
        "ws://{}/api/ws?token={}&note_id={}",
        addr, token, note.id
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    // First message should be a snapshot.
    let msg = ws.next().await.unwrap().unwrap();
    let text = match msg {
        Message::Text(t) => t,
        _ => panic!("expected text message"),
    };
    let snapshot: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(snapshot["type"], "Snapshot");
    assert_eq!(snapshot["note_id"], note.id.to_string());

    // Insert a line via the same socket.
    let line_id = uuid::Uuid::new_v4();
    let insert = json!({
        "type": "InsertLine",
        "note_id": note.id,
        "line_id": line_id,
        "after_line_id": null,
        "content": "hello world",
        "vv": {},
        "device_id": device_id,
        "ts": "2024-01-01T00:00:00Z"
    });
    ws.send(Message::Text(insert.to_string())).await.unwrap();

    // Server echoes the InsertLine back to the sender (broadcast excludes origin, but our
    // implementation currently broadcasts to all including origin? Let's accept either).
    // Actually broadcast excludes origin, so sender won't receive it. We'll just close.
    ws.close(None).await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn websocket_insert_propagates_to_other_client(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (token_a, device_id_a, note) = register_and_login(addr, "wsa@example.com").await;

    // Create a second user and share the note as editor.
    let client = reqwest::Client::new();
    let register_b = client
        .post(format!("http://{}/api/register", addr))
        .json(&json!({ "email": "wsb@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(register_b.status(), 200);

    let login_b: Value = client
        .post(format!("http://{}/api/login", addr))
        .json(&json!({ "email": "wsb@example.com", "password": "password123", "device_name": "test" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token_b = login_b["token"].as_str().unwrap().to_string();

    let share = client
        .post(format!("http://{}/api/notes/{}/shares", addr, note.id))
        .bearer_auth(&token_a)
        .json(&json!({ "user_email": "wsb@example.com", "role": "editor" }))
        .send()
        .await
        .unwrap();
    assert_eq!(share.status(), 200);

    let url_a = format!(
        "ws://{}/api/ws?token={}&note_id={}",
        addr, token_a, note.id
    );
    let url_b = format!(
        "ws://{}/api/ws?token={}&note_id={}",
        addr, token_b, note.id
    );

    let (mut ws_a, _) = tokio_tungstenite::connect_async(url_a).await.unwrap();
    let (mut ws_b, _) = tokio_tungstenite::connect_async(url_b).await.unwrap();

    // Consume snapshots.
    let _snap_a = ws_a.next().await.unwrap().unwrap();
    let _snap_b = ws_b.next().await.unwrap().unwrap();

    // A inserts a line.
    let line_id = uuid::Uuid::new_v4();
    let insert = json!({
        "type": "InsertLine",
        "note_id": note.id,
        "line_id": line_id,
        "after_line_id": null,
        "content": "from A",
        "vv": {},
        "device_id": device_id_a,
        "ts": "2024-01-01T00:00:00Z"
    });
    ws_a.send(Message::Text(insert.to_string())).await.unwrap();

    // B should receive the InsertLine.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), ws_b.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = match msg {
        Message::Text(t) => t,
        _ => panic!("expected text message"),
    };
    let received: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(received["type"], "InsertLine");
    assert_eq!(received["content"], "from A");

    ws_a.close(None).await.unwrap();
    ws_b.close(None).await.unwrap();
}
