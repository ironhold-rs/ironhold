use std::fmt;

use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Errors a handler can return.
///
/// Responses never include internal details: an [`Error::Internal`] is logged
/// in full on the server and the client only sees `500 Internal Server Error`.
///
/// Any other error type converts into [`Error::Internal`] with `?`, so
/// database and I/O errors can be passed straight up:
///
/// ```ignore
/// async fn show(db: SqliteDb, Path(id): Path<i64>) -> Result<Markup> {
///     let post: Option<(String,)> = sqlx::query_as("SELECT title FROM posts WHERE id = $1")
///         .bind(id)
///         .fetch_optional(&*db)
///         .await?; // a database error becomes a logged 500
///     let (title,) = post.ok_or(Error::NotFound)?;
///     Ok(html! { h1 { (title) } })
/// }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// `400`. The message is shown to the client, so keep it user-facing.
    BadRequest(String),
    /// `401`: the request needs authentication.
    Unauthorized,
    /// `403`: authenticated but not allowed.
    Forbidden,
    /// `404`.
    NotFound,
    /// `429`, with a `Retry-After` header in seconds.
    TooManyRequests {
        /// Seconds until the client may try again.
        retry_after_secs: u64,
    },
    /// `500`. Logged server-side; never shown to the client.
    Internal(BoxError),
}

impl Error {
    /// Wraps any error, or a message, as an [`Error::Internal`].
    pub fn internal(error: impl Into<BoxError>) -> Self {
        Self::Internal(error.into())
    }

    /// The HTTP status code for this error.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(message) => write!(f, "bad request: {message}"),
            Self::Unauthorized => f.write_str("unauthorized"),
            Self::Forbidden => f.write_str("forbidden"),
            Self::NotFound => f.write_str("not found"),
            Self::TooManyRequests { retry_after_secs } => {
                write!(f, "too many requests (retry after {retry_after_secs}s)")
            }
            Self::Internal(error) => write!(f, "internal error: {error}"),
        }
    }
}

// `Error` deliberately doesn't implement `std::error::Error`. That is what
// allows this blanket conversion (the same approach as `anyhow`), so `?`
// works on any error type inside a handler.
impl<E> From<E> for Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(error: E) -> Self {
        Self::Internal(Box::new(error))
    }
}

/// A `Result` whose error is [`Error`], for use as a handler return type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        // Plain text, so error bodies can never be interpreted as HTML.
        let body = match &self {
            Self::BadRequest(message) => message.clone(),
            Self::Internal(error) => {
                tracing::error!(error = %error, source = ?error.source(), "internal server error");
                status.canonical_reason().unwrap_or("error").to_owned()
            }
            Self::TooManyRequests { retry_after_secs } => {
                let body = "Too many attempts. Please wait a moment and try again.";
                return (
                    status,
                    [(header::RETRY_AFTER, retry_after_secs.to_string())],
                    body,
                )
                    .into_response();
            }
            _ => status.canonical_reason().unwrap_or("error").to_owned(),
        };
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_config() -> Result<String> {
        let text = std::fs::read_to_string("/definitely/not/a/real/path")?;
        Ok(text)
    }

    #[test]
    fn question_mark_turns_any_error_into_internal() {
        let error = read_config().unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(matches!(error, Error::Internal(_)));
    }
}
