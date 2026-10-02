//! Helpers shared by the end-to-end tests.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use http_body_util::BodyExt;
use ironhold::body::Body;
use ironhold::http::{Request, Response, StatusCode, header};
use tower::ServiceExt;

pub async fn body_text(response: Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Sends `request` and returns the status and body text.
pub async fn status_and_body(
    router: &ironhold::Router,
    request: Request<Body>,
) -> (StatusCode, String) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    (status, body_text(response).await)
}

/// `GET path`, returning the status and body text.
pub async fn get_text(router: &ironhold::Router, path: &str) -> (StatusCode, String) {
    status_and_body(router, Request::get(path).body(Body::empty()).unwrap()).await
}

/// Loads a page containing a form and returns
/// `(session cookie, csrf token, Set-Cookie header)`.
pub async fn open_form(router: &ironhold::Router, path: &str) -> (String, String, String) {
    let response = router
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie is set")
        .to_str()
        .unwrap()
        .to_owned();
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    let html = body_text(response).await;
    let token = html
        .split(r#"name="_csrf" value=""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("form contains the csrf field")
        .to_owned();
    (cookie, token, set_cookie)
}

/// A urlencoded form `POST` carrying the session cookie.
pub fn submit(path: &str, cookie: &str, body: String) -> Request<Body> {
    Request::post(path)
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

/// A fresh SQLite database file in the temp directory, deleted (with its
/// WAL files) when the returned guard is dropped.
pub fn temp_sqlite_url() -> (String, TempFile) {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let path = std::env::temp_dir().join(format!("ironhold-test-{name}.db"));
    (format!("sqlite://{}", path.display()), TempFile(path))
}

pub struct TempFile(std::path::PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.0.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
    }
}
