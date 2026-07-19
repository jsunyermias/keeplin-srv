# `sync.rs` — the device sync relay

Self-contained companion for `crates/keeplin-srv/src/sync.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able
to understand `sync.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `sync.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the
section documenting it here (starting below the file title). Grep the marker text to
jump code → doc; grep the section's block name (or the marker path) in the `.rs` to
jump doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview`
at the top of the file.

**Code** — complete and verbatim:

```rust
// md:Overview
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::{auth, state::AppState, store::ChangeRow};
```

**What it does** — The WebSocket sync relay behind `GET /api/sync`: the server side of
keeplin-core's `DbBackend` wire protocol. It relays each device's `Change` batches to
the user's **other** devices and journals every batch so an offline device catches up
on reconnect. This channel carries the non-collaborative entities (notebooks, tags,
note↔tag associations, resources); notes travel over the collaborative channel
(`/api/ws`, `collab.rs`) instead.

The wire protocol — exactly what the client's `DbBackend::connect_ws` /
`send_changes` / `receive_changes` speak:

1. The client connects and immediately sends the handshake frame
   `{"type":"auth","token":"<jwt>"}` (a device token from `POST /api/login`,
   identifying both user and device).
2. The client pushes batches as
   `{"type":"changes","batch_id":"…","device_id":"…","changes":[Change…]}`.
3. The server sends `{"type":"changes","changes":[Change…]}` — first the **backlog**
   the device has not seen, then live batches from the user's other devices. A device
   never receives its own batches back.

`Change` payloads are treated as **opaque JSON**: the relay stores and forwards them
without interpreting keeplin-core's model, so client-side model evolution never
requires a server change. On top of that pass-through, the relay **materialises** the
entities the server owns (see `fn materialize`); anything it does not model stays
opaque.

Delivery guarantees: every accepted batch is persisted to the journal **before**
fan-out, and each device has a durable delivery cursor that only advances after a
successful send. Because the client's `apply_change` is idempotent, the relay prefers
**duplicate delivery over loss** — re-receiving a batch is always safe.

**Dependencies** — `axum` WebSocket types, `tokio::sync::{broadcast, RwLock}`,
`serde_json`, `uuid`, `anyhow`, `tracing` (external);
`keeplin_core::models::Change` (client repo, in `materialize`). Internal:
`crate::auth` (`verify_token`), `crate::state::AppState`,
`crate::store::{ChangeRow, UserDevice}` and the journal/cursor/materialisation
methods of `store.rs`, `crate::bus::CH_SYNC_BATCH` (`bus.rs`).

**Used by** — `http.rs` routes `GET /api/sync` to `handler`; `state.rs` holds the
`SyncHub`; `bus.rs::handle_sync_batch` calls `SyncHub::wake_user`; `http.rs::metrics`
reads `SyncHub::live_users`. Exercised end to end by `tests/integration.rs` (real
`DbBackend` clients) and `tests/materialize.rs`.

**Repeated context** — Cross-cutting invariants this file enforces:
(1) **opaque relay** — the envelope is parsed, the payloads are not (materialisation
parses a *copy* and ignores unknowns); (2) **per-user isolation** — fan-out channels
are keyed by user id, so a batch never crosses accounts; sharing happens at the REST/
collab layer, never in the relay; (3) **echo suppression by origin device id** — the
device is the concurrency actor (JWT `device_id`); (4) **duplicate-over-loss** —
journal-first persistence, cursor-after-send, lag → re-scan; (5) journal growth is
bounded by `CHANGES_RETENTION_DAYS` pruning (only rows delivered to every device of
the user), run by `main.rs`'s maintenance loop.

---

## Constants

**Identification** — logical section: the four tuning constants; marker
`// md:Constants`.

**Code** — complete and verbatim:

```rust
// md:Constants
const CHUNK_SIZE: i64 = 200;
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(30);
const FANOUT_CAPACITY: usize = 256;
```

