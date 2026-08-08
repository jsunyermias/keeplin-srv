// md:Overview
use sqlx::{Pool, Postgres};
use uuid::Uuid;

use crate::{
    collab::CollabRegistry, config::Config, ratelimit::RateLimiter, store::Store, sync::SyncHub,
};

// md:HttpTestHooks
#[cfg(debug_assertions)]
#[derive(Default)]
pub struct HttpTestHooks {
    pause: tokio::sync::Mutex<Option<(&'static str, &'static str)>>,
    reached: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    serialization_failures: std::sync::atomic::AtomicUsize,
    fail_after_mutation: std::sync::atomic::AtomicBool,
    attempts: std::sync::atomic::AtomicUsize,
    commits: std::sync::atomic::AtomicUsize,
    exhausted: std::sync::atomic::AtomicUsize,
    external_effects: std::sync::atomic::AtomicUsize,
}

// md:impl HttpTestHooks
#[cfg(debug_assertions)]
impl HttpTestHooks {
    pub async fn pause_at(&self, handler: &'static str, point: &'static str) {
        *self.pause.lock().await = Some((handler, point));
    }

    pub async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    pub fn resume(&self) {
        self.resume.notify_one();
    }

    pub fn inject_serialization_failures(&self, count: usize) {
        self.serialization_failures
            .store(count, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn inject_failure_after_mutation(&self) {
        self.fail_after_mutation
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn observations(&self) -> (usize, usize, usize, usize) {
        (
            self.attempts.load(std::sync::atomic::Ordering::SeqCst),
            self.commits.load(std::sync::atomic::Ordering::SeqCst),
            self.exhausted.load(std::sync::atomic::Ordering::SeqCst),
            self.external_effects
                .load(std::sync::atomic::Ordering::SeqCst),
        )
    }

    pub(crate) async fn checkpoint(&self, handler: &'static str, point: &'static str) {
        if *self.pause.lock().await == Some((handler, point)) {
            self.reached.notify_one();
            self.resume.notified().await;
            *self.pause.lock().await = None;
        }
    }

    pub(crate) fn begin_attempt(&self) {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn should_inject_serialization_failure(&self) -> bool {
        self.serialization_failures
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |remaining| remaining.checked_sub(1),
            )
            .is_ok()
    }

    pub(crate) fn should_fail_after_mutation(&self) -> bool {
        self.fail_after_mutation
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn committed(&self) {
        self.commits
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn exhausted(&self) {
        self.exhausted
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn external_effect(&self) {
        self.external_effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

// md:AppState
pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub hub: SyncHub,
    pub collab: CollabRegistry,
    pub rate_limiter: RateLimiter,
    pub instance_id: Uuid,
    pub mailer: crate::mail::Mailer,
    #[cfg(debug_assertions)]
    pub http_test_hooks: HttpTestHooks,
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
            #[cfg(debug_assertions)]
            http_test_hooks: HttpTestHooks::default(),
        }
    }
}
