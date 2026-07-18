// md:Overview
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

// md:Bucket
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

// md:LimiterState
struct LimiterState {
    buckets: HashMap<IpAddr, Bucket>,
    last_sweep: Instant,
}

// md:RateLimiter
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    state: Mutex<LimiterState>,
}

// md:SWEEP_INTERVAL
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

// md:impl RateLimiter
impl RateLimiter {
    // md:impl RateLimiter > fn new
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

    // md:impl RateLimiter > fn enabled
    pub fn enabled(&self) -> bool {
        self.capacity > 0.0
    }

    // md:impl RateLimiter > fn projected_tokens
    fn projected_tokens(&self, bucket: &Bucket, now: Instant) -> f64 {
        let elapsed = now
            .saturating_duration_since(bucket.last_refill)
            .as_secs_f64();
        (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity)
    }

    // md:impl RateLimiter > fn check
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

    // md:impl RateLimiter > fn bucket_count
    #[cfg(test)]
    async fn bucket_count(&self) -> usize {
        self.state.lock().await.buckets.len()
    }
}

// md:fn rate_limit_mw
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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // md:mod tests > fn ip
    fn ip() -> IpAddr {
        IpAddr::from([127, 0, 0, 1])
    }

    // md:mod tests > fn disabled_always_allows
    #[tokio::test]
    async fn disabled_always_allows() {
        let rl = RateLimiter::new(0);
        let now = Instant::now();
        for _ in 0..1000 {
            assert!(rl.check(ip(), now).await);
        }
    }

    // md:mod tests > fn burst_then_throttle_then_refill
    #[tokio::test]
    async fn burst_then_throttle_then_refill() {
        let rl = RateLimiter::new(60);
        let t0 = Instant::now();
        for _ in 0..60 {
            assert!(rl.check(ip(), t0).await);
        }
        assert!(!rl.check(ip(), t0).await);
        let t1 = t0 + Duration::from_secs(1);
        assert!(rl.check(ip(), t1).await);
        assert!(!rl.check(ip(), t1).await);
    }

    // md:mod tests > fn separate_ips_have_separate_buckets
    #[tokio::test]
    async fn separate_ips_have_separate_buckets() {
        let rl = RateLimiter::new(1);
        let now = Instant::now();
        let a = IpAddr::from([10, 0, 0, 1]);
        let b = IpAddr::from([10, 0, 0, 2]);
        assert!(rl.check(a, now).await);
        assert!(!rl.check(a, now).await);
        assert!(rl.check(b, now).await);
    }

    // md:mod tests > fn idle_buckets_are_swept_after_the_interval
    #[tokio::test]
    async fn idle_buckets_are_swept_after_the_interval() {
        let rl = RateLimiter::new(60);
        let t0 = Instant::now();
        for i in 0..100u32 {
            let ip = IpAddr::from([10, 0, (i >> 8) as u8, i as u8]);
            assert!(rl.check(ip, t0).await);
        }
        assert_eq!(rl.bucket_count().await, 100);

        let later = t0 + Duration::from_secs(120);
        let active = IpAddr::from([10, 0, 0, 0]);
        rl.check(active, later).await;
        assert_eq!(rl.bucket_count().await, 1);
    }
}
