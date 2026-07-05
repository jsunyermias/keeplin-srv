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

/// Shared limiter: one bucket per seen IP. `capacity == 0` means disabled.
pub struct RateLimiter {
    capacity: f64,
    /// Tokens refilled per second.
    refill_per_sec: f64,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl RateLimiter {
    /// `per_min` requests per IP per minute; `0` disables limiting.
    pub fn new(per_min: u32) -> Self {
        Self {
            capacity: per_min as f64,
            refill_per_sec: per_min as f64 / 60.0,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.capacity > 0.0
    }

    /// Try to spend one token for `ip`. Returns `true` if allowed.
    pub async fn check(&self, ip: IpAddr, now: Instant) -> bool {
        if !self.enabled() {
            return true;
        }
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets.entry(ip).or_insert(Bucket {
            tokens: self.capacity,
            last_refill: now,
        });
        // Refill for the elapsed time, capped at capacity.
        let elapsed = now
            .saturating_duration_since(bucket.last_refill)
            .as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last_refill = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
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
}
