use std::{collections::HashMap, fmt, fmt::Write};

use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Database, Pool};
use tower_sessions::{session::Id, session_store};

use crate::Db;

/// Stores sessions in the app's database, in the `ironhold_sessions` table.
///
/// [`database`](crate::AppDatabaseExt::database) sets this up for you,
/// including a background task that deletes expired sessions. Expired
/// sessions are never loaded, even before that task removes them.
///
/// The table holds a SHA-256 hash of each session id, never the id itself,
/// so someone who can read the table (from a backup, or through a read-only
/// SQL injection) can't use the rows to take over sessions.
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

/// The value stored in the `id` column for session `id`: the SHA-256 of
/// its cookie value, in hex. Session ids are 128 random bits, so no salt
/// is needed.
pub(crate) fn storage_id(id: &Id) -> String {
    let digest = Sha256::digest(id.to_string().as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest.iter() {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

pub(crate) fn encode(data: &HashMap<String, Value>) -> session_store::Result<String> {
    serde_json::to_string(data).map_err(|e| session_store::Error::Encode(e.to_string()))
}

pub(crate) fn decode(text: &str) -> session_store::Result<HashMap<String, Value>> {
    serde_json::from_str(text).map_err(|e| session_store::Error::Decode(e.to_string()))
}

pub(crate) fn backend(error: sqlx::Error) -> session_store::Error {
    session_store::Error::Backend(error.to_string())
}
