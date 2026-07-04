use sqlx::{Pool, Postgres};

use crate::{collab::CollabRegistry, config::Config, store::Store, sync::SyncHub};

pub struct AppState {
    pub config: Config,
    pub store: Store,
    /// Per-user fan-out for the device sync relay (`/api/sync`).
    pub hub: SyncHub,
    /// Per-note collaborative sessions (`/api/ws`).
    pub collab: CollabRegistry,
}

impl AppState {
    pub fn new(config: Config, pool: Pool<Postgres>) -> Self {
        Self {
            config,
            store: Store::new(pool),
            hub: SyncHub::default(),
            collab: CollabRegistry::default(),
        }
    }
}
