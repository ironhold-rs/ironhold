use std::{fmt, sync::Arc};

use axum::{extract::FromRequestParts, http::StatusCode, http::request::Parts};

/// A random, single-use nonce for the request's Content-Security-Policy.
///
/// Inline `<script>` and `<style>` tags only run if they carry this nonce,
/// so injected markup can't execute scripts. Use it as a handler argument:
///
/// ```ignore
/// async fn page(nonce: CspNonce) -> Markup {
///     html! { script nonce=(nonce.as_str()) { "console.log('allowed')" } }
/// }
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct CspNonce(Arc<str>);

impl CspNonce {
    /// Generates a nonce from 128 bits of OS randomness.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)?;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self(hex.into()))
    }

    /// The nonce value, for use in a `nonce="..."` attribute.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CspNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for CspNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CspNonce").field(&&*self.0).finish()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for CspNonce {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<CspNonce>().cloned().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "CspNonce requires the ironhold security layer (ironhold_security::harden)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_are_128_bit_hex_and_unique() {
        let a = CspNonce::generate().unwrap();
        let b = CspNonce::generate().unwrap();
        assert_eq!(a.as_str().len(), 32);
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
