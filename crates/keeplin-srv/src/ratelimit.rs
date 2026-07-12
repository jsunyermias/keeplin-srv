//! A dependency-free per-client-IP token-bucket rate limiter.
//!
//! Each source IP gets a bucket of `capacity` tokens that refills at
//! `capacity` tokens per minute. Every request spends one token; an empty
//! bucket yields `429 Too Many Requests`. This bounds abuse such as a
//! reconnect/login loop from a single host without a new dependency.
//!
//! **Behind a reverse proxy** every request carries the proxy's IP, so all
//! clients would share one bucket — rate-limit at the proxy instead and leave
//! `RATE_LIMIT_PER_MIN` at `0` (disabled).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::sync::Mutex;

use crate::state::AppState;

/// Token bucket state for one IP.
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

/// The mutable, lock-guarded state: the per-IP buckets and when they were last swept.
struct LimiterState {
    buckets: HashMap<IpAddr, Bucket>,
    last_sweep: Instant,
}

/// Shared limiter: one bucket per seen IP. `capacity == 0` means disabled.
pub struct RateLimiter {
    capacity: f64,
    /// Tokens refilled per second.
    refill_per_sec: f64,
    state: Mutex<LimiterState>,
}

/// How often idle buckets are swept out of the map. A bucket refills fully within
/// `capacity / refill_per_sec` seconds (60s by construction), so anything swept has been
/// idle at least this long and is indistinguishable from a fresh full bucket.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

impl RateLimiter {
    /// `per_min` requests per IP per minute; `0` disables limiting.
    pub fn new(per_min: u32) -> Self {
        Self {
            capacity: per_min as f64,
            refill_per_sec: per_min as f64 / 60.0,
            state: Mutex::new(LimiterState {
                buckets: HashMap::new(),
                last_sweep: Instant::now(),
            }),
        }
    }

    pub fn enabled(&self) -> bool {
        self.capacity > 0.0
    }

    /// Tokens `bucket` would hold at `now` after refilling, capped at capacity.
    fn projected_tokens(&self, bucket: &Bucket, now: Instant) -> f64 {
        let elapsed = now
            .saturating_duration_since(bucket.last_refill)
            .as_secs_f64();
        (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity)
    }

    /// Try to spend one token for `ip`. Returns `true` if allowed.
    pub async fn check(&self, ip: IpAddr, now: Instant) -> bool {
        if !self.enabled() {
            return true;
        }
        let mut state = self.state.lock().await;
        let bucket = state.buckets.entry(ip).or_insert(Bucket {
            tokens: self.capacity,
            last_refill: now,
        });
        bucket.tokens = self.projected_tokens(bucket, now);
        bucket.last_refill = now;
        let allowed = if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        };

        // Periodically drop fully-refilled (idle) buckets so the map does not grow without
        // bound under IP churn (issue #33). A bucket at capacity is identical to a fresh one,
        // so removing it is behaviour-preserving. The just-touched bucket has spent a token
        // and is under capacity, so it is retained.
        if now.saturating_duration_since(state.last_sweep) >= SWEEP_INTERVAL {
            let cap = self.capacity;
            let rate = self.refill_per_sec;
            state.buckets.retain(|_, b| {
                let elapsed = now.saturating_duration_since(b.last_refill).as_secs_f64();
                (b.tokens + elapsed * rate) < cap
            });
            state.last_sweep = now;
        }
        allowed
    }

    /// Number of live buckets (test/introspection helper).
    #[cfg(test)]
    async fn bucket_count(&self) -> usize {
        self.state.lock().await.buckets.len()
    }
}

/// Axum middleware enforcing [`RateLimiter`] on every request, keyed by the
/// peer socket IP. A no-op when limiting is disabled.
pub async fn rate_limit_mw(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    if state.rate_limiter.check(addr.ip(), Instant::now()).await {
        next.run(req).await
    } else {
        (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({ "error": "rate limit exceeded" })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ip() -> IpAddr {
        IpAddr::from([127, 0, 0, 1])
    }

    #[tokio::test]
    async fn disabled_always_allows() {
        let rl = RateLimiter::new(0);
        let now = Instant::now();
        for _ in 0..1000 {
            assert!(rl.check(ip(), now).await);
        }
    }

    #[tokio::test]
    async fn burst_then_throttle_then_refill() {
        let rl = RateLimiter::new(60); // 1 token/sec, burst 60
        let t0 = Instant::now();
        // Spend the whole burst at one instant.
        for _ in 0..60 {
            assert!(rl.check(ip(), t0).await);
        }
        // Next request at the same instant is denied.
        assert!(!rl.check(ip(), t0).await);
        // One second later, ~1 token has refilled → one request allowed.
        let t1 = t0 + Duration::from_secs(1);
        assert!(rl.check(ip(), t1).await);
        assert!(!rl.check(ip(), t1).await);
    }

    #[tokio::test]
    async fn separate_ips_have_separate_buckets() {
        let rl = RateLimiter::new(1);
        let now = Instant::now();
        let a = IpAddr::from([10, 0, 0, 1]);
        let b = IpAddr::from([10, 0, 0, 2]);
        assert!(rl.check(a, now).await);
        assert!(!rl.check(a, now).await); // a exhausted
        assert!(rl.check(b, now).await); // b unaffected
    }

    #[tokio::test]
    async fn idle_buckets_are_swept_after_the_interval() {
        let rl = RateLimiter::new(60); // capacity 60, refill 1/sec
        let t0 = Instant::now();
        // Spend one token each from 100 distinct IPs → 100 live buckets.
        for i in 0..100u32 {
            let ip = IpAddr::from([10, 0, (i >> 8) as u8, i as u8]);
            assert!(rl.check(ip, t0).await);
        }
        assert_eq!(rl.bucket_count().await, 100);

        // Well past the sweep interval and the refill window, a request from one active IP
        // triggers a sweep that reclaims every idle (now fully-refilled) bucket.
        let later = t0 + Duration::from_secs(120);
        let active = IpAddr::from([10, 0, 0, 0]);
        rl.check(active, later).await;
        // Only the just-touched bucket (under capacity) survives.
        assert_eq!(rl.bucket_count().await, 1);
    }
}
