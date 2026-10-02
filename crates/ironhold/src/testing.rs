//! Test your app the way a browser uses it. Requires the `testing` feature.
//!
//! ```ignore
//! #[tokio::test]
//! async fn signing_in_shows_the_dashboard() {
//!     let mut client = TestClient::new(app().await);
//!     client.get("/login").await; // picks up the session cookie and CSRF token
//!     let response = client
//!         .post_form("/login", &[("email", "raj@example.com"), ("password", "correct horse battery")])
//!         .await;
//!     assert_eq!(response.location(), Some("/dashboard"));
//! }
//! ```
//!
//! [`TestClient`](crate::testing::TestClient) runs requests in memory (no network), keeps cookies
//! between requests, and adds the CSRF token from the last page it loaded
//! to form submissions, like a real browser submitting a real form.

use std::collections::BTreeMap;

use http_body_util::BodyExt;
use ironhold_core::{
    App, Router,
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
};
use tower::ServiceExt;

/// An in-memory browser for tests. See the [module docs](self).
pub struct TestClient {
    router: Router,
    cookies: BTreeMap<String, String>,
    csrf_token: Option<String>,
}

impl TestClient {
    /// A client for `app`. Background tasks registered with
    /// `App::spawn_on_serve` don't run in tests.
    pub fn new(app: App) -> Self {
        Self::from_router(app.into_router())
    }

    /// A client for an already built router.
    pub fn from_router(router: Router) -> Self {
        Self {
            router,
            cookies: BTreeMap::new(),
            csrf_token: None,
        }
    }

    /// Loads a page, like typing the URL into the address bar.
    pub async fn get(&mut self, path: &str) -> TestResponse {
        let request = Request::get(path)
            .header(header::ACCEPT, "text/html")
            .header("sec-fetch-site", "same-origin")
            .body(Body::empty());
        self.send_built(request).await
    }

    /// Submits a form from this site. The CSRF token from the last page
    /// loaded is added automatically unless `fields` already has `_csrf`.
    pub async fn post_form(&mut self, path: &str, fields: &[(&str, &str)]) -> TestResponse {
        let mut fields: Vec<(&str, &str)> = fields.to_vec();
        let token = self.csrf_token.clone();
        if let Some(token) = token.as_deref()
            && !fields.iter().any(|(name, _)| *name == "_csrf")
        {
            fields.push(("_csrf", token));
        }
        let body = serde_urlencoded::to_string(&fields).unwrap_or_default();
        let request = Request::post(path)
            .header(header::ACCEPT, "text/html")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("sec-fetch-site", "same-origin")
            .body(Body::from(body));
        self.send_built(request).await
    }

    /// Sends any request, adding this client's cookies.
    pub async fn send(&mut self, request: Request<Body>) -> TestResponse {
        self.send_built(Ok(request)).await
    }

    /// The CSRF token from the last page that contained a form.
    pub fn csrf_token(&self) -> Option<&str> {
        self.csrf_token.as_deref()
    }

    /// The current value of cookie `name`.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    /// Forgets all cookies, like closing a private browsing window.
    pub fn clear_cookies(&mut self) {
        self.cookies.clear();
        self.csrf_token = None;
    }

    async fn send_built(
        &mut self,
        request: Result<Request<Body>, ironhold_core::http::Error>,
    ) -> TestResponse {
        let mut request = match request {
            Ok(request) => request,
            Err(error) => return TestResponse::client_error(&error.to_string()),
        };
        if !self.cookies.is_empty() {
            let cookie_header = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            if let Ok(value) = HeaderValue::from_str(&cookie_header) {
                request.headers_mut().insert(header::COOKIE, value);
            }
        }

        let response = match self.router.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        let (parts, body) = response.into_parts();
        let body = match body.collect().await {
            Ok(collected) => String::from_utf8_lossy(&collected.to_bytes()).into_owned(),
            Err(error) => return TestResponse::client_error(&error.to_string()),
        };

        for set_cookie in parts.headers.get_all(header::SET_COOKIE) {
            if let Ok(set_cookie) = set_cookie.to_str() {
                self.store_cookie(set_cookie);
            }
        }
        if let Some(token) = find_csrf_token(&body) {
            self.csrf_token = Some(token.to_owned());
        }

        TestResponse {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }

    fn store_cookie(&mut self, set_cookie: &str) {
        let mut attributes = set_cookie.split(';');
        let Some((name, value)) = attributes.next().and_then(|pair| pair.split_once('=')) else {
            return;
        };
        let expired = attributes.any(|attribute| {
            let attribute = attribute.trim().to_ascii_lowercase();
            attribute == "max-age=0" || attribute.starts_with("max-age=-")
        });
        if expired || value.is_empty() {
            self.cookies.remove(name.trim());
        } else {
            self.cookies
                .insert(name.trim().to_owned(), value.trim().to_owned());
        }
    }
}

/// A response received by [`TestClient`].
#[derive(Debug)]
pub struct TestResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl TestResponse {
    /// The status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// A header value as text, if present and valid.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// The `Location` of a redirect.
    pub fn location(&self) -> Option<&str> {
        self.header("location")
    }

    /// The body as text.
    pub fn text(&self) -> &str {
        &self.body
    }

    fn client_error(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            headers: HeaderMap::new(),
            body: format!("test client could not build or read the request: {message}"),
        }
    }
}

fn find_csrf_token(html: &str) -> Option<&str> {
    let start = html.find(r#"name="_csrf" value=""#)? + r#"name="_csrf" value=""#.len();
    let rest = html.get(start..)?;
    rest.split('"').next()
}
