// md:Overview
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

// md:fn test_config
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
async fn spawn_server(pool: PgPool) -> SocketAddr {
    spawn_server_with_state(pool).await.0
}

// md:fn spawn_instance
async fn spawn_instance(pool: PgPool) -> SocketAddr {
    let state = Arc::new(AppState::new(test_config(), pool));
    keeplin_srv::bus::spawn(state.clone());
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

// md:fn spawn_server_with_state
async fn spawn_server_with_state(pool: PgPool) -> (SocketAddr, Arc<AppState>) {
    let state = Arc::new(AppState::new(test_config(), pool));
    let app: Router = router(state.clone());
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
    (addr, state)
}

// md:fn user
async fn user(addr: SocketAddr, email: &str) -> (String, String, String) {
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
    (
        user_id,
        login["device_id"].as_str().unwrap().to_string(),
        login["token"].as_str().unwrap().to_string(),
    )
}

// md:fn create_note
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

// md:fn share
async fn share(addr: SocketAddr, token: &str, note_id: &str, email: &str, role: &str) {
    let capabilities = match role {
        "editor" => 3,
        "viewer" => 1,
        other => panic!("unknown test role {other}"),
    };
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/notes/{note_id}/share"))
        .bearer_auth(token)
        .json(&json!({ "user_email": email, "capabilities": capabilities }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// md:fn ws_connect
async fn ws_connect(addr: SocketAddr, token: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={token}"))
        .await
        .unwrap();
    ws
}

// md:fn send
async fn send(ws: &mut Ws, msg: Value) {
    ws.send(Message::Text(msg.to_string())).await.unwrap();
}

// md:fn recv_until
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

// md:fn join
fn join(note_id: &str) -> Value {
    json!({ "type": "Join", "note_id": note_id })
}

// md:fn insert_op
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

// md:fn update_op
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

// md:fn export_body
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

// md:fn wait_export
async fn wait_export(addr: SocketAddr, token: &str, note_id: &str, expected: &str) {
    let mut last = String::new();
    for _ in 0..50 {
        last = export_body(addr, token, note_id).await;
        if last == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("body never converged to {expected:?}; last seen: {last:?}");
}

// md:Timestamps
const T1: &str = "2026-01-01T10:00:00Z";
const T2: &str = "2026-01-01T10:00:01Z";
const T3: &str = "2026-01-01T10:00:02Z";

// md:fn join_receives_welcome_snapshot
#[sqlx::test(migrations = "../../migrations")]
async fn join_receives_welcome_snapshot(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Nota").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;

    let welcome = recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;
    assert_eq!(welcome["note_id"].as_str().unwrap(), note_id);
    assert_eq!(welcome["snapshot"]["order"].as_array().unwrap().len(), 0);
    assert_eq!(welcome["snapshot"]["lines"].as_array().unwrap().len(), 0);

    let presence = recv_until(&mut ws, "Presence", |v| v["type"] == "Presence").await;
    assert_eq!(presence["users"].as_array().unwrap().len(), 1);
}

// md:fn ops_propagate_between_participants
#[sqlx::test(migrations = "../../migrations")]
async fn ops_propagate_between_participants(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Compartida").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "hola desde A", &did_a, 1, T1),
    )
    .await;
    let op_at_b = recv_until(&mut ws_b, "Op at B", |v| v["type"] == "Op").await;
    assert_eq!(op_at_b["user_id"].as_str().unwrap(), uid_a);
    assert!(op_at_b["server_seq"].as_u64().unwrap() >= 1);
    let received = &op_at_b["ops"][0];
    assert_eq!(received["op"], "Insert");
    assert_eq!(received["content"], "hola desde A");

    send(
        &mut ws_b,
        update_op(
            &note_id,
            &line_id,
            "editada por B",
            json!({ did_a.clone(): 1, did_b.clone(): 1 }),
            &did_b,
            T2,
        ),
    )
    .await;
    let op_at_a = recv_until(&mut ws_a, "Op at A", |v| v["type"] == "Op").await;
    assert_eq!(op_at_a["ops"][0]["op"], "Update");
    assert_eq!(op_at_a["ops"][0]["content"], "editada por B");

    assert_eq!(export_body(addr, &token_a, &note_id).await, "editada por B");
}

// md:fn ops_and_presence_propagate_across_instances
#[sqlx::test(migrations = "../../migrations")]
async fn ops_and_presence_propagate_across_instances(pool: PgPool) {
    let addr_a = spawn_instance(pool.clone()).await;
    let addr_b = spawn_instance(pool.clone()).await;

    let (uid_a, did_a, token_a) = user(addr_a, "a@example.com").await;
    let (_uid_b, did_b, token_b) = user(addr_a, "b@example.com").await;
    let note_id = create_note(addr_a, &token_a, "Cross-instance").await;
    share(addr_a, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_a = ws_connect(addr_a, &token_a).await;
    let mut ws_b = ws_connect(addr_b, &token_b).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    recv_until(&mut ws_a, "merged presence", |v| {
        v["type"] == "Presence" && v["users"].as_array().map(|u| u.len()) == Some(2)
    })
    .await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "from instance A", &did_a, 1, T1),
    )
    .await;
    let op_at_b = recv_until(&mut ws_b, "Op at B across instances", |v| v["type"] == "Op").await;
    assert_eq!(op_at_b["user_id"].as_str().unwrap(), uid_a);
    assert_eq!(op_at_b["ops"][0]["content"], "from instance A");

    let line_b = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(
            &note_id,
            &line_b,
            Some(&line_id),
            "from instance B",
            &did_b,
            1,
            T2,
        ),
    )
    .await;
    let op_at_a = recv_until(&mut ws_a, "Op at A across instances", |v| v["type"] == "Op").await;
    assert_eq!(op_at_a["ops"][0]["content"], "from instance B");

    wait_export(
        addr_b,
        &token_b,
        &note_id,
        "from instance A\nfrom instance B",
    )
    .await;
}

// md:fn concurrent_updates_resolve_deterministically
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_updates_resolve_deterministically(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Conflicto").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "base", &did_a, 1, T1),
    )
    .await;
    recv_until(&mut ws_b, "insert at B", |v| v["type"] == "Op").await;

    send(
        &mut ws_a,
        update_op(
            &note_id,
            &line_id,
            "versión de A",
            json!({ did_a.clone(): 2 }),
            &did_a,
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
            json!({ did_a.clone(): 1, did_b.clone(): 1 }),
            &did_b,
            T3,
        ),
    )
    .await;

    wait_export(addr, &token_a, &note_id, "versión de B").await;
}

// md:fn stale_op_is_ignored
#[sqlx::test(migrations = "../../migrations")]
async fn stale_op_is_ignored(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token_a, "Stale").await;

    let mut ws = ws_connect(addr, &token_a).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws,
        insert_op(&note_id, &line_id, None, "v1", &did_a, 1, T1),
    )
    .await;
    send(
        &mut ws,
        update_op(
            &note_id,
            &line_id,
            "v2",
            json!({ did_a.clone(): 2 }),
            &did_a,
            T2,
        ),
    )
    .await;
    send(
        &mut ws,
        update_op(
            &note_id,
            &line_id,
            "v1-replay",
            json!({ did_a.clone(): 2 }),
            &did_a,
            T2,
        ),
    )
    .await;

    wait_export(addr, &token_a, &note_id, "v2").await;
}

