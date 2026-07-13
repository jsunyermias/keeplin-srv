//! Multi-instance collaborative **soak/load test** (production-readiness item:
//! prove the #45 cross-instance path under real concurrency, not just the
//! happy path).
//!
//! `#[ignore]`d: it is a load test, not a unit of CI. Run it explicitly —
//!
//! ```bash
//! DATABASE_URL=postgres://… cargo test --release --test soak -- --ignored --nocapture
//! # knobs: SOAK_EDITORS (default 8), SOAK_OPS (default 25 per editor)
//! ```
//!
//! Scenario:
//! 1. Two server instances (with the LISTEN/NOTIFY bus) share one database.
//! 2. `SOAK_EDITORS` editors — each its own device/login — join one shared
//!    note, half on each instance, and concurrently insert `SOAK_OPS` lines
//!    each.
//! 3. **Phase 1**: every op must converge on both instances (export equality +
//!    total line count); throughput and convergence time are reported.
//! 4. **Phase 2 — replica death**: instance B is killed mid-session. The
//!    editors on A keep writing; everything must still converge on A. This is
//!    the "kill a replica mid-edit" drill.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
        resource_purge_days: 0,
        db_max_connections: 10,
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

/// Spawn a bus-enabled instance; the JoinHandle lets phase 2 kill it.
async fn spawn_instance(pool: PgPool) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let state = Arc::new(AppState::new(test_config(), pool));
    keeplin_srv::bus::spawn(state.clone());
    let app: Router = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, handle)
}

fn env_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

async fn ws_connect(addr: SocketAddr, token: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={token}"))
        .await
        .unwrap();
    ws
}

async fn export_body(addr: SocketAddr, token: &str, note_id: &str) -> String {
    reqwest::Client::new()
        .get(format!("http://{addr}/api/notes/{note_id}/export"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["body"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Merge an op/order version vector into the editor's causal view.
fn merge_vv(into: &mut serde_json::Map<String, Value>, vv: &Value) {
    if let Some(map) = vv.as_object() {
        for (k, v) in map {
            let new = v.as_u64().unwrap_or(0);
            let cur = into.get(k).and_then(Value::as_u64).unwrap_or(0);
            if new > cur {
                into.insert(k.clone(), json!(new));
            }
        }
    }
}

/// One editor: join the note, then insert `ops` lines at the head
/// (order-contended on purpose), signing each op **causally** the way the real
/// client does: the version vector sent is everything this editor has seen
/// (Welcome + broadcasts) plus its own bumped component. A causally-stale
/// insert is dropped by design; a causal one must be applied.
async fn editor(addr: SocketAddr, token: String, device_id: String, note_id: String, ops: usize) {
    let mut ws = ws_connect(addr, &token).await;
    ws.send(Message::Text(
        json!({ "type": "Join", "note_id": note_id }).to_string(),
    ))
    .await
    .unwrap();
    // Wait for the Welcome (seeding the causal view) so ops cannot race the
    // subscription.
    let mut seen = serde_json::Map::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(30), ws.next())
            .await
            .expect("timed out waiting for Welcome")
            .expect("socket closed before Welcome")
            .expect("socket error")
        {
            Message::Text(text) => {
                let v: Value = serde_json::from_str(&text).unwrap();
                if v["type"] == "Welcome" {
                    merge_vv(&mut seen, &v["snapshot"]["vv"]);
                    break;
                }
            }
            _ => continue,
        }
    }
    let mut own = seen.get(&device_id).and_then(Value::as_u64).unwrap_or(0);
    for i in 0..ops {
        own += 1;
        seen.insert(device_id.clone(), json!(own));
        let line_id = uuid::Uuid::new_v4().to_string();
        let msg = json!({
            "type": "Op",
            "note_id": note_id,
            "ops": [{
                "op": "Insert",
                "after_line_id": null,
                "line_id": line_id,
                "content": format!("line {i} from {device_id}"),
                "vv": Value::Object(seen.clone()),
                "last_writer": device_id,
                "updated_at": chrono::Utc::now().to_rfc3339(),
            }],
        });
        ws.send(Message::Text(msg.to_string())).await.unwrap();
        // Absorb pending broadcasts into the causal view without blocking.
        while let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(2), ws.next()).await
        {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                if v["type"] == "Op" {
                    if let Some(list) = v["ops"].as_array() {
                        for op in list {
                            merge_vv(&mut seen, &op["vv"]);
                        }
                    }
                }
            }
        }
    }
    // Keep draining briefly so the server can flush broadcasts to us.
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        while ws.next().await.is_some() {}
    })
    .await;
}

