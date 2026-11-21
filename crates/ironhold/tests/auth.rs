//! Login, logout and access control, end to end through `TestClient`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use ironhold::auth::{self, PasswordHash, safe_redirect_path};
use ironhold::extract::Extension;
use ironhold::http::{Request, StatusCode, header, request::Parts};
use ironhold::prelude::*;
use ironhold::response::Response;
use ironhold::testing::TestClient;
use ironhold::{Config, Environment, body::Body};
use serde::Deserialize;

#[derive(Clone)]
struct User {
    id: i64,
    email: String,
    is_admin: bool,
}

/// An in-memory user table, so these tests don't need a database.
#[derive(Clone, Default)]
struct Users(Arc<Mutex<HashMap<i64, (User, PasswordHash)>>>);

impl Users {
    fn find_by_email(&self, email: &str) -> Option<(User, PasswordHash)> {
        let users = self.0.lock().unwrap();
        users.values().find(|(u, _)| u.email == email).cloned()
    }

    fn remove(&self, id: i64) {
        self.0.lock().unwrap().remove(&id);
    }
}

impl LoadUser for User {
    type Id = i64;

    async fn load_user(id: i64, parts: &mut Parts) -> Result<Option<Self>> {
        let users = parts.extensions.get::<Users>().unwrap();
        Ok(users.0.lock().unwrap().get(&id).map(|(u, _)| u.clone()))
    }
}

struct Admins;

impl Policy<User> for Admins {
    fn allows(user: &User) -> bool {
        user.is_admin
    }
}

#[derive(Deserialize)]
struct LoginForm {
    email: String,
    password: Secret<String>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct Empty {}

async fn login_page(csrf: CsrfToken) -> Markup {
    html! { form method="post" action="/login" { (csrf) input name="email"; input name="password"; } }
}

async fn login(
    session: Session,
    throttle: LoginThrottle,
    Extension(users): Extension<Users>,
    Form(form): Form<LoginForm>,
) -> Result<Response> {
    let attempt = throttle.begin(&form.email)?;
    let found = users.find_by_email(&form.email);
    let verified = auth::verify_password(found.as_ref().map(|(_, h)| h), &form.password).await?;
    match found {
        Some((user, _)) if verified => {
            attempt.succeeded();
            auth::login(&session, &user.id).await?;
            let next = safe_redirect_path(form.next.as_deref(), "/dashboard");
            Ok(Redirect::to(next).into_response())
        }
        _ => {
            attempt.failed();
            Ok((StatusCode::UNAUTHORIZED, "Invalid email or password.").into_response())
        }
    }
}

async fn logout(session: Session, Form(_): Form<Empty>) -> Result<Redirect> {
    auth::logout(&session).await?;
    Ok(Redirect::to("/"))
}

async fn dashboard(user: AuthUser<User>, csrf: CsrfToken) -> Markup {
    html! {
        p { "Hello " (user.email) }
        form method="post" action="/logout" { (csrf) button { "Log out" } }
    }
}

async fn home(user: Option<AuthUser<User>>) -> String {
    user.map_or_else(|| "anonymous".to_owned(), |u| u.email.clone())
}

async fn admin(admin: Authorize<User, Admins>) -> String {
    format!("admin {}", admin.email)
}

async fn app() -> (TestClient, Users) {
    let (app, users) = build_app().await;
    (TestClient::new(app), users)
}

async fn build_app() -> (App, Users) {
    let users = Users::default();
    for (id, email, is_admin) in [
        (1, "raj@example.com", false),
        (2, "admin@example.com", true),
    ] {
        let hash = auth::hash_password(&Secret::new("correct horse battery".to_owned()))
            .await
            .unwrap();
        let user = User {
            id,
            email: email.to_owned(),
            is_admin,
        };
        users.0.lock().unwrap().insert(id, (user, hash));
    }

    let app = App::with_config(Config::new(Environment::Development))
        .extension(users.clone())
        // A private throttle per test, so tests don't affect each other.
        .extension(LoginThrottle::default())
        .route("/", get(home))
        .route("/login", get(login_page).post(login))
        .route("/logout", post(logout))
        .route("/dashboard", get(dashboard))
        .route("/admin", get(admin))
        .route(
            "/api/me",
            get(|user: AuthUser<User>| async move { user.email.clone() }),
        );
    (app, users)
}

async fn log_in(client: &mut TestClient, email: &str) -> ironhold::testing::TestResponse {
    client.get("/login").await;
    client
        .post_form(
            "/login",
            &[("email", email), ("password", "correct horse battery")],
        )
        .await
}

#[tokio::test]
async fn protected_pages_redirect_browsers_and_reject_api_calls() {
    let (mut client, _) = app().await;

    let page = client.get("/dashboard?tab=1").await;
    assert_eq!(page.status(), StatusCode::SEE_OTHER);
    assert_eq!(page.location(), Some("/login?next=%2Fdashboard%3Ftab%3D1"));

    let api = client
        .send(Request::get("/api/me").body(Body::empty()).unwrap())
        .await;
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED);

    assert_eq!(client.get("/").await.text(), "anonymous");
}

