use std::time::Duration;

/// Session cookie settings.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SessionConfig {
    /// Cookie name. Deliberately generic so it doesn't reveal the framework.
    pub cookie_name: String,
    /// Only send the cookie over HTTPS.
    pub secure: bool,
    /// A session expires after this long without changes.
    pub idle_timeout: Duration,
}

impl SessionConfig {
    /// Development: plain HTTP works, so the cookie isn't `Secure`.
    pub fn development() -> Self {
        Self {
            cookie_name: "id".to_owned(),
            secure: false,
            idle_timeout: Duration::from_secs(12 * 60 * 60),
        }
    }

    /// Production: `Secure` cookie with the `__Host-` prefix, which browsers
    /// only accept over HTTPS, on path `/`, without a `Domain`. That stops
    /// subdomains from setting or overwriting the session cookie.
    pub fn production() -> Self {
        Self {
            cookie_name: "__Host-id".to_owned(),
            secure: true,
            ..Self::development()
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self::development()
    }
}
