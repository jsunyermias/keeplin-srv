//! The collaborative-editing engine: per-note sessions, presence, and the
//! application of [`LineOp`]s against the versioned line + order entities.
//!
//! The server is the broker and the durable source of truth (design §2.4): it
//! validates each operation, resolves it against current state with
//! `note_log::resolve`, persists it, and fans it out to the note's other
//! subscribers with a monotonically increasing `server_seq`. Clients are
//! stateful: they keep their own copy and rebuild from the `Welcome` snapshot
//! on (re)connect — there is no infinite op log.
//!
//! Conflict rules (design §5):
//! - per line: `resolve(local, incoming)`; the op is applied iff the incoming
//!   write wins (causally newer, or concurrent and winning the deterministic
//!   `(timestamp, writer)` tiebreak). A dominated op is silently ignored.
//! - per order (`Insert`/`Move`): the same resolution against the note's
//!   order entity; the applied op merges its vector into the order's.
//! - No locks anywhere: resolution is always by version vector.

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

/// Design limits (§11.1).
const MAX_LINE_LEN: usize = 10_000;
const MAX_LINES_PER_NOTE: usize = 100_000;
const MAX_WS_MESSAGE: usize = 1024 * 1024;

/// Bounded outbound queue per connection: a slow/stalled consumer is dropped rather than
/// buffering without limit (issue #34). A stateful client rebuilds from the next `Welcome`
/// snapshot on reconnect, so dropping it is safe.
const OUTBOUND_CAPACITY: usize = 256;
/// How often the server pings an idle connection to keep NAT/proxy paths open and to detect a
/// dead peer promptly (issue #35).
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// If no frame at all (not even a pong to our ping) arrives within this window, the peer is
/// treated as dead and the connection is closed (issue #35).
const ACTIVITY_TIMEOUT: Duration = Duration::from_secs(90);

// ── Sessions ─────────────────────────────────────────────────────────────────

struct Subscriber {
    tx: mpsc::Sender<String>,
}

/// One live collaborative session per note with at least one subscriber
/// (design §3.5). Created on demand, destroyed when the last subscriber
/// leaves. If the server restarts, clients reconnect and get a fresh snapshot
/// from the database — the session itself holds no durable state.
pub struct CollabSession {
    /// Monotonic per-session sequence stamped on each fanned-out `Op` by *this*
    /// instance; a connection only ever talks to one instance, so a per-instance
    /// counter is enough for the client's ordering (issue #45).
    seq: AtomicU64,
    /// Serialises op application and join snapshots for this note, so a
    /// joiner can never miss an op between reading the snapshot and
    /// subscribing, and two ops never interleave their read-modify-write.
    apply_lock: Mutex<()>,
    subscribers: RwLock<HashMap<u64, Subscriber>>,
}

#[derive(Default)]
pub struct CollabRegistry {
    sessions: RwLock<HashMap<Uuid, Arc<CollabSession>>>,
    next_conn_id: AtomicU64,
}

impl CollabRegistry {
    /// (live note sessions, live subscriber connections) for `/api/metrics`.
    pub async fn stats(&self) -> (usize, usize) {
        let sessions = self.sessions.read().await;
        let mut connections = 0;
        for session in sessions.values() {
            connections += session.subscribers.read().await.len();
        }
        (sessions.len(), connections)
    }

    /// The live session for a note on *this* instance, if any. Used by the
    /// cross-instance bus to deliver a sibling's op/presence to local
    /// subscribers (issue #45).
    pub async fn get(&self, note_id: Uuid) -> Option<Arc<CollabSession>> {
        self.sessions.read().await.get(&note_id).cloned()
    }

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

    async fn drop_if_empty(&self, note_id: Uuid) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get(&note_id) {
            if session.subscribers.read().await.is_empty() {
                sessions.remove(&note_id);
            }
        }
    }
}

impl CollabSession {
    /// Send `msg` to every subscriber, optionally skipping one connection
    /// (the originator of an op already has it applied locally).
    ///
    /// A subscriber whose bounded outbound queue is full (a slow/stalled consumer) is dropped
    /// rather than allowed to buffer without bound (issue #34); it reconnects and rebuilds
    /// from a fresh snapshot.
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
}