#[tokio::test]
async fn login_rotates_the_session_and_csrf_token() {
    let (mut client, _) = app().await;
    client.get("/login").await;
    let session_before = client.cookie("id").unwrap().to_owned();
    let token_before = client.csrf_token().unwrap().to_owned();

    let response = client
        .post_form(
            "/login",
            &[
                ("email", "raj@example.com"),
                ("password", "correct horse battery"),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.location(), Some("/dashboard"));
    assert_ne!(client.cookie("id").unwrap(), session_before);

    let dashboard = client.get("/dashboard").await;
    assert_eq!(dashboard.status(), StatusCode::OK);
    assert!(dashboard.text().contains("Hello raj@example.com"));
    assert_eq!(client.get("/").await.text(), "raj@example.com");

    // The pre-login CSRF token no longer works.
    let stale = client
        .post_form("/logout", &[("_csrf", token_before.as_str())])
        .await;
    assert_eq!(stale.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn wrong_password_and_unknown_account_look_the_same() {
    let (mut client, _) = app().await;
    client.get("/login").await;

    let wrong = client
        .post_form(
            "/login",
            &[
                ("email", "raj@example.com"),
                ("password", "not the password"),
            ],
        )
        .await;
    let unknown = client
        .post_form(
            "/login",
            &[
                ("email", "nobody@example.com"),
                ("password", "not the password"),
            ],
        )
        .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.status(), unknown.status());
    assert_eq!(wrong.text(), unknown.text());
}

#[tokio::test]
async fn repeated_failures_are_throttled_per_account() {
    let (mut client, _) = app().await;
    client.get("/login").await;
    for _ in 0..5 {
        let response = client
            .post_form(
                "/login",
                &[("email", "raj@example.com"), ("password", "guess")],
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // Even the right password is refused until the window passes.
    let response = log_in(&mut client, "raj@example.com").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.header("retry-after").is_some());

    // Other accounts still work.
    let response = log_in(&mut client, "admin@example.com").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn next_only_redirects_within_the_site() {
    let (mut client, _) = app().await;
    client.get("/login").await;
    let response = client
        .post_form(
            "/login",
            &[
                ("email", "raj@example.com"),
                ("password", "correct horse battery"),
                ("next", "/admin"),
            ],
        )
        .await;
    assert_eq!(response.location(), Some("/admin"));

    client.clear_cookies();
    client.get("/login").await;
    let response = client
        .post_form(
            "/login",
            &[
                ("email", "raj@example.com"),
                ("password", "correct horse battery"),
                ("next", "https://evil.example/phish"),
            ],
        )
        .await;
    assert_eq!(response.location(), Some("/dashboard"));
}

#[tokio::test]
async fn policies_guard_pages() {
    let (mut client, _) = app().await;
    log_in(&mut client, "raj@example.com").await;
    assert_eq!(client.get("/admin").await.status(), StatusCode::FORBIDDEN);

    client.clear_cookies();
    log_in(&mut client, "admin@example.com").await;
    let response = client.get("/admin").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text(), "admin admin@example.com");
}

#[tokio::test]
async fn deleted_users_are_logged_out() {
    let (mut client, users) = app().await;
    log_in(&mut client, "raj@example.com").await;
    assert_eq!(client.get("/dashboard").await.status(), StatusCode::OK);

    users.remove(1);
    let response = client.get("/dashboard").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(client.cookie("id").is_none(), "session cookie is cleared");
}

#[tokio::test]
async fn logout_ends_the_session() {
    let (mut client, _) = app().await;
    log_in(&mut client, "raj@example.com").await;
    client.get("/dashboard").await; // page with the logout form

    let response = client.post_form("/logout", &[]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(client.cookie("id").is_none());

    let response = client.get("/dashboard").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/login?next=%2Fdashboard"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_guesses_cannot_get_past_the_limit() {
    let (app, _) = build_app().await;
    let router = app.into_router();
    let mut client = TestClient::from_router(router.clone());
    client.get("/login").await;
    let cookie = format!("id={}", client.cookie("id").unwrap());
    let token = client.csrf_token().unwrap().to_owned();

    // 50 wrong passwords for one account, all sent at once.
    let guesses: Vec<_> = (0..50)
        .map(|i| {
            let body = format!("email=raj%40example.com&password=guess{i}&_csrf={token}");
            let request = Request::post("/login")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("sec-fetch-site", "same-origin")
                .body(Body::from(body))
                .unwrap();
            let router = router.clone();
            tokio::spawn(async move {
                use tower::ServiceExt;
                router.oneshot(request).await.unwrap().status()
            })
        })
        .collect();
    let mut checked = 0;
    for guess in guesses {
        match guess.await.unwrap() {
            StatusCode::UNAUTHORIZED => checked += 1,
            StatusCode::TOO_MANY_REQUESTS => {}
            other => panic!("unexpected {other}"),
        }
    }
    assert!(checked <= 5, "{checked} passwords were checked, limit 5");
}

#[tokio::test]
async fn an_attacker_cannot_lock_out_the_real_user() {
    let (app, _) = build_app().await;
    let router = app.into_router();
    let mut attacker = TestClient::from_router(router.clone())
        .with_client_addr("203.0.113.66:5000".parse().unwrap());
    attacker.get("/login").await;
    for i in 0..5 {
        let response = attacker
            .post_form(
                "/login",
                &[
                    ("email", "RAJ@example.com"),
                    ("password", &format!("guess {i}")),
                ],
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = log_in(&mut attacker, "raj@example.com").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    // The owner, from their own address, still gets in.
    let mut owner =
        TestClient::from_router(router).with_client_addr("198.51.100.7:6000".parse().unwrap());
    let response = log_in(&mut owner, "raj@example.com").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.location(), Some("/dashboard"));
}
