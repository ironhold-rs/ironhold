use std::time::Duration;

/// Connection pool settings.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DbConfig {
    /// Most connections the pool keeps open.
    pub max_connections: u32,
    /// How long a request waits for a free connection before failing.
    /// Short on purpose: under overload, failing fast keeps the server
    /// responsive instead of queueing requests until everything times out.
    pub acquire_timeout: Duration,
    /// Close connections that have been idle this long.
    pub idle_timeout: Option<Duration>,
    /// Replace connections after this long, which picks up database
    /// failovers and limits slow memory growth on the database server.
    pub max_lifetime: Option<Duration>,
}

impl DbConfig {
    /// SQLite defaults: one connection per CPU core (between 2 and 16).
    /// SQLite work is CPU-bound and writes are serialised, so more
    /// connections than cores only adds contention.
    pub fn sqlite() -> Self {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        let max_connections = u32::try_from(cores.clamp(2, 16)).unwrap_or(4);
        Self {
            max_connections,
            ..Self::postgres()
        }
    }

    /// Postgres defaults: 10 connections. A good starting point is
    /// `database CPU cores * 2 + 1`; raise it only if requests wait for
    /// connections while the database still has spare CPU.
    pub fn postgres() -> Self {
        Self {
            max_connections: 10,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Some(Duration::from_secs(10 * 60)),
            max_lifetime: Some(Duration::from_secs(30 * 60)),
        }
    }
}
