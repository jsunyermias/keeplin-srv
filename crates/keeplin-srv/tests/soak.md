# `tests/soak.rs` — multi-instance collaborative soak/load drill

Self-contained companion for `crates/keeplin-srv/tests/soak.rs`. It documents **every
code block of the source file, in source order** — a reader with only this file must be
able to understand the drill without opening anything else, so project-wide conventions
are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports and the `Ws` type alias. Marker
`// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
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
```

**What it does** — The multi-instance collaborative **soak/load drill**
(production-readiness item: prove the issue #45 cross-instance path under real
concurrency, not just the happy path). **`#[ignore]`d** — a load drill, not a unit of
CI; run explicitly:

```bash
DATABASE_URL=postgres://… cargo test --release --test soak -- --ignored --nocapture
# knobs: SOAK_EDITORS (default 8), SOAK_OPS (default 25 per editor)
```

Scenario: two bus-enabled server instances (Postgres LISTEN/NOTIFY) share one
database; `SOAK_EDITORS` editors — each with its own device/login, half per
instance — join one shared note and concurrently insert `SOAK_OPS` lines each over
raw `/api/ws` WebSockets. **Phase 1** asserts both instances settle on a
byte-identical exported body (and reports throughput/convergence time). **Phase 2**
kills instance B mid-session; the editors on A keep writing and everything must
still converge on A — the "kill a replica mid-edit" drill.

**Dependencies** — `tokio_tungstenite` (raw WS clients), `futures_util`,
`keeplin_srv` (`Config`, `router`, `AppState`, `bus::spawn`), `reqwest`, `sqlx`,
`serde_json`, `uuid`, `chrono`.

