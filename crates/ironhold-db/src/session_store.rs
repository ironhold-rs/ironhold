use std::{collections::HashMap, fmt};

use serde_json::Value;
use sqlx::{Database, Pool};
use tower_sessions::session_store;

use crate::Db;

/// Stores sessions in the app's database, in the `ironhold_sessions` table.
///
/// [`database`](crate::AppDatabaseExt::database) sets this up for you,
/// including a background task that deletes expired sessions. Expired
/// sessions are never loaded, even before that task removes them.
pub struct DbSessionStore<DB: Database> {
    db: Db<DB>,
}

impl<DB: Database> DbSessionStore<DB> {
    /// A session store backed by `db`.
    pub fn new(db: Db<DB>) -> Self {
        Self { db }
    }

    pub(crate) fn pool(&self) -> &Pool<DB> {
        self.db.pool()
    }
}

impl<DB: Database> Clone for DbSessionStore<DB> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
        }
    }
}

impl<DB: Database> fmt::Debug for DbSessionStore<DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbSessionStore").finish_non_exhaustive()
    }
}

/// How many fresh IDs `create` tries before giving up. A collision between
/// 128-bit random IDs is practically impossible; the limit only guarantees
/// the loop ends.
pub(crate) const MAX_CREATE_ATTEMPTS: usize = 8;

pub(crate) fn encode(data: &HashMap<String, Value>) -> session_store::Result<String> {
    serde_json::to_string(data).map_err(|e| session_store::Error::Encode(e.to_string()))
}

pub(crate) fn decode(text: &str) -> session_store::Result<HashMap<String, Value>> {
    serde_json::from_str(text).map_err(|e| session_store::Error::Decode(e.to_string()))
}

pub(crate) fn backend(error: sqlx::Error) -> session_store::Error {
    session_store::Error::Backend(error.to_string())
}
