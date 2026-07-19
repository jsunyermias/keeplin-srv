# `collab.rs` — the collaborative session engine

Self-contained companion for `crates/keeplin-srv/src/collab.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able to
understand `collab.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `collab.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::HeaderMap,
    response::Response,
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use keeplin_core::storage::note_log::{resolve, VersionVector, Winner};
use tokio::sync::{mpsc, Mutex, RwLock};
use uuid::Uuid;

use crate::{
    auth,
    error::AppError,
    permissions::resolve_note_access,
    protocol::{
        CollabClientMsg, CollabServerMsg, Cursor, LineOp, LineSnapshot, NoteLinesSnapshot,
        PresenceInfo,
    },
    state::AppState,
    store::Line,
};
```

**What it does** — The collaborative-editing engine behind `GET /api/ws` (design §7):
per-note live sessions, presence, and the application of `LineOp`s against the
versioned line + order entities. The server is the **broker and durable source of
truth** (design §2.4): it validates each operation, resolves it against current state
with keeplin-core's `note_log::resolve`, persists it, and fans the applied ops out to
the note's other subscribers with a monotonically increasing `server_seq`. Clients are
stateful: they keep their own copy and rebuild from the `Welcome` snapshot on
(re)connect — **there is no infinite op log**.

Conflict rules (design §5): per **line**, `resolve(local, incoming)` — the op is
applied iff the incoming write wins (causally newer by version vector, or concurrent
and winning the deterministic `(timestamp, writer)` tiebreak); a dominated op is
silently ignored. Per **order** (`Insert`/`Move`), the same resolution against the
note's order entity; an applied op merges its vector into the order's. **No locks
anywhere for conflict resolution** — the in-process and advisory locks below serialise
*application*, never decide *winners*.

**Dependencies** — external: `axum` (WS upgrade, extractors), `tokio` (`mpsc`,
`Mutex`, `RwLock`, atomics, time), `futures_util` (socket split),
`keeplin_core::storage::note_log::{resolve, VersionVector, Winner}` (client repo, the
shared resolution function), `serde_json`, `chrono`, `uuid`, `tracing`. Internal:
`auth::verify_token` (`auth.rs`), `AppError` (`error.rs`), `resolve_note_access`
(`permissions.rs`), the wire types (`protocol.rs`), `AppState` (`state.rs`), and the
line/order/presence/outbox methods of `store.rs`.

**Used by** — `http.rs` routes `GET /api/ws` to `handler`; `state.rs` holds the
`CollabRegistry`; `bus.rs` calls `deliver_event` / `deliver_presence`;
`http.rs::metrics` reads `CollabRegistry::stats`. Exercised by `tests/collab.rs`
(raw-JSON protocol tests), the three `collab_client_*_e2e` tests (real keeplin-core
client), and `tests/soak.rs` (multi-instance drill).

