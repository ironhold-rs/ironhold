//! Reads `.env` files for `ironhold dev`.

/// Parses `KEY=VALUE` lines. Blank lines and `#` comments are skipped, an
/// `export ` prefix is allowed, and matching quotes around a value are
/// removed.
pub(crate) fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_owned(), unquote(value.trim()).to_owned()))
        })
        .collect()
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_common_forms() {
        let vars = parse(
            "# comment\n\nDATABASE_URL=sqlite://app.db\nexport RUST_LOG = debug\nNAME=\"My App\"\nQUOTE='x=y'\nbroken line\n=nokey\n",
        );
        assert_eq!(
            vars,
            vec![
                ("DATABASE_URL".into(), "sqlite://app.db".into()),
                ("RUST_LOG".into(), "debug".into()),
                ("NAME".into(), "My App".into()),
                ("QUOTE".into(), "x=y".into()),
            ]
        );
    }
}
