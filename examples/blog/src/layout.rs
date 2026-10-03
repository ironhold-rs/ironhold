//! The page layout shared by every page.

use ironhold::prelude::*;
use ironhold::raw_unchecked;

/// Wraps `body` in the site layout. The inline `<style>` only runs because
/// it carries this request's CSP nonce.
pub fn page(nonce: &CspNonce, title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " | blog" }
                // A constant we wrote, so it's safe to insert unescaped.
                style nonce=(nonce.as_str()) { (raw_unchecked(STYLE)) }
            }
            body {
                header { a href="/" { "blog" } }
                main { (body) }
            }
        }
    }
}

const STYLE: &str = "
body { font-family: system-ui, sans-serif; max-width: 36rem; margin: 2rem auto; padding: 0 1rem; line-height: 1.6; color: #1f2328; }
header { margin-bottom: 2rem; font-weight: 600; }
a { color: #0b57d0; }
form { display: grid; gap: 0.75rem; margin: 1rem 0; }
label { display: grid; gap: 0.25rem; }
input, textarea { padding: 0.5rem; font: inherit; border: 1px solid #c4c8cc; border-radius: 6px; }
button { padding: 0.55rem 1rem; font: inherit; border: 0; border-radius: 6px; background: #1f2328; color: white; cursor: pointer; justify-self: start; }
.error, .field-error { color: #b3261e; }
.field-error { margin: -0.4rem 0 0; font-size: 0.9rem; }
time { color: #5d6167; font-size: 0.9rem; }
.posts { list-style: none; padding: 0; display: grid; gap: 0.5rem; }
.post p { white-space: pre-line; }
";
