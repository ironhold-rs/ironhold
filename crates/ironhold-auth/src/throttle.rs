use std::{
    collections::HashMap,
    convert::Infallible,
    net::IpAddr,
    sync::{Arc, Mutex, OnceLock, PoisonError},
    time::{Duration, Instant},
};

use axum::{extract::FromRequestParts, http::request::Parts};
use ironhold_core::Error;
use ironhold_security::ClientIp;

/// Most (account, client) pairs tracked at once. Beyond this, expired
/// entries are dropped first, then arbitrary ones, so memory stays bounded
/// under attack.
const MAX_TRACKED: usize = 100_000;

/// When the table is full, look for expired entries at most this often.
/// Scanning the whole table on every new entry would hold the lock for
/// milliseconds per login once it's full.
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

/// Limits failed logins, to stop password guessing.
///
/// The default allows 5 failures per account from each client address in
/// 5 minutes; after that, [`begin`](Self::begin) returns
/// `429 Too Many Requests` until the window ends. A successful login clears
/// the count.
///
/// ```ignore
/// async fn login(throttle: LoginThrottle, Form(form): Form<LoginForm>) -> Result<Response> {
///     // Counts as a failure from here on, unless marked as a success.
///     let attempt = throttle.begin(&form.email)?;
///     if password_is_right(&form).await? {
///         attempt.succeeded();
///         // ... log in ...
///     }
///     // ... show the form again ...
/// }
/// ```
///
/// `begin` reserves the attempt before the password is checked, and the
/// attempt counts until it ends, so sending many guesses at once doesn't
/// get more of them checked. If the request is cancelled before it
/// finishes, the attempt counts as a failure.
///
/// Counting per account *and* client address means guessing from one
/// address only ever locks out that address: someone typing the right
/// password from anywhere else still gets in, so the throttle can't be
/// used to lock people out of their accounts. An attacker with many
/// addresses gets 5 guesses per address; that's why Ironhold asks for
/// passwords of at least 12 characters. IPv6 addresses are grouped by
/// `/64` network. Behind a reverse proxy, set
/// [`SecurityConfig::trusted_proxies`](ironhold_security::SecurityConfig::trusted_proxies)
/// so the throttle sees visitors' addresses rather than the proxy's.
///
/// Use it as a handler argument; the client address comes from the
/// request. Without any setup, every app shares one default throttle;
/// register your own with `App::extension` to change the limits. The count
/// lives in memory, per server instance.
#[derive(Clone)]
pub struct LoginThrottle {
    shared: Arc<Shared>,
    client: Option<IpAddr>,
}

struct Shared {
    table: Mutex<Table>,
    max_failures: u32,
    window: Duration,
}

struct Table {
    entries: HashMap<Key, Window>,
    last_sweep: Option<Instant>,
    #[cfg(test)]
    sweeps: usize,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    account: String,
    client: Option<IpAddr>,
}

struct Window {
    failures: u32,
    /// Attempts that have started but not finished yet. They count against
    /// the limit, so concurrent guesses can't all get through.
    in_flight: u32,
    started: Instant,
}

impl Window {
    fn new(now: Instant) -> Self {
        Self {
            failures: 0,
            in_flight: 0,
            started: now,
        }
    }

    fn restart_if_expired(&mut self, now: Instant, window: Duration) {
        if now.duration_since(self.started) >= window {
            self.failures = 0;
            self.started = now;
        }
    }
}

