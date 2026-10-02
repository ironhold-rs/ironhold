//! Rejects cross-origin state-changing requests (the first CSRF defence).
//!
//! Browsers label every request with `Sec-Fetch-Site`, which a page on
//! another site can't forge. Older browsers that don't send it still send
//! `Origin` on cross-origin POSTs. Requests with neither header don't come
//! from a browser page, so they can't carry a victim's cookies and aren't a
//! CSRF risk. This is the same approach as Go's `CrossOriginProtection`.
//!
//! CSRF tokens (checked by `ironhold-forms`) are the second defence.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Origins allowed to make cross-origin requests, e.g. `https://admin.example.com`.
pub(crate) struct TrustedOrigins(pub(crate) Vec<String>);

pub(crate) async fn cross_origin_protection(
    State(trusted): State<Arc<TrustedOrigins>>,
    request: Request,
    next: Next,
) -> Response {
    if is_allowed(
        request.method(),
        request.uri(),
        request.headers(),
        &trusted.0,
    ) {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "Cross-origin request blocked").into_response()
    }
}

fn is_allowed(method: &Method, uri: &Uri, headers: &HeaderMap, trusted: &[String]) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }

    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if origin.is_some_and(|o| trusted.iter().any(|t| t.eq_ignore_ascii_case(o))) {
        return true;
    }

    match headers.get("sec-fetch-site").map(|v| v.as_bytes()) {
        // `none` means the user typed the URL or used a bookmark.
        Some(b"same-origin" | b"none") => return true,
        Some(_) => return false,
        None => {}
    }

    let Some(origin) = origin else {
        return true;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| uri.authority().map(|a| a.as_str()));
    match (origin.parse::<Uri>(), host) {
        (Ok(origin), Some(host)) => origin
            .authority()
            .is_some_and(|a| a.as_str().eq_ignore_ascii_case(host)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn check(method: Method, headers: &[(&'static str, &'static str)], trusted: &[&str]) -> bool {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.insert(*name, HeaderValue::from_static(value));
        }
        let trusted: Vec<String> = trusted.iter().map(|s| s.to_string()).collect();
        is_allowed(&method, &Uri::from_static("/"), &map, &trusted)
    }

    #[test]
    fn safe_methods_always_pass() {
        assert!(check(Method::GET, &[("sec-fetch-site", "cross-site")], &[]));
        assert!(check(
            Method::HEAD,
            &[("origin", "https://evil.example")],
            &[]
        ));
    }

    #[test]
    fn fetch_metadata_decides_when_present() {
        assert!(check(
            Method::POST,
            &[("sec-fetch-site", "same-origin")],
            &[]
        ));
        assert!(check(Method::POST, &[("sec-fetch-site", "none")], &[]));
        assert!(!check(
            Method::POST,
            &[("sec-fetch-site", "cross-site")],
            &[]
        ));
        assert!(!check(
            Method::POST,
            &[("sec-fetch-site", "same-site")],
            &[]
        ));
    }

    #[test]
    fn origin_must_match_host_without_fetch_metadata() {
        let same = [("origin", "http://app.test"), ("host", "app.test")];
        assert!(check(Method::POST, &same, &[]));
        let other = [("origin", "https://evil.example"), ("host", "app.test")];
        assert!(!check(Method::DELETE, &other, &[]));
        assert!(!check(
            Method::POST,
            &[("origin", "null"), ("host", "app.test")],
            &[]
        ));
    }

    #[test]
    fn non_browser_requests_pass() {
        assert!(check(Method::POST, &[], &[]));
    }

    #[test]
    fn trusted_origins_pass() {
        let headers = [
            ("origin", "https://admin.example"),
            ("sec-fetch-site", "cross-site"),
        ];
        assert!(check(Method::POST, &headers, &["https://admin.example"]));
        assert!(!check(Method::POST, &headers, &["https://other.example"]));
    }
}
