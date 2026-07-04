use std::sync::Arc;

use sqlx::{Pool, Postgres};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{config::Config, store::Store, websocket::Room};

pub type Rooms = RwLock<std::collections::HashMap<Uuid, Arc<Room>>>;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub rooms: Arc<Rooms>,
}

impl AppState {
    pub fn new(config: Config, pool: Pool<Postgres>) -> Self {
        Self {
            config,
            store: Store::new(pool),
            rooms: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }
}
