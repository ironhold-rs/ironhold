# Ironhold

A full-stack Rust web framework that is secure by default, productive and fast.

> Ironhold is in early development. APIs will change, and it isn't ready for production yet.

```rust
use ironhold::prelude::*;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    App::new()
        .route("/hello/{name}", get(hello))
        .serve()
        .await
}

async fn hello(Path(name): Path<String>) -> Markup {
    // `name` is escaped automatically. XSS is not possible here.
    html! { h1 { "Hello, " (name) "!" } }
}
```

## Why Ironhold

Most frameworks make security something you have to remember. Ironhold makes it the default and uses Rust's type system to make insecure code hard to write.

Every app gets these with zero configuration:

| Protection | How |
|---|---|
| XSS | Templates escape every value. Raw HTML needs an explicit `raw_unchecked(...)`. |
| Script injection | Content-Security-Policy with a fresh nonce per request (`CspNonce`) |
| CSRF | Cross-origin POST/PUT/PATCH/DELETE requests are blocked, and `Form<T>` rejects submissions without the session's CSRF token |
| Session theft | Server-side sessions; the cookie is `HttpOnly`, `SameSite=Lax`, and `Secure` with the `__Host-` prefix in production |
| Clickjacking, MIME sniffing | `X-Frame-Options`, `frame-ancestors`, `nosniff`, COOP/CORP |
| HTTPS downgrade | HSTS and `upgrade-insecure-requests` in production |
| Oversized or slow requests | Body size limit (2 MiB) and request timeout (30 s) |
| Overload | Beyond 1024 requests in flight, new requests get an instant `503` with `Retry-After`, so the server stays fast for the requests it accepts |
| Crashes | A panic in a handler becomes a `500`; the server keeps running. Framework code can't contain `unwrap`, `expect` or `panic` (enforced in CI) |
| SQL injection | sqlx 0.9 only accepts SQL string literals; values always go through `.bind()` |
| Leaking internals | `500` responses never include error details; they're logged instead |
| Leaking secrets | `Secret<T>` prints `[REDACTED]` and can't be serialized |
| Weak password storage | Argon2id with OWASP parameters, hashed off the async threads with limited concurrency |
| Pages that forget to check login | A handler taking `AuthUser<User>` can't run for logged-out visitors; `Authorize<User, Policy>` can't run unless the policy allows it |
| Password guessing | 5 failed logins per account per 5 minutes, then `429` |
| Account discovery | Logins for unknown emails take as long as real ones |
| Session fixation, open redirects | Login issues a new session id and CSRF token; `?next=` only redirects within the site |
| Memory safety bugs | Rust, with `#![forbid(unsafe_code)]` in every framework crate |

Coming next: a model layer with generated migrations, and live components (interactive pages without writing JavaScript). See [ARCHITECTURE.md](ARCHITECTURE.md) for the design and roadmap.

## Getting started

