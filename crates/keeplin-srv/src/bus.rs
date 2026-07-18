// md:Overview
use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgListener;
use uuid::Uuid;

use crate::state::AppState;

// md:Channel constants
pub const CH_COLLAB_OP: &str = "collab_op";
pub const CH_COLLAB_PRESENCE: &str = "collab_presence";
pub const CH_SYNC_BATCH: &str = "sync_batch";

// md:fn spawn
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

// md:fn run
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

// md:fn handle_collab_op
async fn handle_collab_op(state: &Arc<AppState>, payload: &str) {
    let Some((seq, origin)) = payload.split_once(':') else {
        return;
    };
    let (Ok(seq), Ok(origin)) = (seq.parse::<i64>(), origin.parse::<Uuid>()) else {
        return;
    };
    if origin == state.instance_id {
        return;
    }
    match state.store.get_collab_event(seq).await {
        Ok(Some(event)) => crate::collab::deliver_event(state, event).await,
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, seq, "collab event load failed"),
    }
}

// md:fn handle_collab_presence
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

// md:fn handle_sync_batch
async fn handle_sync_batch(state: &Arc<AppState>, payload: &str) {
    let Some((user, origin)) = payload.split_once(':') else {
        return;
    };
    let (Ok(user_id), Ok(origin)) = (user.parse::<Uuid>(), origin.parse::<Uuid>()) else {
        return;
    };
    if origin == state.instance_id {
        return;
    }
    state.hub.wake_user(user_id).await;
}
