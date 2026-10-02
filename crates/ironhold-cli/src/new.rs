//! `ironhold new`: create an app from the built-in template.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{Database, Result};

/// A file in the generated app, with its template for each database.
struct File {
    path: &'static str,
    sqlite: &'static str,
    postgres: &'static str,
}

macro_rules! template {
    ($file:literal) => {
        include_str!(concat!("../templates/app/", $file))
    };
}

/// A file whose template is the same for every database.
macro_rules! same {
    ($path:literal, $file:literal) => {
        File {
            path: $path,
            sqlite: template!($file),
            postgres: template!($file),
        }
    };
}

const FILES: &[File] = &[
    same!("Cargo.toml", "Cargo.toml.tmpl"),
    same!("README.md", "README.md.tmpl"),
    same!(".gitignore", "gitignore.tmpl"),
    File {
        path: ".env.example",
        sqlite: template!("env.example.sqlite.tmpl"),
        postgres: template!("env.example.postgres.tmpl"),
    },
    same!("src/main.rs", "src/main.rs.tmpl"),
    same!("src/lib.rs", "src/lib.rs.tmpl"),
    same!("src/auth.rs", "src/auth.rs.tmpl"),
    same!("src/layout.rs", "src/layout.rs.tmpl"),
    same!("src/pages.rs", "src/pages.rs.tmpl"),
    same!("src/user.rs", "src/user.rs.tmpl"),
    File {
        path: "migrations/0001_create_users.sql",
        sqlite: template!("migrations/0001_create_users.sqlite.sql.tmpl"),
        postgres: template!("migrations/0001_create_users.postgres.sql.tmpl"),
    },
    same!("tests/auth.rs", "tests/auth.rs.tmpl"),
    File {
        path: "tests/common/mod.rs",
        sqlite: template!("tests/common/mod.sqlite.rs.tmpl"),
        postgres: template!("tests/common/mod.postgres.rs.tmpl"),
    },
];

pub(crate) fn run(name: &str, db: Database, ironhold_path: Option<&Path>) -> Result<()> {
    check_name(name)?;
    let root = PathBuf::from(name);
    if root.exists() {
        return Err(format!("`{name}` already exists; choose another name or remove it").into());
    }

    let source = match ironhold_path {
        Some(path) => IronholdSource::Path(find_ironhold_crate(path)?),
        None => IronholdSource::CratesIo,
    };
    let vars = variables(name, db, &source);

    for file in FILES {
        let template = match db {
            Database::Sqlite => file.sqlite,
            Database::Postgres => file.postgres,
        };
        let path = root.join(file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, render(template, &vars))?;
    }

    println!("Created `{name}` with {}.\n", database_label(db));
    println!("Next steps:\n");
    println!("    cd {name}");
    if db == Database::Postgres {
        println!("    cp .env.example .env    # then set DATABASE_URL");
        println!("    createdb {name}");
    }
    println!("    ironhold dev\n");
    println!("Then open http://127.0.0.1:3000");
    Ok(())
}

enum IronholdSource {
    CratesIo,
    Path(PathBuf),
}

fn variables(name: &str, db: Database, source: &IronholdSource) -> Vec<(&'static str, String)> {
    let crate_ident = name.replace('-', "_");
    let (db_type, feature, sqlx_backend) = match db {
        Database::Sqlite => ("SqliteDb", "sqlite", "sqlite-bundled"),
        Database::Postgres => ("PgDb", "postgres", "postgres"),
    };
    let dependency = |features: &str| match source {
        IronholdSource::CratesIo => format!(
            "{{ version = \"{}\", features = [{features}] }}",
            env!("CARGO_PKG_VERSION")
        ),
        IronholdSource::Path(path) => format!(
            "{{ path = \"{}\", features = [{features}] }}",
            toml_escape(&path.display().to_string())
        ),
    };
    let database_url_line = match db {
        Database::Sqlite => format!(
            "let database_url = std::env::var(\"DATABASE_URL\").unwrap_or_else(|_| \"sqlite://{name}.db\".to_owned());"
        ),
        Database::Postgres => "let database_url = std::env::var(\"DATABASE_URL\")\n        .map_err(|_| \"DATABASE_URL is not set (see .env.example)\")?;".to_owned(),
    };
    let database_notes = match db {
        Database::Sqlite => format!(
            "## Database\n\nSQLite, in `{name}.db` (created on first run). Migrations in `migrations/` are applied on startup. Set `DATABASE_URL` to use a different file."
        ),
        Database::Postgres => format!(
            "## Database\n\nPostgres. Before the first run:\n\n```bash\ncp .env.example .env\ncreatedb {name}\ncreatedb {name}_test\n```\n\nMigrations in `migrations/` are applied on startup. Tests use `TEST_DATABASE_URL` and are skipped when it isn't set."
        ),
    };

    vec![
        ("name", name.to_owned()),
        ("crate_ident", crate_ident),
        ("Db", db_type.to_owned()),
        ("sqlx_backend", sqlx_backend.to_owned()),
        ("ironhold_dep", dependency(&format!("\"{feature}\""))),
        ("ironhold_test_dep", dependency("\"testing\"")),
        ("database_url_line", database_url_line),
        ("database_notes", database_notes),
    ]
}