**What it does** — `CHUNK_SIZE`: changes per outgoing `{"type":"changes"}` frame; the
client caps one `receive_changes` call at 1 000 frames, so even a huge backlog drains
quickly at this size. `AUTH_TIMEOUT`: how long the server waits for the `auth`
handshake before dropping the connection. `PING_INTERVAL`: how often the relay pings
an idle connection to keep NAT/proxy paths open and surface a dead peer through a
failed write (issue #35). `FANOUT_CAPACITY`: capacity of each per-user broadcast
channel — a lagging receiver falls back to a journal re-scan, so overflow degrades to
duplicate delivery, never loss.

**Dependencies** — none.

**Used by** — `deliver_backlog` (`CHUNK_SIZE`), `authenticate` (`AUTH_TIMEOUT`),
`relay_loop` (`PING_INTERVAL`), `SyncHub::join` (`FANOUT_CAPACITY`).

**Repeated context** — Each constant implements one of the delivery guarantees in
*Overview*; none is operator-configurable (they are protocol tuning, not policy).

---

## FanoutBatch

**Identification** — struct; marker `// md:FanoutBatch`.

**Code** — complete and verbatim:

```rust
// md:FanoutBatch
pub struct FanoutBatch {
    pub origin: Uuid,
    pub frame: String,
}
```

**What it does** — One batch already persisted to the journal, as fanned out to the
user's live local connections: the frame is pre-serialised **once** (not per
receiver), and `origin` (the authoring device id) lets each connection drop its own
batches (echo suppression).

**Dependencies** — `uuid`.

**Used by** — `FanoutMsg::Batch` (wrapped in `Arc` so cloning per receiver is
pointer-cheap); produced by `handle_incoming`; consumed in `relay_loop`.

**Repeated context** — Echo suppression, restated: a device never receives its own
changes back; on the backlog path the same rule is applied by filtering
`origin_device_id` (see `deliver_backlog`).

---

## FanoutMsg

**Identification** — enum; marker `// md:FanoutMsg`.

**Code** — complete and verbatim:

```rust
// md:FanoutMsg
#[derive(Clone)]
pub enum FanoutMsg {
    Batch(Arc<FanoutBatch>),
    Rescan,
}
```

**What it does** — What travels on a user's fan-out channel. `Batch` is a live batch
from a device connected to **this** instance. `Rescan` is a wake from the
cross-instance bus (issue #45): a batch landed for this user on another replica, so
local connections must re-scan the journal from their durable cursor to pick it up.

**Dependencies** — `FanoutBatch` (this file).

**Used by** — sent by `handle_incoming` (`Batch`) and `SyncHub::wake_user`
(`Rescan`); consumed in `relay_loop`.

**Repeated context** — The bus is wake-only/at-least-once (see `bus.md` context):
`Rescan` carries no data — durable state lives in the journal, and a missed wake
delays but never loses delivery.

---

## SyncHub

**Identification** — struct; marker `// md:SyncHub`.

**Code** — complete and verbatim:

```rust
// md:SyncHub
#[derive(Default)]
pub struct SyncHub {
    channels: RwLock<HashMap<Uuid, broadcast::Sender<FanoutMsg>>>,
}
```

**What it does** — Per-user fan-out: one `tokio::sync::broadcast` channel per user
with at least one device connected, behind an async `RwLock`. Senders are dropped
lazily when a user's last connection leaves. Lives in `AppState` (built
`::default()`, i.e. empty, at boot) — in-memory, per-instance, rebuildable state.

**Dependencies** — `tokio::sync::{RwLock, broadcast}`, `FanoutMsg` (this file).

**Used by** — `state.rs::AppState.hub`; `bus.rs::handle_sync_batch`
(`wake_user`); `http.rs::metrics` (`live_users`); the relay functions in this file.

**Repeated context** — Per-user keying is the isolation boundary: there is no global
channel, so no code path can fan a batch to another account's connections.

---

## impl SyncHub

**Identification** — inherent impl block; marker `// md:impl SyncHub`. Contains
`live_users`, `wake_user`, `join`, `leave` (next sections).

**Code** — container: members documented as sub-blocks below: fn live_users, fn wake_user, fn join, fn leave.

**What it does** — The hub's four operations: metrics, cross-instance wake,
subscribe, unsubscribe.

**Dependencies** — `SyncHub` (this file).

**Used by** — see the method sections.

**Repeated context** — none beyond the methods' own (below).

