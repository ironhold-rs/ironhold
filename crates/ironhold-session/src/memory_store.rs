use std::{
    collections::{BTreeSet, HashMap},
    fmt,
    sync::{Arc, Mutex, PoisonError},
};

use async_trait::async_trait;
use tower_sessions::{
    cookie::time::OffsetDateTime,
    session::{Id, Record},
    session_store::{self, ExpiredDeletion, SessionStore},
};

/// Most sessions a [`MemoryStore`] keeps by default.
pub const DEFAULT_MAX_SESSIONS: usize = 100_000;

/// Keeps sessions in memory. The default store for apps without a database.
///
/// Expired sessions are never loaded, are removed by
/// [`remove_expired`](Self::remove_expired) (`App::serve` calls it every few
/// minutes), and the store never holds more than its maximum: when it's
/// full, the session closest to expiring is dropped to make room. So a flood
/// of visitors without cookies can't grow memory without bound.
///
/// Sessions are lost on restart and aren't shared between server
/// instances. Production apps should store them in a database.
#[derive(Clone)]
pub struct MemoryStore {
    inner: Arc<Mutex<Sessions>>,
    max_sessions: usize,
}

#[derive(Default)]
struct Sessions {
    records: HashMap<Id, Record>,
    /// Every session ordered by expiry, so the next one to go is cheap to
    /// find. Holds exactly the ids in `records`.
    by_expiry: BTreeSet<(OffsetDateTime, i128)>,
}

impl Sessions {
    fn insert(&mut self, record: Record) {
        if let Some(old) = self.records.remove(&record.id) {
            self.by_expiry.remove(&(old.expiry_date, old.id.0));
        }
        self.by_expiry.insert((record.expiry_date, record.id.0));
        self.records.insert(record.id, record);
    }

    fn remove(&mut self, id: &Id) {
        if let Some(old) = self.records.remove(id) {
            self.by_expiry.remove(&(old.expiry_date, old.id.0));
        }
    }

    /// Removes sessions that expired before `now`; returns how many.
    fn remove_expired(&mut self, now: OffsetDateTime) -> usize {
        let mut removed = 0;
        while let Some(&(expiry, id)) = self.by_expiry.first() {
            if expiry > now {
                break;
            }
            self.by_expiry.pop_first();
            self.records.remove(&Id(id));
            removed += 1;
        }
        removed
    }

    /// Makes room for one more session if the store is at `max`.
    fn make_room(&mut self, max: usize, now: OffsetDateTime) {
        if self.records.len() < max {
            return;
        }
        self.remove_expired(now);
        while self.records.len() >= max.max(1) {
            let Some((_, id)) = self.by_expiry.pop_first() else {
                break;
            };
            self.records.remove(&Id(id));
        }
    }
}

impl MemoryStore {
    /// A store holding at most [`DEFAULT_MAX_SESSIONS`] sessions.
    pub fn new() -> Self {
        Self::with_max_sessions(DEFAULT_MAX_SESSIONS)
    }

    /// A store holding at most `max_sessions` sessions.
    pub fn with_max_sessions(max_sessions: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Sessions::default())),
            max_sessions,
        }
    }

    /// Removes every expired session and returns how many were removed.
    pub fn remove_expired(&self) -> usize {
        self.lock().remove_expired(OffsetDateTime::now_utc())
    }

    /// How many sessions are stored, including expired ones not yet
    /// removed.
    pub fn len(&self) -> usize {
        self.lock().records.len()
    }

    /// Whether the store holds no sessions.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Sessions> {
        // Every update leaves both maps consistent before it can panic, so
        // keep serving rather than failing every request afterwards.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryStore")
            .field("max_sessions", &self.max_sessions)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SessionStore for MemoryStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        let mut sessions = self.lock();
        while sessions.records.contains_key(&record.id) {
            // A new session must never overwrite someone else's.
            record.id = Id::default();
        }
        sessions.make_room(self.max_sessions, OffsetDateTime::now_utc());
        sessions.insert(record.clone());
        Ok(())
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let mut sessions = self.lock();
        if !sessions.records.contains_key(&record.id) {
            sessions.make_room(self.max_sessions, OffsetDateTime::now_utc());
        }
        sessions.insert(record.clone());
        Ok(())
    }

    async fn load(&self, id: &Id) -> session_store::Result<Option<Record>> {
        let mut sessions = self.lock();
        match sessions.records.get(id) {
            Some(record) if record.expiry_date > OffsetDateTime::now_utc() => {
                Ok(Some(record.clone()))
            }
            Some(_) => {
                sessions.remove(id);
                Ok(None)
            }
            None => Ok(None),
        }
    }

    async fn delete(&self, id: &Id) -> session_store::Result<()> {
        self.lock().remove(id);
        Ok(())
    }
}

#[async_trait]
impl ExpiredDeletion for MemoryStore {
    async fn delete_expired(&self) -> session_store::Result<()> {
        self.remove_expired();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_sessions::cookie::time::Duration;

    fn record(expires_in: Duration) -> Record {
        Record {
            id: Id::default(),
            data: HashMap::new(),
            expiry_date: OffsetDateTime::now_utc() + expires_in,
        }
    }

    #[tokio::test]
    async fn expired_sessions_are_not_loaded_and_are_removed() {
        let store = MemoryStore::new();
        let mut live = record(Duration::hours(1));
        let mut expired = record(Duration::seconds(-1));
        store.create(&mut live).await.unwrap();
        store.create(&mut expired).await.unwrap();
        assert_eq!(store.len(), 2);

        assert!(store.load(&expired.id).await.unwrap().is_none());
        assert_eq!(store.load(&live.id).await.unwrap(), Some(live.clone()));

        let mut another = record(Duration::seconds(-1));
        store.save(&another).await.unwrap();
        another.id = Id::default();
        store.save(&another).await.unwrap();
        assert_eq!(store.remove_expired(), 2);
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn the_store_stays_bounded() {
        let store = MemoryStore::with_max_sessions(3);
        let mut oldest = record(Duration::minutes(1));
        store.create(&mut oldest).await.unwrap();
        for minutes in 2..=10 {
            store
                .create(&mut record(Duration::minutes(minutes)))
                .await
                .unwrap();
        }
        assert_eq!(store.len(), 3);
        // The session closest to expiring made room first.
        assert!(store.load(&oldest.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn saving_moves_a_session_in_the_expiry_order() {
        let store = MemoryStore::with_max_sessions(2);
        let mut first = record(Duration::minutes(1));
        let mut second = record(Duration::minutes(2));
        store.create(&mut first).await.unwrap();
        store.create(&mut second).await.unwrap();

        // Activity extends `first`, so `second` is now the next to expire.
        first.expiry_date = OffsetDateTime::now_utc() + Duration::minutes(5);
        store.save(&first).await.unwrap();
        store
            .create(&mut record(Duration::minutes(3)))
            .await
            .unwrap();

        assert!(store.load(&first.id).await.unwrap().is_some());
        assert!(store.load(&second.id).await.unwrap().is_none());
        assert_eq!(store.len(), 2);
    }

    #[tokio::test]
    async fn delete_removes_both_indexes() {
        let store = MemoryStore::new();
        let mut session = record(Duration::hours(1));
        store.create(&mut session).await.unwrap();
        store.delete(&session.id).await.unwrap();
        assert!(store.is_empty());
        assert_eq!(store.lock().by_expiry.len(), 0);
    }
}
