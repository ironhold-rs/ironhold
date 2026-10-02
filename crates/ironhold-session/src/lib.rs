//! Server-side sessions and CSRF tokens for Ironhold.
//!
//! Sessions are stored on the server and identified by a random cookie. The
//! cookie is `HttpOnly`, `SameSite=Lax`, and in production `Secure` with the
//! `__Host-` prefix, so it can't be read by scripts, sent cross-site on
//! POSTs, or overwritten by a subdomain.
//!
//! [`CsrfToken`] gives each session a random token that forms must send back.
//! The `ironhold-forms` crate checks it before handing form data to a handler.

#![forbid(unsafe_code)]

mod config;
mod csrf;

pub use config::SessionConfig;
pub use csrf::{CSRF_FIELD, CSRF_HEADER, CsrfToken};
pub use tower_sessions::{MemoryStore, Session, SessionStore};

use tower_sessions::{
    Expiry, SessionManagerLayer,
    cookie::{SameSite, time},
};

/// Builds the session middleware for `store` using `config`.
pub fn layer<Store>(store: Store, config: &SessionConfig) -> SessionManagerLayer<Store>
where
    Store: SessionStore + Clone,
{
    let idle_seconds = i64::try_from(config.idle_timeout.as_secs()).unwrap_or(i64::MAX);

    SessionManagerLayer::new(store)
        .with_name(config.cookie_name.clone())
        .with_secure(config.secure)
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        .with_path("/")
        .with_expiry(Expiry::OnInactivity(time::Duration::seconds(idle_seconds)))
}
