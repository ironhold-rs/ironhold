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
