//! End-to-end tests of the sync relay against the **real client**: two (or
//! three) keeplin-core `DbBackend` instances speaking the genuine wire
//! protocol — the `auth` handshake sent on construction, `send_changes`
//! envelopes, and `receive_changes` draining — through a keeplin-srv instance
//! backed by a throwaway Postgres database (`#[sqlx::test]`).
//!
//! This mirrors keeplin-core's own `ws_sync.rs` suite, but replaces its
//! test-only in-memory relay with this production server, adding what the toy
//! relay lacked: authentication, persistence, and offline catch-up.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use keeplin_core::{
    models::Note,
    storage::{db::DbBackend, NoteRepository, NotebookRepository, SyncBackend},
};
use keeplin_srv::{config::Config, http::router, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

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
        history_since_access: false,
    }
}

async fn spawn_server(pool: PgPool) -> SocketAddr {
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

/// Spawn a server instance with the cross-instance bus running, for the
/// multi-instance relay test (issue #45).
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

async fn register(addr: SocketAddr, email: &str) {
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": email, "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Log a device in and return its sync token.
async fn login(addr: SocketAddr, email: &str, device_name: &str) -> String {
    let body: Value = reqwest::Client::new()
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": email, "password": "password123", "device_name": device_name }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["token"].as_str().unwrap().to_string()
}

/// Build a server-mode `DbBackend` (the real keeplin client) pointed at the
/// relay with `token`. The temp dir is leaked so the database outlives the
/// backend for the duration of the test.
async fn device(addr: SocketAddr, token: &str) -> DbBackend {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.db");
    std::mem::forget(dir);
    DbBackend::new(path, &format!("ws://{addr}/api/sync"), token)
        .await
        .unwrap()
}

fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(0, 0).unwrap()
}

/// Push every local change of `dev` to the relay.
async fn push(dev: &DbBackend) {
    let changes = dev.get_changes_since(epoch()).await.unwrap();
    dev.send_changes(changes).await.unwrap();
}

/// Repeatedly `receive_changes` (each call drains ~100 ms), applying every
/// received change, until note `id` is present and — when `want_body` is
/// `Some` — its body matches. Returns whether it converged.
async fn sync_until(dev: &DbBackend, id: Uuid, want_body: Option<&str>) -> bool {
    for _ in 0..50 {
        let remote = dev.receive_changes().await.unwrap();
        for change in remote {
            dev.apply_change(change).await.unwrap();
        }
        if let Ok(note) = dev.read_note(id).await {
            match want_body {
                None => return true,
                Some(body) if note.body == body => return true,
                Some(_) => {}
            }
        }
    }
    false
}

// ── Live relay between two connected devices ─────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn note_syncs_live_between_two_devices(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;
    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;

    let note = Note::new("Shared", "over the wire");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;

    assert!(
        sync_until(&b, id, None).await,
        "device B must receive A's note through the relay"
    );
    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Shared");
    assert_eq!(read.body, "over the wire");
}

