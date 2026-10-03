# Tutorial: build a blog with Ironhold

In this tutorial you'll build a small blog. People sign up, write posts, and edit or delete their own. Along the way you'll use Ironhold's database support, validation, permissions and test client. It takes about 30 minutes, and the finished code is in [`examples/blog`](../examples/blog).

You'll need Rust 1.94 or newer and some Rust experience. You don't need to know Ironhold.

## 1. Install the CLI

Ironhold isn't on crates.io yet, so install the `ironhold` command from a clone of the repository:

```bash
git clone https://github.com/ironhold-rs/ironhold
```

```bash
cargo install --path ironhold/crates/ironhold-cli
```

## 2. Create the app

```bash
ironhold new blog --ironhold-path ./ironhold
```

```bash
cd blog && ironhold dev
```

Open <http://127.0.0.1:3000>. You already have a working app: sign up, log in, log out, and a dashboard that only logged-in users can see. `ironhold dev` rebuilds and restarts the app every time you save, so keep it running while you work. If a change doesn't compile, the last working version keeps running and the error shows in your terminal.

These are the files you'll work with:

| Path | What's in it |
|---|---|
| `src/lib.rs` | Routes |
| `src/pages.rs` | Pages |
| `src/user.rs` | Users and their queries |
| `src/layout.rs` | The layout shared by every page |
| `migrations/` | Database migrations, applied on startup |
| `tests/` | Tests |

## 3. Add a posts table

Create `migrations/0002_create_posts.sql`:

```sql
-- migrations/0002_create_posts.sql
-- Blog posts. Deleting a user deletes their posts.
CREATE TABLE posts (
    id INTEGER PRIMARY KEY,
    author_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX posts_created_at ON posts (created_at);
```

Migrations run when the app starts, in the order of their numbers. Save the file and `ironhold dev` restarts the app, which creates the table. `ON DELETE CASCADE` means deleting a user also deletes their posts.

Never edit a migration after it has run. To change the table later, add a new migration.

## 4. Describe a post

Create `src/posts.rs` and start with the types:

```rust
// src/posts.rs
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
```

`Post` is a post as the site shows it. `PostForm` is what the post form submits, and its `Validate` implementation lists the rules: a title of up to 120 characters and a body of up to 20,000, both required.

`EditPost` is a permission: only a post's author may edit or delete it. Handlers check it with `EditPost::check(&user, &post)?`, which answers `403 Forbidden` when it fails.

## 5. Read and write posts

Add the queries below the types:

```rust
// src/posts.rs
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
```

Two things keep this safe. sqlx only accepts SQL written as a string literal, and every value goes through `.bind()`, so user input can never become part of the SQL. And `insert_post` and `update_post` take `Valid<PostForm>`, which only `validate()` can produce. Forgetting to validate is a compile error instead of a bug in production.

`find_post` turns a missing row into `Error::NotFound`, which becomes a `404`. The `?` after each query turns a database error into a `500` that's logged on the server and shows no details to visitors.

## 6. Show posts

Add the home page and the page for one post:

```rust
// src/posts.rs
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
```

`index` takes `Option<AuthUser<User>>`, so it works for everyone and can still tell whether someone is logged in. `show` only creates a CSRF token for the post's author, who needs it for the Delete button, so readers don't get a session just for reading.

Everything inside `html!` is escaped. If someone writes `<script>` in a post, readers see the text `<script>` and nothing runs.

## 7. Write, edit and delete

Add the rest of `src/posts.rs`:

```rust
// src/posts.rs
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
```

`new_post` takes `AuthUser<User>` even though it doesn't use it. That's what makes the page login-only: logged-out visitors are sent to the login page and brought back afterwards.

When `validate()` fails, it returns `Invalid`, which holds what was typed and the errors. `invalid_form` shows the form again with both, with the status `422 Unprocessable Entity`. In the template, `(errors.field("title"))` puts the title's problem right under its input.

`edit`, `update` and `destroy` load the post and call `EditPost::check` before changing anything. Every form carries the CSRF token through `(csrf)`, and `Form<T>` rejects any submission without it.

## 8. Connect the routes

Replace `src/lib.rs` with:

```rust
// src/lib.rs
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
```

HTML forms can only send GET and POST, so updates go to `POST /posts/{id}` and deletes to `POST /posts/{id}/delete`.

The home page is now the list of posts, so `src/pages.rs` no longer needs its `home` function. Replace it with this version, which also links to the new pages from the dashboard:

```rust
// src/pages.rs
//! Pages.

use ironhold::prelude::*;

use crate::layout::page;
use crate::user::User;

/// Only logged-in users get here: `AuthUser` sends everyone else to the
/// login page and brings them back afterwards.
pub async fn dashboard(nonce: CspNonce, csrf: CsrfToken, user: AuthUser<User>) -> Markup {
    page(
        &nonce,
        "Dashboard",
        html! {
            h1 { "Dashboard" }
            p { "Hello, " (user.email) "!" }
            p { a href="/posts/new" { "Write a post" } " or " a href="/" { "read the blog" } "." }
            form method="post" action="/logout" {
                (csrf)
                button { "Log out" }
            }
        },
    )
}
```

Last, style the new elements. In `STYLE` in `src/layout.rs`, change `input {` to `input, textarea {` and add these lines:

```css
/* src/layout.rs */
time { color: #5d6167; font-size: 0.9rem; }
.posts { list-style: none; padding: 0; display: grid; gap: 0.5rem; }
.post p { white-space: pre-line; }
```

Save, and try it in the browser: sign up, write a post, edit it and delete it. Then log in as a second user and check that you can't edit the first user's post.

## 9. Test it

Tests drive the app like a browser, with a fresh database for each test. Create `tests/posts.rs`:

```rust
// tests/posts.rs
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
```

`TestClient` keeps cookies between requests and fills in the CSRF token from the last page it loaded, the way a browser submits a real form. Run the tests:

```bash
cargo test
```

The finished [`tests/posts.rs`](../examples/blog/tests/posts.rs) also checks validation errors, that only the author can edit or delete a post, and that a missing post is a `404`.

## 10. Ship it

```bash
cargo build --release
```

That builds one binary, `target/release/blog`, with the migrations inside it. On your server, run it like this:

```bash
IRONHOLD_ENV=production IRONHOLD_ADDR=0.0.0.0:8080 DATABASE_URL=sqlite:///var/lib/blog/blog.db ./blog
```

Put it behind an HTTPS proxy such as Caddy or Nginx. HTTPS is required in production, because the session cookie is only sent over HTTPS.

The whole database is one SQLite file. Back it up with `sqlite3 /var/lib/blog/blog.db ".backup backup.db"`, which is safe while the app is running.

## Where to go next

- The [README](../README.md) shows every feature in one place.
- [ARCHITECTURE.md](../ARCHITECTURE.md) explains how Ironhold works and what's coming next.
