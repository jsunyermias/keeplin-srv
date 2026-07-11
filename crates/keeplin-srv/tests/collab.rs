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
        lines_gc_days: 0,
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
    }
}

async fn spawn_server(pool: PgPool) -> SocketAddr {
    spawn_server_with_state(pool).await.0
}

/// Like `spawn_server` but also hands back the state, for tests that poke the
/// store directly (e.g. the tombstone GC).
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

/// Register a user; returns (user_id, device_id, token) for a fresh device
/// login. Ops must be signed with the *device* id — the vv actor.
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
    // Capability bits: READ=1, WRITE=2. editor = READ|WRITE, viewer = READ.
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

/// Poll the export endpoint until the materialised body equals `expected`.
/// Ops are applied asynchronously to the HTTP surface, so tests must wait for
/// the converged state instead of sleeping a fixed amount (a fixed sleep is
/// exactly what flakes on slow CI runners). Panics with the last seen body if
/// convergence does not happen within ~5s.
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

const T1: &str = "2026-01-01T10:00:00Z";
const T2: &str = "2026-01-01T10:00:01Z";
const T3: &str = "2026-01-01T10:00:02Z";

// ── Tests ────────────────────────────────────────────────────────────────────

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

    // Presence includes ourselves.
    let presence = recv_until(&mut ws, "Presence", |v| v["type"] == "Presence").await;
    assert_eq!(presence["users"].as_array().unwrap().len(), 1);
}

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

    // A inserts a line; B must receive the op with a server_seq.
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

    // B updates that line, having seen A's write: vv covers both components.
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

    // The materialised body reflects the final state.
    assert_eq!(export_body(addr, &token_a, &note_id).await, "editada por B");
}

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

    // A creates the line; wait until B has seen it.
    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_a,
        insert_op(&note_id, &line_id, None, "base", &did_a, 1, T1),
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

    // Whichever order the server processed them in, the converged state is
    // the same: B's edit wins the deterministic tiebreak.
    wait_export(addr, &token_a, &note_id, "versión de B").await;
}

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
    // A replay of the very same update (same vv) must not regress anything.
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

    // Ops on one connection apply in order, and the replay can never win its
    // vv check (its writer component does not advance), so converging to "v2"
    // proves the update applied and the replay was dropped.
    wait_export(addr, &token_a, &note_id, "v2").await;
}

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
                "vv": { did.clone(): 4 },
                "last_writer": did,
                "updated_at": T2,
            }],
        }),
    )
    .await;

    wait_export(addr, &token, &note_id, "tres\nuno\ndos").await;
}

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
    let (_uid_a, did_a, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _did_b, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "Firma").await;
    share(addr, &token_a, &note_id, "b@example.com", "editor").await;

    let mut ws_b = ws_connect(addr, &token_b).await;
    send(&mut ws_b, join(&note_id)).await;
    recv_until(&mut ws_b, "Welcome", |v| v["type"] == "Welcome").await;

    // B tries to sign with A's device id — last_writer must be B's device.
    let line_id = uuid::Uuid::new_v4().to_string();
    send(
        &mut ws_b,
        insert_op(&note_id, &line_id, None, "suplantación", &did_a, 1, T1),
    )
    .await;
    let err = recv_until(&mut ws_b, "Error", |v| v["type"] == "Error").await;
    assert_eq!(err["code"], "bad_writer");
}

// ── Production hardening ─────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn ws_accepts_authorization_header(pool: PgPool) {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "Header").await;

    // No token in the query string: only the Authorization header.
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

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_device_revokes_its_token(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    let client = reqwest::Client::new();

    // Register a second device with its own token.
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

    // The second token works…
    let ok = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(second_token)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);

    // …until its device is revoked from the first device.
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