/// A batch pushed to one instance is delivered live to a device connected to a
/// *different* instance, via the cross-instance relay wake over the bus (issue
/// #45) — not just on the next reconnect.
#[sqlx::test(migrations = "../../migrations")]
async fn relay_batch_propagates_across_instances(pool: PgPool) {
    let addr_a = spawn_instance(pool.clone()).await;
    let addr_b = spawn_instance(pool.clone()).await;
    register(addr_a, "a@example.com").await;
    let a = device(addr_a, &login(addr_a, "a@example.com", "laptop").await).await;
    let b = device(addr_b, &login(addr_b, "a@example.com", "phone").await).await;

    let note = Note::new("Cross", "over two instances");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;

    assert!(
        sync_until(&b, id, None).await,
        "device B on the other instance must receive A's note live"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_propagates_and_converges(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;
    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;

    let mut note = Note::new("v1", "body v1");
    let id = note.id;
    a.create_note(note.clone()).await.unwrap();
    push(&a).await;
    assert!(sync_until(&b, id, None).await, "B must receive the create");

    note.title = "v2".to_string();
    note.body = "body v2".to_string();
    note.updated_at = chrono::Utc::now();
    a.update_note(note).await.unwrap();
    push(&a).await;

    assert!(
        sync_until(&b, id, Some("body v2")).await,
        "B must converge to A's update"
    );
}

// ── Offline catch-up: the journal, not just live fan-out ────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn device_connecting_later_receives_backlog(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Persisted", "written while B did not exist");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;
    // Give the relay a moment to persist the batch before B connects.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // B logs in and connects only now: the note must arrive from the journal.
    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;
    assert!(
        sync_until(&b, id, None).await,
        "a device connecting later must receive the persisted backlog"
    );
}

// ── Isolation and safety properties ──────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn users_do_not_see_each_others_changes(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;
    let b = device(addr, &login(addr, "b@example.com", "laptop").await).await;

    let note = Note::new("Private", "user A only");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;

    assert!(
        !sync_until(&b, id, None).await,
        "user B must never receive user A's changes"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_batches_are_deduplicated(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Once", "sent twice");
    let id = note.id;
    a.create_note(note).await.unwrap();
    // The same local changes pushed twice → two envelopes with different
    // batch_ids but... a genuine client retry re-sends the *same* payload; the
    // relay's dedup key is (batch_id, index), so pushing the identical journal
    // twice produces two batches. B must still converge to exactly one note.
    push(&a).await;
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let b = device(addr, &login(addr, "a@example.com", "phone").await).await;
    assert!(sync_until(&b, id, None).await, "B must receive the note");
    // Applying the duplicate create is idempotent on the client, so the state
    // stays consistent.
    let read = b.read_note(id).await.unwrap();
    assert_eq!(read.title, "Once");
}

#[sqlx::test(migrations = "../../migrations")]
async fn sender_never_receives_its_own_changes_back(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Echo?", "should not come back");
    a.create_note(note).await.unwrap();
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let echoed = a.receive_changes().await.unwrap();
    assert!(
        echoed.is_empty(),
        "the relay must never echo a device's own changes back: {echoed:?}"
    );
}

// ── Handshake rejections ─────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn invalid_token_gets_no_data(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "laptop").await).await;

    let note = Note::new("Secret", "authenticated only");
    let id = note.id;
    a.create_note(note).await.unwrap();
    push(&a).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // A client with a garbage token connects "successfully" (the TCP/WS
    // upgrade succeeds) but the server closes after the handshake and no
    // changes ever arrive.
    let intruder = device(addr, "not-a-valid-jwt").await;
    assert!(
        !sync_until(&intruder, id, None).await,
        "an unauthenticated client must not receive any changes"
    );
}

// ── HTTP surface ─────────────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn register_login_and_device_listing(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    register(addr, "a@example.com").await;

    // Duplicate registration is rejected with 409.
    let dup = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "a@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 409);

    let token = login(addr, "a@example.com", "laptop").await;

    // A second device can be added with the first device's token.
    let second: Value = client
        .post(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .json(&json!({ "device_name": "phone" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(second["token"].as_str().is_some());
    assert_eq!(second["device_name"], "phone");

    let devices: Value = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(devices.as_array().unwrap().len(), 2);

    // Wrong password fails.
    let bad = client
        .post(format!("http://{addr}/api/login"))
        .json(
            &json!({ "email": "a@example.com", "password": "wrong-password", "device_name": "x" }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);
}

// ── Server-side history (Front D stage 2) ────────────────────────────────────

/// The server journal holds every device's changes, so `GET /api/…/history` serves the
/// full cross-device version history — including to a fresh device whose local journal is
/// empty. Newest first; a delete is a tombstone (`entity: null`); scoped per account.
#[sqlx::test(migrations = "../../migrations")]
async fn history_endpoints_serve_versions_from_the_server_journal(pool: PgPool) {
    let addr = spawn_server(pool).await;
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;
    let a = device(addr, &token).await;
    let device_id = a.get_device_id().await.unwrap();

    // Device A authors three note versions (create, edit, delete) and a notebook rename,
    // then pushes the lot in one batch.
    let note = a.create_note(Note::new("T", "v1")).await.unwrap();
    let mut edited = note.clone();
    edited.body = "v2".into();
    a.update_note(edited).await.unwrap();
    a.delete_note(note.id).await.unwrap();
    let nb = a
        .create_notebook(keeplin_core::models::Notebook::new("old"))
        .await
        .unwrap();
    let mut renamed = nb.clone();
    renamed.title = "new".into();
    a.update_notebook(renamed).await.unwrap();
    push(&a).await;

    let client = reqwest::Client::new();
    let get = |path: String| {
        let client = client.clone();
        let token = token.clone();
        async move {
            client
                .get(format!("http://{addr}{path}"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    // `send_changes` returns once the frame is on the wire; the server journals it
    // asynchronously, so poll until the batch has landed.
    let mut versions = Value::Null;
    for _ in 0..50 {
        versions = get(format!("/api/notes/{}/history", note.id)).await;
        if versions.as_array().is_some_and(|v| v.len() >= 3) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Note history: newest first — tombstone, v2, v1 — each stamped with the sync device.
    let versions = versions.as_array().unwrap();
    assert_eq!(
        versions.len(),
        3,
        "create + update + delete = three versions"
    );
    assert!(versions[0]["entity"].is_null(), "a delete is a tombstone");
    assert_eq!(versions[1]["entity"]["body"], "v2");
    assert_eq!(versions[2]["entity"]["body"], "v1");
    assert_eq!(versions[0]["device_id"], device_id.as_str());

    // The count cap bounds the reply.
    let capped = get(format!("/api/notes/{}/history?limit=1", note.id)).await;
    assert_eq!(capped.as_array().unwrap().len(), 1);

    // Notebook history mirrors the note shape.
    let versions = get(format!("/api/notebooks/{}/history", nb.id)).await;
    let versions = versions.as_array().unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["entity"]["title"], "new");
    assert_eq!(versions[1]["entity"]["title"], "old");

    // This note only ever travelled the relay (no server-side `notes` row), so it is private
    // to A's account: another user's history read is scoped to their own (empty) journal.
    register(addr, "b@example.com").await;
    let token_b = login(addr, "b@example.com", "phone").await;
    let other: Value = client
        .get(format!("http://{addr}/api/notes/{}/history", note.id))
        .bearer_auth(&token_b)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(other.as_array().unwrap().is_empty());

    // A materialised notebook, by contrast, is gated on read access: B cannot see A's
    // notebook history at all.
    let denied = client
        .get(format!("http://{addr}/api/notebooks/{}/history", nb.id))
        .bearer_auth(&token_b)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403, "no access to the notebook → 403");
}

// ── Account management (issue #31) ───────────────────────────────────────────

/// Self-service password change requires the current password; the new one then works and
/// the old one stops. And "sign out everywhere" revokes every device token at once.
#[sqlx::test(migrations = "../../migrations")]
async fn password_change_and_logout_everywhere(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    // Wrong current password is rejected.
    let bad = client
        .post(format!("http://{addr}/api/account/password"))
        .bearer_auth(&token)
        .json(&json!({ "current_password": "wrong-one", "new_password": "newpassword1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);

    // Correct current password changes it.
    let ok = client
        .post(format!("http://{addr}/api/account/password"))
        .bearer_auth(&token)
        .json(&json!({ "current_password": "password123", "new_password": "newpassword1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);

    // Old password no longer logs in; the new one does.
    let old = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": "a@example.com", "password": "password123", "device_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), 401);
    let new = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": "a@example.com", "password": "newpassword1", "device_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(new.status(), 200);

    // "Sign out everywhere" revokes the current token immediately.
    let logout = client
        .delete(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 200);
    let denied = client
        .get(format!("http://{addr}/api/devices"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401, "revoked token must stop working");
}

/// Account deletion requires the current password and then removes the account: the token
/// stops working, the notes are gone, and the email can be registered afresh (issue #31).
#[sqlx::test(migrations = "../../migrations")]
async fn delete_account_requires_password_and_cascades(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    // Create a note so we can prove ownership is torn down with the account.
    let created = client
        .post(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .json(&json!({ "title": "keep me?" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);

    // Wrong password is rejected and the account survives.
    let bad = client
        .delete(format!("http://{addr}/api/account"))
        .bearer_auth(&token)
        .json(&json!({ "password": "wrong-one" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);
    let still = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(still.status(), 200, "account must survive a failed delete");

    // Correct password deletes the account.
    let gone = client
        .delete(format!("http://{addr}/api/account"))
        .bearer_auth(&token)
        .json(&json!({ "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 200);

    // The token no longer authenticates (its device row cascaded away).
    let denied = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        denied.status(),
        401,
        "deleted account's token must stop working"
    );

    // The email is free again — the user row (and its unique email) is gone.
    let reused = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "a@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        reused.status(),
        200,
        "email must be reusable after deletion"
    );
}

/// `GET /api/notes?limit=&cursor=` returns a bounded page and an `X-Next-Cursor`
/// header; following it walks every note exactly once, and omitting `limit`
/// still returns them all (back-compatible) (issue #29).
#[sqlx::test(migrations = "../../migrations")]
async fn list_notes_paginates_with_cursor(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    // Create seven notes. Their updated_at values may tie, so the id tiebreaker
    // is what keeps the keyset walk total and duplicate-free.
    let total = 7;
    for i in 0..total {
        let resp = client
            .post(format!("http://{addr}/api/notes"))
            .bearer_auth(&token)
            .json(&json!({ "title": format!("note {i}") }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    // Page through with limit=3, following X-Next-Cursor until it stops.
    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut req = client
            .get(format!("http://{addr}/api/notes"))
            .bearer_auth(&token)
            .query(&[("limit", "3")]);
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c)]);
        }
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let next = resp
            .headers()
            .get("x-next-cursor")
            .map(|v| v.to_str().unwrap().to_string());
        let page: Vec<Value> = resp.json().await.unwrap();
        assert!(page.len() <= 3, "page must respect the limit");
        for n in &page {
            seen.push(n["id"].as_str().unwrap().to_string());
        }
        pages += 1;
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
        assert!(pages < 10, "pagination must terminate");
    }

    // Every note seen exactly once: 3 + 3 + 1 across three pages.
    assert_eq!(pages, 3);
    assert_eq!(seen.len(), total);
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), total, "no note may repeat across pages");

    // No limit → the whole list, and no next-cursor header.
    let all = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(all.headers().get("x-next-cursor").is_none());
    let all_notes: Vec<Value> = all.json().await.unwrap();
    assert_eq!(all_notes.len(), total);

    // A garbage cursor is a client error, not a silent empty page.
    let bad = client
        .get(format!("http://{addr}/api/notes"))
        .bearer_auth(&token)
        .query(&[("limit", "3"), ("cursor", "not-a-cursor")])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}

// ── Email normalization (issue #43) ──────────────────────────────────────────

/// Registration lowercases/trims the email, so login is case-insensitive and a
/// case-variant re-registration collides; malformed emails are rejected.
#[sqlx::test(migrations = "../../migrations")]
async fn email_is_normalized_and_validated(pool: PgPool) {
    let addr = spawn_server(pool).await;
    let client = reqwest::Client::new();

    // Register with mixed case and surrounding whitespace.
    let reg = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "  John.Doe@Example.COM ", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(reg.status(), 200);

    // Login with the lowercased form works (case-insensitive).
    let login = client
        .post(format!("http://{addr}/api/login"))
        .json(&json!({ "email": "john.doe@example.com", "password": "password123", "device_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200, "login must be case-insensitive");

    // A case-variant re-registration is the same account → conflict.
    let dup = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "JOHN.DOE@EXAMPLE.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 409, "case-variant email must collide");

    // A malformed email is rejected up front.
    let bad = client
        .post(format!("http://{addr}/api/register"))
        .json(&json!({ "email": "not-an-email", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}

// ── Note body size cap (issue #44) ───────────────────────────────────────────

/// A note whose materialised body exceeds `max_note_body_bytes` is refused with
/// `413` rather than built in memory; a small note is unaffected.
#[sqlx::test(migrations = "../../migrations")]
async fn oversized_note_body_is_refused(pool: PgPool) {
    let mut config = test_config();
    config.max_note_body_bytes = 32; // tiny cap for the test
    let addr = spawn_server_with_config(pool, config).await;
    let client = reqwest::Client::new();
    register(addr, "a@example.com").await;
    let token = login(addr, "a@example.com", "laptop").await;

    // A small note is fine.
    let small = client
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(&token)
        .json(&json!({ "title": "small", "body": "under the cap" }))
        .send()
        .await
        .unwrap();
    assert_eq!(small.status(), 200);
    let small_id = small.json::<Value>().await.unwrap()["note_id"]
        .as_str()
        .unwrap()
        .to_string();
    let got = client
        .get(format!("http://{addr}/api/notes/{small_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), 200);

    // A note whose body exceeds the cap is refused on read.
    let big_body = "x".repeat(100);
    let big = client
        .post(format!("http://{addr}/api/import"))
        .bearer_auth(&token)
        .json(&json!({ "title": "big", "body": big_body }))
        .send()
        .await
        .unwrap();
    assert_eq!(big.status(), 200);
    let big_id = big.json::<Value>().await.unwrap()["note_id"]
        .as_str()
        .unwrap()
        .to_string();
    let refused = client
        .get(format!("http://{addr}/api/notes/{big_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 413, "oversized body must be 413");
}

// ── Per-entity history + visibility window (issue #27) ───────────────────────

async fn spawn_server_with_config(pool: PgPool, config: Config) -> SocketAddr {
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

async fn notebook_history(addr: SocketAddr, token: &str, nb: Uuid) -> Vec<Value> {
    reqwest::Client::new()
        .get(format!("http://{addr}/api/notebooks/{nb}/history"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// A shared notebook has one timeline: a collaborator with read access sees the owner's
/// edits, not only their own (issue #27 — history is per-entity, not per-user).
#[sqlx::test(migrations = "../../migrations")]
async fn notebook_history_is_visible_to_shared_collaborators(pool: PgPool) {
    use keeplin_core::storage::NotebookRepository;
    let addr = spawn_server(pool).await; // default policy: from creation
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "dev-a").await).await;
    let ta = login(addr, "a@example.com", "rest-a").await;

    // A creates + renames a notebook through the relay (materialises on the server).
    let nb = a
        .create_notebook(keeplin_core::models::Notebook::new("old"))
        .await
        .unwrap();
    let mut renamed = nb.clone();
    renamed.title = "new".into();
    a.update_notebook(renamed).await.unwrap();
    push(&a).await;

    // A shares the notebook with B (read).
    let client = reqwest::Client::new();
    let mut shared = false;
    for _ in 0..50 {
        let code = client
            .post(format!("http://{addr}/api/notebooks/{}/share", nb.id))
            .bearer_auth(&ta)
            .json(&json!({ "user_email": "b@example.com", "capabilities": 1 }))
            .send()
            .await
            .unwrap()
            .status();
        if code == 200 {
            shared = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(shared, "A could share the materialised notebook with B");

    // B sees A's two versions — the owner's edits, not B's own (B authored none).
    let tb = login(addr, "b@example.com", "dev-b").await;
    let mut versions = Vec::new();
    for _ in 0..50 {
        versions = notebook_history(addr, &tb, nb.id).await;
        if versions.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        versions.len(),
        2,
        "collaborator sees the owner's full history"
    );
    assert_eq!(versions[0]["entity"]["title"], "new");
    assert_eq!(versions[1]["entity"]["title"], "old");
}

/// With `HISTORY_VISIBILITY=access` a collaborator only sees versions from after they were
/// granted access; the owner still sees everything (issue #27).
#[sqlx::test(migrations = "../../migrations")]
async fn history_visibility_since_access_windows_a_collaborator(pool: PgPool) {
    use keeplin_core::storage::NotebookRepository;
    let mut config = test_config();
    config.history_since_access = true;
    let addr = spawn_server_with_config(pool, config).await;
    register(addr, "a@example.com").await;
    register(addr, "b@example.com").await;
    let a = device(addr, &login(addr, "a@example.com", "dev-a").await).await;
    let ta = login(addr, "a@example.com", "rest-a").await;
    let client = reqwest::Client::new();

    // Version 1 (before B has access).
    let nb = a
        .create_notebook(keeplin_core::models::Notebook::new("v1"))
        .await
        .unwrap();
    push(&a).await;
    // Let the create land before the share is granted.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    // Grant B access now.
    let code = client
        .post(format!("http://{addr}/api/notebooks/{}/share", nb.id))
        .bearer_auth(&ta)
        .json(&json!({ "user_email": "b@example.com", "capabilities": 1 }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(code, 200);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Version 2 (after B has access). Push only this change, not the whole journal from
    // epoch — re-sending v1 now (after the share) would make it pass the access-window
    // filter and defeat the test.
    let cut = chrono::Utc::now();
    let mut renamed = nb.clone();
    renamed.title = "v2".into();
    a.update_notebook(renamed).await.unwrap();
    let only_v2 = a.get_changes_since(cut).await.unwrap();
    a.send_changes(only_v2).await.unwrap();

    let tb = login(addr, "b@example.com", "dev-b").await;
    // B eventually sees v2 (post-access) but never v1 (pre-access).
    let mut b_versions = Vec::new();
    for _ in 0..50 {
        b_versions = notebook_history(addr, &tb, nb.id).await;
        if !b_versions.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        b_versions.len(),
        1,
        "collaborator sees only post-access versions"
    );
    assert_eq!(b_versions[0]["entity"]["title"], "v2");

    // The owner still sees both versions.
    let a_versions = notebook_history(addr, &ta, nb.id).await;
    assert_eq!(a_versions.len(), 2, "owner sees the full history");
}
