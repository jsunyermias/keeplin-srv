//! The WebSocket sync relay — the server side of keeplin-core's `DbBackend`
//! wire protocol.
//!
//! Protocol (exactly what `DbBackend::connect_ws` / `send_changes` /
//! `receive_changes` speak):
//!
//! 1. The client connects and immediately sends the handshake frame
//!    `{"type":"auth","token":"<jwt>"}`. The token is a device token issued by
//!    `POST /api/login`; it identifies both the user and the device.
//! 2. The client pushes batches as
//!    `{"type":"changes","batch_id":"…","device_id":"…","changes":[Change…]}`.
//! 3. The server sends batches to the client as
//!    `{"type":"changes","changes":[Change…]}` — first the backlog the device
//!    has not seen yet, then live batches from the user's other devices as
//!    they arrive. A device never receives its own batches back.
//!
//! `Change` payloads are treated as **opaque JSON**: the relay stores and
//! forwards them without interpreting keeplin-core's model, so client-side
//! model evolution never requires a server change.
//!
//! Delivery guarantees: every accepted batch is persisted to the journal
//! before it is fanned out, and each device has a durable delivery cursor that
//! only advances after a successful send. Because `apply_change` on the client
//! is idempotent, the relay prefers duplicate delivery over loss — a
//! reconnecting device may re-receive changes that were already forwarded live
//! on a previous connection, and that is safe.

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

/// How many changes go into one outgoing `{"type":"changes"}` frame. The
/// client caps one `receive_changes` call at 1 000 frames, so even a huge
/// backlog drains quickly at this chunk size.
const CHUNK_SIZE: i64 = 200;

/// How long the server waits for the `auth` handshake frame before dropping
/// the connection.
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the relay pings an idle connection to keep it alive and surface a dead peer
/// through a failed write (issue #35).
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Capacity of each per-user broadcast channel. A receiver that lags behind
/// (slow consumer) falls back to a journal backlog scan, so overflow degrades
/// to duplicate delivery, never to loss.
const FANOUT_CAPACITY: usize = 256;

/// A batch already persisted to the journal, fanned out to the user's live
/// connections. The frame is pre-serialised once; `origin` lets each
/// connection drop its own batches.
pub struct FanoutBatch {
    pub origin: Uuid,
    pub frame: String,
}

/// What travels on a user's fan-out channel. `Batch` is a live batch from a
/// device on *this* instance; `Rescan` is a wake from the cross-instance bus
/// telling connections to re-scan the journal because a batch landed on another
/// replica (issue #45).
#[derive(Clone)]
pub enum FanoutMsg {
    Batch(Arc<FanoutBatch>),
    Rescan,
}

/// Per-user fan-out: one broadcast channel per user with at least one device
/// connected. Senders are dropped lazily when a user's last connection closes.
#[derive(Default)]
pub struct SyncHub {
    channels: RwLock<HashMap<Uuid, broadcast::Sender<FanoutMsg>>>,
}

impl SyncHub {
    /// Number of users with at least one live relay connection.
    pub async fn live_users(&self) -> usize {
        self.channels.read().await.len()
    }

    /// Wake a user's local relay connections to re-scan the journal, because a
    /// batch was appended for them on another instance (issue #45). No-op if the
    /// user has no live connection here.
    pub async fn wake_user(&self, user_id: Uuid) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(&user_id) {
            let _ = tx.send(FanoutMsg::Rescan);
        }
    }

    /// Subscribe to a user's fan-out channel, creating it if needed. Returns
    /// the sender (to publish) and a fresh receiver (to consume).
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

    /// Drop the user's channel if no receiver is listening any more.
    async fn leave(&self, user_id: Uuid) {
        let mut channels = self.channels.write().await;
        if let Some(tx) = channels.get(&user_id) {
            if tx.receiver_count() == 0 {
                channels.remove(&user_id);
            }
        }
    }
}

/// `GET /api/sync` — upgrade to WebSocket and run the relay loop.
pub async fn handler(State(state): State<Arc<AppState>>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(64 * 1024 * 1024)
        .max_frame_size(16 * 1024 * 1024)
        .on_upgrade(move |socket| async move {
            if let Err(e) = run_connection(state, socket).await {
                tracing::debug!(error = %e, "sync connection ended with error");
            }
        })
}

async fn run_connection(state: Arc<AppState>, mut socket: WebSocket) -> anyhow::Result<()> {
    // ── Handshake ────────────────────────────────────────────────────────────
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

    // Subscribe *before* the backlog scan: anything persisted after the scan's
    // snapshot arrives through the channel, so the two phases cannot leave a gap
    // (overlap is possible and safe — the client applies idempotently).
    let (tx, mut rx) = state.hub.join(user_id).await;

    // ── Backlog ──────────────────────────────────────────────────────────────
    if let Err(e) = deliver_backlog(&state, &mut socket, user_id, device_id).await {
        state.hub.leave(user_id).await;
        return Err(e);
    }

    // ── Relay loop ───────────────────────────────────────────────────────────
    let result = relay_loop(&state, &mut socket, &tx, &mut rx, user_id, device_id).await;

    state.store.touch_device(device_id).await.ok();
    state.hub.leave(user_id).await;
    tracing::info!(%user_id, %device_id, "sync device disconnected");
    result
}