impl LoginThrottle {
    /// Allows `max_failures` failed logins per account and client within
    /// `window`.
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            shared: Arc::new(Shared {
                table: Mutex::new(Table {
                    entries: HashMap::new(),
                    last_sweep: None,
                    #[cfg(test)]
                    sweeps: 0,
                }),
                max_failures,
                window,
            }),
            client: None,
        }
    }

    /// The same throttle, counting attempts for the client at `ip`. The
    /// extractor does this for you; use it when calling the throttle
    /// outside a handler.
    pub fn for_client(&self, ip: IpAddr) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            client: Some(ClientIp::new(ip).limit_key()),
        }
    }

    /// Starts a login attempt for `account` (usually the email or username
    /// typed in). Call it before checking the password.
    ///
    /// Returns `Err(Error::TooManyRequests)` if this client has used up
    /// its failures for the account, counting attempts still in progress.
    /// Otherwise the returned [`LoginAttempt`] counts as a failure unless
    /// you call [`succeeded`](LoginAttempt::succeeded).
    pub fn begin(&self, account: &str) -> Result<LoginAttempt, Error> {
        let key = Key {
            account: normalize(account),
            client: self.client,
        };
        let (max, window) = (self.shared.max_failures, self.shared.window);
        let now = Instant::now();
        let mut table = self.shared.lock();
        table.make_room(&key, now, window);
        let entry = table
            .entries
            .entry(key.clone())
            .or_insert_with(|| Window::new(now));
        entry.restart_if_expired(now, window);

        if entry.failures.saturating_add(entry.in_flight) >= max {
            let retry_after = if entry.failures >= max {
                window.saturating_sub(now.duration_since(entry.started))
            } else {
                // Only attempts in progress are in the way; they end soon.
                Duration::from_secs(1)
            };
            if entry.failures == 0 && entry.in_flight == 0 {
                // `max` is 0: nothing to remember.
                table.entries.remove(&key);
            }
            return Err(Error::TooManyRequests {
                retry_after_secs: retry_after.as_secs().max(1),
            });
        }
        entry.in_flight += 1;
        Ok(LoginAttempt {
            shared: Arc::clone(&self.shared),
            key: Some(key),
        })
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        // A panic while holding the lock can't leave the table inconsistent,
        // so keep using it rather than failing every login afterwards.
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finish(&self, key: &Key, succeeded: bool) {
        let now = Instant::now();
        let mut table = self.lock();
        if succeeded {
            if let Some(entry) = table.entries.get_mut(key) {
                entry.in_flight = entry.in_flight.saturating_sub(1);
                entry.failures = 0;
                if entry.in_flight == 0 {
                    table.entries.remove(key);
                }
            }
            return;
        }

        // The entry may have been evicted while the password was checked.
        table.make_room(key, now, self.window);
        let entry = table
            .entries
            .entry(key.clone())
            .or_insert_with(|| Window::new(now));
        entry.in_flight = entry.in_flight.saturating_sub(1);
        entry.restart_if_expired(now, self.window);
        entry.failures = entry.failures.saturating_add(1);
        if entry.failures == self.max_failures {
            tracing::warn!("login throttled after repeated failures");
        }
    }
}

impl Table {
    /// Keeps the table under [`MAX_TRACKED`] before `key` is added.
    fn make_room(&mut self, key: &Key, now: Instant, window: Duration) {
        if self.entries.len() < MAX_TRACKED || self.entries.contains_key(key) {
            return;
        }
        let due = self
            .last_sweep
            .is_none_or(|last| now.duration_since(last) >= SWEEP_INTERVAL);
        if due {
            self.last_sweep = Some(now);
            #[cfg(test)]
            {
                self.sweeps += 1;
            }
            self.entries.retain(|_, entry| {
                entry.in_flight > 0 || now.duration_since(entry.started) < window
            });
        }
        while self.entries.len() >= MAX_TRACKED {
            let Some(victim) = self.entries.keys().next().cloned() else {
                break;
            };
            self.entries.remove(&victim);
        }
    }
}

/// A login attempt started with [`LoginThrottle::begin`]. It counts as a
/// failure when dropped, unless [`succeeded`](Self::succeeded) was called,
/// so an attempt that is cancelled or returns early with `?` still counts.
#[must_use = "an attempt counts as a failure when dropped; call `succeeded()` after a correct password"]
pub struct LoginAttempt {
    shared: Arc<Shared>,
    key: Option<Key>,
}

impl LoginAttempt {
    /// The password was right: clears this client's failures for the
    /// account.
    pub fn succeeded(mut self) {
        if let Some(key) = self.key.take() {
            self.shared.finish(&key, true);
        }
    }

    /// The password was wrong. The same as dropping the attempt, but says
    /// so in the code.
    pub fn failed(self) {}
}

impl Drop for LoginAttempt {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.shared.finish(&key, false);
        }
    }
}

impl std::fmt::Debug for LoginAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginAttempt").finish_non_exhaustive()
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
            .field("max_failures", &self.shared.max_failures)
            .field("window", &self.shared.window)
            .finish_non_exhaustive()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for LoginThrottle {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        static SHARED: OnceLock<LoginThrottle> = OnceLock::new();
        let throttle = parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or_else(|| SHARED.get_or_init(Self::default).clone());
        Ok(match ClientIp::from_parts(parts) {
            Some(client) => throttle.for_client(client.ip()),
            None => throttle,
        })
    }
}

