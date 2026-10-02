//! Migrations for Ironhold's own tables.
//!
//! These are tracked in `ironhold_migrations`, separate from sqlx's
//! `_sqlx_migrations` table, so they never clash with an app's migrations.
//! Every statement must work on both SQLite and Postgres. A released
//! migration is never edited; changes go in a new one.

use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct Migration {
    pub(crate) version: i64,
    pub(crate) description: &'static str,
    pub(crate) statements: &'static [&'static str],
}

pub(crate) const CREATE_MIGRATIONS_TABLE: &str = "CREATE TABLE IF NOT EXISTS ironhold_migrations (\
     version BIGINT PRIMARY KEY NOT NULL, \
     description TEXT NOT NULL, \
     applied_at BIGINT NOT NULL)";

pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    description: "create sessions table",
    statements: &[
        "CREATE TABLE ironhold_sessions (\
         id TEXT PRIMARY KEY NOT NULL, \
         data TEXT NOT NULL, \
         expires_at BIGINT NOT NULL)",
        "CREATE INDEX ironhold_sessions_expires_at ON ironhold_sessions (expires_at)",
    ],
}];

/// Seconds since the Unix epoch.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
