//! Form handling for Ironhold.
//!
//! [`Form<T>`] parses a `application/x-www-form-urlencoded` body into `T`,
//! but only after checking the session's CSRF token. A handler that takes
//! `Form<T>` can't run for a forged request, so there's nothing to forget.
//!
//! [`Validate`] checks the parsed input against rules written in plain Rust
//! and returns [`Valid<T>`], or [`ValidationErrors`] to show next to each
//! field.

#![forbid(unsafe_code)]

mod validate;

pub use validate::{Check, NumberCheck, Valid, Validate, ValidationErrors, Validator};

use axum::{
    body::Bytes,
    extract::{FromRequest, FromRequestParts, Request},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use ironhold_core::Error;
use ironhold_session::{CSRF_FIELD, CSRF_HEADER, CsrfToken, Session};
use serde::de::DeserializeOwned;

/// A form submission, checked against the session's CSRF token.
///
/// The token is read from the `_csrf` field (render it in the form with
/// `(csrf)`, see [`CsrfToken`]) or from the `X-CSRF-Token` header. It is
/// removed before `T` is deserialized, so `T` doesn't need a `_csrf` field
/// and works with `#[serde(deny_unknown_fields)]`.
///
/// ```ignore
/// #[derive(Deserialize)]
/// struct NewPost { title: String }
///
/// async fn create(Form(post): Form<NewPost>) -> Redirect { ... }
/// ```
///
/// Responses on failure:
/// - `403` if the token is missing or wrong
/// - `400` if the body isn't a form or doesn't match `T`
/// - `413` if the body is larger than the configured limit
#[derive(Debug, Clone, Copy, Default)]
pub struct Form<T>(pub T);

impl<T, S> FromRequest<S> for Form<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = request.into_parts();

        if !is_form(&parts.headers) {
            return Err(Error::BadRequest(
                "expected a form submission (application/x-www-form-urlencoded)".to_owned(),
            )
            .into_response());
        }

        let session = Session::from_request_parts(&mut parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        let header_token = parts
            .headers
            .get(CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let bytes = Bytes::from_request(Request::from_parts(parts, body), state)
            .await
            .map_err(IntoResponse::into_response)?;

        let mut fields: Vec<(String, String)> = serde_urlencoded::from_bytes(&bytes)
            .map_err(|_| Error::BadRequest("malformed form data".to_owned()).into_response())?;

        let mut field_token = None;
        fields.retain(|(name, value)| {
            if name == CSRF_FIELD {
                field_token = Some(value.clone());
                false
            } else {
                true
            }
        });

        let valid = match field_token.or(header_token) {
            Some(token) => CsrfToken::verify(&session, &token)
                .await
                .map_err(|e| Error::internal(e).into_response())?,
            None => false,
        };
        if !valid {
            return Err((
                StatusCode::FORBIDDEN,
                "Invalid or missing CSRF token. Reload the page and try again.",
            )
                .into_response());
        }

        let encoded =
            serde_urlencoded::to_string(&fields).map_err(|e| Error::internal(e).into_response())?;
        let value = serde_urlencoded::from_str(&encoded)
            .map_err(|e| Error::BadRequest(format!("invalid form data: {e}")).into_response())?;
        Ok(Self(value))
    }
}

fn is_form(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|mime| {
            mime.trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
}
