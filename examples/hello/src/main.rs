//! Minimal Ironhold app. Run with `cargo run -p hello`, then open
//! <http://127.0.0.1:3000>.

use ironhold::prelude::*;
use serde::Deserialize;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    App::new()
        .route("/", get(home))
        .route("/hello/{name}", get(hello))
        .route("/greet", get(greet_form).post(greet))
        .route("/boom", get(boom))
        .serve()
        .await
}

async fn home(nonce: CspNonce) -> Markup {
    layout(
        &nonce,
        "Ironhold",
        html! {
            h1 { "Welcome to Ironhold" }
            p { "Secure by default. Try these:" }
            ul {
                li { a href="/hello/Raj" { "A normal greeting" } }
                li { a href="/greet" { "A form protected against CSRF" } }
                li {
                    a href="/hello/%3Cscript%3Ealert(1)%3C%2Fscript%3E" { "An XSS attempt" }
                    " (it's escaped and rendered as text)"
                }
                li { a href="/boom" { "An internal error" } " (details are logged, not shown)" }
                li { a href="/missing" { "A missing page" } }
            }
        },
    )
}

async fn hello(Path(name): Path<String>, nonce: CspNonce) -> Markup {
    layout(
        &nonce,
        "Hello",
        html! {
            h1 { "Hello, " (name) "!" }
            p { a href="/" { "Back" } }
        },
    )
}

async fn greet_form(nonce: CspNonce, csrf: CsrfToken) -> Markup {
    layout(
        &nonce,
        "Greet",
        html! {
            h1 { "Say hello" }
            form method="post" action="/greet" {
                (csrf)
                label { "Your name " input name="name" required; }
                " "
                button { "Greet me" }
            }
        },
    )
}

#[derive(Deserialize)]
struct Greeting {
    name: String,
}

/// `Form` only runs this handler if the CSRF token from the form matches
/// the session.
async fn greet(nonce: CspNonce, Form(greeting): Form<Greeting>) -> Markup {
    layout(
        &nonce,
        "Hello",
        html! {
            h1 { "Hello, " (greeting.name) "!" }
            p { a href="/greet" { "Again" } }
        },
    )
}

async fn boom() -> Result<Markup> {
    Err(Error::internal("could not connect to the database"))
}

/// A layout is a plain function. The inline `<style>` only runs because it
/// carries this request's CSP nonce.
fn layout(nonce: &CspNonce, title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                style nonce=(nonce.as_str()) {
                    "body { font-family: system-ui, sans-serif; max-width: 40rem; margin: 3rem auto; padding: 0 1rem; line-height: 1.6 }"
                }
            }
            body { (body) }
        }
    }
}
