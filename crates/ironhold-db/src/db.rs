use std::{fmt, ops::Deref};

use axum::{extract::FromRequestParts, http::request::Parts};
use ironhold_core::Error;
use sqlx::{Database, Pool, pool::PoolOptions};

use crate::DbConfig;

/// A handle to the app's database: a connection pool that's cheap to clone.
///
/// Use it as a handler argument once the app is built with
/// [`database`](crate::AppDatabaseExt::database). It dereferences to the
/// sqlx [`Pool`], so it works anywhere sqlx expects an executor:
///
/// ```ignore
/// async fn count(db: SqliteDb) -> Result<String> {
///     let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts")
///         .fetch_one(&*db)
///         .await?;
///     Ok(n.to_string())
/// }
/// ```
pub struct Db<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> Db<DB> {
    pub(crate) fn from_pool(pool: Pool<DB>) -> Self {
        Self { pool }
    }

    /// The database registered with the app, read from a request. Handy
    /// inside code that receives request `Parts`, such as
    /// `LoadUser::load_user`.
    pub fn from_parts(parts: &Parts) -> Result<Self, Error> {
        parts.extensions.get::<Self>().cloned().ok_or_else(|| {
            Error::internal("no database configured: add .database(db) when building the App")
        })
    }

    /// The underlying sqlx connection pool.
    pub fn pool(&self) -> &Pool<DB> {
        &self.pool
    }

    /// Closes every connection, waiting for queries in progress to finish.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

impl<DB: Database> Clone for Db<DB> {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
        }
    }
}

impl<DB: Database> fmt::Debug for Db<DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db").field("pool", &self.pool).finish()
    }
}

impl<DB: Database> Deref for Db<DB> {
    type Target = Pool<DB>;

    fn deref(&self) -> &Pool<DB> {
        &self.pool
    }
}

impl<DB, S> FromRequestParts<S> for Db<DB>
where
    DB: Database,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Self::from_parts(parts)
    }
}

pub(crate) fn pool_options<DB: Database>(config: &DbConfig) -> PoolOptions<DB> {
    PoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(0)
        .acquire_timeout(config.acquire_timeout)
        .idle_timeout(config.idle_timeout)
        .max_lifetime(config.max_lifetime)
}