/// Publish a note's merged presence (across every instance) to another instance
/// so all replicas broadcast the same list. Records/removes this connection's row
/// and fires a `collab_presence` notification; the actual broadcast to local
/// subscribers happens in [`deliver_presence`] when the notification comes back.
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

async fn clear_presence(state: &AppState, note_id: Uuid, conn_id: u64) -> Result<(), AppError> {
    state
        .store
        .delete_presence(note_id, state.instance_id, conn_id as i64)
        .await?;
    announce_presence(state, note_id).await;
    Ok(())
}

/// Broadcast the merged presence to this instance's subscribers now, and notify
/// the other instances to do the same. The local broadcast means presence works
/// with a single instance even if the bus is not running; the notification
/// carries our instance id so a sibling's bus handler skips it back to us (we
/// already broadcast it) — see [`deliver_presence`] and `bus`.
async fn announce_presence(state: &AppState, note_id: Uuid) {
    deliver_presence(state, note_id).await;
    let payload = format!("{}:{}", note_id, state.instance_id);
    if let Err(e) = state.store.notify("collab_presence", &payload).await {
        tracing::warn!(error = %e, %note_id, "presence notify failed");
    }
}

/// Bus entrypoint: a `collab_presence` notification for `note_id`. If this
/// instance has a live session for the note, rebuild the merged presence list
/// from the shared table and broadcast it to the local subscribers (issue #45).
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

/// Bus entrypoint: a `collab_op` event authored by *another* instance. Deliver
/// its ops to this instance's local subscribers of the note, stamped with this
/// instance's own session sequence (issue #45). Events this instance authored
/// are filtered out by the caller (they were already broadcast locally).
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
    // Serialise against a concurrent local join so a just-subscribed connection
    // cannot miss this op between its snapshot and subscribe (the op is already
    // durable, so a duplicate delivery here is resolved away by the client).
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

// ── Connection handling ──────────────────────────────────────────────────────

/// `GET /api/ws?token=<jwt>` — the collaborative channel (design §7.1). The
/// token authenticates the *user*; which notes the connection may touch is
/// checked per `Join` against the note's shares.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    // Prefer the Authorization header — a token in the query string ends up
    // in proxy/access logs. The `?token=` form stays as a fallback.
    let header_token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    let token = header_token
        .or(params.get("token").map(String::as_str))
        .ok_or(AppError::MissingToken)?;
    let authed = auth::verify_token(token, &state.config.jwt_secret)?;
    // Deleting a device revokes its token immediately: the collaborative channel must
    // re-check that the token's device still exists and belongs to the same user, exactly
    // like the REST middleware and the sync relay do — otherwise a revoked token keeps
    // editing notes until it expires (issue #20).
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

async fn run_connection(
    state: Arc<AppState>,
    socket: WebSocket,
    user_id: Uuid,
    device_id: Uuid,
    display_name: String,
) {
    let conn_id = state.collab.next_conn_id.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = socket.split();

    // All outbound traffic (welcomes, fan-out, presence, errors) funnels through one bounded
    // channel so a single task owns the sink (issue #34). The writer also emits periodic pings
    // to keep the connection alive and surface a dead peer via a failed write (issue #35).
    let (tx, mut rx) = mpsc::channel::<String>(OUTBOUND_CAPACITY);
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.reset(); // first tick after PING_INTERVAL, not immediately
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

    // Notes this connection has joined. Access is re-resolved per operation (issue #30), not
    // cached here, so a revoked share takes effect without waiting for a reconnect.
    let mut joined: HashMap<Uuid, Arc<CollabSession>> = HashMap::new();

    loop {
        // Bound the wait so a peer that has gone silent — not even answering our pings — is
        // dropped instead of leaking a subscriber slot forever (issue #35). Any frame,
        // including a pong, counts as activity and resets the window.
        let msg = match tokio::time::timeout(ACTIVITY_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(msg))) => msg,
            // Timeout, stream end, or transport error: close the connection.
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

    // Disconnect: leave every joined session and drop this connection's shared
    // presence rows (issue #45).
    for (note_id, session) in joined {
        session.subscribers.write().await.remove(&conn_id);
        let _ = clear_presence(&state, note_id, conn_id).await;
        state.collab.drop_if_empty(note_id).await;
    }
    writer.abort();
}

