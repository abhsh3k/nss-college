use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub upload_dir: String,
    /// Sign-in throttling. Shared across both listeners because it lives here
    /// rather than on a router.
    pub login_limiter: Arc<LoginLimiter>,
}

/// A small in-memory sliding-window limiter for sign-in attempts.
///
/// It is deliberately keyed on *both* the account identifier and the client
/// address, so one attacker cannot grind a single account and one account
/// cannot be hammered from many addresses. Because the reply never says which
/// dimension was hit — and a missing account follows the same path — it does
/// not reveal whether an identifier exists.
///
/// In-memory rather than a table: the process is single-tenant for this app, and
/// a restart clearing the counters is acceptable. A denial-of-service across
/// restarts is not the threat this defends against.
pub struct LoginLimiter {
    window: Duration,
    account_max: usize,
    ip_max: usize,
    hits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl LoginLimiter {
    pub fn new(window: Duration, account_max: usize, ip_max: usize) -> Self {
        Self {
            window,
            account_max,
            ip_max,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// The production policy: five minutes, 20 attempts per account and 60 per
    /// address, comfortably above a person fat-fingering their password.
    pub fn default_policy() -> Self {
        Self::new(Duration::from_secs(300), 20, 60)
    }

    /// Record one attempt under `key` at `now` and report whether it is still
    /// within `max` for the window.
    fn record_at(&self, key: &str, max: usize, now: Instant) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let entry = hits.entry(key.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < self.window);
        entry.push(now);
        entry.len() <= max
    }

    /// Whether an attempt from `ip` on `account` may proceed, recording it.
    /// Both dimensions must be under their limits.
    pub fn allow(&self, account: &str, ip: &str) -> bool {
        self.allow_at(account, ip, Instant::now())
    }

    /// `allow`, with the clock injected so the sliding window can be tested
    /// without sleeping.
    pub fn allow_at(&self, account: &str, ip: &str, now: Instant) -> bool {
        let account_ok = self.record_at(&format!("a:{account}"), self.account_max, now);
        let ip_ok = self.record_at(&format!("i:{ip}"), self.ip_max, now);
        account_ok && ip_ok
    }
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::default_policy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> Instant {
        // A single fixed origin plus an offset, so the window math is exact.
        Instant::now() + Duration::from_secs(seconds)
    }

    #[test]
    fn an_account_is_blocked_after_its_limit() {
        let limiter = LoginLimiter::new(Duration::from_secs(300), 3, 100);
        assert!(limiter.allow_at("user", "ip", at(0)));
        assert!(limiter.allow_at("user", "ip", at(1)));
        assert!(limiter.allow_at("user", "ip", at(2)));
        // Fourth attempt in the window is refused.
        assert!(!limiter.allow_at("user", "ip", at(3)));
    }

    #[test]
    fn the_window_slides() {
        let limiter = LoginLimiter::new(Duration::from_secs(300), 2, 100);
        assert!(limiter.allow_at("user", "ip", at(0)));
        assert!(limiter.allow_at("user", "ip", at(1)));
        assert!(!limiter.allow_at("user", "ip", at(2)));
        // Once the first two attempts fall outside the 5-minute window, room
        // opens up again.
        assert!(limiter.allow_at("user", "ip", at(301)));
    }

    #[test]
    fn one_address_cannot_be_used_to_grind_many_accounts() {
        let limiter = LoginLimiter::new(Duration::from_secs(300), 100, 2);
        assert!(limiter.allow_at("a", "1.2.3.4", at(0)));
        assert!(limiter.allow_at("b", "1.2.3.4", at(1)));
        // Different account, same address: the address limit is what trips.
        assert!(!limiter.allow_at("c", "1.2.3.4", at(2)));
        // A different address is unaffected.
        assert!(limiter.allow_at("c", "5.6.7.8", at(2)));
    }
}
