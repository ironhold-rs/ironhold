//! End-to-end checks that every Ironhold app is secure by default.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use http_body_util::BodyExt;
use ironhold::body::Body;
use ironhold::http::{HeaderValue, Request, Response, StatusCode, header};
use ironhold::prelude::*;
use ironhold::{Config, Environment};
use tower::ServiceExt;

fn app(environment: Environment) -> App {
    App::with_config(Config::new(environment))
        .route("/", get(|| async { "ok" }))
        .route(
            "/nonce",
            get(|nonce: CspNonce| async move { nonce.as_str().to_owned() }),
        )
        .route(
            "/hello/{name}",
            get(|Path(name): Path<String>| async move {
                html! { p { (name) } }
            }),
        )
        .route(
            "/boom",
            get(|| async { Err::<(), _>(Error::internal("db password is hunter2")) }),
        )
        .route(
            "/framed",
            get(|| async {
                (
                    [(
                        header::X_FRAME_OPTIONS,
                        HeaderValue::from_static("SAMEORIGIN"),
                    )],
                    "custom",
                )
            }),
        )
        .route("/echo", post(|body: String| async move { body }))
        .route(
            "/panic",
            get(|| async {
                if true {
                    panic!("bug in handler: secret=hunter2");
                }
                "unreachable"
            }),
        )
}

async fn send(app: App, request: Request<Body>) -> Response<Body> {
    app.into_router().oneshot(request).await.unwrap()
}

async fn get_path(app: App, path: &str) -> Response<Body> {
    send(app, Request::get(path).body(Body::empty()).unwrap()).await
}

async fn body_text(response: Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn header_str<'a>(response: &'a Response<Body>, name: &str) -> Option<&'a str> {
    response.headers().get(name).map(|v| v.to_str().unwrap())
}

#[tokio::test]
async fn security_headers_are_set_on_every_response() {
    let response = get_path(app(Environment::Development), "/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_str(&response, "x-content-type-options"),
        Some("nosniff")
    );
    assert_eq!(header_str(&response, "x-frame-options"), Some("DENY"));
    assert_eq!(
        header_str(&response, "referrer-policy"),
        Some("strict-origin-when-cross-origin")
    );
    assert_eq!(
        header_str(&response, "cross-origin-opener-policy"),
        Some("same-origin")
    );
    assert!(header_str(&response, "permissions-policy").is_some());

    let csp = header_str(&response, "content-security-policy").unwrap();
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("object-src 'none'"));
    assert!(csp.contains("frame-ancestors 'none'"));
}

#[tokio::test]
async fn hsts_only_in_production() {
    let dev = get_path(app(Environment::Development), "/").await;
    assert_eq!(header_str(&dev, "strict-transport-security"), None);

    let prod = get_path(app(Environment::Production), "/").await;
    assert!(
        header_str(&prod, "strict-transport-security")
            .unwrap()
            .starts_with("max-age=63072000")
    );
    let csp = header_str(&prod, "content-security-policy").unwrap();
    assert!(csp.contains("upgrade-insecure-requests"));
}

#[tokio::test]
async fn csp_nonce_matches_extractor_and_changes_per_request() {
    let first = get_path(app(Environment::Development), "/nonce").await;
    let csp = header_str(&first, "content-security-policy")
        .unwrap()
        .to_owned();
    let nonce = body_text(first).await;
    assert!(csp.contains(&format!("'nonce-{nonce}'")));

    let second = body_text(get_path(app(Environment::Development), "/nonce").await).await;
    assert_ne!(nonce, second);
}

#[tokio::test]
async fn html_output_is_escaped() {
    let response = get_path(
        app(Environment::Development),
        "/hello/%3Cscript%3Ealert(1)%3C%2Fscript%3E",
    )
    .await;
    let body = body_text(response).await;
    assert_eq!(body, "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>");
}

#[tokio::test]
async fn internal_errors_do_not_leak_details() {
    let response = get_path(app(Environment::Development), "/boom").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(header_str(&response, "content-security-policy").is_some());
    let body = body_text(response).await;
    assert_eq!(body, "Internal Server Error");
}

#[tokio::test]
async fn unknown_routes_get_a_404_with_security_headers() {
    let response = get_path(app(Environment::Development), "/missing").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(header_str(&response, "x-frame-options"), Some("DENY"));
    assert!(
        header_str(&response, "content-type")
            .unwrap()
            .starts_with("text/plain")
    );
}

#[tokio::test]
async fn handlers_can_override_a_header_deliberately() {
    let response = get_path(app(Environment::Development), "/framed").await;
    assert_eq!(header_str(&response, "x-frame-options"), Some("SAMEORIGIN"));
    assert_eq!(
        header_str(&response, "x-content-type-options"),
        Some("nosniff")
    );
}

#[tokio::test]
async fn oversized_bodies_are_rejected() {
    fn limited_app() -> App {
        let mut config = Config::new(Environment::Development);
        config.security.max_body_bytes = 16;
        App::with_config(config).route("/echo", post(|body: String| async move { body }))
    }

    let small = Request::post("/echo").body(Body::from("hi")).unwrap();
    let response = send(limited_app(), small).await;
    assert_eq!(response.status(), StatusCode::OK);

    let big = Request::post("/echo")
        .header(header::CONTENT_LENGTH, "100")
        .body(Body::from("a".repeat(100)))
        .unwrap();
    let response = send(limited_app(), big).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(header_str(&response, "x-frame-options"), Some("DENY"));
}

#[tokio::test]
async fn handler_panics_become_500_without_details() {
    let router = app(Environment::Development).into_router();

    let response = router
        .clone()
        .oneshot(Request::get("/panic").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(header_str(&response, "x-frame-options"), Some("DENY"));
    assert_eq!(body_text(response).await, "Internal Server Error");

    // The same router keeps serving after the panic.
    let response = router
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn overload_returns_503_immediately_and_recovers() {
    use std::sync::Arc;
    use tokio::sync::Notify;

    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let mut config = Config::new(Environment::Development);
    config.security.max_concurrent_requests = 1;
    let router = App::with_config(config)
        .route("/fast", get(|| async { "fast" }))
        .route(
            "/slow",
            get({
                let started = started.clone();
                let release = release.clone();
                move || async move {
                    started.notify_one();
                    release.notified().await;
                    "slow"
                }
            }),
        )
        .into_router();

    let in_flight = tokio::spawn(
        router
            .clone()
            .oneshot(Request::get("/slow").body(Body::empty()).unwrap()),
    );
    started.notified().await;

    // The only slot is taken, so the next request is shed at once.
    let response = router
        .clone()
        .oneshot(Request::get("/fast").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(header_str(&response, "retry-after"), Some("1"));
    assert_eq!(header_str(&response, "x-frame-options"), Some("DENY"));

    release.notify_one();
    assert_eq!(in_flight.await.unwrap().unwrap().status(), StatusCode::OK);

    // Capacity is back.
    let response = router
        .oneshot(Request::get("/fast").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
