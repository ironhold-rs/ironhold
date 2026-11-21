use std::time::Duration;

use crate::IpRange;

/// Security settings: the layers applied by [`harden`](crate::harden), and
/// the connection limits `App::serve` applies.
///
/// Every field has a safe default. Loosen them deliberately, not by accident.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SecurityConfig {
    /// Maximum accepted request body size in bytes. Larger requests get `413`.
    pub max_body_bytes: usize,
    /// Maximum time a handler may take once the request body has arrived.
    /// Slower requests are aborted with `408`.
    pub request_timeout: Duration,
    /// How long a client gets to send the request body. A client that keeps
    /// sending at least [`min_body_rate`](Self::min_body_rate) on average
    /// may take longer, up to [`request_timeout`](Self::request_timeout).
    /// Slower uploads are aborted with `408`.
    pub body_timeout: Duration,
    /// Slowest average upload speed, in bytes per second, accepted once
    /// [`body_timeout`](Self::body_timeout) has passed.
    pub min_body_rate: u64,
    /// Request bodies one client address may be uploading at the same time.
    /// Further uploads get `429`. IPv6 clients are grouped by their `/64`
    /// network, which is usually one household or one server.
    pub max_uploads_per_client: usize,
    /// Requests allowed in flight at once. Beyond this, new requests are
    /// rejected immediately with `503` and `Retry-After`, so an overloaded
    /// server stays fast for the requests it accepts instead of getting
    /// slower for everyone.
    ///
    /// Request bodies are read before a request takes one of these slots,
    /// so slow uploads can't use them up.
    pub max_concurrent_requests: usize,
    /// How long a client gets to send the request line and headers. A
    /// connection that stays idle this long between requests is closed too.
    pub header_read_timeout: Duration,
    /// Most connections open at once. Beyond this, new connections wait in
    /// the operating system's queue until one closes. Keep it below the
    /// process's open file limit (`ulimit -n`), which the database needs too.
    pub max_connections: usize,
    /// Send `Strict-Transport-Security` and `upgrade-insecure-requests`.
    /// Only enable this when the app is served over HTTPS.
    pub hsts: bool,
    /// Other origins allowed to send POST, PUT, PATCH and DELETE requests,
    /// written as `scheme://host[:port]`, e.g. `https://admin.example.com`.
    /// Empty by default: only same-origin requests are accepted.
    pub trusted_origins: Vec<String>,
    /// Reverse proxies (load balancers, nginx, Caddy) in front of the app.
    /// For requests from these addresses the client address is read from
    /// `X-Forwarded-For` (see [`ClientIp`](crate::ClientIp)), so per-client
    /// limits and the login throttle apply to real visitors instead of the
    /// proxy. Empty by default: the header is ignored, because anyone can
    /// send it.
    pub trusted_proxies: Vec<IpRange>,
}

impl SecurityConfig {
    /// Defaults for local development: HSTS off (no HTTPS locally).
    pub fn development() -> Self {
        Self {
            max_body_bytes: 2 * 1024 * 1024,
            request_timeout: Duration::from_secs(30),
            body_timeout: Duration::from_secs(10),
            min_body_rate: 1024,
            max_uploads_per_client: 32,
            max_concurrent_requests: 1024,
            header_read_timeout: Duration::from_secs(10),
            max_connections: 10_000,
            hsts: false,
            trusted_origins: Vec::new(),
            trusted_proxies: Vec::new(),
        }
    }

    /// Defaults for production: HSTS on.
    pub fn production() -> Self {
        Self {
            hsts: true,
            ..Self::development()
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self::development()
    }
}
