//! Cross-instance coordination bus (issue #45).
//!
//! When the server runs more than one replica, the collaborative channel and the
//! device relay must reach subscribers connected to *other* instances. Instances
//! coordinate over Postgres `LISTEN/NOTIFY` — no extra infrastructure beyond the
//! database they already share.
//!
//! Channels:
//! - `collab_op` — payload `"<event_seq>:<origin_instance>"`. A collaborative op
//!   batch was applied; the row lives in `collab_events`. Each instance loads it
//!   and delivers it to its local subscribers, except the instance that authored
//!   it (which already broadcast it locally).
//! - `collab_presence` — payload `"<note_id>:<origin_instance>"`. A note's
//!   presence changed; every instance except the origin (which already
//!   broadcast it locally) rebuilds the merged list for its local subscribers.
//! - `sync_batch` — payload `"<user_id>:<origin_instance>"`. A relay batch landed
//!   for a user; sibling instances wake that user's local devices to re-scan the
//!   journal (the authoring instance already fanned it out live).

use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgListener;
use uuid::Uuid;

use crate::state::AppState;

pub const CH_COLLAB_OP: &str = "collab_op";
pub const CH_COLLAB_PRESENCE: &str = "collab_presence";
pub const CH_SYNC_BATCH: &str = "sync_batch";

/// Spawn the listener task. It reconnects with a short backoff if the listen
/// connection drops, so a transient database blip does not permanently sever
/// cross-instance delivery.
pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run(&state).await {
                tracing::warn!(error = %e, "collab bus listener error; reconnecting");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });
}

async fn run(state: &Arc<AppState>) -> anyhow::Result<()> {
    let mut listener = PgListener::connect_with(state.store.pool()).await?;
    listener
        .listen_all([CH_COLLAB_OP, CH_COLLAB_PRESENCE, CH_SYNC_BATCH])
        .await?;
    tracing::info!(instance = %state.instance_id, "collab bus listening");
    loop {
        let notification = listener.recv().await?;
        match notification.channel() {
            CH_COLLAB_OP => handle_collab_op(state, notification.payload()).await,
            CH_COLLAB_PRESENCE => handle_collab_presence(state, notification.payload()).await,
            CH_SYNC_BATCH => handle_sync_batch(state, notification.payload()).await,
            _ => {}
        }
    }
}

/// `"<seq>:<origin_instance>"`. Skip our own events (already broadcast locally);
/// otherwise load the outbox row and deliver it to local subscribers.
async fn handle_collab_op(state: &Arc<AppState>, payload: &str) {
    let Some((seq, origin)) = payload.split_once(':') else {
        return;
    };
    let (Ok(seq), Ok(origin)) = (seq.parse::<i64>(), origin.parse::<Uuid>()) else {
        return;
    };
    if origin == state.instance_id {
        return; // our own op; local subscribers already have it
    }
    match state.store.get_collab_event(seq).await {
        Ok(Some(event)) => crate::collab::deliver_event(state, event).await,
        Ok(None) => {} // pruned already; a reconnecting client resyncs from a snapshot
        Err(e) => tracing::warn!(error = %e, seq, "collab event load failed"),
    }
}

/// `"<note_id>:<origin_instance>"`. Skip our own change (already broadcast
/// locally); otherwise rebuild and broadcast the merged presence to local subs.
async fn handle_collab_presence(state: &Arc<AppState>, payload: &str) {
    let Some((note_id, origin)) = payload.split_once(':') else {
        return;
    };
    let (Ok(note_id), Ok(origin)) = (note_id.parse::<Uuid>(), origin.parse::<Uuid>()) else {
        return;
    };
    if origin == state.instance_id {
        return;
    }
    crate::collab::deliver_presence(state, note_id).await;
}

/// `"<user_id>:<origin_instance>"`. Skip our own batches; otherwise wake the
/// user's local relay connections to re-scan the journal.
async fn handle_sync_batch(state: &Arc<AppState>, payload: &str) {
    let Some((user, origin)) = payload.split_once(':') else {
        return;
    };
    let (Ok(user_id), Ok(origin)) = (user.parse::<Uuid>(), origin.parse::<Uuid>()) else {
        return;
    };
    if origin == state.instance_id {
        return; // our own batch; local devices already fanned out
    }
    state.hub.wake_user(user_id).await;
}
