//! The database integration, end to end with SQLite.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use ironhold::http::StatusCode;
use ironhold::prelude::*;
use ironhold::{Config, Environment};
use serde::Deserialize;
use support::{get_text, open_form, status_and_body, submit, temp_sqlite_url};

#[derive(Deserialize)]
struct Note {
    text: String,
}

fn app(db: SqliteDb) -> ironhold::Router {
    App::with_config(Config::new(Environment::Development))
        .database(db)
        .route(
            "/notes/new",
            get(|csrf: CsrfToken| async move {
                html! { form method="post" action="/notes" { (csrf) input name="text"; } }
            }),
        )
        .route(
            "/notes",
            post(|Form(note): Form<Note>| async move { format!("saved {}", note.text) }),
        )
        .route(
            "/sessions",
            get(|db: SqliteDb| async move {
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ironhold_sessions")
                    .fetch_one(&*db)
                    .await?;
                Ok::<_, Error>(count.to_string())
            }),
        )
        .route(
            "/broken",
            get(|db: SqliteDb| async move {
                let _: i64 = sqlx::query_scalar("SELECT secret FROM missing_table")
                    .fetch_one(&*db)
                    .await?;
                Ok::<_, Error>("unreachable")
            }),
        )
        .into_router()
}

#[tokio::test]
async fn sessions_are_stored_in_the_database_and_survive_a_restart() {
    let (url, _guard) = temp_sqlite_url();

    let before_restart = app(SqliteDb::connect(&url).await.unwrap());
    let (cookie, token, _) = open_form(&before_restart, "/notes/new").await;
    assert_eq!(get_text(&before_restart, "/sessions").await.1, "1");
    drop(before_restart);

    // A brand new app on the same database accepts the old session's form.
    let after_restart = app(SqliteDb::connect(&url).await.unwrap());
    let request = submit("/notes", &cookie, format!("text=hello&_csrf={token}"));
    let (status, body) = status_and_body(&after_restart, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "saved hello");
}

#[tokio::test]
async fn database_errors_become_a_500_without_details() {
    let (url, _guard) = temp_sqlite_url();
    let router = app(SqliteDb::connect(&url).await.unwrap());

    let (status, body) = get_text(&router, "/broken").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, "Internal Server Error");
}

#[tokio::test]
async fn a_handler_needing_a_database_fails_clearly_without_one() {
    let router = App::with_config(Config::new(Environment::Development))
        .route("/", get(|_db: SqliteDb| async { "never reached" }))
        .into_router();

    let (status, body) = get_text(&router, "/").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, "Internal Server Error");
}
