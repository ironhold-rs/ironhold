//! Helpers shared by the tests.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ironhold::prelude::SqliteDb;

/// A fresh database for one test. The file is deleted when this is dropped.
pub struct TestDb {
    pub db: SqliteDb,
    path: PathBuf,
}

impl Drop for TestDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.path.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
    }
}

pub async fn test_db() -> Option<TestDb> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("blog-test-{}-{n}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let db = SqliteDb::connect(&format!("sqlite://{}", path.display()))
        .await
        .expect("open the test database");
    sqlx::migrate!()
        .run(db.pool())
        .await
        .expect("run migrations");
    Some(TestDb { db, path })
}

/// An email address no other test uses.
pub fn unique_email(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{name}-{nanos}@example.com")
}
