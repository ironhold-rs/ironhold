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
//! - Remote Postgres connections require TLS unless the URL sets `sslmode`.
//! - A request waits at most 5 seconds for a free connection, then fails
//!   instead of piling up.
//!
//! sqlx 0.9 only accepts SQL written as string literals; building SQL from
//! runtime strings needs an explicit `AssertSqlSafe`. Values always go
//! through `.bind()`, so SQL injection needs a deliberate opt-out.

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
mod schema;
mod session_store;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use app::AppDatabaseExt;
pub use config::DbConfig;
pub use db::Db;
pub use session_store::DbSessionStore;
pub use sqlx;

/// A SQLite database handle. Use it as a handler argument.
#[cfg(feature = "sqlite")]
pub type SqliteDb = Db<sqlx::Sqlite>;

/// A Postgres database handle. Use it as a handler argument.
#[cfg(feature = "postgres")]
pub type PgDb = Db<sqlx::Postgres>;
