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

// md:Constants
const CHUNK_SIZE: i64 = 200;
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(30);
const FANOUT_CAPACITY: usize = 256;

// md:FanoutBatch
pub struct FanoutBatch {
    pub origin: Uuid,
    pub frame: String,
}

// md:FanoutMsg
#[derive(Clone)]
pub enum FanoutMsg {
    Batch(Arc<FanoutBatch>),
    Rescan,
}

// md:SyncHub
#[derive(Default)]
pub struct SyncHub {
    channels: RwLock<HashMap<Uuid, broadcast::Sender<FanoutMsg>>>,
}

// md:impl SyncHub
impl SyncHub {
    // md:impl SyncHub > fn live_users
    pub async fn live_users(&self) -> usize {
        self.channels.read().await.len()
    }

    // md:impl SyncHub > fn wake_user
    pub async fn wake_user(&self, user_id: Uuid) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(&user_id) {
            let _ = tx.send(FanoutMsg::Rescan);
        }
    }

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

    // md:impl SyncHub > fn leave
    async fn leave(&self, user_id: Uuid) {
        let mut channels = self.channels.write().await;
        if let Some(tx) = channels.get(&user_id) {
            if tx.receiver_count() == 0 {
                channels.remove(&user_id);
            }
        }
    }
}

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

// md:fn changes_frame
fn changes_frame<'a>(payloads: impl Iterator<Item = &'a serde_json::Value>) -> String {
    serde_json::json!({
        "type": "changes",
        "changes": payloads.collect::<Vec<_>>(),
    })
    .to_string()
}
