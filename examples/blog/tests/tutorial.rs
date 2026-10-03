//! `docs/tutorial.md` quotes this example's files. This test fails when a
//! quoted snippet no longer matches the code, so the tutorial can't drift
//! out of date.

use std::fs;
use std::path::Path;

#[test]
fn tutorial_snippets_match_the_example() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tutorial =
        fs::read_to_string(root.join("../../docs/tutorial.md")).expect("read docs/tutorial.md");

    let mut checked = 0;
    let mut lines = tutorial.lines();
    while let Some(line) = lines.next() {
        // An opening fence names a language; a closing fence is bare.
        if !line.starts_with("```") || line == "```" {
            continue;
        }
        let block: Vec<&str> = lines.by_ref().take_while(|l| *l != "```").collect();
        let Some(path) = block.first().and_then(|first| quoted_path(first)) else {
            continue;
        };
        let snippet = block[1..].join("\n");
        let source = fs::read_to_string(root.join(path))
            .unwrap_or_else(|e| panic!("docs/tutorial.md quotes {path}, which can't be read: {e}"));
        assert!(
            source.contains(&snippet),
            "docs/tutorial.md quotes {path}, but this no longer matches the file:\n\n{snippet}"
        );
        checked += 1;
    }
    assert!(
        checked >= 8,
        "expected the tutorial to quote the example's files, found {checked} snippets"
    );
}

/// The file a snippet comes from, named on its first line as
/// `// src/posts.rs`, `-- migrations/0002_create_posts.sql` or
/// `/* src/layout.rs */`.
fn quoted_path(line: &str) -> Option<&str> {
    let path = line
        .strip_prefix("// ")
        .or_else(|| line.strip_prefix("-- "))
        .or_else(|| line.strip_prefix("/* ").and_then(|l| l.strip_suffix(" */")))?;
    let looks_like_path = path.contains('/') && path.contains('.') && !path.contains(' ');
    looks_like_path.then_some(path)
}
