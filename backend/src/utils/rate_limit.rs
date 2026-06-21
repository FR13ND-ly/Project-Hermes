//! Tiny in-memory sliding-window rate limiter.
//!
//! Used to throttle brute-force attempts against sensitive endpoints (currently
//! login). It is intentionally process-local: a single-node self-hosted deploy
//! doesn't need a distributed store, and the failure mode (a restart resets
//! counters) is acceptable for this use case.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    inner: Mutex<HashMap<String, Vec<Instant>>>,
    max_attempts: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            max_attempts,
            window,
        }
    }

    /// Records an attempt for `key` and returns `true` if it is allowed, or
    /// `false` if the caller has exceeded `max_attempts` within `window`.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());

        // Opportunistic sweep so stale keys from one-off IPs don't accumulate.
        if map.len() > 10_000 {
            map.retain(|_, hits| hits.iter().any(|t| now.duration_since(*t) < self.window));
        }

        let hits = map.entry(key.to_string()).or_default();
        hits.retain(|t| now.duration_since(*t) < self.window);

        if hits.len() >= self.max_attempts {
            return false;
        }
        hits.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let rl = RateLimiter::new(3, Duration::from_secs(60));
        assert!(rl.check("1.2.3.4"));
        assert!(rl.check("1.2.3.4"));
        assert!(rl.check("1.2.3.4"));
        // 4th attempt within the window is rejected.
        assert!(!rl.check("1.2.3.4"));
    }

    #[test]
    fn buckets_are_per_key() {
        let rl = RateLimiter::new(1, Duration::from_secs(60));
        assert!(rl.check("a"));
        assert!(!rl.check("a"));
        // A different key has its own independent budget.
        assert!(rl.check("b"));
    }

    #[test]
    fn expired_hits_are_forgotten() {
        let rl = RateLimiter::new(1, Duration::from_millis(1));
        assert!(rl.check("k"));
        std::thread::sleep(Duration::from_millis(5));
        // The previous hit has aged out of the window, so this is allowed again.
        assert!(rl.check("k"));
    }
}
