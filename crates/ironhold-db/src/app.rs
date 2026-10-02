use std::time::Duration;

use ironhold_core::App;
use sqlx::Database;
use tokio::time::MissedTickBehavior;
use tower_sessions::session_store::{ExpiredDeletion, SessionStore};

use crate::{Db, DbSessionStore};

/// How often expired sessions are deleted.
const SESSION_CLEANUP_PERIOD: Duration = Duration::from_secs(5 * 60);

/// Adds [`database`](AppDatabaseExt::database) to [`App`].
pub trait AppDatabaseExt: Sized {
    /// Connects the app to `db`:
    ///
    /// - handlers can take the database as an argument (`db: SqliteDb`)
    /// - sessions are stored in the database, so they survive restarts and
    ///   are shared between app instances
    /// - expired sessions are deleted every few minutes while the server runs
    fn database<DB>(self, db: Db<DB>) -> Self
    where
        DB: Database,
        DbSessionStore<DB>: SessionStore + ExpiredDeletion;
}

impl AppDatabaseExt for App {
    fn database<DB>(self, db: Db<DB>) -> Self
    where
        DB: Database,
        DbSessionStore<DB>: SessionStore + ExpiredDeletion,
    {
        let store = DbSessionStore::new(db.clone());
        let cleanup = store.clone();
        self.extension(db)
            .session_store(store)
            .spawn_on_serve(delete_expired_sessions(cleanup))
    }
}

async fn delete_expired_sessions<S: ExpiredDeletion>(store: S) {
    let mut interval = tokio::time::interval(SESSION_CLEANUP_PERIOD);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(error) = store.delete_expired().await {
            tracing::warn!(%error, "could not delete expired sessions");
        }
    }
}