**Repeated context** — The device-as-actor rule: ops are signed with the JWT's
**device id** (`last_writer` + vv components); presence is **user**-scoped. Snapshot
rebuild is the universal recovery path (lag, drop, reconnect, missed bus event). All
durable state is PostgreSQL rows (lines, `note_line_order`, presence table,
`collab_events` outbox); everything in this module's memory is per-instance and
rebuildable — which is what makes the multi-replica model (issue #45) sound.

---

## Constants

**Identification** — logical section: the six tuning constants; marker
`// md:Constants`.

**Code** — complete and verbatim:

```rust
// md:Constants
const MAX_LINE_LEN: usize = 10_000;
const MAX_LINES_PER_NOTE: usize = 100_000;
const MAX_WS_MESSAGE: usize = 1024 * 1024;

const OUTBOUND_CAPACITY: usize = 256;
const PING_INTERVAL: Duration = Duration::from_secs(30);
const ACTIVITY_TIMEOUT: Duration = Duration::from_secs(90);
```

**What it does** — The design limits (§11.1): max line length (10 k chars), max lines
per note (100 k), max incoming WS message (1 MiB). Plus the connection hygiene knobs:
`OUTBOUND_CAPACITY` — bounded outbound queue per connection; a slow/stalled consumer
is **dropped** rather than buffering without limit (issue #34), safe because a
stateful client rebuilds from the next `Welcome` snapshot. `PING_INTERVAL` — periodic
pings keep NAT/proxy paths open and surface a dead peer via a failed write
(issue #35). `ACTIVITY_TIMEOUT` — if no frame at all (not even a pong) arrives within
this window, the peer is treated as dead and the connection closed (issue #35).

**Dependencies** — none.

**Used by** — `apply_op` (limits), `handler` (`MAX_WS_MESSAGE`), `run_connection`
(the three connection knobs).

**Repeated context** — Limits are enforced **before persisting** (in `apply_op`), so
the database can never hold an over-limit line/order. 10 k × 100 k bounds a note's
theoretical materialised body at ~1 GB, which is why the REST read path has its own
`MAX_NOTE_BODY_BYTES` cap (issue #44, `config.rs`).

---

## Subscriber

**Identification** — private struct; marker `// md:Subscriber`.

**Code** — complete and verbatim:

```rust
// md:Subscriber
struct Subscriber {
    tx: mpsc::Sender<String>,
}
```

**What it does** — One live connection's entry in a session: the sending half of its
bounded outbound channel. Everything else about the connection (user, device, joined
notes) lives on the connection task's stack; presence lives in the shared table.

**Dependencies** — `tokio::mpsc`.

**Used by** — `CollabSession.subscribers`; inserted by `handle_msg` (`Join`), removed
on `Leave`/disconnect/slow-consumer drop.

**Repeated context** — Frames are pre-serialised `String`s so one serialisation
serves every subscriber (see `broadcast`).

---

## CollabSession

**Identification** — public struct; marker `// md:CollabSession`.

**Code** — complete and verbatim:

```rust
// md:CollabSession
pub struct CollabSession {
    seq: AtomicU64,
    apply_lock: Mutex<()>,
    subscribers: RwLock<HashMap<u64, Subscriber>>,
}
```

**What it does** — One live collaborative session per note with at least one
subscriber (design §3.5), created on demand and destroyed when the last subscriber
leaves. Fields: `seq` — the monotonic per-session sequence stamped on each fanned-out
`Op` by *this* instance (a connection only ever talks to one instance, so a
per-instance counter is enough for the client's gap detection — issue #45);
`apply_lock` — serialises op application and join snapshots for this note, so a
joiner can never miss an op between reading the snapshot and subscribing, and two op
batches never interleave their read-modify-write; `subscribers` — the live
connections keyed by `conn_id`. If the server restarts, clients reconnect and get a
fresh snapshot from the database — the session holds **no durable state**.

**Dependencies** — `Subscriber` (this file), tokio sync primitives.

**Used by** — `CollabRegistry.sessions`; `handle_msg`, `deliver_event`,
`deliver_presence`, `run_connection` (this file).

**Repeated context** — The apply lock is in-process only; cross-instance
serialisation of order writes is the Postgres advisory lock
(`store::lock_note_order`) taken inside the `Op` path. Winners are still decided by
version-vector resolution — the locks only serialise application.

---

## CollabRegistry

**Identification** — public struct; marker `// md:CollabRegistry`.

**Code** — complete and verbatim:

```rust
// md:CollabRegistry
#[derive(Default)]
pub struct CollabRegistry {
    sessions: RwLock<HashMap<Uuid, Arc<CollabSession>>>,
    next_conn_id: AtomicU64,
}
```

**What it does** — All live sessions on this instance, keyed by note id, plus the
connection-id allocator (`next_conn_id`, unique per instance). Lives in `AppState`
(`::default()` at boot — empty).

**Dependencies** — `CollabSession` (this file).

**Used by** — `state.rs::AppState.collab`; `bus.rs` (via `get`),
`http.rs::metrics` (via `stats`), this file's connection handling.

**Repeated context** — Per-instance, rebuildable memory; the shared truth for
"who is present" is the presence *table*, not this map.

---

## impl CollabRegistry

**Identification** — inherent impl block; marker `// md:impl CollabRegistry`.
Contains `stats`, `get`, `get_or_create`, `drop_if_empty` (next sections).

**Code** — container: members documented as sub-blocks below: fn stats, fn get, fn get_or_create, fn drop_if_empty.

**What it does** — Session lookup/lifecycle: metrics, bus lookup, on-demand
creation, lazy destruction.

**Dependencies / Used by / Repeated context** — see the method subsections.

### fn stats

**Identification** — public async method; marker
`// md:impl CollabRegistry > fn stats`.
`pub async fn stats(&self) -> (usize, usize)`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabRegistry > fn stats
    pub async fn stats(&self) -> (usize, usize) {
        let sessions = self.sessions.read().await;
        let mut connections = 0;
        for session in sessions.values() {
            connections += session.subscribers.read().await.len();
        }
        (sessions.len(), connections)
    }
```

**What it does** — `(live note sessions, live subscriber connections)` for
`GET /api/metrics`.

**Dependencies** — none. **Used by** — `http.rs::metrics`.

**Repeated context** — none.

### fn get

**Identification** — public async method; marker
`// md:impl CollabRegistry > fn get`.
`pub async fn get(&self, note_id: Uuid) -> Option<Arc<CollabSession>>`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabRegistry > fn get
    pub async fn get(&self, note_id: Uuid) -> Option<Arc<CollabSession>> {
        self.sessions.read().await.get(&note_id).cloned()
    }
```

**What it does** — The live session for a note on *this* instance, if any. The bus
entrypoints use it to deliver a sibling's op/presence to local subscribers
(issue #45); no session → nothing to deliver → no-op.

**Dependencies** — none. **Used by** — `deliver_event`, `deliver_presence` (this
file).

**Repeated context** — none.

### fn get_or_create

**Identification** — private async method; marker
`// md:impl CollabRegistry > fn get_or_create`.
`async fn get_or_create(&self, note_id: Uuid) -> Arc<CollabSession>`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabRegistry > fn get_or_create
    async fn get_or_create(&self, note_id: Uuid) -> Arc<CollabSession> {
        let mut sessions = self.sessions.write().await;
        sessions
            .entry(note_id)
            .or_insert_with(|| {
                Arc::new(CollabSession {
                    seq: AtomicU64::new(0),
                    apply_lock: Mutex::new(()),
                    subscribers: RwLock::new(HashMap::new()),
                })
            })
            .clone()
    }
```

**What it does** — The session for a note, created (fresh `seq = 0`, empty
subscribers) if absent, under the map's write lock.

**Dependencies** — `CollabSession` (this file). **Used by** — `handle_msg`
(`Join`).

**Repeated context** — `seq` restarting at 0 for a fresh session is fine: clients
use `server_seq` only for gap detection within one connection's stream.

### fn drop_if_empty

**Identification** — private async method; marker
`// md:impl CollabRegistry > fn drop_if_empty`.
`async fn drop_if_empty(&self, note_id: Uuid)`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabRegistry > fn drop_if_empty
    async fn drop_if_empty(&self, note_id: Uuid) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get(&note_id) {
            if session.subscribers.read().await.is_empty() {
                sessions.remove(&note_id);
            }
        }
    }
```

**What it does** — Removes the note's session if it has no subscribers left. Called
after `Leave` and on disconnect — lazy cleanup keeps the map bounded by live notes.

**Dependencies** — none. **Used by** — `handle_msg` (`Leave`), `run_connection`
(teardown).

**Repeated context** — none.

---

## impl CollabSession

**Identification** — inherent impl block; marker `// md:impl CollabSession`.
Contains `broadcast` (next section).

**Code** — container: members documented as sub-blocks below: fn broadcast.

**What it does / Dependencies / Used by / Repeated context** — see `fn broadcast`.

### fn broadcast

**Identification** — private async method; marker
`// md:impl CollabSession > fn broadcast`.

**Code** — complete and verbatim:

```rust
    // md:impl CollabSession > fn broadcast
    async fn broadcast(&self, msg: &CollabServerMsg, skip_conn: Option<u64>) {
        let text = serde_json::to_string(msg).expect("serializable server msg");
        let mut slow = Vec::new();
        {
            let subscribers = self.subscribers.read().await;
            for (conn_id, sub) in subscribers.iter() {
                if Some(*conn_id) == skip_conn {
                    continue;
                }
                if sub.tx.try_send(text.clone()).is_err() {
                    slow.push(*conn_id);
                }
            }
        }
        if !slow.is_empty() {
            let mut subscribers = self.subscribers.write().await;
            for conn_id in slow {
                subscribers.remove(&conn_id);
            }
        }
    }
```

**What it does** — Sends `msg` to every subscriber, optionally skipping one
connection (the originator of an op already has it applied locally). Serialises the
message **once**, then `try_send`s to each subscriber's bounded queue. A subscriber
whose queue is full (slow/stalled consumer) is collected and **removed from the
session** after the read pass (issue #34) — it reconnects and rebuilds from a fresh
snapshot rather than buffering without bound.

**Dependencies** — `Subscriber`, `CollabServerMsg` (`protocol.rs`), `serde_json`.

**Used by** — `deliver_presence`, `deliver_event`, `handle_msg` (`Op` fan-out).

**Repeated context** — Dropping a slow consumer is safe *because* of the
snapshot-rebuild model: no collab message is load-bearing for durability — the rows
are.

---

## fn touch_presence

**Identification** — private async function; marker `// md:fn touch_presence`.

**Code** — complete and verbatim:

```rust
// md:fn touch_presence
async fn touch_presence(
    state: &AppState,
    note_id: Uuid,
    conn_id: u64,
    user_id: Uuid,
    display_name: &str,
    cursor: Option<&Cursor>,
) -> Result<(), AppError> {
    let cursor_json = cursor.map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null));
    state
        .store
        .upsert_presence(
            note_id,
            state.instance_id,
            conn_id as i64,
            user_id,
            display_name,
            cursor_json.as_ref(),
        )
        .await?;
    announce_presence(state, note_id).await;
    Ok(())
}
```

**What it does** — Records (upserts) this connection's presence row — keyed
`(note_id, instance_id, conn_id)`, carrying user id, display name and the optional
caret as opaque JSON — in the shared presence table, then `announce_presence` so
every replica rebroadcasts the merged list (issue #45).

**Dependencies** — `Store::upsert_presence` (`store.rs`), `announce_presence`
(this file), `Cursor` (`protocol.rs`).

**Used by** — `handle_msg` (`Join` with no cursor, `Cursor` with one).

**Repeated context** — Presence is ephemeral and unversioned: rows are
heartbeat-touched by the maintenance loop (`main.rs`) for this instance and swept by
TTL when an instance crashes; receivers always get the **full** list and replace.

---

## fn clear_presence

**Identification** — private async function; marker `// md:fn clear_presence`.
`async fn clear_presence(state, note_id, conn_id) -> Result<(), AppError>`.

**Code** — complete and verbatim:

```rust
// md:fn clear_presence
async fn clear_presence(state: &AppState, note_id: Uuid, conn_id: u64) -> Result<(), AppError> {
    state
        .store
        .delete_presence(note_id, state.instance_id, conn_id as i64)
        .await?;
    announce_presence(state, note_id).await;
    Ok(())
}
```

**What it does** — Deletes this connection's presence row and announces the new
merged list.

**Dependencies** — `Store::delete_presence` (`store.rs`), `announce_presence`
(this file).

**Used by** — `handle_msg` (`Leave`), `run_connection` (disconnect teardown).

**Repeated context** — Rows a crashed instance leaves behind are reclaimed by the
TTL sweep (`main.rs` maintenance loop) and by each instance clearing its own rows at
startup — this function is only the orderly path.

---

## fn announce_presence

**Identification** — private async function; marker `// md:fn announce_presence`.
`async fn announce_presence(state: &AppState, note_id: Uuid)`.

**Code** — complete and verbatim:

```rust
// md:fn announce_presence
async fn announce_presence(state: &AppState, note_id: Uuid) {
    deliver_presence(state, note_id).await;
    let payload = format!("{}:{}", note_id, state.instance_id);
    if let Err(e) = state.store.notify("collab_presence", &payload).await {
        tracing::warn!(error = %e, %note_id, "presence notify failed");
    }
}
```

**What it does** — Broadcasts the merged presence to **this** instance's
subscribers now (`deliver_presence`), then notifies the other instances
(`collab_presence` channel, payload `"<note_id>:<instance_id>"`) to do the same.
The local broadcast means presence works single-instance even with no bus running;
the instance id in the payload lets a sibling's bus handler skip the echo back to
us. A notify failure is logged, not propagated.

**Dependencies** — `deliver_presence` (this file), `Store::notify` (`store.rs`),
`bus.rs` channel semantics.

**Used by** — `touch_presence`, `clear_presence` (this file).

**Repeated context** — Origin-delivers-locally is the bus's core convention
(`bus.rs`): the origin instance never depends on its own notification coming back.

---

## fn deliver_presence

**Identification** — public async function; marker `// md:fn deliver_presence`.
`pub async fn deliver_presence(state: &AppState, note_id: Uuid)`.

**Code** — complete and verbatim:

```rust
// md:fn deliver_presence
pub async fn deliver_presence(state: &AppState, note_id: Uuid) {
    let Some(session) = state.collab.get(note_id).await else {
        return;
    };
    let rows = match state.store.list_presence(note_id).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, %note_id, "presence read failed");
            return;
        }
    };
    let mut by_user: HashMap<Uuid, PresenceInfo> = HashMap::new();
    for row in rows {
        let cursor = row
            .cursor
            .and_then(|c| serde_json::from_value::<Cursor>(c).ok());
        by_user
            .entry(row.user_id)
            .and_modify(|p| {
                if p.cursor.is_none() {
                    p.cursor = cursor.clone();
                }
            })
            .or_insert_with(|| PresenceInfo {
                user_id: row.user_id.to_string(),
                display_name: row.display_name,
                cursor,
            });
    }
    session
        .broadcast(
            &CollabServerMsg::Presence {
                note_id,
                users: by_user.into_values().collect(),
            },
            None,
        )
        .await;
}
```

**What it does** — Bus entrypoint (also used locally): if this instance has a live
session for the note, read all presence rows (every instance's) from the shared
table, merge them **per user** — a user connected twice appears once; the first
non-`None` cursor wins — and broadcast the full `CollabServerMsg::Presence` list to
local subscribers. No session or a read failure → warn/return.

**Dependencies** — `CollabRegistry::get`, `CollabSession::broadcast` (this file);
`Store::list_presence` (`store.rs`); `PresenceInfo`/`Cursor` (`protocol.rs`).

**Used by** — `announce_presence` (local path) and `bus.rs::handle_collab_presence`
(cross-instance path).

**Repeated context** — Presence lists are **replace, never merge** on the client;
user-scoped (the UI shows people, not devices).

---

## fn deliver_event

**Identification** — public async function; marker `// md:fn deliver_event`.
`pub async fn deliver_event(state: &AppState, event: crate::store::CollabEvent)`.

**Code** — complete and verbatim:

```rust
// md:fn deliver_event
pub async fn deliver_event(state: &AppState, event: crate::store::CollabEvent) {
    let Some(session) = state.collab.get(event.note_id).await else {
        return;
    };
    let ops: Vec<LineOp> = match serde_json::from_value(event.ops) {
        Ok(ops) => ops,
        Err(e) => {
            tracing::warn!(error = %e, note_id = %event.note_id, "collab event ops unparseable");
            return;
        }
    };
    let _guard = session.apply_lock.lock().await;
    let server_seq = session.seq.fetch_add(1, Ordering::Relaxed) + 1;
    session
        .broadcast(
            &CollabServerMsg::Op {
                server_seq,
                note_id: event.note_id,
                user_id: event.user_id.to_string(),
                ops,
            },
            None,
        )
        .await;
}
```

**What it does** — Bus entrypoint: a `collab_op` outbox event authored by
*another* instance (the caller — `bus.rs::handle_collab_op` — already filtered out
our own). If this instance has a live session for the note: parse the stored ops
JSON back into `Vec<LineOp>` (unparseable → warn/return), then **take the session's
apply lock** — serialising against a concurrent local join so a just-subscribed
connection cannot miss this op between its snapshot and subscribe (the op is
already durable; a duplicate delivery is resolved away by the client) — stamp this
instance's own next `server_seq`, and broadcast `CollabServerMsg::Op` to all local
subscribers.

**Dependencies** — `CollabRegistry::get`, `CollabSession::{apply_lock, seq,
broadcast}` (this file); `store::CollabEvent` (`store.rs`); `LineOp`
(`protocol.rs`).

**Used by** — `bus.rs::handle_collab_op` only.

**Repeated context** — Each instance stamps its **own** sequence on cross-instance
ops: `server_seq` is a per-connection-stream ordering aid, not a global order; the
global order is settled by vv resolution at the database.

---

## fn handler

**Identification** — public async function (axum handler); marker
`// md:fn handler`.

**Code** — complete and verbatim:

```rust
// md:fn handler
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let header_token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    let token = header_token
        .or(params.get("token").map(String::as_str))
        .ok_or(AppError::MissingToken)?;
    let authed = auth::verify_token(token, &state.config.jwt_secret)?;
    match state.store.get_device(authed.device_id).await? {
        Some(device) if device.user_id == authed.user_id => {}
        _ => return Err(AppError::InvalidToken),
    }
    let user = state
        .store
        .get_user_by_id(authed.user_id)
        .await?
        .ok_or(AppError::InvalidToken)?;
    Ok(ws
        .max_message_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| async move {
            run_connection(state, socket, user.id, authed.device_id, user.display_name).await;
        }))
}
```

**What it does** — `GET /api/ws` (design §7.1). Token resolution: prefer the
`Authorization: Bearer` header — a token in the query string ends up in
proxy/access logs — with `?token=` kept as a fallback for WS clients that cannot
set headers; absent → `MissingToken` (401). Verify the JWT; then the **revocation
check** (issue #20): the token's device must still exist and belong to the token's
user (`store.get_device`), exactly like REST's `auth_mw` and the sync relay's
handshake — otherwise a revoked token would keep editing notes until `exp`. Load
the user row (for the display name; a vanished user → `InvalidToken`). Upgrade with
`MAX_WS_MESSAGE` and run `run_connection`. The token authenticates the **user**;
which notes the connection may touch is checked per `Join`/`Op` against the shares.

**Dependencies** — `auth::verify_token` (`auth.rs`); `Store::{get_device,
get_user_by_id}` (`store.rs`); `MAX_WS_MESSAGE`, `run_connection` (this file).

**Used by** — `http.rs::router` (the `/api/ws` route).

**Repeated context** — The crate-wide revocation invariant: every authenticated
surface re-checks the device row (REST, `/api/sync`, here), which is what makes the
365-day default `TOKEN_TTL_DAYS` acceptable.

---

## fn run_connection

**Identification** — private async function; marker `// md:fn run_connection`.

**Code** — complete and verbatim:

```rust
// md:fn run_connection
async fn run_connection(
    state: Arc<AppState>,
    socket: WebSocket,
    user_id: Uuid,
    device_id: Uuid,
    display_name: String,
) {
    let conn_id = state.collab.next_conn_id.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = socket.split();

    let (tx, mut rx) = mpsc::channel::<String>(OUTBOUND_CAPACITY);
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.reset();
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(text) => {
                        if sink.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut joined: HashMap<Uuid, Arc<CollabSession>> = HashMap::new();

    loop {
        let msg = match tokio::time::timeout(ACTIVITY_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(msg))) => msg,
            _ => break,
        };
        let text = match msg {
            Message::Text(text) => text,
            Message::Close(_) => break,
            _ => continue,
        };
        let client_msg: CollabClientMsg = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                send_error(&tx, "bad_message", &format!("unparseable message: {e}"));
                continue;
            }
        };
        if let Err(e) = handle_msg(
            &state,
            &tx,
            conn_id,
            user_id,
            device_id,
            &display_name,
            &mut joined,
            client_msg,
        )
        .await
        {
            tracing::warn!(error = %e, %user_id, "collab message failed");
            send_error(&tx, "internal", "internal error");
        }
    }

    for (note_id, session) in joined {
        session.subscribers.write().await.remove(&conn_id);
        let _ = clear_presence(&state, note_id, conn_id).await;
        state.collab.drop_if_empty(note_id).await;
    }
    writer.abort();
}
```

**What it does** — One collaborative connection. Setup: allocate a `conn_id`;
split the socket; spawn the **writer task** — all outbound traffic (welcomes,
fan-out, presence, errors) funnels through one bounded `mpsc` channel
(`OUTBOUND_CAPACITY`) so a single task owns the sink (issue #34), and that task
also emits periodic pings (`PING_INTERVAL`, first tick after the interval, not
immediately) whose failed write surfaces a dead peer (issue #35).

Read loop: wait for the next frame **bounded by `ACTIVITY_TIMEOUT`** — a peer gone
silent (not even answering pings) is dropped instead of leaking a subscriber slot
forever (issue #35); any frame, including a pong, counts as activity. Text frames
parse as `CollabClientMsg` (parse failure → `bad_message` error frame, connection
stays open) and dispatch to `handle_msg` (an internal error is logged and answered
with an `internal` error frame). `Close`/timeout/transport error → exit the loop.

The `joined` map holds the sessions this connection subscribed to. **Access is
deliberately not cached in it** (issue #30): it is re-resolved per operation so a
revoked share takes effect without waiting for a reconnect.

Teardown: for every joined note — remove the subscriber, clear the shared presence
row, drop the session if empty — then abort the writer task.

**Dependencies** — `CollabRegistry::next_conn_id`, `handle_msg`, `send_error`,
`clear_presence`, `drop_if_empty`, the connection constants (this file);
`futures_util` split; `tokio` mpsc/timeout.

**Used by** — `handler` (this file) only.

**Repeated context** — One-writer-per-socket is what makes the bounded-queue drop
policy sound (no interleaved partial writes); the timeout/ping pair is the leak
defence of issues #34/#35.

---

## fn send_error

**Identification** — private function; marker `// md:fn send_error`.
`fn send_error(tx: &mpsc::Sender<String>, code: &str, message: &str)`.

**Code** — complete and verbatim:

```rust
// md:fn send_error
fn send_error(tx: &mpsc::Sender<String>, code: &str, message: &str) {
    let msg = CollabServerMsg::Error {
        code: code.into(),
        message: message.into(),
    };
    let _ = tx.try_send(serde_json::to_string(&msg).expect("serializable error"));
}
```

**What it does** — Serialises a `CollabServerMsg::Error { code, message }` and
`try_send`s it to the connection's outbound queue. Best-effort: if the queue is
full the error is dropped (the connection is already being dropped as a slow
consumer). Errors are per-frame; the connection stays open.

**Dependencies** — `CollabServerMsg` (`protocol.rs`), `serde_json`.

**Used by** — `run_connection`, `handle_msg` (this file).

**Repeated context** — Error codes used across this file: `bad_message`,
`not_found`, `forbidden`, `not_joined`, `bad_writer`, `bad_content`, `too_long`,
`too_many_lines`, `line_exists`, `bad_after`, `bad_move`, `internal`. They are a
client-facing contract (tests assert them).

---

## fn handle_msg

**Identification** — private async function; marker `// md:fn handle_msg`.

**Code** — complete and verbatim:

```rust
// md:fn handle_msg
#[allow(clippy::too_many_arguments)]
async fn handle_msg(
    state: &Arc<AppState>,
    tx: &mpsc::Sender<String>,
    conn_id: u64,
    user_id: Uuid,
    device_id: Uuid,
    display_name: &str,
    joined: &mut HashMap<Uuid, Arc<CollabSession>>,
    msg: CollabClientMsg,
) -> Result<(), AppError> {
    match msg {
        CollabClientMsg::Join { note_id } => {
            let note = match state.store.get_note(note_id).await? {
                Some(note) => note,
                None => {
                    send_error(tx, "not_found", "note not found");
                    return Ok(());
                }
            };
            let access = match resolve_note_access(&state.store, &note, user_id).await {
                Ok(access) if access.can_read() => access,
                Ok(_) | Err(AppError::Forbidden) => {
                    send_error(tx, "forbidden", "no access to this note");
                    return Ok(());
                }
                Err(e) => return Err(e),
            };

            let session = state.collab.get_or_create(note_id).await;
            {
                let _guard = session.apply_lock.lock().await;
                let snapshot = read_snapshot(state, note_id).await?;
                session
                    .subscribers
                    .write()
                    .await
                    .insert(conn_id, Subscriber { tx: tx.clone() });
                let welcome = CollabServerMsg::Welcome { note_id, snapshot };
                let _ = tx.try_send(serde_json::to_string(&welcome).expect("serializable welcome"));
            }
            let _ = access;
            joined.insert(note_id, session.clone());

            touch_presence(state, note_id, conn_id, user_id, display_name, None).await?;
        }

        CollabClientMsg::Leave { note_id } => {
            if let Some(session) = joined.remove(&note_id) {
                session.subscribers.write().await.remove(&conn_id);
                let _ = clear_presence(state, note_id, conn_id).await;
                state.collab.drop_if_empty(note_id).await;
            }
        }

        CollabClientMsg::Op { note_id, ops } => {
            let session = match joined.get(&note_id) {
                Some(session) => session.clone(),
                None => {
                    send_error(tx, "not_joined", "join the note before sending ops");
                    return Ok(());
                }
            };
            let note = match state.store.get_note(note_id).await? {
                Some(note) => note,
                None => {
                    send_error(tx, "not_found", "note not found");
                    return Ok(());
                }
            };
            let access = match resolve_note_access(&state.store, &note, user_id).await {
                Ok(access) => access,
                Err(AppError::Forbidden) => {
                    send_error(tx, "forbidden", "access to this note was revoked");
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            if !access.can_write() {
                send_error(tx, "forbidden", "no write access to this note");
                return Ok(());
            }

            let mut applied = Vec::new();
            {
                let _guard = session.apply_lock.lock().await;
                let mut lock_tx = state.store.lock_note_order(note_id).await?;
                for op in ops {
                    match apply_op(state, &mut lock_tx, note_id, device_id, op).await? {
                        OpOutcome::Applied(op) => applied.push(op),
                        OpOutcome::Ignored => {}
                        OpOutcome::Invalid { code, message } => {
                            send_error(tx, &code, &message);
                        }
                    }
                }
                lock_tx.commit().await?;
                if !applied.is_empty() {
                    let server_seq = session.seq.fetch_add(1, Ordering::Relaxed) + 1;
                    session
                        .broadcast(
                            &CollabServerMsg::Op {
                                server_seq,
                                note_id,
                                user_id: user_id.to_string(),
                                ops: applied.clone(),
                            },
                            Some(conn_id),
                        )
                        .await;
                }
            }
            if !applied.is_empty() {
                if let Ok(ops_json) = serde_json::to_value(&applied) {
                    match state
                        .store
                        .insert_collab_event(
                            note_id,
                            state.instance_id,
                            conn_id as i64,
                            user_id,
                            &ops_json,
                        )
                        .await
                    {
                        Ok(seq) => {
                            let _ = state
                                .store
                                .notify("collab_op", &format!("{}:{}", seq, state.instance_id))
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, %note_id, "collab outbox insert failed")
                        }
                    }
                }
            }
        }

        CollabClientMsg::Cursor { note_id, cursor } => {
            if joined.contains_key(&note_id) {
                touch_presence(
                    state,
                    note_id,
                    conn_id,
                    user_id,
                    display_name,
                    Some(&cursor),
                )
                .await?;
            }
        }

        CollabClientMsg::Ack { .. } => {}
    }
    Ok(())
}
```

**What it does** — Dispatch on the client message:

- **`Join { note_id }`** — load the note (`not_found` if absent/invisible); resolve
  access and require `can_read` (`forbidden` otherwise — viewers may join and
  watch). Then, **under the session's apply lock**: read the snapshot, insert the
  subscriber, and enqueue the `Welcome` — all three together, so (a) no op can slip
  between snapshot and subscription (local ops and cross-instance ops both take
  this lock before broadcasting) and (b) the `Welcome` is queued on the
  connection's channel before any op can be, keeping `Welcome` first. The resolved
  access is intentionally **not stored** (issue #30) — writes re-resolve per op
  batch. Record the note in `joined` and `touch_presence` (shared table + notify,
  issue #45).
- **`Leave { note_id }`** — remove the subscriber, clear presence, drop the
  session if empty.
- **`Op { note_id, ops }`** — must have joined (`not_joined` otherwise).
  **Re-resolve access** on every batch (issue #30): note gone → `not_found`; access
  revoked → `forbidden`; `can_write` required (viewers get `forbidden`). Then apply
  sequentially under the session's in-process apply lock **and** a Postgres
  advisory lock keyed by the note (`store.lock_note_order` — a transaction holding
  `pg_advisory_xact_lock`), so two instances editing the same order serialise at
  the database and cannot lose an update (issue #45). Per op, `apply_op` on the
  lock's connection: `Applied` ops are collected, `Ignored` ops dropped silently,
  `Invalid` ops answered with an error frame. Commit (releases the advisory lock);
  if anything applied, stamp `server_seq` and broadcast to the note's other local
  subscribers (skipping the originator). Finally, cross-instance fan-out
  (issue #45): serialise the applied ops into the `collab_events` outbox and NOTIFY
  `collab_op` with `"<seq>:<instance_id>"` — done **after** the local broadcast so
  local latency is unaffected; siblings deliver under their own sequence; outbox
  failures are logged, not fatal (a missed sibling delivery heals by snapshot
  rebuild).
- **`Cursor { note_id, cursor }`** — if joined, `touch_presence` with the caret
  (which broadcasts the merged list).
- **`Ack { .. }`** — client-side bookkeeping; nothing to do server-side.

**Dependencies** — `resolve_note_access` (`permissions.rs`); `Store::{get_note,
lock_note_order, insert_collab_event, notify}` (`store.rs`); `read_snapshot`,
`apply_op`, `send_error`, `touch_presence`, `clear_presence`, session/registry
methods (this file); wire types (`protocol.rs`).

**Used by** — `run_connection` (this file) only.

**Repeated context** — The Join-under-lock choreography and the per-op access
re-resolution are the two auditor-visible fixes this file carries (join-gap
soundness; issue #30 revocation). The advisory lock closes the cross-instance
lost-update window on the order (issue #45; also the reason tombstone GC must
serialise against it — issue #25).

---

## fn read_snapshot

**Identification** — private async function; marker `// md:fn read_snapshot`.

**Code** — complete and verbatim:

```rust
// md:fn read_snapshot
async fn read_snapshot(state: &AppState, note_id: Uuid) -> Result<NoteLinesSnapshot, AppError> {
    let order = state
        .store
        .get_note_order(note_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let lines = state.store.list_lines(note_id).await?;
    Ok(NoteLinesSnapshot {
        note_id,
        order: order.order,
        updated_at: order.updated_at,
        vv: order.vv,
        last_writer: order.last_writer,
        lines: lines.into_iter().map(line_snapshot).collect(),
    })
}
```

**What it does** — Builds the `Welcome` payload: the note's order entity
(`get_note_order` — `NotFound` if the note has no order row) plus every line
(`list_lines`, tombstones included), converted via `line_snapshot`.

**Dependencies** — `Store::{get_note_order, list_lines}` (`store.rs`);
`NoteLinesSnapshot` (`protocol.rs`); `line_snapshot` (this file).

**Used by** — `handle_msg` (`Join`) — under the apply lock.

**Repeated context** — Snapshots include tombstoned lines (soft-delete: deletion
is a `deleted_at` timestamp, never row removal) so a client that was offline
during a delete still converges on it.

---

## fn line_snapshot

**Identification** — private function; marker `// md:fn line_snapshot`.
`fn line_snapshot(line: Line) -> LineSnapshot`.

**Code** — complete and verbatim:

```rust
// md:fn line_snapshot
fn line_snapshot(line: Line) -> LineSnapshot {
    LineSnapshot {
        id: line.id,
        content: line.content,
        created_at: line.created_at,
        updated_at: line.updated_at,
        deleted_at: line.deleted_at,
        vv: line.vv.0,
        last_writer: line.last_writer,
    }
}
```

**What it does** — Converts the store's `Line` row into the wire shape
(`LineSnapshot`), unwrapping the stored vv (`line.vv.0` — the store wraps
`VersionVector` in a JSONB newtype).

**Dependencies** — `Line` (`store.rs`), `LineSnapshot` (`protocol.rs`).

**Used by** — `read_snapshot` (this file).

**Repeated context** — none.

---

## OpOutcome

**Identification** — private enum; marker `// md:OpOutcome`.

**Code** — complete and verbatim:

```rust
// md:OpOutcome
enum OpOutcome {
    Applied(LineOp),
    Ignored,
    Invalid { code: String, message: String },
}
```

**What it does** — The result of resolving one op. `Applied(op)`: the op won its
resolution and was persisted — fan it out. `Ignored`: dominated by current state
(design §4.3.3) — drop **silently** (this is normal convergence, not an error).
`Invalid { code, message }`: malformed or referencing missing entities — tell the
sender.

**Dependencies** — `LineOp` (`protocol.rs`).

**Used by** — `apply_op` (produces), `handle_msg` (consumes).

**Repeated context** — Silence on `Ignored` is deliberate: a replica replaying an
op it already sent, or losing a race it will learn about via fan-out, needs no
signal.

---

## fn invalid

**Identification** — private function; marker `// md:fn invalid`.
`fn invalid(code: &str, message: impl Into<String>) -> OpOutcome`.

**Code** — complete and verbatim:

```rust
// md:fn invalid
fn invalid(code: &str, message: impl Into<String>) -> OpOutcome {
    OpOutcome::Invalid {
        code: code.into(),
        message: message.into(),
    }
}
```

**What it does** — Constructor shorthand for `OpOutcome::Invalid`.

**Dependencies** — `OpOutcome` (this file). **Used by** — `apply_op`.

**Repeated context** — none.

---

## fn merge_vv

**Identification** — private function; marker `// md:fn merge_vv`.
`fn merge_vv(a: &VersionVector, b: &VersionVector) -> VersionVector`.

**Code** — complete and verbatim:

```rust
// md:fn merge_vv
fn merge_vv(a: &VersionVector, b: &VersionVector) -> VersionVector {
    let mut out = a.clone();
    for (k, v) in b {
        let entry = out.entry(k.clone()).or_insert(0);
        if *v > *entry {
            *entry = *v;
        }
    }
    out
}
```

**What it does** — The pointwise maximum of two version vectors — the merged
causal frontier stored on an entity after an op is applied. (A `VersionVector` is
a map from actor id — always a device id here — to a monotonically increasing
counter.)

**Dependencies** — `VersionVector` (keeplin-core).

**Used by** — `apply_op` (all four arms, when writing the winning state).

**Repeated context** — Storing the *merge* (not the op's vv verbatim) is what
makes the entity's vector dominate both histories afterwards — the foundation of
convergence.

---

## fn advances_writer

**Identification** — private function; marker `// md:fn advances_writer`.
`fn advances_writer(current: &VersionVector, op_vv: &VersionVector, writer: &str) -> bool`.

**Code** — complete and verbatim:

```rust
// md:fn advances_writer
fn advances_writer(current: &VersionVector, op_vv: &VersionVector, writer: &str) -> bool {
    op_vv.get(writer).copied().unwrap_or(0) > current.get(writer).copied().unwrap_or(0)
}
```

**What it does** — Design §4.3.5: an op's vector must advance its **own writer's**
component past the entity's current one. Replays of an already-applied op fail
this and are ignored — which is what keeps application **idempotent**.

**Dependencies** — `VersionVector` (keeplin-core).

**Used by** — `apply_op` (all four arms, before resolution).

**Repeated context** — Idempotency is a system-wide requirement (relay redelivery,
bus at-least-once, client retries all rely on it); this check is its collab-side
enforcement point.

---

## fn apply_op

**Identification** — private async function; marker `// md:fn apply_op`.

**Code** — complete and verbatim:

```rust
// md:fn apply_op
async fn apply_op(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    note_id: Uuid,
    device_id: Uuid,
    op: LineOp,
) -> Result<OpOutcome, AppError> {
    if op.last_writer() != device_id.to_string() {
        return Ok(invalid("bad_writer", "last_writer must be your device id"));
    }

    match &op {
        LineOp::Insert {
            after_line_id,
            line_id,
            content,
            vv,
            last_writer,
            updated_at,
        } => {
            if content.contains('\n') {
                return Ok(invalid("bad_content", "line content must not contain \\n"));
            }
            if content.len() > MAX_LINE_LEN {
                return Ok(invalid("too_long", "line exceeds maximum length"));
            }
            if state
                .store
                .get_line_on(&mut *conn, *line_id)
                .await?
                .is_some()
            {
                return Ok(invalid("line_exists", "line_id already exists"));
            }
            let order = state
                .store
                .get_note_order_on(&mut *conn, note_id)
                .await?
                .ok_or(AppError::NotFound)?;
            if order.order.len() >= MAX_LINES_PER_NOTE {
                return Ok(invalid("too_many_lines", "note line limit reached"));
            }
            let position = match position_after(&order.order, *after_line_id) {
                Some(pos) => pos,
                None => return Ok(invalid("bad_after", "after_line_id not in note order")),
            };
            if !advances_writer(&order.vv, vv, last_writer)
                || winner(&order, vv, *updated_at, last_writer) == Winner::Local
            {
                return Ok(OpOutcome::Ignored);
            }

            state
                .store
                .insert_line_on(
                    &mut *conn,
                    *line_id,
                    note_id,
                    content,
                    vv,
                    last_writer,
                    *updated_at,
                )
                .await?;
            let mut new_order = order.order.clone();
            new_order.insert(position, *line_id);
            state
                .store
                .set_note_order_on(
                    &mut *conn,
                    note_id,
                    &new_order,
                    &merge_vv(&order.vv, vv),
                    last_writer,
                    *updated_at,
                )
                .await?;
            Ok(OpOutcome::Applied(op))
        }

        LineOp::Update {
            line_id,
            content,
            vv,
            last_writer,
            updated_at,
        } => {
            if content.contains('\n') {
                return Ok(invalid("bad_content", "line content must not contain \\n"));
            }
            if content.len() > MAX_LINE_LEN {
                return Ok(invalid("too_long", "line exceeds maximum length"));
            }
            let line = match state.store.get_line_on(&mut *conn, *line_id).await? {
                Some(line) if line.note_id == note_id => line,
                _ => return Ok(invalid("not_found", "line not found in this note")),
            };
            if !advances_writer(&line.vv.0, vv, last_writer)
                || line_winner(&line, vv, *updated_at, last_writer) == Winner::Local
            {
                return Ok(OpOutcome::Ignored);
            }
            state
                .store
                .update_line_on(
                    &mut *conn,
                    *line_id,
                    content,
                    &merge_vv(&line.vv.0, vv),
                    last_writer,
                    *updated_at,
                )
                .await?;
            Ok(OpOutcome::Applied(op))
        }

        LineOp::Delete {
            line_id,
            deleted_at,
            vv,
            last_writer,
            updated_at,
        } => {
            let line = match state.store.get_line_on(&mut *conn, *line_id).await? {
                Some(line) if line.note_id == note_id => line,
                _ => return Ok(invalid("not_found", "line not found in this note")),
            };
            if !advances_writer(&line.vv.0, vv, last_writer)
                || line_winner(&line, vv, *updated_at, last_writer) == Winner::Local
            {
                return Ok(OpOutcome::Ignored);
            }
            state
                .store
                .soft_delete_line_on(
                    &mut *conn,
                    *line_id,
                    *deleted_at,
                    &merge_vv(&line.vv.0, vv),
                    last_writer,
                    *updated_at,
                )
                .await?;
            Ok(OpOutcome::Applied(op))
        }

        LineOp::Move {
            line_ids,
            after_line_id,
            vv,
            last_writer,
            updated_at,
        } => {
            if line_ids.is_empty() {
                return Ok(invalid("bad_move", "line_ids must not be empty"));
            }
            let order = state
                .store
                .get_note_order_on(&mut *conn, note_id)
                .await?
                .ok_or(AppError::NotFound)?;
            if line_ids.iter().any(|id| !order.order.contains(id)) {
                return Ok(invalid(
                    "not_found",
                    "a moved line is not in the note order",
                ));
            }
            if let Some(after) = after_line_id {
                if line_ids.contains(after) {
                    return Ok(invalid("bad_move", "after_line_id cannot be a moved line"));
                }
            }
            if !advances_writer(&order.vv, vv, last_writer)
                || winner(&order, vv, *updated_at, last_writer) == Winner::Local
            {
                return Ok(OpOutcome::Ignored);
            }

            let mut new_order: Vec<Uuid> = order
                .order
                .iter()
                .copied()
                .filter(|id| !line_ids.contains(id))
                .collect();
            let position = match position_after(&new_order, *after_line_id) {
                Some(pos) => pos,
                None => return Ok(invalid("bad_after", "after_line_id not in note order")),
            };
            new_order.splice(position..position, line_ids.iter().copied());
            state
                .store
                .set_note_order_on(
                    &mut *conn,
                    note_id,
                    &new_order,
                    &merge_vv(&order.vv, vv),
                    last_writer,
                    *updated_at,
                )
                .await?;
            Ok(OpOutcome::Applied(op))
        }
    }
}
```

**What it does** — Applies one op. All reads and writes go through `conn` — the
connection holding the note's advisory lock — so the whole batch runs on a single
connection (cannot deadlock against the bounded pool) and the order's
read-modify-write is serialised across instances (issue #45).

First gate, all variants: **writer identity** — `op.last_writer()` must equal the
authenticated device id (`bad_writer` otherwise). Clients cannot forge edits in
someone else's name, and two devices of one user never share a vv component
(sharing one would make the server treat the second device's concurrent edits as
replays). Presence stays user-based; only the vv actor is the device.

Per variant:

- **`Insert`** — content checks (`bad_content` on `\n`, `too_long` over
  `MAX_LINE_LEN`); `line_exists` if the line id is already taken; load the order
  (`NotFound` without one); `too_many_lines` at `MAX_LINES_PER_NOTE`; `bad_after`
  if the anchor is not in the order (`None` anchor = insert at the beginning).
  Resolution **against the order entity** (design §5.2): `advances_writer` +
  `winner` — a stale insert loses against the current order and is `Ignored`.
  On win: insert the line row, then write the new order with
  `merge_vv(order.vv, op.vv)`.
- **`Update`** — content checks; the line must exist **in this note**
  (`not_found`); resolution against the **line** entity (`advances_writer` +
  `line_winner`); on win, update content with the merged vv.
- **`Delete`** — the line must exist in this note; resolution against the line;
  on win, **soft-delete**: set `deleted_at` (tombstone), merged vv. The row
  remains; tombstones ship in snapshots and are GC'd only after `LINES_GC_DAYS`.
- **`Move`** — `bad_move` on empty `line_ids` or when the anchor is itself moved;
  every moved id must be in the order (`not_found`); resolution against the
  order. On win: extract the moved block, reinsert it after the anchor
  (`bad_after` if the anchor vanished from the filtered order), write the new
  order with the merged vv.

**Dependencies** — `LineOp::last_writer` (`protocol.rs`); the `_on(executor)`
store variants `get_line_on`, `get_note_order_on`, `insert_line_on`,
`update_line_on`, `soft_delete_line_on`, `set_note_order_on` (`store.rs`);
`invalid`, `merge_vv`, `advances_writer`, `winner`, `line_winner`,
`position_after`, the limits (this file).

**Used by** — `handle_msg` (`Op` arm) only.

**Repeated context** — The complete op pipeline, restated: writer gate →
shape/limit validation → existence → `advances_writer` (idempotency) →
`note_log::resolve` (vv dominance, then the deterministic
`(updated_at, last_writer)` LWW tiebreak) → persist with merged vv → fan out.
`Insert`/`Move` resolve against the **order** entity; `Update`/`Delete` against
the **line** — the two-entity model that makes structural edits and content edits
independently mergeable.

---

## fn winner

**Identification** — private function; marker `// md:fn winner`.

**Code** — complete and verbatim:

```rust
// md:fn winner
fn winner(
    order: &crate::store::NoteOrder,
    op_vv: &VersionVector,
    op_ts: DateTime<Utc>,
    op_writer: &str,
) -> Winner {
    resolve(
        &order.vv,
        order.updated_at,
        &order.last_writer,
        op_vv,
        op_ts,
        op_writer,
    )
}
```

**What it does** — Resolves an op against the **order** entity by delegating to
keeplin-core's `note_log::resolve(current_vv, current_ts, current_writer, op_vv,
op_ts, op_writer)`. `Winner::Incoming` = apply.

**Dependencies** — `resolve`/`Winner` (keeplin-core), `NoteOrder` (`store.rs`).

**Used by** — `apply_op` (`Insert`, `Move`).

**Repeated context** — Using the *same* `resolve` as every client is what
guarantees server and clients pick the same winner — the convergence contract.

---

## fn line_winner

**Identification** — private function; marker `// md:fn line_winner`.

**Code** — complete and verbatim:

```rust
// md:fn line_winner
fn line_winner(
    line: &Line,
    op_vv: &VersionVector,
    op_ts: DateTime<Utc>,
    op_writer: &str,
) -> Winner {
    resolve(
        &line.vv.0,
        line.updated_at,
        &line.last_writer,
        op_vv,
        op_ts,
        op_writer,
    )
}
```

**What it does** — The same resolution against a **line** entity
(`line.vv.0`, `line.updated_at`, `line.last_writer`).

**Dependencies** — `resolve`/`Winner` (keeplin-core), `Line` (`store.rs`).

**Used by** — `apply_op` (`Update`, `Delete`).

**Repeated context** — as `fn winner`.

---

## fn position_after

**Identification** — private function; marker `// md:fn position_after`.
`fn position_after(order: &[Uuid], after_line_id: Option<Uuid>) -> Option<usize>`.

**Code** — complete and verbatim:

```rust
// md:fn position_after
fn position_after(order: &[Uuid], after_line_id: Option<Uuid>) -> Option<usize> {
    match after_line_id {
        None => Some(0),
        Some(after) => order.iter().position(|id| *id == after).map(|i| i + 1),
    }
}
```

**What it does** — The index right after `after_line_id` in `order`
(`None` anchor = index 0, the beginning). Returns `None` when the anchor line is
absent — the caller maps that to `bad_after`.

**Dependencies** — none.

**Used by** — `apply_op` (`Insert`, `Move`).

**Repeated context** — Anchor-based positioning (rather than numeric indices) is
what keeps concurrent inserts meaningful after resolution reorders things.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `handle_msg()` — defined here (EXTRACTED; 4 cross-file edge(s))
- `touch_presence()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `read_snapshot()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `apply_op()` — defined here (EXTRACTED; 3 cross-file edge(s))
- `clear_presence()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `deliver_event()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `handler()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `line_snapshot()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `winner()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `line_winner()` — defined here (EXTRACTED; 2 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: references×6; e.g. `AppError`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×2; e.g. `.resolve()`)
- `crates/keeplin-srv/src/permissions.rs` — note capabilities (EXTRACTED: calls×1; e.g. `resolve_note_access()`)
- `crates/keeplin-srv/src/protocol.rs` — collaborative wire types (EXTRACTED: references×7; e.g. `CollabServerMsg`, `Cursor`, `CollabClientMsg`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×10; e.g. `AppState`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×4; e.g. `CollabEvent`, `Line`, `NoteOrder`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)

## Coverage checklist

Every code block of `collab.rs`, in source order, each documented above (five points)
and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | the six consts | `// md:Constants` | Constants |
| 3 | `struct Subscriber` | `// md:Subscriber` | Subscriber |
| 4 | `struct CollabSession` | `// md:CollabSession` | CollabSession |
| 5 | `struct CollabRegistry` | `// md:CollabRegistry` | CollabRegistry |
| 6 | `impl CollabRegistry` | `// md:impl CollabRegistry` | impl CollabRegistry |
| 7 | `fn stats` | `// md:impl CollabRegistry > fn stats` | impl CollabRegistry › fn stats |
| 8 | `fn get` | `// md:impl CollabRegistry > fn get` | impl CollabRegistry › fn get |
| 9 | `fn get_or_create` | `// md:impl CollabRegistry > fn get_or_create` | impl CollabRegistry › fn get_or_create |
| 10 | `fn drop_if_empty` | `// md:impl CollabRegistry > fn drop_if_empty` | impl CollabRegistry › fn drop_if_empty |
| 11 | `impl CollabSession` | `// md:impl CollabSession` | impl CollabSession |
| 12 | `fn broadcast` | `// md:impl CollabSession > fn broadcast` | impl CollabSession › fn broadcast |
| 13 | `fn touch_presence` | `// md:fn touch_presence` | fn touch_presence |
| 14 | `fn clear_presence` | `// md:fn clear_presence` | fn clear_presence |
| 15 | `fn announce_presence` | `// md:fn announce_presence` | fn announce_presence |
| 16 | `fn deliver_presence` | `// md:fn deliver_presence` | fn deliver_presence |
| 17 | `fn deliver_event` | `// md:fn deliver_event` | fn deliver_event |
| 18 | `fn handler` | `// md:fn handler` | fn handler |
| 19 | `fn run_connection` | `// md:fn run_connection` | fn run_connection |
| 20 | `fn send_error` | `// md:fn send_error` | fn send_error |
| 21 | `fn handle_msg` | `// md:fn handle_msg` | fn handle_msg |
| 22 | `fn read_snapshot` | `// md:fn read_snapshot` | fn read_snapshot |
| 23 | `fn line_snapshot` | `// md:fn line_snapshot` | fn line_snapshot |
| 24 | `enum OpOutcome` | `// md:OpOutcome` | OpOutcome |
| 25 | `fn invalid` | `// md:fn invalid` | fn invalid |
| 26 | `fn merge_vv` | `// md:fn merge_vv` | fn merge_vv |
| 27 | `fn advances_writer` | `// md:fn advances_writer` | fn advances_writer |
| 28 | `fn apply_op` | `// md:fn apply_op` | fn apply_op |
| 29 | `fn winner` | `// md:fn winner` | fn winner |
| 30 | `fn line_winner` | `// md:fn line_winner` | fn line_winner |
| 31 | `fn position_after` | `// md:fn position_after` | fn position_after |
