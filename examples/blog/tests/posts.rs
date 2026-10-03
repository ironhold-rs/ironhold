//! Writing, reading, editing and deleting posts, driven like a browser.

mod common;

use ironhold::http::StatusCode;
use ironhold::testing::TestClient;

const PASSWORD: &str = "correct horse battery";

/// A browser that has signed up and is logged in.
async fn signed_in(test_db: &common::TestDb, name: &str) -> TestClient {
    let mut client = TestClient::new(blog::app(test_db.db.clone()));
    client.get("/signup").await;
    let email = common::unique_email(name);
    let response = client
        .post_form(
            "/signup",
            &[("email", email.as_str()), ("password", PASSWORD)],
        )
        .await;
    assert_eq!(response.location(), Some("/dashboard"));
    client
}

/// Writes a post and returns its address.
async fn write_post(client: &mut TestClient, title: &str, body: &str) -> String {
    client.get("/posts/new").await;
    let response = client
        .post_form("/posts", &[("title", title), ("body", body)])
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    response.location().unwrap().to_owned()
}

#[tokio::test]
async fn visitors_can_read_but_must_log_in_to_write() {
    let Some(test_db) = common::test_db().await else {
        return;
    };
    let mut visitor = TestClient::new(blog::app(test_db.db.clone()));

    assert!(visitor.get("/").await.text().contains("No posts yet."));

    let response = visitor.get("/posts/new").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.location(), Some("/login?next=%2Fposts%2Fnew"));
}

#[tokio::test]
async fn writing_a_post() {
    let Some(test_db) = common::test_db().await else {
        return;
    };
    let mut author = signed_in(&test_db, "author").await;

    let url = write_post(
        &mut author,
        "Hello, world",
        "First paragraph.\r\n\r\n<script>alert(1)</script>",
    )
    .await;

    let page = author.get(&url).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert!(page.text().contains("<h1>Hello, world</h1>"));
    assert!(page.text().contains("<p>First paragraph.</p>"));
    // Whatever people type is shown as text, never run as code.
    assert!(
        page.text()
            .contains("<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>")
    );

    assert!(author.get("/").await.text().contains("Hello, world"));
}

#[tokio::test]
async fn invalid_posts_are_shown_again_with_errors() {
    let Some(test_db) = common::test_db().await else {
        return;
    };
    let mut author = signed_in(&test_db, "author").await;

    author.get("/posts/new").await;
    let response = author
        .post_form("/posts", &[("title", ""), ("body", "Kept body")])
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.text().contains(r#"id="title-error""#));
    // What was typed is still there.
    assert!(response.text().contains("Kept body"));
}

#[tokio::test]
async fn only_the_author_can_edit_or_delete() {
    let Some(test_db) = common::test_db().await else {
        return;
    };
    let mut author = signed_in(&test_db, "author").await;
    let url = write_post(&mut author, "Mine", "Body").await;

    let mut other = signed_in(&test_db, "other").await;
    assert!(!other.get(&url).await.text().contains("Delete"));
    assert_eq!(
        other.get(&format!("{url}/edit")).await.status(),
        StatusCode::FORBIDDEN
    );
    other.get("/posts/new").await; // any page with a form, for a CSRF token
    let response = other.post_form(&format!("{url}/delete"), &[]).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    author.get(&format!("{url}/edit")).await;
    let response = author
        .post_form(&url, &[("title", "Mine, edited"), ("body", "Body")])
        .await;
    assert_eq!(response.location(), Some(url.as_str()));
    assert!(author.get(&url).await.text().contains("Mine, edited"));

    let response = author.post_form(&format!("{url}/delete"), &[]).await;
    assert_eq!(response.location(), Some("/"));
    assert_eq!(author.get(&url).await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_posts_are_not_found() {
    let Some(test_db) = common::test_db().await else {
        return;
    };
    let mut visitor = TestClient::new(blog::app(test_db.db.clone()));
    assert_eq!(
        visitor.get("/posts/999999").await.status(),
        StatusCode::NOT_FOUND
    );
}
