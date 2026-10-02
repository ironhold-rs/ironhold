use std::fmt;

use serde::{Deserialize, Deserializer};

/// A value that must never be logged, printed or serialized.
///
/// `Debug` and `Display` print `[REDACTED]`, and `Secret` deliberately does
/// not implement `Serialize`, so it can't leak into JSON responses or logs.
/// It also doesn't implement `PartialEq`: comparing secrets with `==` is not
/// constant-time, so call [`expose`](Secret::expose) and use a constant-time
/// comparison instead.
///
/// ```
/// use ironhold_security::Secret;
///
/// let key = Secret::new(String::from("sk_live_123"));
/// assert_eq!(format!("{key:?}"), "Secret([REDACTED])");
/// assert_eq!(key.expose(), "sk_live_123");
/// ```
#[derive(Clone)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wraps a sensitive value.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrows the secret value. Every call site is an explicit, greppable
    /// point where the secret is used.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// Consumes the wrapper and returns the secret value.
    pub fn expose_owned(self) -> T {
        self.0
    }
}

impl<T> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Secrets can be read from config files and forms, but never written out.
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Secret<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_are_redacted() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "Secret([REDACTED])");
        assert_eq!(format!("{s}"), "[REDACTED]");
        assert!(!format!("{s:?} {s} {s:#?}").contains("hunter2"));
    }

    #[test]
    fn redacted_inside_derived_debug() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Login {
            user: &'static str,
            password: Secret<&'static str>,
        }
        let login = Login {
            user: "raj",
            password: Secret::new("hunter2"),
        };
        assert!(!format!("{login:?}").contains("hunter2"));
    }
}
