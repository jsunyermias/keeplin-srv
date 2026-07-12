use sqlx::{Pool, Postgres};
use uuid::Uuid;

use crate::{
    collab::CollabRegistry, config::Config, ratelimit::RateLimiter, store::Store, sync::SyncHub,
};

pub struct AppState {
    pub config: Config,
    pub store: Store,
    /// Per-user fan-out for the device sync relay (`/api/sync`).
    pub hub: SyncHub,
    /// Per-note collaborative sessions (`/api/ws`).
    pub collab: CollabRegistry,
    /// Per-IP request rate limiter (a no-op when disabled).
    pub rate_limiter: RateLimiter,
    /// Identity of this server process, minted at startup. Stamped on collab
    /// fan-out events and presence rows so an instance can tell its own writes
    /// apart from a sibling's over the cross-instance bus (issue #45).
    pub instance_id: Uuid,
}

impl AppState {
    pub fn new(config: Config, pool: Pool<Postgres>) -> Self {
        let rate_limiter = RateLimiter::new(config.rate_limit_per_min);
        // A present-but-invalid AT_REST_KEY is a fatal misconfiguration; validate
        // it at startup (main also checks it, so this never fires in practice).
        let cipher = crate::crypto::Cipher::from_key(config.at_rest_key.as_deref())
            .expect("valid AT_REST_KEY (validated at startup)");
        Self {
            config,
            store: Store::with_cipher(pool, cipher),
            hub: SyncHub::default(),
            collab: CollabRegistry::default(),
            rate_limiter,
            instance_id: Uuid::new_v4(),
        }
    }
}
