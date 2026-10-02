//! Code shared by every backend. The SQL is identical for SQLite and
//! Postgres, but sqlx's types differ per database, so a macro stamps out one
//! copy per backend instead of a generic version with long trait bounds.

/// Implements internal migrations and the session store for `$DB`.
/// `$lock` is an optional statement that serialises concurrent migration
/// runs (for example two app instances starting at once).
macro_rules! impl_backend {
    ($DB:ty, $lock:expr) => {
        impl $crate::Db<$DB> {
            /// Applies Ironhold's own migrations in one transaction.
            pub(crate) async fn prepare(&self) -> Result<(), sqlx::Error> {
                let mut tx = self.pool().begin().await?;

                let lock: Option<&'static str> = $lock;
                if let Some(lock) = lock {
                    sqlx::query(lock).execute(&mut *tx).await?;
                }

                sqlx::query($crate::schema::CREATE_MIGRATIONS_TABLE)
                    .execute(&mut *tx)
                    .await?;
                let applied: Vec<i64> =
                    sqlx::query_scalar("SELECT version FROM ironhold_migrations")
                        .fetch_all(&mut *tx)
                        .await?;

                for migration in $crate::schema::MIGRATIONS {
                    if applied.contains(&migration.version) {
                        continue;
                    }
                    for statement in migration.statements {
                        sqlx::query(*statement).execute(&mut *tx).await?;
                    }
                    sqlx::query(
                        "INSERT INTO ironhold_migrations (version, description, applied_at) \
                         VALUES ($1, $2, $3)",
                    )
                    .bind(migration.version)
                    .bind(migration.description)
                    .bind($crate::schema::unix_now())
                    .execute(&mut *tx)
                    .await?;
                    tracing::info!(
                        version = migration.version,
                        description = migration.description,
                        "applied Ironhold migration"
                    );
                }

                tx.commit().await
            }
        }

        #[async_trait::async_trait]
        impl tower_sessions::session_store::SessionStore for $crate::DbSessionStore<$DB> {
            async fn create(
                &self,
                record: &mut tower_sessions::session::Record,
            ) -> tower_sessions::session_store::Result<()> {
                use $crate::session_store::{MAX_CREATE_ATTEMPTS, backend, encode};

                let data = encode(&record.data)?;
                for _ in 0..MAX_CREATE_ATTEMPTS {
                    // DO NOTHING on conflict: a new session must never
                    // overwrite someone else's.
                    let result = sqlx::query(
                        "INSERT INTO ironhold_sessions (id, data, expires_at) \
                         VALUES ($1, $2, $3) ON CONFLICT (id) DO NOTHING",
                    )
                    .bind(record.id.to_string())
                    .bind(&data)
                    .bind(record.expiry_date.unix_timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(backend)?;
                    if result.rows_affected() == 1 {
                        return Ok(());
                    }
                    record.id = tower_sessions::session::Id::default();
                }
                Err(tower_sessions::session_store::Error::Backend(
                    "could not allocate a unique session id".to_owned(),
                ))
            }

            async fn save(
                &self,
                record: &tower_sessions::session::Record,
            ) -> tower_sessions::session_store::Result<()> {
                use $crate::session_store::{backend, encode};

                sqlx::query(
                    "INSERT INTO ironhold_sessions (id, data, expires_at) VALUES ($1, $2, $3) \
                     ON CONFLICT (id) DO UPDATE \
                     SET data = excluded.data, expires_at = excluded.expires_at",
                )
                .bind(record.id.to_string())
                .bind(encode(&record.data)?)
                .bind(record.expiry_date.unix_timestamp())
                .execute(self.pool())
                .await
                .map_err(backend)?;
                Ok(())
            }

            async fn load(
                &self,
                id: &tower_sessions::session::Id,
            ) -> tower_sessions::session_store::Result<Option<tower_sessions::session::Record>>
            {
                use tower_sessions::cookie::time::OffsetDateTime;
                use $crate::session_store::{backend, decode};

                let row: Option<(String, i64)> = sqlx::query_as(
                    "SELECT data, expires_at FROM ironhold_sessions \
                     WHERE id = $1 AND expires_at > $2",
                )
                .bind(id.to_string())
                .bind($crate::schema::unix_now())
                .fetch_optional(self.pool())
                .await
                .map_err(backend)?;

                let Some((data, expires_at)) = row else {
                    return Ok(None);
                };
                let expiry_date = OffsetDateTime::from_unix_timestamp(expires_at)
                    .map_err(|e| tower_sessions::session_store::Error::Decode(e.to_string()))?;
                Ok(Some(tower_sessions::session::Record {
                    id: *id,
                    data: decode(&data)?,
                    expiry_date,
                }))
            }

            async fn delete(
                &self,
                id: &tower_sessions::session::Id,
            ) -> tower_sessions::session_store::Result<()> {
                sqlx::query("DELETE FROM ironhold_sessions WHERE id = $1")
                    .bind(id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err($crate::session_store::backend)?;
                Ok(())
            }
        }

        #[async_trait::async_trait]
        impl tower_sessions::session_store::ExpiredDeletion for $crate::DbSessionStore<$DB> {
            async fn delete_expired(&self) -> tower_sessions::session_store::Result<()> {
                sqlx::query("DELETE FROM ironhold_sessions WHERE expires_at <= $1")
                    .bind($crate::schema::unix_now())
                    .execute(self.pool())
                    .await
                    .map_err($crate::session_store::backend)?;
                Ok(())
            }
        }
    };
}