// md:fn move_reorders_lines
#[sqlx::test(migrations = "../../migrations")]
async fn move_reorders_lines(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Orden").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    let l1 = uuid::Uuid::new_v4().to_string();
    let l2 = uuid::Uuid::new_v4().to_string();
    let l3 = uuid::Uuid::new_v4().to_string();
    send(&mut ws, insert_op(&note_id, &l1, None, "uno", &did, 1, T1)).await;
    send(
        &mut ws,
        insert_op(&note_id, &l2, Some(&l1), "dos", &did, 2, T1),
    )
    .await;
    send(
        &mut ws,
        insert_op(&note_id, &l3, Some(&l2), "tres", &did, 3, T1),
    )
    .await;

    send(
        &mut ws,
        json!({
            "type": "Op",
            "note_id": note_id,
            "ops": [{
                "op": "Move",
                "line_ids": [l3],
                "after_line_id": null,
                "vv": { did.clone(): 4 },
                "last_writer": did,
                "updated_at": T2,
            }],
        }),
    )
    .await;

    wait_export(addr, &token, &note_id, "tres\nuno\ndos").await;
}

// md:fn viewer_can_watch_but_not_edit
#[sqlx::test(migrations = "../../migrations")]
async fn viewer_can_watch_but_not_edit(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, _did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Solo lectura").await;
    share(addr, &token_a, &note_id, "b@example.com", "viewer").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome viewer", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &line_id, None, "no debería", &did_b, 1, T1),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "forbidden");
    assert_eq!(export_body(addr, &token_b, &note_id).await, "");
}