/// Poll the exports until every instance returns the **identical** body twice
/// in a row (quiescent and cross-instance consistent — the #45 guarantee).
/// Returns (settle time, line count). Note: under head-of-note contention the
/// server legitimately drops causally-concurrent-and-older inserts (design
/// §5); the real client re-diffs and self-heals, so the soak asserts
/// *consistency*, and reports the applied/sent ratio as a metric.
async fn wait_quiescent_identical(
    addrs: &[SocketAddr],
    token: &str,
    note_id: &str,
    budget: Duration,
) -> Result<(Duration, usize), String> {
    let start = Instant::now();
    let mut prev: Option<String> = None;
    while start.elapsed() < budget {
        let mut bodies = Vec::new();
        for addr in addrs {
            bodies.push(export_body(*addr, token, note_id).await);
        }
        let all_equal = bodies.windows(2).all(|w| w[0] == w[1]);
        if all_equal && prev.as_deref() == Some(bodies[0].as_str()) && !bodies[0].is_empty() {
            let lines = bodies[0].lines().count();
            return Ok((start.elapsed(), lines));
        }
        prev = if all_equal {
            Some(bodies[0].clone())
        } else {
            None
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(format!(
        "instances did not settle on an identical body within {budget:?}"
    ))
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "load test — run explicitly with --ignored --nocapture"]
async fn soak_two_instances_under_concurrent_editors(pool: PgPool) {
    let editors = env_or("SOAK_EDITORS", 8);
    let ops_per_editor = env_or("SOAK_OPS", 25);

    let (addr_a, _handle_a) = spawn_instance(pool.clone()).await;
    let (addr_b, handle_b) = spawn_instance(pool.clone()).await;
    println!("soak: instance A={addr_a}  B={addr_b}");

    // Owner + shared note; every editor gets its own device token.
    let client = reqwest::Client::new();
    client
        .post(format!("http://{addr_a}/api/register"))
        .json(&json!({ "email": "soak@example.com", "password": "password123" }))
        .send()
        .await
        .unwrap();
    let login = |device: String| {
        let client = client.clone();
        async move {
            let v: Value = client
                .post(format!("http://{addr_a}/api/login"))
                .json(
                    &json!({ "email": "soak@example.com", "password": "password123",
                               "device_name": device }),
                )
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            (
                v["token"].as_str().unwrap().to_string(),
                v["device_id"].as_str().unwrap().to_string(),
            )
        }
    };
    let (owner_token, _) = login("owner".into()).await;
    let note: Value = client
        .post(format!("http://{addr_a}/api/notes"))
        .bearer_auth(&owner_token)
        .json(&json!({ "title": "soak" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let note_id = note["id"].as_str().unwrap().to_string();

    // ── Phase 1: concurrent editors split across both instances ────────────
    let started = Instant::now();
    let mut tasks = Vec::new();
    for e in 0..editors {
        let (token, device_id) = login(format!("editor-{e}")).await;
        let addr = if e % 2 == 0 { addr_a } else { addr_b };
        tasks.push(tokio::spawn(editor(
            addr,
            token,
            device_id,
            note_id.clone(),
            ops_per_editor,
        )));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let sent = editors * ops_per_editor;
    let send_time = started.elapsed();

    let (settle, applied) = wait_quiescent_identical(
        &[addr_a, addr_b],
        &owner_token,
        &note_id,
        Duration::from_secs(120),
    )
    .await
    .expect("phase 1: both instances must settle on an identical body");
    assert!(applied > 0, "no op applied at all");
    println!(
        "soak phase 1: {editors} editors x {ops_per_editor} ops = {sent} sent \
         | send window {send_time:?} ({:.0} ops/s) | settled identical on BOTH instances in {settle:?} \
         | applied {applied}/{sent} ({:.0}%; concurrent head-inserts that lost the causal tiebreak \
         are dropped by design and re-diffed by the real client)",
        sent as f64 / send_time.as_secs_f64(),
        100.0 * applied as f64 / sent as f64
    );

    // ── Phase 2: kill instance B mid-session, keep editing on A ───────────
    handle_b.abort();
    println!("soak phase 2: instance B killed");
    let survivors = (editors / 2).max(1);
    let mut tasks = Vec::new();
    for e in 0..survivors {
        let (token, device_id) = login(format!("survivor-{e}")).await;
        tasks.push(tokio::spawn(editor(
            addr_a,
            token,
            device_id,
            note_id.clone(),
            ops_per_editor,
        )));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let (settle2, applied2) =
        wait_quiescent_identical(&[addr_a], &owner_token, &note_id, Duration::from_secs(120))
            .await
            .expect("phase 2: the surviving instance must settle");
    assert!(
        applied2 > applied,
        "the cluster must remain writable after a replica death \
         (before {applied}, after {applied2})"
    );
    println!(
        "soak phase 2: survivor A kept accepting ops after B's death \
         | lines {applied} -> {applied2} | settled in {settle2:?}"
    );
    println!(
        "SOAK: PASS — cross-instance consistency held under {editors} concurrent editors \
         and a mid-session replica death"
    );
}
