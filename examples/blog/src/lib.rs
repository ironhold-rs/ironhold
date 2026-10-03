//! The blog built in the Ironhold tutorial (`docs/tutorial.md`).

mod auth;
mod layout;
mod pages;
mod posts;
pub mod user;

use ironhold::prelude::*;

/// Builds the app. `main.rs` serves it, and the tests drive it with
/// `ironhold::testing::TestClient`.
pub fn app(db: SqliteDb) -> App {
    App::new()
        .database(db)
        .route("/", get(posts::index))
        .route("/posts", post(posts::create))
        .route("/posts/new", get(posts::new_post))
        .route("/posts/{id}", get(posts::show).post(posts::update))
        .route("/posts/{id}/edit", get(posts::edit))
        .route("/posts/{id}/delete", post(posts::destroy))
        .route("/dashboard", get(pages::dashboard))
        .route("/signup", get(auth::signup_page).post(auth::signup))
        .route("/login", get(auth::login_page).post(auth::login))
        .route("/logout", post(auth::logout))
}
