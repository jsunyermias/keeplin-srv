// md:Overview
use sqlx::{Pool, Postgres};
use uuid::Uuid;

use crate::{
    collab::CollabRegistry, config::Config, ratelimit::RateLimiter, store::Store, sync::SyncHub,
};

// md:AppState
pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub hub: SyncHub,
    pub collab: CollabRegistry,
    pub rate_limiter: RateLimiter,
    pub instance_id: Uuid,
    pub mailer: crate::mail::Mailer,
}

// md:impl AppState
impl AppState {
    // md:impl AppState > fn new
    pub fn new(config: Config, pool: Pool<Postgres>) -> Self {
        let rate_limiter = RateLimiter::new(config.rate_limit_per_min);
        let cipher = crate::crypto::Cipher::from_key(config.at_rest_key.as_deref())
            .expect("valid AT_REST_KEY (validated at startup)");
        let mailer = crate::mail::Mailer::new(
            config.mail_webhook_url.clone(),
            config.mail_webhook_token.clone(),
        );
        Self {
            config,
            store: Store::with_cipher(pool, cipher),
            hub: SyncHub::default(),
            collab: CollabRegistry::default(),
            rate_limiter,
            instance_id: Uuid::new_v4(),
            mailer,
        }
    }
}
