# Contributing to Ironhold

Thanks for helping. This guide covers how to get set up and what we expect from a pull request.

## Setup

1. Install Rust with [rustup](https://rustup.rs).
2. Clone the repository and run the checks:

```bash
cargo test --workspace
```

## Before opening a pull request

Run the same checks as CI. `--all-features` includes the SQLite and Postgres backends; Postgres tests run only when `IRONHOLD_TEST_POSTGRES_URL` points at a test database.

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```bash
cargo test --workspace --all-features
```

## Ground rules

- A change must not make apps less secure unless the developer explicitly opts out, and opt-outs need names that stand out in review, like `raw_unchecked`.
- No `unsafe` code. Every crate has `#![forbid(unsafe_code)]`.
- Framework code returns errors instead of calling `unwrap`, `expect` or `panic`. Clippy enforces this.
- Changes to headers, escaping, limits, sessions or auth need a test. The end-to-end tests live in `crates/ironhold/tests/`.
- Every public item needs a doc comment, ideally with an example.
- Open an issue before starting a large feature so we can agree on the design. [ARCHITECTURE.md](ARCHITECTURE.md) describes the current one.

## Code of conduct

Be kind and assume good intent. Harassment, personal attacks and discriminatory language aren't tolerated in issues, pull requests or any other project space. Maintainers may remove content or contributors that break this rule.

## License

By contributing, you agree that your contributions are dual licensed under MIT and Apache-2.0, as described in the [README](README.md#license).
