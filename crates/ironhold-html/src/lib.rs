//! HTML templates for Ironhold.
//!
//! Templates are checked at compile time and escape every interpolated
//! value, so user input can't inject markup:
//!
//! ```
//! use ironhold_html::html;
//!
//! let name = "<script>alert(1)</script>";
//! let page = html! { p { "Hello, " (name) } };
//! assert_eq!(page.into_string(), "<p>Hello, &lt;script&gt;alert(1)&lt;/script&gt;</p>");
//! ```
//!
//! Inserting raw HTML requires calling [`raw_unchecked`]. Its name makes
//! every use stand out in code review.
//!
//! The template engine is [maud](https://maud.lambda.xyz). Components are
//! plain Rust functions that return [`Markup`].

#![forbid(unsafe_code)]

// Note: maud's `html!` expands to `extern crate maud;`, so apps using it
// must also list `maud` in their Cargo.toml. Ironhold's own template macro
// (planned for phase 2, with islands) will remove that requirement.
pub use maud::{DOCTYPE, Markup, Render, html};

/// Inserts `html` **without escaping**.
///
/// Only use this for markup you fully control, never for anything that
/// came from a user, a database or another service.
pub fn raw_unchecked(html: impl Into<String>) -> Markup {
    maud::PreEscaped(html.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_escapes_attributes_and_text() {
        let evil = r#""><img src=x onerror=alert(1)>"#;
        let out = html! { a href=(evil) { (evil) } }.into_string();
        assert!(!out.contains("<img"));
        assert!(out.contains("&quot;&gt;&lt;img"));
    }

    #[test]
    fn raw_unchecked_is_not_escaped() {
        let out = html! { (raw_unchecked("<b>ok</b>")) }.into_string();
        assert_eq!(out, "<b>ok</b>");
    }
}
