# Blog example

The blog built step by step in the [tutorial](../../docs/tutorial.md): sign up, log in, and write, edit and delete your own posts. It uses SQLite, validation, permissions and the test client.

Run it from the repository root:

```bash
cargo run -p blog
```

Then open <http://127.0.0.1:3000>. The database is created in `blog.db`.

Run its tests:

```bash
cargo test -p blog
```

`tests/tutorial.rs` checks that every snippet the tutorial quotes still matches this code.
