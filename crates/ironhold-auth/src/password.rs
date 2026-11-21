use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use ironhold_core::Error;
use ironhold_security::Secret;
use tokio::sync::{OnceCell, Semaphore};

/// Minimum length of a new password, in characters.
pub const MIN_PASSWORD_CHARS: usize = 12;

/// Maximum length of a password, in bytes. Longer input is rejected before
/// hashing so it can't be used to waste server time.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// An Argon2id password hash in PHC string format. Safe to store; never
/// printed in logs.
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    /// Wraps a hash previously produced by [`hash_password`] and stored in
    /// the database.
    pub fn from_stored(phc: impl Into<String>) -> Self {
        Self(phc.into())
    }

    /// The PHC string to store in the database.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the hash, returning the PHC string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordHash([REDACTED])")
    }
}

/// Why a new password was rejected. `Display` gives a message suitable for
/// showing to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PasswordProblem {
    /// Shorter than [`MIN_PASSWORD_CHARS`].
    TooShort,
    /// Longer than [`MAX_PASSWORD_BYTES`].
    TooLong,
}

impl fmt::Display for PasswordProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(
                f,
                "Use at least {MIN_PASSWORD_CHARS} characters. A few unrelated words make a strong, memorable password."
            ),
            Self::TooLong => f.write_str("That password is too long."),
        }
    }
}

/// Checks a new password (at signup or password change) against the length
/// rules. Length matters far more than character classes, so there are no
/// "must contain a digit" rules.
pub fn check_new_password(password: &Secret<String>) -> Result<(), PasswordProblem> {
    let password = password.expose();
    if password.len() > MAX_PASSWORD_BYTES {
        Err(PasswordProblem::TooLong)
    } else if password.chars().count() < MIN_PASSWORD_CHARS {
        Err(PasswordProblem::TooShort)
    } else {
        Ok(())
    }
}

/// Hashes a password with Argon2id and a random salt.
pub async fn hash_password(password: &Secret<String>) -> Result<PasswordHash, Error> {
    if password.expose().len() > MAX_PASSWORD_BYTES {
        return Err(Error::BadRequest("That password is too long.".to_owned()));
    }
    let password = password.expose().clone();
    run_limited(move || {
        hasher()?
            .hash_password(password.as_bytes())
            .map(|hash| PasswordHash(hash.to_string()))
            .map_err(|e| Error::internal(e.to_string()))
    })
    .await
}

/// Checks `password` against `hash`.
///
/// Pass `None` when no user matched the login name: a dummy hash is checked
/// instead, so the response takes the same time and attackers can't tell
/// which accounts exist. Returns `false` in that case.
pub async fn verify_password(
    hash: Option<&PasswordHash>,
    password: &Secret<String>,
) -> Result<bool, Error> {
    if password.expose().len() > MAX_PASSWORD_BYTES {
        return Ok(false);
    }
    let user_exists = hash.is_some();
    let stored = match hash {
        Some(hash) => hash.0.clone(),
        None => dummy_hash().await?.0.clone(),
    };
    let password = password.expose().clone();
    let matches = run_limited(move || {
        match hasher()?.verify_password(password.as_bytes(), stored.as_str()) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
            Err(e) => Err(Error::internal(e.to_string())),
        }
    })
    .await?;
    Ok(matches && user_exists)
}

/// Argon2id memory cost in KiB: 19 MiB, OWASP's recommended minimum.
const ARGON2_MEMORY_KIB: u32 = 19_456;
/// Argon2id iterations.
const ARGON2_ITERATIONS: u32 = 2;
/// Argon2id lanes.
const ARGON2_LANES: u32 = 1;

/// Argon2id v1.3 with OWASP's recommended minimum cost: 19 MiB of memory,
/// 2 iterations, 1 lane. Pinned here rather than taken from the library's
/// defaults, so a library update can't change it silently. Verification
/// reads the parameters stored in each hash, so raising them later doesn't
/// break existing passwords.
fn hasher() -> Result<Argon2<'static>, Error> {
    let params = Params::new(ARGON2_MEMORY_KIB, ARGON2_ITERATIONS, ARGON2_LANES, None)
        .map_err(|e| Error::internal(e.to_string()))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Runs CPU-heavy hashing on the blocking thread pool, at most one job per
