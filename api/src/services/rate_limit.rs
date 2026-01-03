//! Per-user rate limiting for daemon API endpoints
//!
//! Uses a simple token bucket algorithm with in-memory storage.
//! Tokens are stored per user_id and refill over time.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Rate limiter configuration
pub struct RateLimitConfig {
    /// Maximum tokens (burst capacity)
    pub max_tokens: u32,
    /// Tokens added per second
    pub refill_rate: f64,
}

struct UserBucket {
    tokens: f64,
    last_update: Instant,
}

/// Per-user rate limiter using token bucket algorithm
pub struct UserRateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<HashMap<i64, UserBucket>>,
}

impl UserRateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Check if a request is allowed for the given user_id.
    /// Returns true if allowed, false if rate limited.
    pub fn check(&self, user_id: i64) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let now = Instant::now();

        let bucket = buckets.entry(user_id).or_insert_with(|| UserBucket {
            tokens: self.config.max_tokens as f64,
            last_update: now,
        });

        // Refill tokens based on time elapsed
        let elapsed = now.duration_since(bucket.last_update);
        let refill = elapsed.as_secs_f64() * self.config.refill_rate;
        bucket.tokens = (bucket.tokens + refill).min(self.config.max_tokens as f64);
        bucket.last_update = now;

        // Try to consume a token
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Clean up old entries (users who haven't made requests in a while)
    /// Call this periodically to prevent memory growth
    #[allow(dead_code)]
    pub fn cleanup(&self, max_age: Duration) {
        let mut buckets = self.buckets.lock().unwrap();
        let now = Instant::now();
        buckets.retain(|_, bucket| now.duration_since(bucket.last_update) < max_age);
    }
}

/// Global rate limiter for daemon API (captures + activity endpoints)
/// - Burst of 60 requests allowed
/// - Sustained rate of 2 requests/second (120/min)
pub static DAEMON_RATE_LIMITER: LazyLock<UserRateLimiter> = LazyLock::new(|| {
    UserRateLimiter::new(RateLimitConfig {
        max_tokens: 60,
        refill_rate: 2.0,
    })
});
