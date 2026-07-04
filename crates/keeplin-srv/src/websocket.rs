use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::{extract::Query, extract::State, response::Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{error, trace};
use uuid::Uuid;

use crate::{
    auth::{verify_token, AuthedUser},
    error::AppError,
    lines::{self, ApplyOutcome, LineMove},
    permissions::resolve_role,
    protocol::{ClientMessage, LineSnapshot, ServerMessage, UserInfo},
    state::{AppState, Rooms},
    store::Store,
};

#[derive(Debug, Deserialize)]
pub struct WsParams {
    token: String,
    note_id: String,
}

pub struct Room {
    pub note_id: Uuid,
    pub subscribers: Mutex<Vec<mpsc::UnboundedSender<String>>>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
    Query(params): Query<WsParams>,
) -> Result<Response, AppError> {
    let user = verify_token(&params.token, &state.config.jwt_secret)?;
    let note_id: Uuid = params
        .note_id
        .parse()
        .map_err(|_| AppError::BadRequest("invalid note_id".into()))?;

    let note = state.store.get_note(note_id).await?.ok_or(AppError::NotFound)?;
    resolve_role(&state.store, &note, user.user_id).await?;

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, note_id, user)))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>, note_id: Uuid, user: AuthedUser) {
    let room = get_or_create_room(&state.rooms, note_id).await;
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    {
        room.subscribers.lock().unwrap().push(tx.clone());
    }

    // Send snapshot.
    if let Ok(snapshot) = build_snapshot(&state.store, note_id).await {
        let msg = serde_json::to_string(&snapshot).unwrap_or_default();
        let _ = tx.send(msg);
    }

    let (mut sender, mut receiver) = socket.split();

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    while let Some(result) = receiver.next().await {
        match result {
            Ok(Message::Text(text)) => {
                if let Err(e) =
                    handle_message(&room, &state, note_id, &user, &text, &tx).await
                {
                    error!(?e, "websocket message error");
                    let reject = ServerMessage::Rejected {
                        reason: e.to_string(),
                    };
                    let _ = tx.send(serde_json::to_string(&reject).unwrap_or_default());
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }

    {
        let mut subs = room.subscribers.lock().unwrap();
        subs.retain(|s| !s.same_channel(&tx));
    }

    send_task.abort();

    let should_remove = {
        let subs = room.subscribers.lock().unwrap();
        subs.is_empty()
    };
    if should_remove {
        state.rooms.write().await.remove(&note_id);
    }
}

async fn get_or_create_room(rooms: &Rooms, note_id: Uuid) -> Arc<Room> {
    {
        let map = rooms.read().await;
        if let Some(room) = map.get(&note_id) {
            return room.clone();
        }
    }

    let mut map = rooms.write().await;
    // Double-check after acquiring write lock.
    if let Some(room) = map.get(&note_id) {
        return room.clone();
    }

    let room = Arc::new(Room {
        note_id,
        subscribers: Mutex::new(Vec::new()),
    });
    map.insert(note_id, room.clone());
    room
}

async fn build_snapshot(store: &Store, note_id: Uuid) -> Result<ServerMessage, AppError> {
    let rows = store.list_note_lines_active(note_id).await?;
    let lines = rows
        .into_iter()
        .map(|(nl, line)| LineSnapshot {
            line_id: line.id,
            position: nl.position,
            content: line.content,
            vv: line.vv.0,
            last_writer: line.last_writer,
            updated_at: line.updated_at,
        })
        .collect();
    Ok(ServerMessage::Snapshot { note_id, lines })
}

async fn handle_message(
    room: &Room,
    state: &Arc<AppState>,
    note_id: Uuid,
    user: &AuthedUser,
    text: &str,
    origin: &mpsc::UnboundedSender<String>,
) -> Result<(), AppError> {
    let msg: ClientMessage =
        serde_json::from_str(text).map_err(|e| AppError::BadRequest(format!("invalid message: {}", e)))?;

    // Verify the message targets the connected note.
    let msg_note_id = match &msg {
        ClientMessage::InsertLine { note_id, .. } => *note_id,
        ClientMessage::UpdateLine { note_id, .. } => *note_id,
        ClientMessage::DeleteLine { note_id, .. } => *note_id,
        ClientMessage::MoveLines { note_id, .. } => *note_id,
        ClientMessage::Cursor { note_id, .. } => *note_id,
        ClientMessage::Presence { note_id, .. } => *note_id,
    };
    if msg_note_id != note_id {
        return Err(AppError::BadRequest("note_id mismatch".into()));
    }

    // Re-check permissions for writes.
    let note = state.store.get_note(note_id).await?.ok_or(AppError::NotFound)?;
    let role = resolve_role(&state.store, &note, user.user_id).await?;

    match msg {
        ClientMessage::InsertLine {
            note_id,
            line_id,
            after_line_id,
            content,
            vv,
            device_id,
            ts,
        } => {
            if !role.can_write() {
                return Err(AppError::Forbidden);
            }
            validate_device(&state.store, user, &device_id).await?;
            let (line, note_line) = lines::insert_line(
                &state.store,
                note_id,
                after_line_id,
                &content,
                &vv,
                &device_id,
            )
            .await?;
            let server_msg = ServerMessage::InsertLine {
                note_id,
                line_id,
                after_line_id,
                content: line.content,
                vv: line.vv.0,
                device_id: line.last_writer,
                ts: line.updated_at,
            };
            broadcast(room, &server_msg, Some(origin));
            let _ = note_line;
            let _ = ts;
        }
        ClientMessage::UpdateLine {
            note_id,
            line_id,
            content,
            vv,
            device_id,
            ts,
        } => {
            if !role.can_write() {
                return Err(AppError::Forbidden);
            }
            validate_device(&state.store, user, &device_id).await?;
            match lines::update_line(
                &state.store,
                note_id,
                line_id,
                &content,
                &vv,
                ts,
                &device_id,
            )
            .await?
            {
                ApplyOutcome::Applied(line) => {
                    let server_msg = ServerMessage::UpdateLine {
                        note_id,
                        line_id,
                        content: line.content,
                        vv: line.vv.0,
                        device_id: line.last_writer,
                        ts: line.updated_at,
                    };
                    broadcast(room, &server_msg, Some(origin));
                }
                ApplyOutcome::Rejected { current } => {
                    let reject = ServerMessage::UpdateLine {
                        note_id,
                        line_id,
                        content: current.content,
                        vv: current.vv.0,
                        device_id: current.last_writer,
                        ts: current.updated_at,
                    };
                    let _ = origin.send(serde_json::to_string(&reject).unwrap_or_default());
                }
            }
        }
        ClientMessage::DeleteLine {
            note_id,
            line_id,
            vv,
            device_id,
            ts,
        } => {
            if !role.can_write() {
                return Err(AppError::Forbidden);
            }
            validate_device(&state.store, user, &device_id).await?;
            match lines::delete_line(
                &state.store,
                note_id,
                line_id,
                &vv,
                ts,
                &device_id,
            )
            .await?
            {
                ApplyOutcome::Applied(line) => {
                    let server_msg = ServerMessage::DeleteLine {
                        note_id,
                        line_id,
                        vv: line.vv.0,
                        device_id: line.last_writer,
                        ts: line.updated_at,
                    };
                    broadcast(room, &server_msg, Some(origin));
                }
                ApplyOutcome::Rejected { current } => {
                    let reject = ServerMessage::UpdateLine {
                        note_id,
                        line_id,
                        content: current.content,
                        vv: current.vv.0,
                        device_id: current.last_writer,
                        ts: current.updated_at,
                    };
                    let _ = origin.send(serde_json::to_string(&reject).unwrap_or_default());
                }
            }
        }
        ClientMessage::MoveLines {
            note_id,
            moves,
            vv,
            device_id,
            ts,
        } => {
            if !role.can_write() {
                return Err(AppError::Forbidden);
            }
            validate_device(&state.store, user, &device_id).await?;
            let moves: Vec<LineMove> = moves
                .into_iter()
                .map(|m| LineMove {
                    line_id: m.line_id,
                    after_line_id: m.after_line_id,
                })
                .collect();
            let outcomes = lines::move_lines(&state.store, note_id, &moves, &vv, ts, &device_id).await?;
            let mut applied_moves = Vec::new();
            let mut last_vv = vv.clone();
            let mut last_ts = ts;
            for outcome in outcomes {
                match outcome {
                    ApplyOutcome::Applied((nl, after)) => {
                        applied_moves.push(crate::protocol::LineMoveMsg {
                            line_id: nl.line_id,
                            after_line_id: after,
                        });
                        last_vv = nl.vv.0;
                        last_ts = nl.updated_at;
                    }
                    ApplyOutcome::Rejected { .. } => {
                        trace!("move_lines rejected");
                    }
                }
            }
            if !applied_moves.is_empty() {
                let server_msg = ServerMessage::MoveLines {
                    note_id,
                    moves: applied_moves,
                    vv: last_vv,
                    device_id: device_id.clone(),
                    ts: last_ts,
                };
                broadcast(room, &server_msg, Some(origin));
            }
        }
        ClientMessage::Cursor { line_id, column, .. } => {
            let server_msg = ServerMessage::Cursor {
                note_id,
                line_id,
                column,
                user: UserInfo {
                    id: user.user_id,
                    email: user.email.clone(),
                },
            };
            broadcast(room, &server_msg, Some(origin));
        }
        ClientMessage::Presence { status, .. } => {
            let server_msg = ServerMessage::Presence {
                note_id,
                status,
                user: UserInfo {
                    id: user.user_id,
                    email: user.email.clone(),
                },
            };
            broadcast(room, &server_msg, Some(origin));
        }
    }

    Ok(())
}

async fn validate_device(
    store: &Store,
    user: &AuthedUser,
    device_id: &str,
) -> Result<(), AppError> {
    let id: Uuid = device_id
        .parse()
        .map_err(|_| AppError::BadRequest("invalid device_id".into()))?;
    if id == user.device_id {
        return Ok(());
    }
    let device = store.get_device(id).await?;
    match device {
        Some(d) if d.user_id == user.user_id => Ok(()),
        _ => Err(AppError::Forbidden),
    }
}

fn broadcast(
    room: &Room,
    msg: &ServerMessage,
    exclude: Option<&mpsc::UnboundedSender<String>>,
) {
    let text = serde_json::to_string(msg).unwrap_or_default();
    if text.is_empty() {
        return;
    }
    let subs = room.subscribers.lock().unwrap();
    for tx in subs.iter() {
        if let Some(ex) = exclude {
            if tx.same_channel(ex) {
                continue;
            }
        }
        let _ = tx.send(text.clone());
    }
}