// md:fn revoking_a_share_stops_edits_mid_session
#[sqlx::test(migrations = "../../migrations")]
async fn revoking_a_share_stops_edits_mid_session(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, _did_a, token_a) = user(addr, "a@example.com").await;
    let (uid_b, did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Colaborativa").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome", |v| v["type"] == "Welcome").await;

    let l1 = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &l1, None, "primera", &did_b, 1, T1),
    )
    .await;
    wait_export(addr, &token_a, &note_id, "primera").await;

    let code = reqwest::Client::new()
        .delete(format!("http://{addr}/api/notes/{note_id}/share/{uid_b}"))
        .bearer_auth(&token_a)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);

    let l2 = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &l2, Some(&l1), "segunda", &did_b, 2, T2),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "forbidden");
}

// md:fn outsider_cannot_join
#[sqlx::test(migrations = "../../migrations")]
async fn outsider_cannot_join(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, _did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Privada").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "forbidden");
}

// md:fn presence_shows_other_participants
#[sqlx::test(migrations = "../../migrations")]
async fn presence_shows_other_participants(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, _did_a, token_a) = user(addr, "ana@example.com").await;
    let (_uid_b, _did_b, token_b) = user(addr, "bob@example.com").await;
    let note_id = create_note(addr, &token_a, "Presencia").await;
    share(addr, &token_a, &note_id, "bob@example.com", "editor").await;

    let mut ws_a = ws_connect(addr, &token_a).await;
    send(&mut ws_a, join(&note_id)).await;
    recv_until(&mut ws_a, "Welcome A", |v| v["type"] == "Welcome").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome B", |v| v["type"] == "Welcome").await;

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

// md:fn import_then_export_roundtrip
#[sqlx::test(migrations = "../../migrations")]
async fn import_then_export_roundtrip(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;

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

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(note_id)).await;
    let welcome = recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;
    assert_eq!(welcome["snapshot"]["order"].as_array().unwrap().len(), 4);
    assert_eq!(welcome["snapshot"]["lines"].as_array().unwrap().len(), 4);
}

// md:fn forged_writer_is_rejected
#[sqlx::test(migrations = "../../migrations")]
async fn forged_writer_is_rejected(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Firma").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome", |v| v["type"] == "Welcome").await;

    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &line_id, None, "suplantación", &did_a, 1, T1),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "bad_writer");
}

// md:fn ws_accepts_authorization_header
#[sqlx::test(migrations = "../../migrations")]
async fn ws_accepts_authorization_header(pool: PgPool) {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Header").await;

    let mut req = format!("ws://{addr}/api/ws").into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome via header auth", |v| {
        v["type"] == "Welcome"
    })
    .await;
}

