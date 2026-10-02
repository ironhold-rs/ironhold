//! Sessions, CSRF tokens and cross-origin protection, end to end.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironhold::body::Body;
use ironhold::http::{Request, StatusCode, header};
use ironhold::prelude::*;
use ironhold::{Config, Environment};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPost {
    title: String,
}

fn router(environment: Environment) -> ironhold::Router {
    App::with_config(Config::new(environment))
        .route(
            "/posts/new",
            get(|csrf: CsrfToken| async move {
                html! { form method="post" action="/posts" { (csrf) input name="title"; } }
            }),
        )
        .route(
            "/posts",
            post(|Form(post): Form<NewPost>| async move { format!("created {}", post.title) }),
        )
        .into_router()
}

mod support;
use support::status_and_body;

async fn open_form(router: &ironhold::Router) -> (String, String, String) {
    support::open_form(router, "/posts/new").await
}

fn submit(cookie: &str, body: String) -> Request<Body> {
    support::submit("/posts", cookie, body)
}

#[tokio::test]
async fn valid_token_is_accepted_and_stripped_before_deserializing() {
    let router = router(Environment::Development);
    let (cookie, token, _) = open_form(&router).await;

    let request = submit(&cookie, format!("title=Hello+world&_csrf={token}"));
    let (status, body) = status_and_body(&router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "created Hello world");
}

#[tokio::test]
async fn token_in_header_is_accepted() {
    let router = router(Environment::Development);
    let (cookie, token, _) = open_form(&router).await;

    let mut request = submit(&cookie, "title=Hi".to_owned());
    request
        .headers_mut()
        .insert("x-csrf-token", token.parse().unwrap());
    let (status, _) = status_and_body(&router, request).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn missing_or_wrong_token_is_rejected() {
    let router = router(Environment::Development);
    let (cookie, _, _) = open_form(&router).await;

    let (status, _) = status_and_body(&router, submit(&cookie, "title=Hi".to_owned())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let wrong = submit(&cookie, "title=Hi&_csrf=0000".to_owned());
    let (status, _) = status_and_body(&router, wrong).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn token_from_another_session_is_rejected() {
    let router = router(Environment::Development);
    let (_, attacker_token, _) = open_form(&router).await;
    let (victim_cookie, _, _) = open_form(&router).await;

    let request = submit(&victim_cookie, format!("title=Hi&_csrf={attacker_token}"));
    let (status, _) = status_and_body(&router, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cross_site_post_is_blocked_even_with_a_valid_token() {
    let router = router(Environment::Development);
    let (cookie, token, _) = open_form(&router).await;

    let mut request = submit(&cookie, format!("title=Hi&_csrf={token}"));
    request
        .headers_mut()
        .insert("sec-fetch-site", "cross-site".parse().unwrap());
    let (status, body) = status_and_body(&router, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, "Cross-origin request blocked");
}

#[tokio::test]
async fn non_form_bodies_are_rejected() {
    let router = router(Environment::Development);
    let (cookie, token, _) = open_form(&router).await;

    let mut request = submit(&cookie, format!(r#"{{"title":"Hi","_csrf":"{token}"}}"#));
    request
        .headers_mut()
        .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    let (status, _) = status_and_body(&router, request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_cookie_is_hardened() {
    let (_, _, dev) = open_form(&router(Environment::Development)).await;
    assert!(dev.starts_with("id="));
    assert!(dev.contains("HttpOnly"));
    assert!(dev.contains("SameSite=Lax"));
    assert!(dev.contains("Path=/"));
    assert!(!dev.contains("Secure"));

    let (_, _, prod) = open_form(&router(Environment::Production)).await;
    assert!(prod.starts_with("__Host-id="));
    assert!(prod.contains("Secure"));
    assert!(prod.contains("HttpOnly"));
    assert!(!prod.contains("Domain"));
}