Requires Rust 1.94 or newer ([install Rust](https://rustup.rs)). Ironhold isn't on crates.io yet, so for now install the CLI from a clone of this repository:

```bash
git clone https://github.com/ironhold-rs/ironhold
```

```bash
cargo install --path ironhold/crates/ironhold-cli
```

Create an app with sign up, log in and log out ready to go, then run it:

```bash
ironhold new my-app --ironhold-path ./ironhold
```

```bash
cd my-app && ironhold dev
```

Open <http://127.0.0.1:3000>. `ironhold dev` rebuilds and restarts the app every time you save; if a change doesn't compile, the last working version keeps running. Use `--db postgres` for Postgres instead of SQLite.

The new app comes with tests that drive it like a browser:

```bash
cargo test
```

To add Ironhold to an existing project instead:

```toml
[dependencies]
ironhold = { version = "0.0.1", features = ["sqlite"] }
maud = "0.27" # needed by the html! macro for now
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `IRONHOLD_ENV` | `development` | `development` or `production`. Production turns on HSTS. |
| `IRONHOLD_ADDR` | `127.0.0.1:3000` | Listen address. Use `0.0.0.0:PORT` to accept outside connections. |
| `RUST_LOG` | `info` | Log filter, e.g. `debug` or `ironhold_core=debug` |

Invalid values stop the app at startup instead of falling back silently.

## Forms and CSRF

Put the session's `CsrfToken` in every form and read submissions with `Form<T>`. The handler only runs if the token matches.

```rust
use ironhold::prelude::*;
use serde::Deserialize;

async fn new_post(csrf: CsrfToken) -> Markup {
    html! {
        form method="post" action="/posts" {
            (csrf)
            input name="title";
            button { "Save" }
        }
    }
}

#[derive(Deserialize)]
struct NewPost {
    title: String,
}

async fn create_post(Form(post): Form<NewPost>) -> Markup {
    // Only reached with a valid CSRF token from a same-origin request.
    html! { p { "Saved " (post.title) } }
}
```

For `fetch` requests, send the token in the `X-CSRF-Token` header instead.

Without a database, sessions are kept in memory and are lost on restart. Connect a database (below) and they're stored there automatically.

## Validation

Write rules in plain Rust. `validate()` returns `Valid<T>`, or `Invalid<T>` with the input as typed and the problems to show next to each field. Code that takes `Valid<T>` can't be called with unchecked input.

```rust
use ironhold::http::StatusCode;
use ironhold::prelude::*;
use ironhold::response::Response;
use serde::Deserialize;

#[derive(Deserialize)]
struct NewPost {
    title: String,
    url: String,
}

impl Validate for NewPost {
    fn rules(&self, v: &mut Validator) {
        v.check("title", &self.title).required().max_chars(120);
        v.check("url", &self.url).url(); // optional: only checked when filled in
    }
}

async fn create_post(csrf: CsrfToken, Form(input): Form<NewPost>) -> Response {
    match input.validate() {
        Ok(post) => save(post).await,
        // Show the form again with what was typed. In the template,
        // `(errors.field("title"))` shows the title's problem next to its input.
        Err(invalid) => {
            let form = post_form(&csrf, &invalid.input, &invalid.errors);
            (StatusCode::UNPROCESSABLE_ENTITY, form).into_response()
        }
    }
}
```

Rules: `required`, `min_chars`, `max_chars`, `max_bytes`, `email`, `url`, `one_of`, `equals`, `custom`, and `min`/`max` for numbers. `.message(...)` replaces the message of the rule before it. For JSON APIs, return the `Invalid` value as the response: a `422` with `{"errors": {"title": ["This field is required."]}}`.

## Login and access control

```rust
use ironhold::prelude::*;

// Only logged-in users get here. Browsers are sent to /login and brought
// back afterwards; API calls get 401.
async fn dashboard(user: AuthUser<User>) -> Markup {
    html! { p { "Hello, " (user.email) } }
}

// Only admins get here; everyone else gets 403.
struct Admins;
impl Policy<User> for Admins {
    fn allows(user: &User) -> bool { user.is_admin }
}
async fn admin(admin: Authorize<User, Admins>) -> Markup { /* ... */ }
```

`ironhold::auth` provides `hash_password`, `verify_password`, `login`, `logout`, `LoginThrottle` and `safe_redirect_path`. The app generated by `ironhold new` shows them working together in `src/auth.rs`.

## Testing

With the `testing` feature, `TestClient` drives your app in memory like a browser: it keeps cookies and fills in CSRF tokens.

```rust
let mut client = TestClient::new(app(db));
client.get("/login").await;
let response = client
    .post_form("/login", &[("email", "raj@example.com"), ("password", "correct horse battery")])
    .await;
assert_eq!(response.location(), Some("/dashboard"));
```

## Database

Enable a backend with a Cargo feature: `sqlite` (recommended to start: no server to run) or `postgres`.

```toml
[dependencies]
ironhold = { version = "0.0.1", features = ["sqlite"] }
sqlx = { version = "0.9", features = ["sqlite", "macros", "migrate"] } # for query!() and migrate!()
```

```rust
use ironhold::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = SqliteDb::connect("sqlite://app.db").await?;
    sqlx::migrate!().run(&*db).await?; // your migrations in ./migrations

    App::new()
        .database(db) // handlers can take `db: SqliteDb`; sessions now live in the database
        .route("/posts/count", get(count))
        .serve()
        .await?;
    Ok(())
}

async fn count(db: SqliteDb) -> Result<String> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts")
        .fetch_one(&*db)
        .await?; // any database error becomes a logged 500
    Ok(n.to_string())
}
```

SQLite runs in WAL mode (readers never block the writer) with `synchronous=NORMAL`, foreign keys on, a 5 second busy timeout and one connection per CPU core. Postgres gets 10 connections, and remote hosts require TLS unless the URL sets `sslmode`. With either database, a request waits at most 5 seconds for a connection and then fails instead of piling up, and expired sessions are cleaned up every 5 minutes.

## Crates

| Crate | Purpose |
|---|---|
| [`ironhold`](crates/ironhold) | The crate apps depend on. Re-exports everything below. |
| [`ironhold-core`](crates/ironhold-core) | App builder, routing, config, errors, graceful shutdown |
| [`ironhold-security`](crates/ironhold-security) | Security headers, CSP nonces, cross-origin protection, limits, `Secret<T>` |
| [`ironhold-session`](crates/ironhold-session) | Server-side sessions and CSRF tokens |
| [`ironhold-forms`](crates/ironhold-forms) | CSRF-checked `Form<T>` extractor and validation |
| [`ironhold-db`](crates/ironhold-db) | SQLite and Postgres with tuned defaults, database-backed sessions |
| [`ironhold-auth`](crates/ironhold-auth) | Passwords, login and logout, `AuthUser`, `Authorize`, login throttling |
| [`ironhold-cli`](crates/ironhold-cli) | The `ironhold` command: `new` and `dev` |
| [`ironhold-html`](crates/ironhold-html) | Auto-escaping templates |

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Please report security issues privately as described in [SECURITY.md](SECURITY.md), not in public issues.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Ironhold by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
