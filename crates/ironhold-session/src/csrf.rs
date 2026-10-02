use std::sync::Arc;

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};
use maud::{Markup, Render, html};
use subtle::ConstantTimeEq;
use tower_sessions::Session;

/// Name of the hidden form field that carries the token.
pub const CSRF_FIELD: &str = "_csrf";

/// Header that carries the token for `fetch` and other script requests.
pub const CSRF_HEADER: &str = "x-csrf-token";

const SESSION_KEY: &str = "ironhold.csrf";

/// The session's CSRF token.
///
/// Take it as a handler argument and put it inside every form. It renders as
/// a hidden input:
///
/// ```ignore
/// async fn new_post(csrf: CsrfToken) -> Markup {
///     html! {
///         form method="post" action="/posts" {
///             (csrf)
///             input name="title";
///             button { "Save" }
///         }
///     }
/// }
/// ```
#[derive(Clone)]
pub struct CsrfToken(Arc<str>);

impl CsrfToken {
    /// Returns the session's token, creating one on first use.
    pub async fn for_session(session: &Session) -> Result<Self, tower_sessions::session::Error> {
        if let Some(token) = session.get::<String>(SESSION_KEY).await? {
            return Ok(Self(token.into()));
        }
        let token = generate().map_err(|_| {
            tower_sessions::session::Error::Store(tower_sessions::session_store::Error::Backend(
                "operating system random number generator failed".to_owned(),
            ))
        })?;
        session.insert(SESSION_KEY, &token).await?;
        Ok(Self(token.into()))
    }

    /// Replaces the session's token with a new one. Call it when the user
    /// logs in or out, so a token seen before then stops working.
    pub async fn rotate(session: &Session) -> Result<Self, tower_sessions::session::Error> {
        session.remove::<String>(SESSION_KEY).await?;
        Self::for_session(session).await
    }

    /// Checks `submitted` against the session's token in constant time.
    /// Returns `false` if the session has no token yet.
    pub async fn verify(
        session: &Session,
        submitted: &str,
    ) -> Result<bool, tower_sessions::session::Error> {
        let Some(expected) = session.get::<String>(SESSION_KEY).await? else {
            return Ok(false);
        };
        Ok(expected.as_bytes().ct_eq(submitted.as_bytes()).into())
    }

    /// The token value, e.g. for a `<meta>` tag read by JavaScript.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for CsrfToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CsrfToken([REDACTED])")
    }
}

/// Renders `<input type="hidden" name="_csrf" value="...">`.
impl Render for CsrfToken {
    fn render(&self) -> Markup {
        html! { input type="hidden" name=(CSRF_FIELD) value=(self.as_str()); }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for CsrfToken {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state).await?;
        Self::for_session(&session)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"))
    }
}

fn generate() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_sessions::MemoryStore;

    fn session() -> Session {
        Session::new(None, Arc::new(MemoryStore::default()), None)
    }

    #[tokio::test]
    async fn token_is_stable_within_a_session() {
        let session = session();
        let a = CsrfToken::for_session(&session).await.unwrap();
        let b = CsrfToken::for_session(&session).await.unwrap();
        assert_eq!(a.as_str(), b.as_str());
        assert_eq!(a.as_str().len(), 64);
    }

    #[tokio::test]
    async fn verify_accepts_only_the_right_token() {
        let session = session();
        assert!(!CsrfToken::verify(&session, "anything").await.unwrap());

        let token = CsrfToken::for_session(&session).await.unwrap();
        assert!(CsrfToken::verify(&session, token.as_str()).await.unwrap());
        assert!(!CsrfToken::verify(&session, "wrong").await.unwrap());
        assert!(!CsrfToken::verify(&session, "").await.unwrap());
    }

    #[tokio::test]
    async fn rotate_invalidates_the_old_token() {
        let session = session();
        let old = CsrfToken::for_session(&session).await.unwrap();
        let new = CsrfToken::rotate(&session).await.unwrap();
        assert_ne!(old.as_str(), new.as_str());
        assert!(!CsrfToken::verify(&session, old.as_str()).await.unwrap());
        assert!(CsrfToken::verify(&session, new.as_str()).await.unwrap());
    }

    #[test]
    fn renders_hidden_input_and_hides_debug() {
        let token = CsrfToken("abc".into());
        assert_eq!(
            token.render().into_string(),
            r#"<input type="hidden" name="_csrf" value="abc">"#
        );
        assert_eq!(format!("{token:?}"), "CsrfToken([REDACTED])");
    }
}
