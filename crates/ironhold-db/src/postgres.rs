use std::str::FromStr;

use sqlx::{
    Postgres,
    postgres::{PgConnectOptions, PgSslMode},
};

use crate::{Db, DbConfig, db::pool_options};

impl Db<Postgres> {
    /// Connects to the Postgres database at `url` (for example
    /// `postgres://user:pass@db.example.com/app`) with
    /// [`DbConfig::postgres`], and prepares Ironhold's own tables.
    ///
    /// Connections to a remote host require TLS unless the URL sets
    /// `sslmode` explicitly. Without this, a network attacker could make
    /// the connection fall back to plain text.
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        Self::connect_with(url, &DbConfig::postgres()).await
    }

    /// Like [`connect`](Self::connect), with custom pool settings.
    pub async fn connect_with(url: &str, config: &DbConfig) -> Result<Self, sqlx::Error> {
        let mut options = PgConnectOptions::from_str(url)?;
        if !url.contains("sslmode=") && !is_local(options.get_host()) {
            options = options.ssl_mode(PgSslMode::Require);
        }

        let pool = pool_options(config).connect_with(options).await?;
        let db = Self::from_pool(pool);
        db.prepare().await?;
        Ok(db)
    }
}

/// Local connections (loopback or a Unix socket) don't cross a network.
fn is_local(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]") || host.starts_with('/')
}

// The advisory lock serialises migration runs when several app instances
// start at the same time. It's released when the transaction ends.
impl_backend!(Postgres, Some("SELECT pg_advisory_xact_lock(7302949381)"));

#[cfg(test)]
mod tests {
    use super::is_local;

    #[test]
    fn only_loopback_and_sockets_are_local() {
        assert!(is_local("localhost"));
        assert!(is_local("127.0.0.1"));
        assert!(is_local("/var/run/postgresql"));
        assert!(!is_local("db.example.com"));
        assert!(!is_local("10.0.0.5"));
    }
}
