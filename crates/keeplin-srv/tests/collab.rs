//! End-to-end tests of the collaborative line-editing protocol over a real
//! WebSocket: Join → Welcome snapshot, op propagation between participants,
//! deterministic conflict resolution, roles, presence, and import/export.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use keeplin_srv::{config::Config, http::router, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn test_config() -> Config {
    Config {
        port: 0,
        database_url: String::new(),
        jwt_secret: "test-secret".into(),
        token_ttl_days: 1,
        retention_days: 0,
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

/// Register a user; returns (user_id, token) for a fresh device login.
async fn user(addr: SocketAddr, email: &str) -> (String, String) {
    let client = reqwest::Client::new();
    let reg: Value = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let user_id = reg["user"]["id"].as_str().unwrap().to_string();
    let login: Value = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": email, "password": "password123", "device_name": "test" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (user_id, login["token"].as_str().unwrap().to_string())
}

async fn create_note(addr: SocketAddr, token: &str, title: &str) -> String {
    let note: Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/notes"))
        .bearer_auth(token)
        .json(&json!({ "title": title }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    note["id"].as_str().unwrap().to_string()
}

async fn share(addr: SocketAddr, token: &str, note_id: &str, email: &str, role: &str) {
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/notes/{note_id}/share"))
        .bearer_auth(token)
        .json(&json!({ "user_email": email, "role": role }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

async fn ws_connect(addr: SocketAddr, token: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={token}"))
        .await
        .unwrap();
    ws
}

async fn send(ws: &mut Ws, msg: Value) {
    ws.send(Message::Text(msg.to_string())).await.unwrap();
}

/// Receive JSON messages until `pred` matches (skipping presence chatter and
/// anything else), or panic after a timeout.
async fn recv_until(ws: &mut Ws, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..50 {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
            .expect("socket closed")
            .expect("socket error");
        if let Message::Text(text) = msg {
            let v: Value = serde_json::from_str(&text).unwrap();
            if pred(&v) {
                return v;
            }
        }
    }
    panic!("gave up waiting for {what}");
}

fn join(note_id: &str) -> Value {
    json!({ "type": "Join", "note_id": note_id })
}

/// Build an Insert op envelope for one line.
#[allow(clippy::too_many_arguments)]
fn insert_op(
    note_id: &str,
    line_id: &str,
    after: Option<&str>,
    content: &str,
    writer: &str,
    counter: u64,
    ts: &str,
) -> Value {
    json!({
        "type": "Op",
        "note_id": note_id,
        "ops": [{
            "op": "Insert",
            "after_line_id": after,
            "line_id": line_id,
            "content": content,
            "vv": { writer: counter },
            "last_writer": writer,
            "updated_at": ts,
        }],
    })
}

fn update_op(
    note_id: &str,
    line_id: &str,
    content: &str,
    vv: Value,
    writer: &str,
    ts: &str,
) -> Value {
    json!({
        "type": "Op",
        "note_id": note_id,
        "ops": [{
            "op": "Update",
            "line_id": line_id,
            "content": content,
            "vv": vv,
            "last_writer": writer,
            "updated_at": ts,
        }],
    })
}

async fn export_body(addr: SocketAddr, token: &str, note_id: &str) -> String {
    let v: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes/{note_id}/export"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["body"].as_str().unwrap().to_string()
}

const T1: &str = "2026-01-01T10:00:00Z";
const T2: &str = "2026-01-01T10:00:01Z";
const T3: &str = "2026-01-01T10:00:02Z";

// ── Tests ────────────────────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn join_receives_welcome_snapshot(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Nota").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;

    let welcome = recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;
    assert_eq!(welcome["note_id"].as_str().unwrap(), note_id);
    assert_eq!(welcome["snapshot"]["order"].as_array().unwrap().len(), 0);
    assert_eq!(welcome["snapshot"]["lines"].as_array().unwrap().len(), 0);

    // Presence includes ourselves.
    let presence = recv_until(&mut ws, "Presence", |v| v["type"] == "Presence").await;
    assert_eq!(presence["users"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn ops_propagate_between_participants(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (uid_a, token_a) = user(addr, "a@example.com").await;
    let (uid_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Compartida").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    // A inserts a line; B must receive the op with a server_seq.
    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "hola desde A", &uid_a, 1, T1),
    )
    .await;
    let op_at_b = recv_until(&mut ws_b, "Op at B", |v| v["type"] == "Op").await;
    assert_eq!(op_at_b["user_id"].as_str().unwrap(), uid_a);
    assert!(op_at_b["server_seq"].as_u64().unwrap() >= 1);
    let received = &op_at_b["ops"][0];
    assert_eq!(received["op"], "Insert");
    assert_eq!(received["content"], "hola desde A");

    // B updates that line, having seen A's write: vv covers both components.
    send(
        &mut ws_b,
        update_op(
            &note_id,
            &line_id,
            "editada por B",
            json!({ uid_a.clone(): 1, uid_b.clone(): 1 }),
            &uid_b,
            T2,
        ),
    )
    .await;
    let op_at_a = recv_until(&mut ws_a, "Op at A", |v| v["type"] == "Op").await;
    assert_eq!(op_at_a["ops"][0]["op"], "Update");
    assert_eq!(op_at_a["ops"][0]["content"], "editada por B");

    // The materialised body reflects the final state.
    assert_eq!(export_body(addr, &token_a, &note_id).await, "editada por B");
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_updates_resolve_deterministically(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (uid_a, token_a) = user(addr, "a@example.com").await;
    let (uid_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Conflicto").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    // A creates the line; wait until B has seen it.
    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "base", &uid_a, 1, T1),
    )
    .await;
    recv_until(&mut ws_b, "insert at B", |v| v["type"] == "Op").await;

    // Both edit concurrently from the same base ({A:1}): neither vector
    // dominates, so the deterministic (timestamp, writer) tiebreak decides —
    // B's edit carries the later timestamp and must win on every replica.
    send(
        &mut ws_a,
        update_op(
            &note_id,
            &line_id,
            "versión de A",
            json!({ uid_a.clone(): 2 }),
            &uid_a,
            T2,
        ),
    )
    .await;
    send(
        &mut ws_b,
        update_op(
            &note_id,
            &line_id,
            "versión de B",
            json!({ uid_a.clone(): 1, uid_b.clone(): 1 }),
            &uid_b,
            T3,
        ),
    )
    .await;

    // Give the server a moment to apply both, then check convergence.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(export_body(addr, &token_a, &note_id).await, "versión de B");
}

#[sqlx::test(migrations = "../../migrations")]
async fn stale_op_is_ignored(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (uid_a, token_a) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token_a, "Stale").await;

    let mut ws = ws_connect(addr, &token_a).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws,
        insert_op(&note_id, &line_id, None, "v1", &uid_a, 1, T1),
    )
    .await;
    send(
        &mut ws,
        update_op(
            &note_id,
            &line_id,
            "v2",
            json!({ uid_a.clone(): 2 }),
            &uid_a,
            T2,
        ),
    )
    .await;
    // A replay of the very same update (same vv) must not regress anything.
    send(
        &mut ws,
        update_op(
            &note_id,
            &line_id,
            "v1-replay",
            json!({ uid_a.clone(): 2 }),
            &uid_a,
            T2,
        ),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(export_body(addr, &token_a, &note_id).await, "v2");
}

#[sqlx::test(migrations = "../../migrations")]
async fn move_reorders_lines(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (uid, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Orden").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    let l1 = uuid::Uuid::new_v4().to_string();
    let l2 = uuid::Uuid::new_v4().to_string();
    let l3 = uuid::Uuid::new_v4().to_string();
    send(&mut ws, insert_op(&note_id, &l1, None, "uno", &uid, 1, T1)).await;
    send(
        &mut ws,
        insert_op(&note_id, &l2, Some(&l1), "dos", &uid, 2, T1),
    )
    .await;
    send(
        &mut ws,
        insert_op(&note_id, &l3, Some(&l2), "tres", &uid, 3, T1),
    )
    .await;

    // Move "tres" to the front.
    send(
        &mut ws,
        json!({
            "type": "Op",
            "note_id": note_id,
            "ops": [{
                "op": "Move",
                "line_ids": [l3],
                "after_line_id": null,
                "vv": { uid.clone(): 4 },
                "last_writer": uid,
                "updated_at": T2,
            }],
        }),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(export_body(addr, &token, &note_id).await, "tres\nuno\ndos");
}

#[sqlx::test(migrations = "../../migrations")]
async fn viewer_can_watch_but_not_edit(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, token_a) = user(addr, "a@example.com").await;
    let (uid_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Solo lectura").await;
    share(addr, &token_a, &note_id, "b@example.com", "viewer").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome viewer", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &line_id, None, "no debería", &uid_b, 1, T1),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "forbidden");
    assert_eq!(export_body(addr, &token_b, &note_id).await, "");
}

#[sqlx::test(migrations = "../../migrations")]
async fn outsider_cannot_join(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Privada").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "forbidden");
}

#[sqlx::test(migrations = "../../migrations")]
async fn presence_shows_other_participants(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, token_a) = user(addr, "ana@example.com").await;
    let (_uid_b, token_b) = user(addr, "bob@example.com").await;
    let note_id = create_note(addr, &token_a, "Presencia").await;
    share(addr, &token_a, &note_id, "bob@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    // A must observe a presence list containing both display names.
    let presence = recv_until(&mut ws_a, "presence with both", |v| {
        v["type"] == "Presence" && v["users"].as_array().is_some_and(|u| u.len() == 2)
    })
    .await;
    let names: Vec<&str> = presence["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["display_name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"ana") && names.contains(&"bob"),
        "{names:?}"
    );

    // B sends a cursor; A sees it attached to B's presence entry.
    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        json!({
            "type": "Cursor",
            "note_id": note_id,
            "cursor": { "line_id": line_id, "column": 3 },
        }),
    )
    .await;
    let presence = recv_until(&mut ws_a, "presence with cursor", |v| {
        v["type"] == "Presence"
            && v["users"]
                .as_array()
                .is_some_and(|u| u.iter().any(|p| p["cursor"]["column"] == 3))
    })
    .await;
    assert_eq!(presence["note_id"].as_str().unwrap(), note_id);
}

#[sqlx::test(migrations = "../../migrations")]
async fn import_then_export_roundtrip(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, token) = user(addr, "a@example.com").await;

    let body = "# Título\n\nprimera línea\nsegunda línea";
    let imported: Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(&token)
        .json(&json!({ "title": "Importada", "body": body }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let note_id = imported["note_id"].as_str().unwrap();
    assert_eq!(imported["line_count"].as_u64().unwrap(), 4);

    assert_eq!(export_body(addr, &token, note_id).await, body);

    // The materialised body also comes back on a plain GET (design §3.4).
    let note: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes/{note_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(note["body"].as_str().unwrap(), body);

    // And a Join sees the imported lines in the snapshot.
    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(note_id)).await;
    let welcome = recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;
    assert_eq!(welcome["snapshot"]["order"].as_array().unwrap().len(), 4);
    assert_eq!(welcome["snapshot"]["lines"].as_array().unwrap().len(), 4);
}

#[sqlx::test(migrations = "../../migrations")]
async fn forged_writer_is_rejected(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (uid_a, token_a) = user(addr, "a@example.com").await;
    let (uid_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Firma").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome", |v| v["type"] == "Welcome").await;

    // B tries to write in A's name — the last_writer does not match B's id.
    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &line_id, None, "suplantación", &uid_a, 1, T1),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "bad_writer");
    let _ = uid_b;
}
