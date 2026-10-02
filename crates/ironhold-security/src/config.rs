use std::time::Duration;

/// Security settings applied by [`harden`](crate::harden).
///
/// Every field has a safe default. Loosen them deliberately, not by accident.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SecurityConfig {
    /// Maximum accepted request body size in bytes. Larger requests get `413`.
    pub max_body_bytes: usize,
    /// Maximum time a request may take before it is aborted with `408`.
    pub request_timeout: Duration,
    /// Requests allowed in flight at once. Beyond this, new requests are
    /// rejected immediately with `503` and `Retry-After`, so an overloaded
    /// server stays fast for the requests it accepts instead of getting
    /// slower for everyone.
    pub max_concurrent_requests: usize,
    /// Send `Strict-Transport-Security` and `upgrade-insecure-requests`.
    /// Only enable this when the app is served over HTTPS.
    pub hsts: bool,
    /// Other origins allowed to send POST, PUT, PATCH and DELETE requests,
    /// written as `scheme://host[:port]`, e.g. `https://admin.example.com`.
    /// Empty by default: only same-origin requests are accepted.
    pub trusted_origins: Vec<String>,
}

impl SecurityConfig {
    /// Defaults for local development: HSTS off (no HTTPS locally).
    pub fn development() -> Self {
        Self {
            max_body_bytes: 2 * 1024 * 1024,
            request_timeout: Duration::from_secs(30),
            max_concurrent_requests: 1024,
            hsts: false,
            trusted_origins: Vec::new(),
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