/// CPU core at a time. Excess logins wait their turn instead of starving
/// the async workers that serve every other request.
///
/// The slot travels into the blocking job and is only released when the
/// job ends. A job on the blocking pool can't be stopped, so if the slot
/// stayed with the request, a client that hangs up mid-hash would free its
/// slot while the hash (and its 19 MiB) kept going, and the next request
/// would start another one.
async fn run_limited<T, F>(job: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    let slot = Arc::clone(hash_slots()).acquire_owned().await?;
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        #[cfg(test)]
        let _running = tests::Running::start();
        job()
    })
    .await?
}

/// One slot per CPU core.
fn hash_slots() -> &'static Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(hash_slot_count())))
}

fn hash_slot_count() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get())
}

/// A hash of a random password, used when a login names an unknown user.
async fn dummy_hash() -> Result<&'static PasswordHash, Error> {
    static DUMMY: OnceCell<PasswordHash> = OnceCell::const_new();
    DUMMY
        .get_or_try_init(|| async {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|e| Error::internal(e.to_string()))?;
            let random: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            hash_password(&Secret::new(random)).await
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use super::*;

    /// Hash jobs running right now, and the most seen at once.
    static RUNNING: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);

    pub(super) struct Running;

    impl Running {
        pub(super) fn start() -> Self {
            let now = RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
            PEAK.fetch_max(now, Ordering::SeqCst);
            Self
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            RUNNING.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn secret(s: &str) -> Secret<String> {
        Secret::new(s.to_owned())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelled_hashes_keep_their_slot_until_they_finish() {
        let limit = hash_slot_count();
        // Each login gives up after 2 ms, the way hyper drops a handler
        // when the client hangs up, while its hash takes much longer.
        let logins: Vec<_> = (0..limit * 8)
            .map(|i| {
                tokio::spawn(async move {
                    let password = secret(&format!("password number {i} xxxxxxxx"));
                    let _ =
                        tokio::time::timeout(Duration::from_millis(2), hash_password(&password))
                            .await;
                })
            })
            .collect();
        for login in logins {
            login.await.unwrap();
        }
        // Let the abandoned jobs finish.
        for _ in 0..500 {
            if RUNNING.load(Ordering::SeqCst) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let peak = PEAK.load(Ordering::SeqCst);
        assert!(peak <= limit, "{peak} hashes ran at once, limit {limit}");
    }

    #[test]
    fn parameters_are_pinned_in_code() {
        let hasher = hasher().unwrap();
        let params = hasher.params();
        assert_eq!(params.m_cost(), 19_456);
        assert_eq!(params.t_cost(), 2);
        assert_eq!(params.p_cost(), 1);
    }

    #[tokio::test]
    async fn hash_then_verify() {
        let hash = hash_password(&secret("correct horse battery"))
            .await
            .unwrap();
        assert!(hash.as_str().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(
            verify_password(Some(&hash), &secret("correct horse battery"))
                .await
                .unwrap()
        );
        assert!(
            !verify_password(Some(&hash), &secret("wrong horse battery"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn same_password_gets_a_different_salt() {
        let a = hash_password(&secret("correct horse battery"))
            .await
            .unwrap();
        let b = hash_password(&secret("correct horse battery"))
            .await
            .unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn unknown_user_never_verifies() {
        assert!(
            !verify_password(None, &secret("anything at all"))
                .await
                .unwrap()
        );
    }

    #[test]
    fn new_password_rules() {
        assert_eq!(
            check_new_password(&secret("short")),
            Err(PasswordProblem::TooShort)
        );
        assert_eq!(check_new_password(&secret("twelve chars")), Ok(()));
        // Counted in characters, not bytes: 11 two-byte characters is too short.
        assert_eq!(
            check_new_password(&secret(&"\u{e9}".repeat(11))),
            Err(PasswordProblem::TooShort)
        );
        let huge = "a".repeat(MAX_PASSWORD_BYTES + 1);
        assert_eq!(
            check_new_password(&secret(&huge)),
            Err(PasswordProblem::TooLong)
        );
    }

    #[test]
    fn hash_is_redacted_in_debug() {
        let hash = PasswordHash::from_stored("$argon2id$secret-looking");
        assert_eq!(format!("{hash:?}"), "PasswordHash([REDACTED])");
    }
}
