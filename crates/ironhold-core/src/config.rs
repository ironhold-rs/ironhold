use std::{env, net::SocketAddr, time::Duration};

use ironhold_security::{IpRange, SecurityConfig};
use ironhold_session::SessionConfig;

/// Which environment the app runs in. Controls secure defaults such as HSTS.
///
/// More environments may be added, so a `match` needs a `_` arm:
///
/// ```compile_fail
/// # use ironhold_core::Environment;
/// fn name(environment: Environment) -> &'static str {
///     match environment {
///         Environment::Development => "development",
///         Environment::Production => "production",
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Environment {
    /// Local development.
    Development,
    /// Production: stricter defaults.
    Production,
}

/// Application configuration.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Config {
    /// The environment the app runs in.
    pub environment: Environment,
    /// Address to listen on. Defaults to `127.0.0.1:3000`: localhost only,
    /// so a dev server is never exposed to the network by accident.
    pub addr: SocketAddr,
    /// Security layer settings.
    pub security: SecurityConfig,
    /// Session cookie settings.
    pub session: SessionConfig,
    /// How long shutting down waits for requests in progress. Connections
    /// still open after this are closed, so a slow or stuck client can't
    /// keep the server running. Set it below the grace period of whatever
    /// stops the app (Docker waits 10 seconds, Kubernetes 30).
    pub shutdown_timeout: Duration,
}

/// A configuration value that couldn't be parsed. The app refuses to start
/// rather than silently falling back to a default.
#[derive(Debug, thiserror::Error)]
#[error("invalid {var}={value:?}: {reason}")]
pub struct ConfigError {
    var: &'static str,
    value: String,
    reason: &'static str,
}

impl Config {
    /// Default configuration for `environment`.
    pub fn new(environment: Environment) -> Self {
        let (security, session) = match environment {
            Environment::Development => {
                (SecurityConfig::development(), SessionConfig::development())
            }
            Environment::Production => (SecurityConfig::production(), SessionConfig::production()),
        };
        Self {
            environment,
            addr: SocketAddr::from(([127, 0, 0, 1], 3000)),
            security,
            session,
            shutdown_timeout: Duration::from_secs(10),
        }
    }

    /// Reads configuration from environment variables:
    ///
    /// - `IRONHOLD_ENV`: `development` (default) or `production`
    /// - `IRONHOLD_ADDR`: listen address, e.g. `0.0.0.0:8080`
    /// - `IRONHOLD_TRUSTED_PROXIES`: reverse proxies in front of the app,
    ///   as comma-separated addresses or CIDR ranges, e.g.
    ///   `127.0.0.1,10.0.0.0/8` (see [`SecurityConfig::trusted_proxies`])
    pub fn from_env() -> Result<Self, ConfigError> {
        let environment = match env::var("IRONHOLD_ENV").ok().as_deref() {
            None | Some("development") => Environment::Development,
            Some("production") => Environment::Production,
            Some(other) => {
                return Err(ConfigError {
                    var: "IRONHOLD_ENV",
                    value: other.to_owned(),
                    reason: "expected `development` or `production`",
                });
            }
        };

        let mut config = Self::new(environment);

        if let Ok(addr) = env::var("IRONHOLD_ADDR") {
            config.addr = addr.parse().map_err(|_| ConfigError {
                var: "IRONHOLD_ADDR",
                value: addr,
                reason: "expected a socket address like `127.0.0.1:3000`",
            })?;
        }

        if let Ok(proxies) = env::var("IRONHOLD_TRUSTED_PROXIES") {
            config.security.trusted_proxies =
                parse_trusted_proxies(&proxies).ok_or_else(|| ConfigError {
                    var: "IRONHOLD_TRUSTED_PROXIES",
                    value: proxies.clone(),
                    reason: "expected comma-separated IP addresses or CIDR ranges like `10.0.0.0/8`",
                })?;
        }

        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(Environment::Development)
    }
}

/// `None` if any entry is invalid. Empty entries are skipped, so a trailing
/// comma is fine.
fn parse_trusted_proxies(list: &str) -> Option<Vec<IpRange>> {
    list.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_trusted_proxies;

    #[test]
    fn trusted_proxies_parse_strictly() {
        let proxies = parse_trusted_proxies("127.0.0.1, 10.0.0.0/8,fd00::/8,").unwrap();
        assert_eq!(proxies.len(), 3);
        assert!(parse_trusted_proxies("10.0.0.0/8, proxy.internal").is_none());
        assert_eq!(parse_trusted_proxies("").unwrap().len(), 0);
    }
}
