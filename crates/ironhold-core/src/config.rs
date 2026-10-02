use std::{env, net::SocketAddr};

use ironhold_security::SecurityConfig;
use ironhold_session::SessionConfig;

/// Which environment the app runs in. Controls secure defaults such as HSTS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        }
    }

    /// Reads configuration from environment variables:
    ///
    /// - `IRONHOLD_ENV`: `development` (default) or `production`
    /// - `IRONHOLD_ADDR`: listen address, e.g. `0.0.0.0:8080`
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

        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(Environment::Development)
    }
}
