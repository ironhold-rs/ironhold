//! Authentication and authorization for Ironhold.
//!
//! Passwords: [`hash_password`] and [`verify_password`] use Argon2id with
//! the OWASP-recommended parameters. Hashing runs on the blocking thread
//! pool, one job per CPU core at most, so a flood of logins can't stall
//! other requests. Checking a login for an unknown account takes as long as
//! checking a real one, so attackers can't tell which accounts exist.
//!
//! Sessions: [`login`] issues a new session id and CSRF token, which
//! prevents session fixation. [`logout`] deletes the session.
//!
//! Access control: a handler that takes [`AuthUser<U>`] can't run for
//! logged-out visitors, and one that takes [`Authorize<U, P>`] can't run
//! unless policy `P` allows the user. Use `Option<AuthUser<U>>` for pages
//! that work either way.
//!
//! Password guessing: [`LoginThrottle`] limits failed logins per account.

#![forbid(unsafe_code)]

mod extract;
mod password;
mod redirect;
mod session;
mod throttle;

pub use extract::{AuthConfig, AuthUser, Authorize, LoadUser, Permission, Policy};
pub use password::{
    MAX_PASSWORD_BYTES, MIN_PASSWORD_CHARS, PasswordHash, PasswordProblem, check_new_password,
    hash_password, verify_password,
};
pub use redirect::safe_redirect_path;
pub use session::{login, logout};
pub use throttle::LoginThrottle;
