use sqlx::{Pool, Postgres};

use crate::{config::Config, store::Store, sync::SyncHub};

pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub hub: SyncHub,
}

impl AppState {
    pub fn new(config: Config, pool: Pool<Postgres>) -> Self {
        Self {
            config,
            store: Store::new(pool),
            hub: SyncHub::default(),
        }
    }
}
