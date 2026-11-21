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
    /// Connections to a remote host use TLS and check the server's
    /// certificate and host name (`sslmode=verify-full`), unless the URL or
    /// the `PGSSLMODE` environment variable sets `sslmode`. Without the
    /// check, anyone on the network path could pretend to be the database
    /// and read or change every query.
    ///
    /// Certificates are checked against the Mozilla root certificates. For
    /// a database whose certificate comes from a private authority (Amazon
    /// RDS, Google Cloud SQL, a company CA), download the authority's
    /// certificate and point to it:
    /// `postgres://user:pass@db.internal/app?sslrootcert=/etc/ssl/rds-ca.pem`.
    ///
    /// Weaker modes are an explicit opt-out in the URL: `sslmode=verify-ca`
    /// skips the host name check, `sslmode=require` encrypts without
    /// checking who answers, and `sslmode=disable` turns TLS off. Ironhold
    /// logs a warning when a remote connection doesn't verify the server.
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        Self::connect_with(url, &DbConfig::postgres()).await
    }

    /// Like [`connect`](Self::connect), with custom pool settings.
    pub async fn connect_with(url: &str, config: &DbConfig) -> Result<Self, sqlx::Error> {
        let mode_from_env = std::env::var_os("PGSSLMODE").is_some();
        let options = connect_options(url, config, mode_from_env)?;
        let pool = pool_options(config).connect_with(options).await?;
        let db = Self::from_pool(pool);
        db.prepare().await?;
        Ok(db)
    }
}

fn connect_options(
    url: &str,
    config: &DbConfig,
    mode_from_env: bool,
) -> Result<PgConnectOptions, sqlx::Error> {
    let mut options =
        PgConnectOptions::from_str(url)?.statement_cache_capacity(config.statement_cache_capacity);
    let host = options.get_host().to_owned();
    if is_local(&host) {
        return Ok(options);
    }
    if !mode_from_env && !url_sets_ssl_mode(url) {
        options = options.ssl_mode(PgSslMode::VerifyFull);
    }
    let mode = options.get_ssl_mode();
    if !matches!(mode, PgSslMode::VerifyFull | PgSslMode::VerifyCa) {
        tracing::warn!(
            %host,
            ?mode,
            "this Postgres connection doesn't verify the server's certificate, so anyone \
             on the network path can read and change queries. Use sslmode=verify-full"
        );
    }
    Ok(options)
}

/// Whether the URL's query string sets the TLS mode itself. sqlx accepts
/// `sslmode` and `ssl-mode`.
fn url_sets_ssl_mode(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else {
        return false;
    };
    query
        .split('&')
        .filter_map(|pair| pair.split('=').next())
        .any(|key| key.eq_ignore_ascii_case("sslmode") || key.eq_ignore_ascii_case("ssl-mode"))
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
    use super::*;

    fn mode(url: &str, mode_from_env: bool) -> PgSslMode {
        connect_options(url, &DbConfig::postgres(), mode_from_env)
            .unwrap()
            .get_ssl_mode()
    }

    #[test]
    fn only_loopback_and_sockets_are_local() {
        assert!(is_local("localhost"));
        assert!(is_local("127.0.0.1"));
        assert!(is_local("/var/run/postgresql"));
        assert!(!is_local("db.example.com"));
        assert!(!is_local("10.0.0.5"));
    }

    #[test]
    fn remote_hosts_verify_the_server_by_default() {
        let remote = "postgres://app:secret@db.example.com/app";
        assert!(matches!(mode(remote, false), PgSslMode::VerifyFull));
        assert!(matches!(
            mode(
                "postgres://app@10.0.0.5:5432/app?application_name=web",
                false
            ),
            PgSslMode::VerifyFull
        ));
    }

    #[test]
    fn an_explicit_sslmode_is_respected() {
        let base = "postgres://app:secret@db.example.com/app";
        assert!(matches!(
            mode(&format!("{base}?sslmode=require"), false),
            PgSslMode::Require
        ));
        assert!(matches!(
            mode(&format!("{base}?ssl-mode=verify-ca"), false),
            PgSslMode::VerifyCa
        ));
        assert!(matches!(
            mode(
                &format!("{base}?application_name=web&sslmode=disable"),
                false
            ),
            PgSslMode::Disable
        ));
        // When `PGSSLMODE` chose the mode, Ironhold leaves sqlx's choice alone.
        let from_env = PgConnectOptions::from_str(base).unwrap().get_ssl_mode();
        assert_eq!(format!("{:?}", mode(base, true)), format!("{from_env:?}"));
    }

    #[test]
    fn local_connections_keep_the_default() {
        let url = "postgres://postgres:postgres@localhost:5432/postgres";
        let sqlx_default = PgConnectOptions::from_str(url).unwrap().get_ssl_mode();
        assert_eq!(
            format!("{:?}", mode(url, false)),
            format!("{sqlx_default:?}")
        );
    }

    #[test]
    fn statement_cache_comes_from_the_config() {
        let config = DbConfig {
            statement_cache_capacity: 7,
            ..DbConfig::postgres()
        };
        let options = connect_options("postgres://localhost/app", &config, false).unwrap();
        assert!(format!("{options:?}").contains("statement_cache_capacity: 7"));
    }
}
