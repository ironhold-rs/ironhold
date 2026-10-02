use std::{fmt, sync::OnceLock};

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
        hasher()
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
    let matches =
        run_limited(
            move || match hasher().verify_password(password.as_bytes(), stored.as_str()) {
                Ok(()) => Ok(true),
                Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
                Err(e) => Err(Error::internal(e.to_string())),
            },
        )
        .await?;
    Ok(matches && user_exists)
}

/// Argon2id v1.3 with OWASP's recommended minimum cost: 19 MiB of memory,
/// 2 iterations, 1 lane. Pinned explicitly so a library update can't change
/// it silently. Verification reads the parameters stored in each hash, so
/// raising them later doesn't break existing passwords.
fn hasher() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::DEFAULT)
}

/// Runs CPU-heavy hashing on the blocking thread pool, at most one job per
/// CPU core at a time. Excess logins wait their turn instead of starving
/// the async workers that serve every other request.
async fn run_limited<T, F>(job: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    let slots = SLOTS.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Semaphore::new(cores)
    });
    let _slot = slots.acquire().await?;
    tokio::task::spawn_blocking(job).await?
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
    use super::*;

    fn secret(s: &str) -> Secret<String> {
        Secret::new(s.to_owned())
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
