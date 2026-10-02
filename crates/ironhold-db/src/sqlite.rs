use std::{str::FromStr, time::Duration};

use sqlx::{
    Sqlite,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};

use crate::{Db, DbConfig, db::pool_options};

impl Db<Sqlite> {
    /// Opens the SQLite database at `url` (for example `sqlite://app.db`)
    /// with [`DbConfig::sqlite`], creating the file if needed, and prepares
    /// Ironhold's own tables.
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        Self::connect_with(url, &DbConfig::sqlite()).await
    }

    /// Like [`connect`](Self::connect), with custom pool settings.
    ///
    /// For an in-memory database (`sqlite::memory:`), set
    /// `max_connections` to 1: every connection to an in-memory database
    /// sees a separate, empty database.
    pub async fn connect_with(url: &str, config: &DbConfig) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            // Readers never block the writer, and the writer never blocks readers.
            .journal_mode(SqliteJournalMode::Wal)
            // Safe with WAL (a power cut can lose the last commits, never
            // corrupt the file) and much faster than FULL.
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            // Wait for a busy writer instead of failing immediately.
            .busy_timeout(Duration::from_secs(5))
            // Keep query planner statistics fresh.
            .optimize_on_close(true, None);

        let pool = pool_options(config).connect_with(options).await?;
        let db = Self::from_pool(pool);
        db.prepare().await?;
        Ok(db)
    }
}

impl_backend!(Sqlite, None);