fn normalize(account: &str) -> String {
    account.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn blocks_after_too_many_failures_per_account() {
        let throttle = LoginThrottle::new(3, Duration::from_secs(60));
        for _ in 0..3 {
            throttle.begin("raj@example.com").unwrap().failed();
        }
        let error = throttle.begin(" RAJ@example.com ").unwrap_err();
        assert!(matches!(error, Error::TooManyRequests { .. }));
        // Other accounts are unaffected.
        assert!(throttle.begin("someone@example.com").is_ok());
    }

    #[test]
    fn attempts_in_progress_count_against_the_limit() {
        let throttle = LoginThrottle::new(5, Duration::from_secs(60));
        // Many guesses at once: only 5 may reach the password check.
        let started: Vec<_> = (0..100)
            .filter_map(|_| throttle.begin("raj@example.com").ok())
            .collect();
        assert_eq!(started.len(), 5);
        drop(started);
        assert!(throttle.begin("raj@example.com").is_err());
    }

    #[test]
    fn concurrent_attempts_from_many_threads_stay_within_the_limit() {
        let throttle = LoginThrottle::new(5, Duration::from_secs(60));
        let barrier = Arc::new(std::sync::Barrier::new(64));
        let checked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let threads: Vec<_> = (0..64)
            .map(|_| {
                let (throttle, barrier, checked) =
                    (throttle.clone(), barrier.clone(), checked.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    if let Ok(attempt) = throttle.begin("raj@example.com") {
                        checked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        // The slow password check.
                        std::thread::sleep(Duration::from_millis(20));
                        attempt.failed();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(checked.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn a_dropped_attempt_counts_as_a_failure() {
        let throttle = LoginThrottle::new(1, Duration::from_secs(60));
        drop(throttle.begin("a@example.com").unwrap());
        assert!(throttle.begin("a@example.com").is_err());
    }

    #[test]
    fn success_clears_failures() {
        let throttle = LoginThrottle::new(2, Duration::from_secs(60));
        throttle.begin("a@example.com").unwrap().failed();
        throttle.begin("a@example.com").unwrap().succeeded();
        throttle.begin("a@example.com").unwrap().failed();
        assert!(throttle.begin("a@example.com").is_ok());
    }

    #[test]
    fn window_expires() {
        let throttle = LoginThrottle::new(1, Duration::from_millis(20));
        throttle.begin("a@example.com").unwrap().failed();
        assert!(throttle.begin("a@example.com").is_err());
        std::thread::sleep(Duration::from_millis(30));
        assert!(throttle.begin("a@example.com").is_ok());
    }

    #[test]
    fn guesses_from_one_client_do_not_lock_out_others() {
        let throttle = LoginThrottle::new(5, Duration::from_secs(60));
        let attacker = throttle.for_client(ip("203.0.113.66"));
        for _ in 0..5 {
            attacker.begin("carol@example.com").unwrap().failed();
        }
        assert!(attacker.begin("carol@example.com").is_err());

        let carol = throttle.for_client(ip("198.51.100.7"));
        carol.begin("carol@example.com").unwrap().succeeded();

        // Another address in the attacker's IPv6 /64 is the same client.
        let v6 = throttle.for_client(ip("2001:db8:0:1::1"));
        for _ in 0..5 {
            v6.begin("carol@example.com").unwrap().failed();
        }
        let same_network = throttle.for_client(ip("2001:db8:0:1::ffff"));
        assert!(same_network.begin("carol@example.com").is_err());
    }

    #[test]
    fn a_full_table_is_not_scanned_on_every_new_entry() {
        let throttle = LoginThrottle::new(5, Duration::from_secs(300));
        for i in 0..MAX_TRACKED {
            throttle
                .begin(&format!("filler{i}@example.com"))
                .unwrap()
                .failed();
        }
        for i in 0..2_000 {
            throttle
                .begin(&format!("new{i}@example.com"))
                .unwrap()
                .failed();
        }
        let table = throttle.shared.lock();
        assert!(table.entries.len() <= MAX_TRACKED);
        // Scanning 100,000 entries takes about 2 ms, under the lock. Doing
        // it for each of these 2,000 logins would stall the server.
        assert!(table.sweeps <= 2, "{} full scans", table.sweeps);
    }
}