// md:fn deleting_a_device_revokes_its_token
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_device_revokes_its_token(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let client = reqwest::Client::new();

    let second: Value = client
        .post(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .json(&json!({ "device_name": "stolen-phone" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_token = second["token"].as_str().unwrap();
    let second_id = second["device_id"].as_str().unwrap();

    let ok = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(second_token)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);

    let del = client
        .delete(format!("http://{addr}/api/devices/{second_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);

    let denied = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(second_token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
}

// md:fn deleting_a_device_revokes_its_collab_token
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_device_revokes_its_collab_token(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let client = reqwest::Client::new();

    let second: Value = client
        .post(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .json(&json!({ "device_name": "stolen-phone" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_token = second["token"].as_str().unwrap();
    let second_id = second["device_id"].as_str().unwrap();

    assert!(
        tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={second_token}"))
            .await
            .is_ok(),
        "a live device's token must connect"
    );

    let del = client
        .delete(format!("http://{addr}/api/devices/{second_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);

    assert!(
        tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={second_token}"))
            .await
            .is_err(),
        "a revoked device's token must be rejected on /api/ws"
    );
}

// md:fn gc_compacts_old_tombstones
#[sqlx::test(migrations = "../../migrations")]
async fn gc_compacts_old_tombstones(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (_uid, did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "GC").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    let l1 = uuid::Uuid::new_v4().to_string();
    let l2 = uuid::Uuid::new_v4().to_string();
    send(&mut ws, insert_op(&note_id, &l1, None, "viva", &did, 1, T1)).await;
    send(
        &mut ws,
        insert_op(&note_id, &l2, Some(&l1), "muerta", &did, 2, T1),
    )
    .await;
    send(
        &mut ws,
        json!({
            "type": "Op",
            "note_id": note_id,
            "ops": [{
                "op": "Delete",
                "line_id": l2,
                "deleted_at": T1,
                "vv": { did.clone(): 3 },
                "last_writer": did,
                "updated_at": T2,
            }],
        }),
    )
    .await;
    let note_uuid = note_id.parse().unwrap();
    let mut settled = false;
    for _ in 0..50 {
        let lines = state.store.list_lines(note_uuid).await.unwrap();
        let tombstones = lines.iter().filter(|l| l.deleted_at.is_some()).count();
        if lines.len() == 2 && tombstones == 1 {
            settled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(settled, "delete never landed as a tombstone");

    let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
    let reclaimed = state.store.gc_line_tombstones(cutoff).await.unwrap();
    assert_eq!(reclaimed, 1);

    assert_eq!(export_body(addr, &token, &note_id).await, "viva");
    let lines = state
        .store
        .list_lines(note_id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    let order = state
        .store
        .get_note_order(note_id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(order.order.len(), 1);
}

// md:fn version_endpoint_advertises_capabilities
#[sqlx::test(migrations = "../../migrations")]
async fn version_endpoint_advertises_capabilities(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let v: Value = reqwest::Client::new()
        .get(format!("http://{addr}/version"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["name"], "keeplin-srv");
    assert!(v["protocol_version"].as_u64().unwrap() >= 1);
    let caps = v["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "history"), "advertises history");
}

// md:fn health_and_readiness_probes
#[sqlx::test(migrations = "../../migrations")]
async fn health_and_readiness_probes(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    let health = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    assert_eq!(health.text().await.unwrap(), "ok");

    let ready = client
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);
    assert_eq!(ready.text().await.unwrap(), "ready");
}

// md:fn metrics_reports_counts
#[sqlx::test(migrations = "../../migrations")]
async fn metrics_reports_counts(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    create_note(addr, &token, "Contada").await;

    let anon = reqwest::Client::new()
        .get(format!("http://{addr}/api/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401, "metrics must not be world-readable");

    let m: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/metrics"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(m["users"].as_i64().unwrap(), 1);
    assert_eq!(m["notes"].as_i64().unwrap(), 1);
    assert!(m["collab_sessions"].as_i64().is_some());
}

// md:fn spawn_rate_limited
async fn spawn_rate_limited(pool: PgPool, per_min: u32) -> SocketAddr {
    let mut cfg = test_config();
    cfg.rate_limit_per_min = per_min;
    let state = Arc::new(AppState::new(cfg, pool));
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

// md:fn rate_limit_throttles_and_spares_health
#[sqlx::test(migrations = "../../migrations")]
async fn rate_limit_throttles_and_spares_health(pool: PgPool) {
    let addr = spawn_rate_limited(pool, 10).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let client = reqwest::Client::new();

    let mut got_ok = false;
    let mut got_throttled = false;
    for _ in 0..40 {
        let code = client
            .get(format!("http://{addr}/api/metrics"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status();
        match code.as_u16() {
            200 => got_ok = true,
            429 => {
                got_throttled = true;
                break;
            }
            other => panic!("unexpected status {other}"),
        }
    }
    assert!(
        got_ok,
        "authenticated requests succeed before the budget is spent"
    );
    assert!(
        got_throttled,
        "burst past the budget must be throttled with 429"
    );

    for _ in 0..10 {
        let code = client
            .get(format!("http://{addr}/health"))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(code, 200);
    }
}

// md:fn share_caps
async fn share_caps(addr: SocketAddr, token: &str, note_id: &str, email: &str, caps: i32) -> u16 {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/notes/{note_id}/share"))
        .bearer_auth(token)
        .json(&json!({ "user_email": email, "capabilities": caps }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

// md:fn note_status
async fn note_status(addr: SocketAddr, token: &str, note_id: &str, method: &str) -> u16 {
    let http = reqwest::Client::new();
    let url = format!("http://{addr}/api/notes/{note_id}");
    let req = match method {
        "GET" => http.get(url),
        "PATCH" => http.patch(url).json(&json!({ "title": "x" })),
        "DELETE" => http.delete(url),
        _ => unreachable!(),
    };
    req.bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

// md:fn capability_grants_enforce_hierarchy_and_escalation
#[sqlx::test(migrations = "../../migrations")]
async fn capability_grants_enforce_hierarchy_and_escalation(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_b, _db, token_b) = user(addr, "b@example.com").await;
    let (_c, _dc, _token_c) = user(addr, "c@example.com").await;
    let note_id = create_note(addr, &token_a, "N").await;

    assert_eq!(
        share_caps(addr, &token_a, &note_id, "b@example.com", 1).await,
        200
    );
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 200);
    assert_eq!(note_status(addr, &token_b, &note_id, "PATCH").await, 403);
    assert_eq!(
        share_caps(addr, &token_b, &note_id, "c@example.com", 1).await,
        403,
        "read-only grantee has no share_write"
    );

    assert_eq!(
        share_caps(addr, &token_a, &note_id, "b@example.com", 8).await,
        200
    );
    assert_eq!(
        share_caps(addr, &token_b, &note_id, "c@example.com", 3).await,
        200,
        "B holds write, so it may grant read+write"
    );
    assert_eq!(
        share_caps(addr, &token_b, &note_id, "c@example.com", 16).await,
        403,
        "B lacks manage, so it cannot grant manage"
    );
}

// md:fn ownership_transfer_moves_delete_rights
#[sqlx::test(migrations = "../../migrations")]
async fn ownership_transfer_moves_delete_rights(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_b, _db, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "N").await;

    let code = reqwest::Client::new()
        .post(format!("http://{addr}/api/notes/{note_id}/transfer"))
        .bearer_auth(&token_a)
        .json(&json!({ "user_email": "b@example.com" }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(code, 200);

    assert_eq!(note_status(addr, &token_a, &note_id, "DELETE").await, 403);
    assert_eq!(note_status(addr, &token_b, &note_id, "DELETE").await, 200);
}

// md:fn move_note
async fn move_note(addr: SocketAddr, token: &str, note_id: &str, notebook_id: &str) {
    let code = reqwest::Client::new()
        .patch(format!("http://{addr}/api/notes/{note_id}"))
        .bearer_auth(token)
        .json(&json!({ "notebook_id": notebook_id }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);
}

// md:fn notebook_share_cascades_to_child_notes
#[sqlx::test(migrations = "../../migrations")]
async fn notebook_share_cascades_to_child_notes(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();

    let note_id = create_note(addr, &token_a, "N").await;
    move_note(addr, &token_a, &note_id, &nb_id).await;
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 403);

    let code = reqwest::Client::new()
        .post(format!("http://{addr}/api/notebooks/{nb_id}/share"))
        .bearer_auth(&token_a)
        .json(&json!({ "user_email": "b@example.com", "capabilities": 1 }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);
    assert_eq!(
        note_status(addr, &token_b, &note_id, "GET").await,
        200,
        "notebook share cascaded read onto the note"
    );
    assert_eq!(note_status(addr, &token_b, &note_id, "PATCH").await, 403);

    let code = reqwest::Client::new()
        .delete(format!("http://{addr}/api/notebooks/{nb_id}/share/{uid_b}"))
        .bearer_auth(&token_a)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 403);
}

// md:fn notebook_share_caps
async fn notebook_share_caps(
    addr: SocketAddr,
    token: &str,
    notebook_id: &str,
    email: &str,
    caps: i32,
) -> u16 {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/notebooks/{notebook_id}/share"))
        .bearer_auth(token)
        .json(&json!({ "user_email": email, "capabilities": caps }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

// md:fn move_note_status
async fn move_note_status(addr: SocketAddr, token: &str, note_id: &str, notebook_id: &str) -> u16 {
    reqwest::Client::new()
        .patch(format!("http://{addr}/api/notes/{note_id}"))
        .bearer_auth(token)
        .json(&json!({ "notebook_id": notebook_id }))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

// md:fn note_move_requires_write_on_destination_notebook
#[sqlx::test(migrations = "../../migrations")]
async fn note_move_requires_write_on_destination_notebook(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();
    let note_id = create_note(addr, &token_b, "N").await;

    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        403
    );
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 1).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        403
    );
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 2).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &uuid::Uuid::new_v4().to_string()).await,
        404
    );
}

// md:fn notebook_owner_can_manage_child_notes_they_do_not_own
#[sqlx::test(migrations = "../../migrations")]
async fn notebook_owner_can_manage_child_notes_they_do_not_own(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();

    let note_id = create_note(addr, &token_b, "N").await;
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 2).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        200
    );

    assert_eq!(note_status(addr, &token_a, &note_id, "GET").await, 200);
    assert_eq!(note_status(addr, &token_a, &note_id, "PATCH").await, 200);
    let notes: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        notes
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == note_id.as_str()),
        "notebook owner sees child notes in their listing"
    );
    assert_eq!(note_status(addr, &token_a, &note_id, "DELETE").await, 403);
    assert_eq!(note_status(addr, &token_b, &note_id, "DELETE").await, 200);
}

// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares
#[sqlx::test(migrations = "../../migrations")]
async fn nil_notebook_id_patch_means_inbox_and_keeps_shares(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_b, _db, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "N").await;
    share(addr, &token_a, &note_id, "b@example.com", "viewer").await;

    let response = reqwest::Client::new()
        .patch(format!("http://{addr}/api/notes/{note_id}"))
        .bearer_auth(&token_a)
        .json(&json!({
            "title": "renamed",
            "notebook_id": "00000000-0000-0000-0000-000000000000",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "nil uuid is the Inbox, not a 404");
    let note: Value = response.json().await.unwrap();
    assert!(note["notebook_id"].is_null(), "stored as NULL (the Inbox)");
    assert_eq!(note["title"], "renamed");
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 200);
}