### fn live_users

**Identification** — public async method; marker
`// md:impl SyncHub > fn live_users`. `pub async fn live_users(&self) -> usize`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncHub > fn live_users
    pub async fn live_users(&self) -> usize {
        self.channels.read().await.len()
    }
```

**What it does** — The number of users with at least one live relay connection (map
size under a read lock).

**Dependencies** — none.

**Used by** — `http.rs::metrics` (the `sync_live_users` gauge).

**Repeated context** — none.

### fn wake_user

**Identification** — public async method; marker
`// md:impl SyncHub > fn wake_user`. `pub async fn wake_user(&self, user_id: Uuid)`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncHub > fn wake_user
    pub async fn wake_user(&self, user_id: Uuid) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(&user_id) {
            let _ = tx.send(FanoutMsg::Rescan);
        }
    }
```

**What it does** — Sends `FanoutMsg::Rescan` into the user's channel, if they have
one on this instance (no-op otherwise, including the no-receiver send error, which
is ignored). Called when a batch was appended for the user on **another** instance
(issue #45).

**Dependencies** — `FanoutMsg` (this file).

**Used by** — `bus.rs::handle_sync_batch` (the only caller).

**Repeated context** — The wake never carries the batch itself: the receiving
connection re-scans the journal from its durable cursor, which is idempotent.

### fn join

**Identification** — private async method; marker `// md:impl SyncHub > fn join`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncHub > fn join
    async fn join(
        &self,
        user_id: Uuid,
    ) -> (broadcast::Sender<FanoutMsg>, broadcast::Receiver<FanoutMsg>) {
        let mut channels = self.channels.write().await;
        let tx = channels
            .entry(user_id)
            .or_insert_with(|| broadcast::channel(FANOUT_CAPACITY).0)
            .clone();
        let rx = tx.subscribe();
        (tx, rx)
    }
```

**What it does** — Subscribes to the user's fan-out channel, creating it
(`FANOUT_CAPACITY`) if absent; returns the sender (to publish own batches) and a
fresh receiver (to consume others').

**Dependencies** — `FANOUT_CAPACITY` (this file), `tokio::broadcast`.

**Used by** — `run_connection` (this file) only.

**Repeated context** — `run_connection` subscribes **before** the backlog scan —
the ordering that closes the gap between the two delivery phases (see there).

### fn leave

**Identification** — private async method; marker `// md:impl SyncHub > fn leave`.
`async fn leave(&self, user_id: Uuid)`.

**Code** — complete and verbatim:

```rust
    // md:impl SyncHub > fn leave
    async fn leave(&self, user_id: Uuid) {
        let mut channels = self.channels.write().await;
        if let Some(tx) = channels.get(&user_id) {
            if tx.receiver_count() == 0 {
                channels.remove(&user_id);
            }
        }
    }
```