/// Wait for the `{"type":"auth","token":…}` frame and resolve it to a device.
/// Any deviation — timeout, wrong frame, bad token, unknown device — returns
/// `None` and the connection is closed without an error response (the client
/// treats the closure as "reconnect later").
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
    // The token must reference a device that still exists and belongs to the
    // token's user (a deleted device's token must not open a channel).
    match state.store.get_device(claims.device_id).await {
        Ok(Some(device)) if device.user_id == claims.user_id => Some(device),
        _ => {
            tracing::debug!(device_id = %claims.device_id, "sync token for unknown device");
            None
        }
    }
}

/// Stream every journal row the device has not passed yet, in chunks, and
/// advance the durable cursor after each successfully sent chunk. Rows that
/// originated from this device are skipped (never echo) but still advance the
/// cursor, so a push-only device's scans stay cheap.
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
        // Only advance after the send succeeded: if the socket died mid-chunk,
        // the next connection re-delivers from the previous cursor.
        state.store.advance_cursor(device_id, last_seq).await?;
        cursor = last_seq;
    }
}

/// Pump incoming batches into the journal + fan-out, and fan-out batches from
/// other devices down this socket, until the connection closes.
async fn relay_loop(
    state: &AppState,
    socket: &mut WebSocket,
    tx: &broadcast::Sender<FanoutMsg>,
    rx: &mut broadcast::Receiver<FanoutMsg>,
    user_id: Uuid,
    device_id: Uuid,
) -> anyhow::Result<()> {
    // Periodic pings keep NAT/proxy paths open and surface a dead peer via a failed write,
    // so a silently-dropped connection is reaped instead of lingering (issue #35).
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
                    Some(Ok(_)) => {} // ping/pong/binary — ignore
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
                    // A batch landed on another instance: re-scan the journal from
                    // our durable cursor to pick it up (issue #45). Idempotent.
                    Ok(FanoutMsg::Rescan) => {
                        deliver_backlog(state, socket, user_id, device_id).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // Overflowed the channel: fall back to a journal scan
                        // from the durable cursor. May re-deliver live batches
                        // already sent on this connection — safe (idempotent).
                        tracing::warn!(%device_id, skipped, "fan-out lagged; re-scanning journal");
                        deliver_backlog(state, socket, user_id, device_id).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

/// Parse one incoming text frame. Only `{"type":"changes"}` envelopes are
/// meaningful; anything else is ignored so future client message types don't
/// kill the connection.
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
    // The client generates a UUID batch_id; tolerate absence by minting one
    // (such a batch simply loses retry-dedup, never the data).
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
        // Duplicate re-send of a batch we already have: it was (or will be)
        // delivered from the journal; do not fan it out twice.
        tracing::debug!(%device_id, %batch_id, "duplicate batch ignored");
        return Ok(());
    }
    tracing::info!(%user_id, %device_id, %batch_id, count = inserted.len(), "batch persisted");

    // Materialise the domain entities carried in this batch so the server is
    // their source of truth (the client DB is a cache). Idempotent: resolution
    // makes a re-applied change a no-op. Failures are logged, not fatal — the
    // journal still holds the batch for relay, and a later change re-converges.
    materialize(state, user_id, &changes).await;

    let frame = changes_frame(changes.iter());
    // Ignore the error: no other device is connected right now; they will get
    // the batch from the journal when they connect.
    let _ = tx.send(FanoutMsg::Batch(Arc::new(FanoutBatch {
        origin: device_id,
        frame,
    })));
    // Tell sibling instances a batch landed so they wake this user's devices to
    // re-scan the journal (issue #45). Our own listener ignores this by origin.
    let _ = state
        .store
        .notify(
            crate::bus::CH_SYNC_BATCH,
            &format!("{}:{}", user_id, state.instance_id),
        )
        .await;
    Ok(())
}

/// Parse each relayed payload as a keeplin-core `Change` and materialise the
/// domain entities the server owns (notebooks, tags, note↔tag associations,
/// resource metadata). Note changes are handled by the collaborative channel
/// (`/api/ws`), not here; anything that does not parse is ignored (opaque relay
/// behaviour preserved for entities the server does not model).
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
                    // Backward compatibility: an older client that still ships
                    // the binary inside the change gets it stored here. New
                    // clients upload via `PUT /api/resources/:id/data` and send
                    // `data: None`.
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
            // Note* changes are materialised by the collaborative channel.
            _ => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, %user_id, "materialize: failed to apply change");
        }
    }
}

/// Serialise payloads into the `{"type":"changes","changes":[…]}` frame the
/// client's `receive_changes` parses.
fn changes_frame<'a>(payloads: impl Iterator<Item = &'a serde_json::Value>) -> String {
    serde_json::json!({
        "type": "changes",
        "changes": payloads.collect::<Vec<_>>(),
    })
    .to_string()
}