#[sqlx::test(migrations = "../../migrations")]
async fn gc_compacts_old_tombstones(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (_uid, did, token) = user(addr, "a@example.com").await;
    let note_id = create_note(addr, &token, "GC").await;

    let mut ws = ws_connect(addr, &token).await;
    send(&mut ws, join(&note_id)).await;
    recv_until(&mut ws, "Welcome", |v| v["type"] == "Welcome").await;

    // Two lines; the second is deleted with an old tombstone (T1 is months
    // in the past, well beyond any GC window).
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
    // Wait for the fully-settled state — 2 lines, exactly 1 tombstoned —
    // before running GC. Polling the exported body is ambiguous here: it reads
    // "viva" both after the first insert (before line 2 exists) and after the
    // delete, so a fast poll could catch the intermediate state and GC before
    // the tombstone exists. Polling the store's line set is unambiguous.
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

    // GC anything tombstoned more than 30 days ago: exactly one line.
    let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
    let reclaimed = state.store.gc_line_tombstones(cutoff).await.unwrap();
    assert_eq!(reclaimed, 1);

    // The body is unchanged, the tombstone is gone from lines and order.
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

#[sqlx::test(migrations = "../../migrations")]
async fn metrics_reports_counts(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_uid, _did, token) = user(addr, "a@example.com").await;
    create_note(addr, &token, "Contada").await;

    let m: Value = reqwest::Client::new()
        .get(format!("http://{addr}/api/metrics"))
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

/// Spawn a server whose per-IP rate limit is `per_min` requests/minute.
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

#[sqlx::test(migrations = "../../migrations")]
async fn rate_limit_throttles_and_spares_health(pool: PgPool) {
    // Budget of 3 requests/minute from this IP.
    let addr = spawn_rate_limited(pool, 3).await;
    let client = reqwest::Client::new();

    // The 4th rapid request to a limited route is throttled.
    let mut statuses = Vec::new();
    for _ in 0..5 {
        let code = client
            .get(format!("http://{addr}/api/metrics"))
            .send()
            .await
            .unwrap()
            .status();
        statuses.push(code);
    }
    assert_eq!(statuses[0], 200);
    assert_eq!(
        statuses[4], 429,
        "burst past the budget must be throttled: {statuses:?}"
    );

    // /health is never rate-limited — orchestrator probes must always pass.
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

// ── Capability model (Front B) ─────────────────────────────────────────────────

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

#[sqlx::test(migrations = "../../migrations")]
async fn capability_grants_enforce_hierarchy_and_escalation(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_b, _db, token_b) = user(addr, "b@example.com").await;
    let (_c, _dc, _token_c) = user(addr, "c@example.com").await;
    let note_id = create_note(addr, &token_a, "N").await;

    // A grants B read-only (READ = 1). B can read but not write, and cannot share at all.
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

    // A upgrades B to SHARE_WRITE (8 → normalises to read|write|share_read|share_write = 15),
    // but not MANAGE. B may now grant C up to its own caps, but not manage (escalation).
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

#[sqlx::test(migrations = "../../migrations")]
async fn ownership_transfer_moves_delete_rights(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let (_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_b, _db, token_b) = user(addr, "b@example.com").await;
    let note_id = create_note(addr, &token_a, "N").await;

    // Only the owner can transfer; hand ownership to B.
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

    // A kept no implicit access; B is the owner now.
    assert_eq!(note_status(addr, &token_a, &note_id, "DELETE").await, 403);
    assert_eq!(note_status(addr, &token_b, &note_id, "DELETE").await, 200);
}

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

#[sqlx::test(migrations = "../../migrations")]
async fn notebook_share_cascades_to_child_notes(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    // A owns a notebook (materialised as if it had synced from A's device).
    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();

    // A creates a note and moves it into the notebook (the move adopts the notebook's
    // grants — currently none, so the note has no shares yet).
    let note_id = create_note(addr, &token_a, "N").await;
    move_note(addr, &token_a, &note_id, &nb_id).await;
    assert_eq!(note_status(addr, &token_b, &note_id, "GET").await, 403);

    // Sharing the *notebook* with B cascades read onto the child note.
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
    // Read-only: B still cannot edit.
    assert_eq!(note_status(addr, &token_b, &note_id, "PATCH").await, 403);

    // Revoking the notebook share re-cascades: B loses access to the note.
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

/// Moving a note into a notebook makes it adopt that notebook's grants, so the mover must
/// hold `write` on the **destination** notebook too — otherwise a note could be disclosed to
/// (or captured by) a notebook the mover cannot even see (issue #13).
#[sqlx::test(migrations = "../../migrations")]
async fn note_move_requires_write_on_destination_notebook(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    // A owns a notebook; B owns a note.
    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();
    let note_id = create_note(addr, &token_b, "N").await;

    // No access to the destination: the move is forbidden.
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        403
    );
    // Read on the destination is not enough — the bar is write.
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 1).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        403
    );
    // With write on the destination the move goes through.
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 2).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        200
    );
    // An unknown destination notebook is NotFound, not a silent move.
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &uuid::Uuid::new_v4().to_string()).await,
        404
    );
}

/// The notebook owner holds implicit `manage` over every note filed in their notebook (the
/// folder-owner model, issue #15): read/write/share administration — but not delete or
/// transfer, which stay with the note's own owner.
#[sqlx::test(migrations = "../../migrations")]
async fn notebook_owner_can_manage_child_notes_they_do_not_own(pool: PgPool) {
    let (addr, state) = spawn_server_with_state(pool).await;
    let (uid_a, _da, token_a) = user(addr, "a@example.com").await;
    let (_uid_b, _db, token_b) = user(addr, "b@example.com").await;
    let owner_a = uuid::Uuid::parse_str(&uid_a).unwrap();

    let nb = keeplin_core::models::Notebook::new("NB");
    let nb_id = nb.id.to_string();
    state.store.upsert_notebook(owner_a, &nb).await.unwrap();

    // B owns a note and files it in A's notebook (B holds write on the notebook).
    let note_id = create_note(addr, &token_b, "N").await;
    assert_eq!(
        notebook_share_caps(addr, &token_a, &nb_id, "b@example.com", 2).await,
        200
    );
    assert_eq!(
        move_note_status(addr, &token_b, &note_id, &nb_id).await,
        200
    );

    // A holds no note share (the cascade copies only `notebook_shares`, which names B), yet
    // as the notebook owner A can read and edit the child note…
    assert_eq!(note_status(addr, &token_a, &note_id, "GET").await, 200);
    assert_eq!(note_status(addr, &token_a, &note_id, "PATCH").await, 200);
    // …and sees it in their note listing…
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
    // …but cannot delete it: ownership stays with B.
    assert_eq!(note_status(addr, &token_a, &note_id, "DELETE").await, 403);
    assert_eq!(note_status(addr, &token_b, &note_id, "DELETE").await, 200);
}
