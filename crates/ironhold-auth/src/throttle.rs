use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, Mutex, OnceLock, PoisonError},
    time::{Duration, Instant},
};

use axum::{extract::FromRequestParts, http::request::Parts};
use ironhold_core::Error;

/// Most accounts tracked at once. Beyond this, expired entries are dropped
/// first, then arbitrary ones, so memory stays bounded under attack.
const MAX_TRACKED: usize = 100_000;

/// Limits failed logins per account, to stop password guessing.
///
/// The default allows 5 failures per account in 5 minutes. After that,
/// [`check`](Self::check) returns `429 Too Many Requests` until the window
/// ends. A successful login clears the count.
///
/// Use it as a handler argument. Without any setup, every app shares one
/// default throttle; register your own with `App::extension` to change the
/// limits. The count lives in memory, per server instance.
#[derive(Clone)]
pub struct LoginThrottle {
    inner: Arc<Mutex<HashMap<String, Window>>>,
    max_failures: u32,
    window: Duration,
}

#[derive(Clone, Copy)]
struct Window {
    failures: u32,
    started: Instant,
}

impl LoginThrottle {
    /// Allows `max_failures` failed logins per account within `window`.
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            max_failures,
            window,
        }
    }

    /// Returns `Err(Error::TooManyRequests)` if `account` (usually the email
    /// or username typed in) has too many recent failures. Call it before
    /// checking the password.
    pub fn check(&self, account: &str) -> Result<(), Error> {
        let key = normalize(account);
        let map = self.lock();
        match map.get(&key) {
            Some(w) if w.started.elapsed() < self.window && w.failures >= self.max_failures => {
                let left = self.window.saturating_sub(w.started.elapsed());
                Err(Error::TooManyRequests {
                    retry_after_secs: left.as_secs().max(1),
                })
            }
            _ => Ok(()),
        }
    }

    /// Records a failed login for `account`.
    pub fn record_failure(&self, account: &str) {
        let key = normalize(account);
        let mut map = self.lock();
        if map.len() >= MAX_TRACKED && !map.contains_key(&key) {
            let window = self.window;
            map.retain(|_, w| w.started.elapsed() < window);
            while map.len() >= MAX_TRACKED {
                let Some(victim) = map.keys().next().cloned() else {
                    break;
                };
                map.remove(&victim);
            }
        }
        let now = Instant::now();
        let entry = map.entry(key).or_insert(Window {
            failures: 0,
            started: now,
        });
        if entry.started.elapsed() >= self.window {
            *entry = Window {
                failures: 0,
                started: now,
            };
        }
        entry.failures = entry.failures.saturating_add(1);
        if entry.failures == self.max_failures {
            tracing::warn!("login throttled after repeated failures");
        }
    }

    /// Clears the failure count for `account` after a successful login.
    pub fn record_success(&self, account: &str) {
        self.lock().remove(&normalize(account));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Window>> {
        // A panic while holding the lock can't leave the map inconsistent,
        // so keep using it rather than failing every login afterwards.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new(5, Duration::from_secs(5 * 60))
    }
}

impl std::fmt::Debug for LoginThrottle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginThrottle")
            .field("max_failures", &self.max_failures)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for LoginThrottle {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        static SHARED: OnceLock<LoginThrottle> = OnceLock::new();
        Ok(parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or_else(|| SHARED.get_or_init(Self::default).clone()))
    }
}

fn normalize(account: &str) -> String {
    account.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_too_many_failures_per_account() {
        let throttle = LoginThrottle::new(3, Duration::from_secs(60));
        for _ in 0..3 {
            assert!(throttle.check("raj@example.com").is_ok());
            throttle.record_failure("raj@example.com");
        }
        let error = throttle.check(" RAJ@example.com ").unwrap_err();
        assert!(matches!(error, Error::TooManyRequests { .. }));
        // Other accounts are unaffected.
        assert!(throttle.check("someone@example.com").is_ok());
    }

    #[test]
    fn success_clears_failures() {
        let throttle = LoginThrottle::new(2, Duration::from_secs(60));
        throttle.record_failure("a@example.com");
        throttle.record_success("a@example.com");
        throttle.record_failure("a@example.com");
        assert!(throttle.check("a@example.com").is_ok());
    }

    #[test]
    fn window_expires() {
        let throttle = LoginThrottle::new(1, Duration::from_millis(20));
        throttle.record_failure("a@example.com");
        assert!(throttle.check("a@example.com").is_err());
        std::thread::sleep(Duration::from_millis(30));
        assert!(throttle.check("a@example.com").is_ok());
    }
}
