//! Database support for Ironhold, built on [sqlx] 0.9.
//!
//! Enable one backend with a Cargo feature:
//!
//! - `sqlite`: the default for new apps. No server to run, and the fastest
//!   option on a single machine.
//! - `postgres`: for apps that run on several servers or need Postgres
//!   features.
//!
//! ```ignore
//! let db = SqliteDb::connect("sqlite://app.db").await?;
//! sqlx::migrate!().run(&*db).await?; // your app's migrations
//!
//! App::new()
//!     .database(db)          // handlers can take `db: SqliteDb`
//!     .route("/", get(home)) // sessions are now stored in the database
//!     .serve()
//!     .await?;
//! ```
//!
//! Defaults are chosen to be fast and to fail fast:
//!
//! - SQLite runs in WAL mode (readers never block the writer) with
//!   `synchronous=NORMAL`, foreign keys on and a 5 second busy timeout.
//! - Remote Postgres connections use TLS and verify the server's
//!   certificate and host name (`sslmode=verify-full`) unless the URL sets
//!   `sslmode`.
//! - A request waits at most 5 seconds for a free connection, then fails
//!   instead of piling up.
//!
//! `sqlx::query` and friends only accept SQL written as string literals;
//! SQL from runtime strings needs an explicit `AssertSqlSafe`. For queries
//! built at runtime use [`QueryBuilder`], which also only accepts literals
//! and bound values (runtime SQL needs `push_unchecked`). sqlx's own
//! `sqlx::QueryBuilder` accepts any string, so Ironhold's `clippy.toml`,
//! and the one in apps made by `ironhold new`, forbid it.

#![forbid(unsafe_code)]
// With no backend feature enabled there's nothing to use the shared code.
#![cfg_attr(
    not(any(feature = "sqlite", feature = "postgres")),
    allow(dead_code, unused_macros)
)]

#[macro_use]
mod backend;
mod app;
mod config;
mod db;
#[cfg(feature = "postgres")]
mod postgres;
mod query_builder;
mod schema;
mod session_store;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use app::AppDatabaseExt;
pub use config::DbConfig;
pub use db::Db;
pub use query_builder::{QueryBuilder, Separated};
pub use session_store::DbSessionStore;
pub use sqlx;

/// A SQLite database handle. Use it as a handler argument.
#[cfg(feature = "sqlite")]
pub type SqliteDb = Db<sqlx::Sqlite>;

/// A Postgres database handle. Use it as a handler argument.
#[cfg(feature = "postgres")]
pub type PgDb = Db<sqlx::Postgres>;
