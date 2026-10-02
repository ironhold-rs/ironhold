//! Internal migrations and the session store, against real databases.
//!
//! SQLite tests always run. Postgres tests run when
//! `IRONHOLD_TEST_POSTGRES_URL` is set and are skipped otherwise, unless
//! `IRONHOLD_REQUIRE_POSTGRES` is set (as in CI), which makes a missing URL
//! a failure.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(any(feature = "sqlite", feature = "postgres"))]

use std::collections::HashMap;

use tower_sessions::cookie::time::{Duration, OffsetDateTime};
use tower_sessions::session::{Id, Record};

fn record(ttl: Duration) -> Record {
    Record {
        id: Id::default(),
        data: HashMap::from([("user".to_owned(), serde_json::json!({ "id": 42 }))]),
        expiry_date: OffsetDateTime::now_utc() + ttl,
    }
}

/// The same tests for every backend.
///
/// `$setup` evaluates to `Option<(url, guard)>`: `None` skips the test, and
/// the guard cleans up when the test ends.
macro_rules! store_tests {
    ($Db:ty, $setup:expr) => {
        use ironhold_db::DbSessionStore;
        use tower_sessions::session_store::{ExpiredDeletion, SessionStore};

        use super::record;
        use tower_sessions::cookie::time::Duration;

        #[tokio::test]
        async fn internal_migrations_are_idempotent() {
            let Some((url, _guard)) = $setup else { return };
            let _first = <$Db>::connect(&url).await.unwrap();
            let db = <$Db>::connect(&url).await.unwrap();
            let versions: Vec<i64> =
                sqlx::query_scalar("SELECT version FROM ironhold_migrations ORDER BY version")
                    .fetch_all(&*db)
                    .await
                    .unwrap();
            assert_eq!(versions, vec![1]);
        }

        #[tokio::test]
        async fn session_round_trip() {
            let Some((url, _guard)) = $setup else { return };
            let store = DbSessionStore::new(<$Db>::connect(&url).await.unwrap());

            let mut session = record(Duration::hours(1));
            store.create(&mut session).await.unwrap();
            let loaded = store.load(&session.id).await.unwrap().unwrap();
            assert_eq!(loaded.data, session.data);
            assert_eq!(
                loaded.expiry_date.unix_timestamp(),
                session.expiry_date.unix_timestamp()
            );

            session
                .data
                .insert("theme".to_owned(), serde_json::json!("dark"));
            store.save(&session).await.unwrap();
            let loaded = store.load(&session.id).await.unwrap().unwrap();
            assert_eq!(loaded.data["theme"], "dark");

            store.delete(&session.id).await.unwrap();
            assert!(store.load(&session.id).await.unwrap().is_none());
        }

        #[tokio::test]
        async fn expired_sessions_are_never_loaded_and_get_deleted() {
            let Some((url, _guard)) = $setup else { return };
            let db = <$Db>::connect(&url).await.unwrap();
            let store = DbSessionStore::new(db.clone());

            let mut expired = record(Duration::seconds(-10));
            store.create(&mut expired).await.unwrap();
            assert!(store.load(&expired.id).await.unwrap().is_none());

            store.delete_expired().await.unwrap();
            let remaining: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM ironhold_sessions WHERE id = $1")
                    .bind(expired.id.to_string())
                    .fetch_one(&*db)
                    .await
                    .unwrap();
            assert_eq!(remaining, 0);
        }

        #[tokio::test]
        async fn create_never_overwrites_an_existing_session() {
            let Some((url, _guard)) = $setup else { return };
            let store = DbSessionStore::new(<$Db>::connect(&url).await.unwrap());

            let mut victim = record(Duration::hours(1));
            store.create(&mut victim).await.unwrap();

            let mut attacker = record(Duration::hours(1));
            attacker.id = victim.id;
            attacker
                .data
                .insert("user".to_owned(), serde_json::json!({ "id": 1 }));
            store.create(&mut attacker).await.unwrap();

            assert_ne!(attacker.id, victim.id, "a colliding id gets replaced");
            let loaded = store.load(&victim.id).await.unwrap().unwrap();
            assert_eq!(loaded.data, victim.data);
        }
    };
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use std::path::PathBuf;

    use ironhold_db::SqliteDb;

    /// Deletes the database file (and its WAL files) when dropped.
    pub(crate) struct TempFile(PathBuf);

    impl Drop for TempFile {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut path = self.0.clone().into_os_string();
                path.push(suffix);
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// A fresh database file in the system temp directory.
    pub(crate) fn temp_sqlite() -> Option<(String, TempFile)> {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("ironhold-test-{name}.db"));
        Some((format!("sqlite://{}", path.display()), TempFile(path)))
    }

    store_tests!(SqliteDb, temp_sqlite());

    #[tokio::test]
    async fn sqlite_uses_wal_and_foreign_keys() {
        let (url, _guard) = temp_sqlite().unwrap();
        let db = SqliteDb::connect(&url).await.unwrap();
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&*db)
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&*db)
            .await
            .unwrap();
        assert_eq!(foreign_keys, 1);
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use ironhold_db::PgDb;

    /// The shared test database from `IRONHOLD_TEST_POSTGRES_URL`, if set.
    pub(crate) fn test_postgres() -> Option<(String, ())> {
        match std::env::var("IRONHOLD_TEST_POSTGRES_URL") {
            Ok(url) => Some((url, ())),
            // CI sets IRONHOLD_REQUIRE_POSTGRES so these tests can't pass by
            // silently skipping.
            Err(_) if std::env::var_os("IRONHOLD_REQUIRE_POSTGRES").is_some() => {
                panic!("IRONHOLD_REQUIRE_POSTGRES is set but IRONHOLD_TEST_POSTGRES_URL is not")
            }
            Err(_) => {
                eprintln!("skipping: IRONHOLD_TEST_POSTGRES_URL is not set");
                None
            }
        }
    }

    store_tests!(PgDb, test_postgres());
}
