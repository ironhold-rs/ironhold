//! Blog posts: the list on the home page, and writing, editing and
//! deleting posts.

use ironhold::http::StatusCode;
use ironhold::prelude::*;
use ironhold::response::Response;
use serde::Deserialize;

use crate::layout::page;
use crate::user::User;

/// A post as shown on the site.
#[derive(Debug, Clone)]
pub struct Post {
    pub id: i64,
    pub author_id: i64,
    pub title: String,
    pub body: String,
    /// The day it was published, as `YYYY-MM-DD`.
    pub published: String,
}

/// The fields of the post form.
#[derive(Debug, Default, Deserialize)]
pub struct PostForm {
    pub title: String,
    pub body: String,
}

impl Validate for PostForm {
    fn rules(&self, v: &mut Validator) {
        v.check("title", &self.title).required().max_chars(120);
        v.check("body", &self.body).required().max_chars(20_000);
    }
}

/// Only a post's author may edit or delete it.
pub struct EditPost;

impl Permission<User, Post> for EditPost {
    fn allows(user: &User, post: &Post) -> bool {
        post.author_id == user.id
    }
}

// Queries. sqlx only accepts SQL written as string literals, and values go
// through `.bind()`, so there's no way to splice user input into the SQL.

type PostRow = (i64, i64, String, String, String);

fn to_post((id, author_id, title, body, published): PostRow) -> Post {
    Post {
        id,
        author_id,
        title,
        body,
        published,
    }
}

async fn newest_posts(db: &SqliteDb) -> Result<Vec<Post>> {
    let rows: Vec<PostRow> = sqlx::query_as(
        "SELECT id, author_id, title, body, date(created_at, 'unixepoch') \
         FROM posts ORDER BY created_at DESC, id DESC LIMIT 50",
    )
    .fetch_all(db.pool())
    .await?;
    Ok(rows.into_iter().map(to_post).collect())
}

async fn find_post(db: &SqliteDb, id: i64) -> Result<Post> {
    let row: Option<PostRow> = sqlx::query_as(
        "SELECT id, author_id, title, body, date(created_at, 'unixepoch') \
         FROM posts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db.pool())
    .await?;
    row.map(to_post).ok_or(Error::NotFound)
}

/// Takes `Valid<PostForm>`, so unchecked input can't be saved.
async fn insert_post(db: &SqliteDb, author: &User, form: &Valid<PostForm>) -> Result<i64> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO posts (author_id, title, body) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(author.id)
    .bind(&form.title)
    .bind(&form.body)
    .fetch_one(db.pool())
    .await?;
    Ok(id)
}

async fn update_post(db: &SqliteDb, id: i64, form: &Valid<PostForm>) -> Result<()> {
    sqlx::query("UPDATE posts SET title = $1, body = $2, updated_at = unixepoch() WHERE id = $3")
        .bind(&form.title)
        .bind(&form.body)
        .bind(id)
        .execute(db.pool())
        .await?;
    Ok(())
}

async fn delete_post(db: &SqliteDb, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM posts WHERE id = $1")
        .bind(id)
        .execute(db.pool())
        .await?;
    Ok(())
}

// Pages.

/// The home page: the newest posts.
pub async fn index(nonce: CspNonce, user: Option<AuthUser<User>>, db: SqliteDb) -> Result<Markup> {
    let posts = newest_posts(&db).await?;
    Ok(page(
        &nonce,
        "Blog",
        html! {
            h1 { "Blog" }
            @if user.is_some() {
                p { a href="/posts/new" { "Write a post" } }
            } @else {
                p { a href="/signup" { "Sign up" } " or " a href="/login" { "log in" } " to write." }
            }
            @if posts.is_empty() {
                p { "No posts yet." }
            } @else {
                ul.posts {
                    @for post in &posts {
                        li {
                            a href={ "/posts/" (post.id) } { (post.title) }
                            " "
                            time datetime=(post.published) { (post.published) }
                        }
                    }
                }
            }
        },
    ))
}