**What it does** — Drops the user's channel if no receiver is listening any more
(the departing connection's receiver is already dropped by then). Lazy cleanup: the
map only holds users with live connections.

**Dependencies** — none.

**Used by** — `run_connection` (on both the error and the normal exit paths).

**Repeated context** — Bounded in-memory state: like the rate limiter's bucket
sweep, the hub must not grow monotonically with users seen.

---

## fn handler

**Identification** — public async function (axum handler); marker `// md:fn handler`.

**Code** — complete and verbatim:

```rust
// md:fn handler
pub async fn handler(State(state): State<Arc<AppState>>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(64 * 1024 * 1024)
        .max_frame_size(16 * 1024 * 1024)
        .on_upgrade(move |socket| async move {
            if let Err(e) = run_connection(state, socket).await {
                tracing::debug!(error = %e, "sync connection ended with error");
            }
        })
}
```

**What it does** — `GET /api/sync`: upgrades to WebSocket — with a 64 MiB message /
16 MiB frame cap (change batches can be large; resource blobs no longer travel here,
but old clients' might) — and runs `run_connection`, logging its error (if any) at
`debug` (client disconnections are normal traffic, not server errors).

**Dependencies** — `run_connection` (this file), axum WS types.

**Used by** — `http.rs::router` (the `/api/sync` route — mounted on the rate-limited
group but *outside* the REST auth middleware: the WS does its own handshake auth).

**Repeated context** — Authentication on this surface happens **inside** the socket
(`authenticate`), because WebSocket clients cannot always set headers; the JWT +
device-row check is the same as REST's `auth_mw`.

---

## fn run_connection

**Identification** — private async function; marker `// md:fn run_connection`.

**Code** — complete and verbatim:

```rust
// md:fn run_connection
async fn run_connection(state: Arc<AppState>, mut socket: WebSocket) -> anyhow::Result<()> {
    let device = match authenticate(&state, &mut socket).await {
        Some(device) => device,
        None => {
            let _ = socket.send(Message::Close(None)).await;
            return Ok(());
        }
    };
    let user_id = device.user_id;
    let device_id = device.id;
    state.store.touch_device(device_id).await.ok();
    tracing::info!(%user_id, %device_id, "sync device connected");

    let (tx, mut rx) = state.hub.join(user_id).await;

    if let Err(e) = deliver_backlog(&state, &mut socket, user_id, device_id).await {
        state.hub.leave(user_id).await;
        return Err(e);
    }

    let result = relay_loop(&state, &mut socket, &tx, &mut rx, user_id, device_id).await;

    state.store.touch_device(device_id).await.ok();
    state.hub.leave(user_id).await;
    tracing::info!(%user_id, %device_id, "sync device disconnected");
    result
}
```

**What it does** — One relay connection, in phases:

1. **Handshake** — `authenticate`; on failure send `Close` and return `Ok` (the
   client treats closure as "reconnect later"; no error frame is defined on this
   protocol).
2. `touch_device` — record `last_seen_at` (best-effort).
3. **Subscribe before the backlog scan** (`hub.join`): anything persisted after the
   scan's snapshot arrives through the channel, so the two phases cannot leave a
   gap; overlap (a batch seen by both) is possible and safe because the client
   applies idempotently.
4. **Backlog** — `deliver_backlog`; on error, leave the hub and propagate.
5. **Relay loop** — `relay_loop` until the connection closes.
6. Teardown: `touch_device` again, `hub.leave`, log, return the loop's result.

**Dependencies** — `authenticate`, `deliver_backlog`, `relay_loop`,
`SyncHub::{join, leave}` (this file); `Store::touch_device` (`store.rs`).

**Used by** — `handler` (this file) only.

**Repeated context** — The subscribe-then-scan ordering is the file's key
correctness argument (no delivery gap); `touch_device` feeds the idle-device
observability that the journal-pruning policy depends on (a device's cursor blocks
pruning — issue #23).

---

## fn authenticate

**Identification** — private async function; marker `// md:fn authenticate`.

**Code** — complete and verbatim:

```rust
// md:fn authenticate
async fn authenticate(
    state: &AppState,
    socket: &mut WebSocket,
) -> Option<crate::store::UserDevice> {
    let frame = tokio::time::timeout(AUTH_TIMEOUT, socket.recv())
        .await
        .ok()??;
    let text = match frame {
        Ok(Message::Text(text)) => text,
        _ => return None,
    };
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if value["type"] != "auth" {
        tracing::debug!("first frame was not an auth handshake");
        return None;
    }
    let token = value["token"].as_str()?;
    let claims = match auth::verify_token(token, &state.config.jwt_secret) {
        Ok(claims) => claims,
        Err(_) => {
            tracing::debug!("sync handshake with invalid token");
            return None;
        }
    };
    match state.store.get_device(claims.device_id).await {
        Ok(Some(device)) if device.user_id == claims.user_id => Some(device),
        _ => {
            tracing::debug!(device_id = %claims.device_id, "sync token for unknown device");
            None
        }
    }
}
```

**What it does** — Waits up to `AUTH_TIMEOUT` for the first frame and requires it to
be a text frame `{"type":"auth","token":…}`; verifies the JWT
(`auth::verify_token`); then the **revocation check**: the token's `device_id` must
reference a device row that still exists and belongs to the token's user
(`store.get_device`) — a deleted device's token must not open a channel. Any
deviation (timeout, wrong frame, bad token, unknown device) returns `None`; failures
are logged at `debug` only.

**Dependencies** — `AUTH_TIMEOUT` (this file), `auth::verify_token` (`auth.rs`),
`Store::get_device` (`store.rs`), `serde_json`, `tokio::time::timeout`.

**Used by** — `run_connection` (this file) only.

**Repeated context** — The crate-wide revocation invariant: **every** authenticated
surface (REST `auth_mw`, this handshake, `collab.rs::handler`) re-checks the device
row, so deleting a device revokes its token immediately despite the long
`TOKEN_TTL_DAYS` (365) default.

---

## fn deliver_backlog

**Identification** — private async function; marker `// md:fn deliver_backlog`.

**Code** — complete and verbatim:

```rust
// md:fn deliver_backlog
async fn deliver_backlog(
    state: &AppState,
    socket: &mut WebSocket,
    user_id: Uuid,
    device_id: Uuid,
) -> anyhow::Result<()> {
    let mut cursor = state.store.get_cursor(device_id).await?;
    loop {
        let rows = state
            .store
            .changes_after(user_id, cursor, CHUNK_SIZE)
            .await?;
        if rows.is_empty() {
            return Ok(());
        }
        let last_seq = rows.last().expect("non-empty").seq;
        let deliverable: Vec<&ChangeRow> = rows
            .iter()
            .filter(|r| r.origin_device_id != device_id)
            .collect();
        if !deliverable.is_empty() {
            let frame = changes_frame(deliverable.iter().map(|r| &r.payload));
            socket.send(Message::Text(frame)).await?;
        }
        state.store.advance_cursor(device_id, last_seq).await?;
        cursor = last_seq;
    }
}
```

**What it does** — Streams every journal row the device has not passed yet, in
`CHUNK_SIZE` chunks, from the device's durable cursor (`get_cursor` →
`changes_after`). Per chunk: filter out rows whose `origin_device_id` is this device
(never echo — but they still advance the cursor, so a push-only device's scans stay
cheap), serialise the rest into one `changes_frame`, send it, and **only then**
`advance_cursor` to the chunk's last `seq` — if the socket died mid-chunk, the next
connection re-delivers from the previous cursor (duplicate-over-loss). Returns when
a scan comes back empty.

**Dependencies** — `CHUNK_SIZE`, `changes_frame` (this file);
`Store::{get_cursor, changes_after, advance_cursor}`, `ChangeRow` (`store.rs`).

**Used by** — `run_connection` (initial backlog), `relay_loop` (the `Rescan` and
lag-recovery paths).

**Repeated context** — The cursor (`device_cursors.last_seq`) is the durable
delivery watermark; advancing it only after a successful send is what makes the
guarantee "duplicate delivery, never loss" true. This same cursor is what journal
pruning consults (a row is prunable only when every device's cursor has passed it).

---

## fn relay_loop

**Identification** — private async function; marker `// md:fn relay_loop`.

**Code** — complete and verbatim:

```rust
// md:fn relay_loop
async fn relay_loop(
    state: &AppState,
    socket: &mut WebSocket,
    tx: &broadcast::Sender<FanoutMsg>,
    rx: &mut broadcast::Receiver<FanoutMsg>,
    user_id: Uuid,
    device_id: Uuid,
) -> anyhow::Result<()> {
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.reset();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        handle_incoming(state, tx, user_id, device_id, &text).await?;
                    }
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e.into()),
                }
            }
            _ = ping.tick() => {
                socket.send(Message::Ping(Vec::new())).await?;
            }
            fanned = rx.recv() => {
                match fanned {
                    Ok(FanoutMsg::Batch(batch)) => {
                        if batch.origin != device_id {
                            socket.send(Message::Text(batch.frame.clone())).await?;
                        }
                    }
                    Ok(FanoutMsg::Rescan) => {
                        deliver_backlog(state, socket, user_id, device_id).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(%device_id, skipped, "fan-out lagged; re-scanning journal");
                        deliver_backlog(state, socket, user_id, device_id).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}
```

**What it does** — The steady-state pump, a `tokio::select!` over three sources:

- **Incoming socket frames**: text → `handle_incoming`; `Close`/end-of-stream →
  return `Ok`; ping/pong/binary → ignored; transport error → propagate.
- **Ping tick** (`PING_INTERVAL`, issue #35): send a WebSocket `Ping` — keeps
  NAT/proxy paths open, and a dead peer surfaces as a failed write instead of
  lingering forever.
- **Fan-out channel**: `Batch` → forward unless `origin == device_id` (echo
  suppression); `Rescan` → `deliver_backlog` (a batch landed on another instance;
  idempotent); `Lagged(n)` → warn and `deliver_backlog` (channel overflow degrades
  to a journal re-scan — may re-deliver live batches already sent on this
  connection, which is safe); `Closed` → return `Ok`.

**Dependencies** — `PING_INTERVAL`, `handle_incoming`, `deliver_backlog`,
`FanoutMsg` (this file); `tokio::select!`/`interval`, broadcast errors (external).

**Used by** — `run_connection` (this file) only.

**Repeated context** — Every degraded path (lag, cross-instance, reconnect)
funnels into the same journal re-scan from the durable cursor — one recovery
mechanism, relied on by all failure modes.

---

## fn handle_incoming

**Identification** — private async function; marker `// md:fn handle_incoming`.

**Code** — complete and verbatim:

```rust
// md:fn handle_incoming
async fn handle_incoming(
    state: &AppState,
    tx: &broadcast::Sender<FanoutMsg>,
    user_id: Uuid,
    device_id: Uuid,
    text: &str,
) -> anyhow::Result<()> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            tracing::debug!(%device_id, "ignoring non-JSON frame");
            return Ok(());
        }
    };
    if value["type"] != "changes" {
        return Ok(());
    }
    let changes = match value.get("changes").and_then(|c| c.as_array()) {
        Some(changes) if !changes.is_empty() => changes.clone(),
        _ => return Ok(()),
    };
    let batch_id = value["batch_id"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(Uuid::new_v4);
    let sync_device_id = value["device_id"].as_str().unwrap_or_default();

    let inserted = state
        .store
        .append_changes(user_id, device_id, sync_device_id, batch_id, &changes)
        .await?;
    if inserted.is_empty() {
        tracing::debug!(%device_id, %batch_id, "duplicate batch ignored");
        return Ok(());
    }
    tracing::info!(%user_id, %device_id, %batch_id, count = inserted.len(), "batch persisted");

    materialize(state, user_id, &changes).await;

    let frame = changes_frame(changes.iter());
    let _ = tx.send(FanoutMsg::Batch(Arc::new(FanoutBatch {
        origin: device_id,
        frame,
    })));
    let _ = state
        .store
        .notify(
            crate::bus::CH_SYNC_BATCH,
            &format!("{}:{}", user_id, state.instance_id),
        )
        .await;
    Ok(())
}
```

**What it does** — Parses one incoming text frame. Non-JSON or non-`changes`
envelopes are ignored (logged at `debug`) so future client message types don't kill
the connection; an empty `changes` array is ignored. Then:

1. `batch_id`: the client-minted UUID; absence is tolerated by minting one (such a
   batch merely loses retry-dedup, never data). `device_id` (the client's sync
   actor string) is read as `sync_device_id`.
2. `store.append_changes(user_id, device_id, sync_device_id, batch_id, changes)` —
   journal-first persistence, deduped per user by `(batch_id, batch_index)`.
   An empty insert result = a duplicate re-send of a batch already journaled: it
   was (or will be) delivered from the journal, so it is **not** fanned out twice.
3. `materialize` — upsert the server-owned domain entities carried in the batch
   (idempotent; failures logged, not fatal — the journal still holds the batch for
   relay, and a later change re-converges).
4. Fan out locally: one pre-serialised `changes_frame` in a `FanoutBatch` tagged
   with the origin device; a send error (no other device connected) is ignored —
   they will get the batch from the journal on connect.
5. Cross-instance: `store.notify(CH_SYNC_BATCH, "<user_id>:<instance_id>")` so
   sibling replicas wake this user's devices to re-scan (issue #45); our own bus
   listener ignores it by origin.

**Dependencies** — `materialize`, `changes_frame`, `FanoutBatch`/`FanoutMsg`
(this file); `Store::{append_changes, notify}` (`store.rs`);
`bus::CH_SYNC_BATCH` (`bus.rs`); `serde_json`, `uuid`.

**Used by** — `relay_loop` (this file) only.

**Repeated context** — **Idempotency at every layer**: journal dedup by
`(user, batch_id, batch_index)`; materialisation resolves by version vector (a
re-applied change is a no-op); client `apply_change` is idempotent. That triple is
why every recovery path may freely re-deliver.

---

## fn materialize

**Identification** — private async function; marker `// md:fn materialize`.

**Code** — complete and verbatim:

```rust
// md:fn materialize
async fn materialize(state: &AppState, user_id: Uuid, changes: &[serde_json::Value]) {
    use keeplin_core::models::Change;
    for payload in changes {
        let change: Change = match serde_json::from_value(payload.clone()) {
            Ok(change) => change,
            Err(_) => continue,
        };
        let result = match change {
            Change::NotebookCreate { notebook } | Change::NotebookUpdate { notebook } => state
                .store
                .upsert_notebook(user_id, &notebook)
                .await
                .map(drop),
            Change::NotebookDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => state
                .store
                .delete_notebook(user_id, id, deleted_at, &vv, &last_writer)
                .await
                .map(drop),
            Change::TagCreate { tag } | Change::TagUpdate { tag } => {
                state.store.upsert_tag(user_id, &tag).await.map(drop)
            }
            Change::TagDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => state
                .store
                .delete_tag(user_id, id, deleted_at, &vv, &last_writer)
                .await
                .map(drop),
            Change::NoteTagAdd {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => state
                .store
                .upsert_note_tag(
                    user_id,
                    note_id,
                    tag_id,
                    updated_at,
                    None,
                    &vv,
                    &last_writer,
                )
                .await
                .map(drop),
            Change::NoteTagRemove {
                note_id,
                tag_id,
                updated_at,
                vv,
                last_writer,
            } => state
                .store
                .upsert_note_tag(
                    user_id,
                    note_id,
                    tag_id,
                    updated_at,
                    Some(updated_at),
                    &vv,
                    &last_writer,
                )
                .await
                .map(drop),
            Change::ResourceCreate { resource, data } => {
                match state.store.upsert_resource_meta(user_id, &resource).await {
                    Ok(true) => match data {
                        Some(bytes) => state.store.put_resource_blob(resource.id, &bytes).await,
                        None => Ok(()),
                    },
                    Ok(false) => Ok(()),
                    Err(e) => Err(e),
                }
            }
            Change::ResourceDelete {
                id,
                deleted_at,
                vv,
                last_writer,
            } => state
                .store
                .delete_resource(id, deleted_at, &vv, &last_writer)
                .await
                .map(drop),
            _ => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, %user_id, "materialize: failed to apply change");
        }
    }
}
```

**What it does** — Parses each relayed payload as a keeplin-core `Change` and
materialises the domain entities the server owns, making the server their source of
truth (the client DB is a cache; a wiped device rehydrates from REST). Mapping:

- `NotebookCreate`/`NotebookUpdate` → `store.upsert_notebook`;
  `NotebookDelete` → `store.delete_notebook` (soft-delete: `deleted_at` + vv).
- `TagCreate`/`TagUpdate` → `store.upsert_tag`; `TagDelete` → `store.delete_tag`.
- `NoteTagAdd` → `store.upsert_note_tag(…, deleted_at: None, …)`;
  `NoteTagRemove` → the same upsert with `deleted_at: Some(updated_at)` — the
  association is itself a versioned, soft-deletable entity.
- `ResourceCreate` → `store.upsert_resource_meta`; if the change still carries the
  binary inline (`data: Some`), store it to `resource_blobs` — backward
  compatibility with older clients; new clients upload via
  `PUT /api/resources/:id/data` and send `data: None`. The blob is stored only when
  the meta upsert reports the incoming version won (`Ok(true)`).
- `ResourceDelete` → `store.delete_resource`.
- **`Note*` changes and anything unparseable → skipped**: notes are materialised by
  the collaborative channel, and unknown payloads preserve the opaque-relay
  behaviour. (Silent skipping of *known-but-newer* variants is the drift hazard
  tracked as issue #28.)

Each store call resolves by version vector against the stored row using
keeplin-core's `note_log::resolve`, so the server converges to the **same winner**
every client computes. Failures are logged (`warn`) and the loop continues.

**Dependencies** — `keeplin_core::models::Change` (client repo);
`Store::{upsert_notebook, delete_notebook, upsert_tag, delete_tag, upsert_note_tag,
upsert_resource_meta, put_resource_blob, delete_resource}` (`store.rs`).

**Used by** — `handle_incoming` (this file) only. Exercised by
`tests/materialize.rs`.

**Repeated context** — **Version vectors + LWW, restated**: every materialised
entity row stores `(vv, updated_at/deleted_at, last_writer)`; an incoming change
wins if its vv dominates, loses if dominated, and ties break deterministically by
`(timestamp, actor id)` — so replicas converge without locks. **Soft-delete**:
deletions set `deleted_at` and keep the row (tombstone) so they replicate; REST
serves live rows and the journal serves history.

---

## fn changes_frame

**Identification** — private function; marker `// md:fn changes_frame`.

**Code** — complete and verbatim:

```rust
// md:fn changes_frame
fn changes_frame<'a>(payloads: impl Iterator<Item = &'a serde_json::Value>) -> String {
    serde_json::json!({
        "type": "changes",
        "changes": payloads.collect::<Vec<_>>(),
    })
    .to_string()
}
```

**What it does** — Serialises payloads into the `{"type":"changes","changes":[…]}`
frame the client's `receive_changes` parses. Pure; no failure mode (the payloads are
already valid JSON values).

**Dependencies** — `serde_json`.

**Used by** — `deliver_backlog` and `handle_incoming` (this file).

**Repeated context** — This envelope is half of the relay's wire contract (the other
half is the incoming `changes` envelope parsed in `handle_incoming`); the client
mirror lives in `keeplin-core/src/storage/db.rs`.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `authenticate()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `SyncHub` — defined here (EXTRACTED; 1 cross-file edge(s))
- `handler()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `run_connection()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `deliver_backlog()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `relay_loop()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `handle_incoming()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `materialize()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `FanoutBatch` — defined here (EXTRACTED; file-local)
- `FanoutMsg` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×7; e.g. `AppState`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×1; e.g. `UserDevice`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)

## Coverage checklist

Every code block of `sync.rs`, in source order, each documented above (five points)
and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `CHUNK_SIZE` / `AUTH_TIMEOUT` / `PING_INTERVAL` / `FANOUT_CAPACITY` | `// md:Constants` | Constants |
| 3 | `struct FanoutBatch` | `// md:FanoutBatch` | FanoutBatch |
| 4 | `enum FanoutMsg` | `// md:FanoutMsg` | FanoutMsg |
| 5 | `struct SyncHub` | `// md:SyncHub` | SyncHub |
| 6 | `impl SyncHub` | `// md:impl SyncHub` | impl SyncHub |
| 7 | `fn live_users` | `// md:impl SyncHub > fn live_users` | impl SyncHub › fn live_users |
| 8 | `fn wake_user` | `// md:impl SyncHub > fn wake_user` | impl SyncHub › fn wake_user |
| 9 | `fn join` | `// md:impl SyncHub > fn join` | impl SyncHub › fn join |
| 10 | `fn leave` | `// md:impl SyncHub > fn leave` | impl SyncHub › fn leave |
| 11 | `fn handler` | `// md:fn handler` | fn handler |
| 12 | `fn run_connection` | `// md:fn run_connection` | fn run_connection |
| 13 | `fn authenticate` | `// md:fn authenticate` | fn authenticate |
| 14 | `fn deliver_backlog` | `// md:fn deliver_backlog` | fn deliver_backlog |
| 15 | `fn relay_loop` | `// md:fn relay_loop` | fn relay_loop |
| 16 | `fn handle_incoming` | `// md:fn handle_incoming` | fn handle_incoming |
| 17 | `fn materialize` | `// md:fn materialize` | fn materialize |
| 18 | `fn changes_frame` | `// md:fn changes_frame` | fn changes_frame |