/// Replaces every `{{key}}` in `template`.
fn render(template: &str, vars: &[(&'static str, String)]) -> String {
    vars.iter().fold(template.to_owned(), |text, (key, value)| {
        text.replace(&format!("{{{{{key}}}}}"), value)
    })
}

/// Accepts the repository root or the `crates/ironhold` folder.
fn find_ironhold_crate(path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    for candidate in [path.join("crates").join("ironhold"), path.clone()] {
        let manifest = fs::read_to_string(candidate.join("Cargo.toml")).unwrap_or_default();
        if manifest.contains("name = \"ironhold\"") {
            return Ok(candidate);
        }
    }
    Err(format!("no Ironhold checkout found at {}", path.display()).into())
}

fn toml_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn database_label(db: Database) -> &'static str {
    match db {
        Database::Sqlite => "SQLite",
        Database::Postgres => "Postgres",
    }
}

/// Names that would clash with Rust keywords or the app's own dependencies.
const RESERVED: &[&str] = &[
    "abstract",
    "alloc",
    "as",
    "async",
    "await",
    "become",
    "box",
    "break",
    "const",
    "continue",
    "core",
    "crate",
    "do",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "final",
    "fn",
    "for",
    "gen",
    "if",
    "impl",
    "in",
    "ironhold",
    "let",
    "loop",
    "macro",
    "match",
    "maud",
    "mod",
    "move",
    "mut",
    "override",
    "priv",
    "proc_macro",
    "pub",
    "ref",
    "return",
    "self",
    "serde",
    "sqlx",
    "static",
    "std",
    "struct",
    "super",
    "test",
    "tokio",
    "trait",
    "true",
    "try",
    "type",
    "typeof",
    "unsafe",
    "unsized",
    "use",
    "virtual",
    "where",
    "while",
    "yield",
];

fn check_name(name: &str) -> Result<()> {
    let valid_chars = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    let starts_with_letter = name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    if !valid_chars || !starts_with_letter || name.len() > 64 {
        return Err(format!(
            "`{name}` isn't a valid app name: use lowercase letters, digits, `-` and `_`, starting with a letter"
        )
        .into());
    }
    if RESERVED.contains(&name.replace('-', "_").as_str()) {
        return Err(format!("`{name}` is reserved; choose another name").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placeholder_is_filled_for_both_databases() {
        for db in [Database::Sqlite, Database::Postgres] {
            let vars = variables("my-app", db, &IronholdSource::CratesIo);
            for file in FILES {
                let template = match db {
                    Database::Sqlite => file.sqlite,
                    Database::Postgres => file.postgres,
                };
                let rendered = render(template, &vars);
                assert!(
                    !rendered.contains("{{"),
                    "{} ({db:?}) has an unfilled placeholder",
                    file.path
                );
            }
        }
    }

    #[test]
    fn templates_follow_the_writing_style() {
        for file in FILES {
            for template in [file.sqlite, file.postgres] {
                assert!(
                    !template.contains('\u{2014}') && !template.contains('\u{2013}'),
                    "{} contains a dash character",
                    file.path
                );
            }
        }
    }

    #[test]
    fn app_names() {
        for good in ["blog", "my-app", "shop_2"] {
            assert!(check_name(good).is_ok(), "{good}");
        }
        for bad in [
            "", "My App", "2fast", "-app", "app!", "fn", "test", "ironhold", "self",
        ] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn path_dependency_is_escaped() {
        assert_eq!(toml_escape(r#"C:\a "b""#), r#"C:\\a \"b\""#);
    }
}
