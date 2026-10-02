/// Returns `target` if it's a safe place to redirect to after login,
/// otherwise `fallback`.
///
/// Only paths on this site are allowed. That blocks open redirects such as
/// `?next=https://evil.example`, `//evil.example` or `/\evil.example`, which
/// browsers treat as links to another site.
pub fn safe_redirect_path<'a>(target: Option<&'a str>, fallback: &'a str) -> &'a str {
    match target {
        Some(path) if is_local_path(path) => path,
        _ => fallback,
    }
}

fn is_local_path(path: &str) -> bool {
    let mut chars = path.chars();
    chars.next() == Some('/')
        && !matches!(chars.next(), Some('/' | '\\'))
        // Browsers drop tabs and newlines and treat `\` as `/`, so
        // "/\t/evil.example" becomes "//evil.example". Reject them all.
        && !path
            .chars()
            .any(|c| c == '\\' || c.is_control() || c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::safe_redirect_path;

    #[test]
    fn allows_local_paths() {
        assert_eq!(safe_redirect_path(Some("/dashboard"), "/"), "/dashboard");
        assert_eq!(
            safe_redirect_path(Some("/posts/1?tab=2#x"), "/"),
            "/posts/1?tab=2#x"
        );
    }

    #[test]
    fn blocks_other_sites() {
        for evil in [
            "https://evil.example",
            "//evil.example",
            "/\\evil.example",
            "/\t/evil.example",
            "/\n/evil.example",
            "javascript:alert(1)",
            "evil.example",
            "",
        ] {
            assert_eq!(safe_redirect_path(Some(evil), "/"), "/", "{evil:?}");
        }
        assert_eq!(safe_redirect_path(None, "/home"), "/home");
    }
}