**Used by** — operators/CI on demand (`--ignored`); `RUNBOOK.md` ("Load / soak
drill").

**Repeated context** — Multi-instance model (issue #45), restated: instances share
only PostgreSQL; collab ops fan out to sibling subscribers via the `collab_events`
outbox + `collab_op` NOTIFY, order writes serialise on the per-note advisory lock,
and a connection only ever talks to one instance. This drill drives the **raw wire
protocol** for maximal op pressure — client behaviour is covered by the
`collab_client_*_e2e` binaries.

---

## fn test_config

**Identification** — helper; marker `// md:fn test_config`. Standard test `Config`
(10-connection pool — two instances share the drill's load).

**Code** — complete and verbatim:

```rust
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
```

**Dependencies** — `Config`. **Used by** — `spawn_instance`.
**Repeated context** — none.

---

## fn spawn_instance

**Identification** — helper; marker `// md:fn spawn_instance`.
`async fn spawn_instance(pool) -> (SocketAddr, JoinHandle<()>)`.

**Code** — complete and verbatim:

```rust
// md:fn spawn_instance
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
```

**What it does** — Spawns a **bus-enabled** instance (`AppState::new` +
`bus::spawn(state)` + router on an ephemeral port). Returns the serve task's
`JoinHandle` so phase 2 can kill the instance (`abort`).

**Dependencies** — `AppState::new`, `bus::spawn`, `router`. **Used by** — the
drill.

**Repeated context** — Each `AppState::new` mints a fresh `instance_id`, so the two
instances correctly skip their own bus echoes.

---

## fn env_or

**Identification** — helper; marker `// md:fn env_or`. Parse a numeric env knob
with a default (`SOAK_EDITORS`, `SOAK_OPS`).

**Code** — complete and verbatim:

```rust
// md:fn env_or
fn env_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
```

**Dependencies** — `std::env`. **Used by** — the drill. **Repeated context** —
none.

---

## fn ws_connect

**Identification** — helper; marker `// md:fn ws_connect`. Opens a raw
`tokio-tungstenite` WebSocket to `ws://…/api/ws?token=…` (the query-string token
fallback).

**Code** — complete and verbatim:

```rust
// md:fn ws_connect
async fn ws_connect(addr: SocketAddr, token: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={token}"))
        .await
        .unwrap();
    ws
}
```

**Dependencies** — `tokio_tungstenite`. **Used by** — `editor`.
**Repeated context** — none.

---

## fn export_body

**Identification** — helper; marker `// md:fn export_body`. Authenticated
`GET /api/notes/:id/export`, returning the materialised body string.

**Code** — complete and verbatim:

```rust
// md:fn export_body
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
```

**Dependencies** — `reqwest`. **Used by** — `wait_quiescent_identical`.
**Repeated context** — the export is the derived line-model join — the
convergence observable.

---

## fn merge_vv

**Identification** — helper; marker `// md:fn merge_vv`. Pointwise-max merge of an
op/order version vector (as JSON) into the editor's causal view — the same merge
rule the server and client use.

**Code** — complete and verbatim:

```rust
// md:fn merge_vv
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
```

**Dependencies** — `serde_json`. **Used by** — `editor`.
**Repeated context** — version vectors: per-device counters; merging = pointwise
max = absorbing another actor's history.

---

## fn editor

**Identification** — helper task; marker `// md:fn editor`.

**Code** — complete and verbatim:

```rust
// md:fn editor
async fn editor(addr: SocketAddr, token: String, device_id: String, note_id: String, ops: usize) {
    let mut ws = ws_connect(addr, &token).await;
    ws.send(Message::Text(
        json!({ "type": "Join", "note_id": note_id }).to_string(),
    ))
    .await
    .unwrap();
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
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        while ws.next().await.is_some() {}
    })
    .await;
}
```

**What it does** — One simulated editor: connect, `Join` the note, and **wait for
the `Welcome`** (seeding the causal view from the snapshot's order vv) so ops
cannot race the subscription. Then insert `ops` lines **at the head**
(`after_line_id: null` — order-contended on purpose), signing each op *causally*
the way the real client does: the sent vv is everything this editor has seen
(Welcome + absorbed broadcasts, merged via `merge_vv`) plus its own bumped
component. Between sends it non-blockingly drains incoming `Op` broadcasts into
the causal view; at the end it drains briefly so the server can flush. A
causally-stale insert is dropped by design; a causal one must be applied.

**Dependencies** — `ws_connect`, `merge_vv`; raw protocol frames.
**Used by** — both phases of the drill.

**Repeated context** — Device-as-actor: each editor logs in as its own device and
signs `last_writer` with that device id — forged or shared writers would be
rejected (`bad_writer`).

---

## fn wait_quiescent_identical

**Identification** — helper; marker `// md:fn wait_quiescent_identical`.

**Code** — complete and verbatim:

```rust
// md:fn wait_quiescent_identical
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
```

**What it does** — Polls the exports until **every** instance returns the
byte-identical body **twice in a row** (quiescent *and* cross-instance
consistent — the issue #45 guarantee), returning the settle time and line count;
errs after the budget. Under head-of-note contention the server legitimately drops
causally-concurrent-and-older inserts (design §5) — the real client re-diffs and
self-heals — so the drill asserts *consistency* and reports the applied/sent ratio
as a metric, never failing on drops.

**Dependencies** — `export_body`. **Used by** — both phases.

**Repeated context** — "Identical twice in a row" distinguishes convergence from a
coincidentally-equal snapshot mid-churn.

---

## fn soak_two_instances_under_concurrent_editors

**Identification** — `#[sqlx::test]` + `#[ignore]`; marker
`// md:fn soak_two_instances_under_concurrent_editors`.

**Code** — complete and verbatim:

```rust
// md:fn soak_two_instances_under_concurrent_editors
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "load test — run explicitly with --ignored --nocapture"]
async fn soak_two_instances_under_concurrent_editors(pool: PgPool) {
    let editors = env_or("SOAK_EDITORS", 8);
    let ops_per_editor = env_or("SOAK_OPS", 25);

    let (addr_a, _handle_a) = spawn_instance(pool.clone()).await;
    let (addr_b, handle_b) = spawn_instance(pool.clone()).await;
    println!("soak: instance A={addr_a}  B={addr_b}");

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
```

**What it does** — The drill: spawn instances A and B; register the owner, create
the shared note on A; **phase 1** — `SOAK_EDITORS` editors (own login each,
alternating instances) run concurrently, then both instances must settle
identically within 120 s (`applied > 0`); throughput, settle time and the
applied/sent ratio are printed. **Phase 2** — `handle_b.abort()` kills B
mid-session; `max(editors/2, 1)` survivor editors keep writing on A, which must
settle again with `applied2 > applied` — the cluster stays writable after a
replica death. Prints `SOAK: PASS` on success.

**Dependencies** — every helper above. **Used by** — explicit `--ignored` runs.

**Repeated context** — What failure would mean: phase 1 divergence = a lost update
or missed cross-instance delivery (advisory lock/outbox bug); phase 2 stall = the
surviving instance depended on its dead sibling (bus liveness bug).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `test_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `spawn_instance()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `env_or()` — defined here (EXTRACTED; file-local)
- `ws_connect()` — defined here (EXTRACTED; file-local)
- `export_body()` — defined here (EXTRACTED; file-local)
- `merge_vv()` — defined here (EXTRACTED; file-local)
- `editor()` — defined here (EXTRACTED; file-local)
- `wait_quiescent_identical()` — defined here (EXTRACTED; file-local)
- `soak_two_instances_under_concurrent_editors()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×1; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports + `type Ws` | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_instance` | `// md:fn spawn_instance` |
| 4 | `fn env_or` | `// md:fn env_or` |
| 5 | `fn ws_connect` | `// md:fn ws_connect` |
| 6 | `fn export_body` | `// md:fn export_body` |
| 7 | `fn merge_vv` | `// md:fn merge_vv` |
| 8 | `fn editor` | `// md:fn editor` |
| 9 | `fn wait_quiescent_identical` | `// md:fn wait_quiescent_identical` |
| 10 | `fn soak_two_instances_under_concurrent_editors` | `// md:fn soak_two_instances_under_concurrent_editors` |