/// One post. Its author also sees Edit and Delete.
pub async fn show(
    nonce: CspNonce,
    session: Session,
    user: Option<AuthUser<User>>,
    db: SqliteDb,
    Path(id): Path<i64>,
) -> Result<Markup> {
    let post = find_post(&db, id).await?;
    let can_edit = user.is_some_and(|user| EditPost::allows(&user, &post));
    // Only the author needs a CSRF token (for the delete button), so readers
    // don't get a session just for reading.
    let csrf = if can_edit {
        Some(CsrfToken::for_session(&session).await?)
    } else {
        None
    };
    // Browsers send line breaks as \r\n; paragraphs are separated by a blank line.
    let body = post.body.replace("\r\n", "\n");

    Ok(page(
        &nonce,
        &post.title,
        html! {
            article.post {
                h1 { (post.title) }
                p.date { time datetime=(post.published) { (post.published) } }
                @for paragraph in body.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
                    p { (paragraph) }
                }
            }
            @if let Some(csrf) = csrf {
                p { a href={ "/posts/" (post.id) "/edit" } { "Edit" } }
                form method="post" action={ "/posts/" (post.id) "/delete" } {
                    (csrf)
                    button { "Delete" }
                }
            }
            p { a href="/" { "All posts" } }
        },
    ))
}

/// The form for a new post. Only for logged-in users.
pub async fn new_post(nonce: CspNonce, csrf: CsrfToken, _user: AuthUser<User>) -> Markup {
    let form = post_form(
        &csrf,
        "/posts",
        &PostForm::default(),
        &ValidationErrors::new(),
    );
    page(&nonce, "New post", html! { h1 { "New post" } (form) })
}

pub async fn create(
    nonce: CspNonce,
    csrf: CsrfToken,
    user: AuthUser<User>,
    db: SqliteDb,
    Form(form): Form<PostForm>,
) -> Result<Response> {
    let form = match form.validate() {
        Ok(form) => form,
        Err(invalid) => return Ok(invalid_form(&nonce, &csrf, "New post", "/posts", &invalid)),
    };
    let id = insert_post(&db, &user, &form).await?;
    Ok(Redirect::to(&format!("/posts/{id}")).into_response())
}

/// The form for editing a post. Only for its author.
pub async fn edit(
    nonce: CspNonce,
    csrf: CsrfToken,
    user: AuthUser<User>,
    db: SqliteDb,
    Path(id): Path<i64>,
) -> Result<Markup> {
    let post = find_post(&db, id).await?;
    EditPost::check(&user, &post)?;
    let values = PostForm {
        title: post.title,
        body: post.body,
    };
    let form = post_form(
        &csrf,
        &format!("/posts/{id}"),
        &values,
        &ValidationErrors::new(),
    );
    Ok(page(
        &nonce,
        "Edit post",
        html! { h1 { "Edit post" } (form) },
    ))
}

pub async fn update(
    nonce: CspNonce,
    csrf: CsrfToken,
    user: AuthUser<User>,
    db: SqliteDb,
    Path(id): Path<i64>,
    Form(form): Form<PostForm>,
) -> Result<Response> {
    let post = find_post(&db, id).await?;
    EditPost::check(&user, &post)?;
    let form = match form.validate() {
        Ok(form) => form,
        Err(invalid) => {
            let action = format!("/posts/{id}");
            return Ok(invalid_form(&nonce, &csrf, "Edit post", &action, &invalid));
        }
    };
    update_post(&db, id, &form).await?;
    Ok(Redirect::to(&format!("/posts/{id}")).into_response())
}

#[derive(Deserialize)]
pub struct DeleteForm {}

pub async fn destroy(
    user: AuthUser<User>,
    db: SqliteDb,
    Path(id): Path<i64>,
    Form(_): Form<DeleteForm>,
) -> Result<Redirect> {
    let post = find_post(&db, id).await?;
    EditPost::check(&user, &post)?;
    delete_post(&db, id).await?;
    Ok(Redirect::to("/"))
}

fn post_form(
    csrf: &CsrfToken,
    action: &str,
    values: &PostForm,
    errors: &ValidationErrors,
) -> Markup {
    html! {
        form method="post" action=(action) novalidate {
            (csrf)
            label {
                "Title"
                input name="title" value=(values.title) maxlength="120" required
                    aria-invalid=[errors.aria_invalid("title")]
                    aria-describedby=[errors.described_by("title")];
            }
            (errors.field("title"))
            label {
                "Post"
                textarea name="body" rows="14" required
                    aria-invalid=[errors.aria_invalid("body")]
                    aria-describedby=[errors.described_by("body")] { (values.body) }
            }
            (errors.field("body"))
            button { "Save" }
        }
    }
}

/// The form again, with what was typed and what's wrong with it.
fn invalid_form(
    nonce: &CspNonce,
    csrf: &CsrfToken,
    heading: &str,
    action: &str,
    invalid: &Invalid<PostForm>,
) -> Response {
    let form = post_form(csrf, action, &invalid.input, &invalid.errors);
    let page = page(nonce, heading, html! { h1 { (heading) } (form) });
    (StatusCode::UNPROCESSABLE_ENTITY, page).into_response()
}