fn send_error(tx: &mpsc::Sender<String>, code: &str, message: &str) {
    let msg = CollabServerMsg::Error {
        code: code.into(),
        message: message.into(),
    };
    let _ = tx.try_send(serde_json::to_string(&msg).expect("serializable error"));
}

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
            // Snapshot, subscribe, and enqueue the Welcome all under the apply
            // lock, so (a) no op can slip between the snapshot and the
            // subscription — local ops and cross-instance ops both take this
            // lock before broadcasting — and (b) the Welcome is queued on the
            // connection's channel before any op can be, keeping Welcome first.
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
            // `access` was only needed to gate the join (read check); it is intentionally not
            // stored — writes re-resolve access on every op batch (issue #30).
            let _ = access;
            joined.insert(note_id, session.clone());

            // Presence is shared across instances: record this connection and
            // notify, so every replica rebroadcasts the merged list (issue #45).
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
            // Re-resolve access on every op batch so a share revoked mid-session takes effect
            // immediately, rather than persisting for the life of the connection (issue #30).
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

            // Apply sequentially under the note's in-process lock (local join
            // ordering) *and* a Postgres advisory lock keyed by the note, so two
            // instances editing the same order serialise at the database and
            // cannot lose an update (issue #45). Keep only the ops that won.
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
                lock_tx.commit().await?; // release the advisory lock
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
            // Fan the applied batch out to *other* instances' subscribers via the
            // outbox + NOTIFY (issue #45). Done after the local broadcast so local
            // latency is unaffected; siblings deliver it under their own sequence.
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

        // Client-side bookkeeping only; nothing to do server-side.
        CollabClientMsg::Ack { .. } => {}
    }
    Ok(())
}

// ── Snapshot ─────────────────────────────────────────────────────────────────

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

// ── Op application ───────────────────────────────────────────────────────────

enum OpOutcome {
    /// The op won its resolution and was persisted: fan it out.
    Applied(LineOp),
    /// The op was dominated by current state (design §4.3.3): drop silently.
    Ignored,
    /// The op was malformed or referenced missing entities: tell the sender.
    Invalid { code: String, message: String },
}

fn invalid(code: &str, message: impl Into<String>) -> OpOutcome {
    OpOutcome::Invalid {
        code: code.into(),
        message: message.into(),
    }
}

/// Pointwise maximum of two version vectors — the merged causal frontier
/// stored on an entity after an op is applied.
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

/// Design §4.3.5: an op's vector must advance its writer's own component past
/// the entity's current one. (Replays of an already-applied op fail this and
/// are ignored, which keeps application idempotent.)
fn advances_writer(current: &VersionVector, op_vv: &VersionVector, writer: &str) -> bool {
    op_vv.get(writer).copied().unwrap_or(0) > current.get(writer).copied().unwrap_or(0)
}

/// Apply one op. All reads and writes go through `conn` — the connection that
/// holds the note's advisory lock — so the whole batch runs on a single
/// connection and cannot deadlock against the bounded pool, and the order's
/// read-modify-write is serialised across instances (issue #45).
async fn apply_op(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    note_id: Uuid,
    device_id: Uuid,
    op: LineOp,
) -> Result<OpOutcome, AppError> {
    // The op's writer must be the authenticated *device* (from the token) —
    // clients cannot forge edits in someone else's name, and two devices of
    // the same user never share a version-vector component (sharing one would
    // make the server treat concurrent edits from the second device as
    // replays). Presence stays user-based; only the vv actor is the device.
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
            // The order is a versioned entity of its own (design §5.2): a
            // stale insert loses against the current order and is dropped.
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

            // Extract the moved block, then reinsert it after the target.
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

/// Resolve an op against the order entity. `Winner::Incoming` = apply.
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

/// Resolve an op against a line entity. `Winner::Incoming` = apply.
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

/// Index right after `after_line_id` in `order` (`None` = the beginning).
/// Returns `None` when the anchor line is absent.
fn position_after(order: &[Uuid], after_line_id: Option<Uuid>) -> Option<usize> {
    match after_line_id {
        None => Some(0),
        Some(after) => order.iter().position(|id| *id == after).map(|i| i + 1),
    }
}
