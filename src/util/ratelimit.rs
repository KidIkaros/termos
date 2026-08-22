//! Token bucket rate limiter.
//!
//! Controls the rate of operations by maintaining a bucket of tokens that
//! refills at a constant rate. Each operation consumes one token. When the
//! bucket is empty, operations are rejected or blocked.
//!
//! Based on the token bucket algorithm from Chapter 4 of System Design Interview.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A token bucket rate limiter.
pub struct TokenBucket {
    inner: Mutex<BucketInner>,
}

struct BucketInner {
    /// Maximum tokens the bucket can hold.
    capacity: f64,
    /// Current token count.
    tokens: f64,
    /// Tokens added per second.
    refill_rate: f64,
    /// Last refill timestamp.
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new rate limiter.
    ///
    /// - `capacity`: maximum burst size (tokens).
    /// - `refill_rate`: tokens added per second.
    pub fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            inner: Mutex::new(BucketInner {
                capacity,
                tokens: capacity,
                refill_rate,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Create a rate limiter that allows N operations per second with burst.
    pub fn per_second(n: u32) -> Self {
        Self::new(n as f64, n as f64)
    }

    /// Create a rate limiter for the PTY pool (conservative defaults).
    pub fn for_pty_pool() -> Self {
        // Allow burst of 8 PTYs, refill 2 per second
        Self::new(8.0, 2.0)
    }

    /// Try to consume one token. Returns `true` if allowed, `false` if throttled.
    pub fn try_acquire(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        inner.refill();
        if inner.tokens >= 1.0 {
            inner.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Block until a token is available or timeout expires.
    /// Returns `true` if a token was acquired, `false` on timeout.
    pub fn acquire_timeout(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.try_acquire() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            // Sleep briefly to avoid busy-waiting
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Get the current token count (approximate, for monitoring).
    pub fn available(&self) -> f64 {
        let mut inner = self.inner.lock().unwrap();
        inner.refill();
        inner.tokens
    }

    /// Check if the bucket is full (no throttling has occurred recently).
    pub fn is_full(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        (inner.tokens - inner.capacity).abs() < f64::EPSILON
    }
}

impl BucketInner {
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
    }
}

/// A multi-tier rate limiter that applies different limits per operation type.
pub struct TieredRateLimiter {
    tiers: std::collections::HashMap<String, TokenBucket>,
}

impl TieredRateLimiter {
    /// Create a new tiered rate limiter.
    pub fn new() -> Self {
        Self {
            tiers: std::collections::HashMap::new(),
        }
    }

    /// Register a rate limit tier.
    pub fn register(&mut self, name: &str, capacity: f64, refill_rate: f64) {
        self.tiers
            .insert(name.to_string(), TokenBucket::new(capacity, refill_rate));
    }

    /// Try to acquire a token for the given tier.
    pub fn try_acquire(&self, tier: &str) -> bool {
        if let Some(bucket) = self.tiers.get(tier) {
            bucket.try_acquire()
        } else {
            // Unknown tier — allow by default
            true
        }
    }

    /// Get available tokens for a tier (for monitoring).
    pub fn available(&self, tier: &str) -> f64 {
        self.tiers
            .get(tier)
            .map(|b| b.available())
            .unwrap_or(f64::INFINITY)
    }
}

impl Default for TieredRateLimiter {
    fn default() -> Self {
        let mut limiter = Self::new();
        // PTY allocation: burst of 8, refill 2/sec
        limiter.register("pty_alloc", 8.0, 2.0);
        // Input per pane: burst of 100 keystrokes, refill 50/sec
        limiter.register("pane_input", 100.0, 50.0);
        // Notifications: burst of 10, refill 2/sec
        limiter.register("notifications", 10.0, 2.0);
        limiter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_allows_burst() {
        let bucket = TokenBucket::new(5.0, 1.0);
        // Should allow 5 rapid acquisitions
        for _ in 0..5 {
            assert!(bucket.try_acquire());
        }
        // 6th should fail
        assert!(!bucket.try_acquire());
    }

    #[test]
    fn token_bucket_refills() {
        let bucket = TokenBucket::new(2.0, 100.0); // Fast refill for testing
        assert!(bucket.try_acquire());
        assert!(bucket.try_acquire());
        assert!(!bucket.try_acquire());
        // Wait for refill
        std::thread::sleep(Duration::from_millis(20));
        assert!(bucket.try_acquire());
    }

    #[test]
    fn acquire_timeout_works() {
        let bucket = TokenBucket::new(1.0, 0.0); // No refill
        assert!(bucket.try_acquire());
        // Should timeout quickly
        assert!(!bucket.acquire_timeout(Duration::from_millis(10)));
    }

    #[test]
    fn tiered_limiter_works() {
        let limiter = TieredRateLimiter::default();
        assert!(limiter.try_acquire("pty_alloc"));
        assert!(limiter.available("pty_alloc") < 8.0);
        // Unknown tier allows by default
        assert!(limiter.try_acquire("unknown_tier"));
    }

    #[test]
    fn default_limiter_has_expected_tiers() {
        let limiter = TieredRateLimiter::default();
        assert!(limiter.tiers.contains_key("pty_alloc"));
        assert!(limiter.tiers.contains_key("pane_input"));
        assert!(limiter.tiers.contains_key("notifications"));
    }
}
