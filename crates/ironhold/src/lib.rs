//! # Ironhold
//!
//! A full-stack Rust web framework that is secure by default, productive
//! and fast.
//!
//! ```no_run
//! use ironhold::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     App::new()
//!         .route("/hello/{name}", get(hello))
//!         .serve()
//!         .await
//! }
//!
//! async fn hello(Path(name): Path<String>) -> Markup {
//!     // `name` is escaped automatically, so XSS is not possible here.
//!     html! { h1 { "Hello, " (name) "!" } }
//! }
//! ```
//!
//! Every app gets these without any configuration:
//! - Content-Security-Policy with a per-request nonce ([`CspNonce`])
//! - `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy`,
//!   `Permissions-Policy`, COOP/CORP, and HSTS in production
//! - Cross-origin protection for POST, PUT, PATCH and DELETE requests
//! - Server-side sessions with hardened cookies ([`Session`])
//! - CSRF tokens checked by the [`Form`] extractor ([`CsrfToken`])
//! - Request body size limits and request timeouts
//! - Auto-escaping HTML templates ([`html!`])
//! - Error responses that never leak internal details ([`Error`])
//! - Login and access control enforced by types ([`auth`])
//! - Overload protection: a fast `503` instead of slowing down for everyone
//! - Handler panics become `500` responses; the server keeps running
//!
//! With the `sqlite` or `postgres` feature, `ironhold::db` adds a database
//! with tuned defaults and database-backed sessions.

#![forbid(unsafe_code)]

pub use ironhold_core::{App, Config, ConfigError, Environment, Error, Result};
pub use ironhold_core::{Router, body, extract, http, response, routing};
pub use ironhold_forms::Form;
pub use ironhold_html::{DOCTYPE, Markup, Render, html, raw_unchecked};
pub use ironhold_security::{CspNonce, Secret, SecurityConfig};
pub use ironhold_session::{CsrfToken, MemoryStore, Session, SessionConfig, SessionStore};

/// Authentication and authorization: passwords, login, `AuthUser`,
/// `Authorize` and login throttling.
pub use ironhold_auth as auth;

/// Testing helpers. Requires the `testing` feature.
#[cfg(feature = "testing")]
pub mod testing;

/// Database support (SQLite and Postgres). Requires the `sqlite` or
/// `postgres` feature.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub use ironhold_db as db;

/// Low-level access to the underlying crates.
pub mod crates {
    pub use ironhold_auth as auth;
    pub use ironhold_core as core;
    pub use ironhold_forms as forms;
    pub use ironhold_html as html;
    pub use ironhold_security as security;
    pub use ironhold_session as session;
}

/// Everything most apps need: `use ironhold::prelude::*;`
pub mod prelude {
    pub use crate::auth::{AuthUser, Authorize, LoadUser, LoginThrottle, Permission, Policy};
    pub use crate::extract::{Json, Path, Query, State};
    pub use crate::response::{IntoResponse, Redirect};
    pub use crate::routing::{delete, get, patch, post, put};
    pub use crate::{
        App, CspNonce, CsrfToken, DOCTYPE, Error, Form, Markup, Result, Secret, Session, html,
    };

    #[cfg(feature = "postgres")]
    pub use crate::db::PgDb;
    #[cfg(feature = "sqlite")]
    pub use crate::db::SqliteDb;
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    pub use crate::db::{AppDatabaseExt, Db};
}
