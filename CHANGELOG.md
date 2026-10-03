# Changelog

Notable changes to Ironhold, newest first. Versions follow [semantic versioning](https://semver.org). Until 1.0, a new minor version (0.x) may include breaking changes; the notes for each release say what changed and how to update.

## 0.1.0 (2026-10-03)

The first release.

### Security by default

- Content-Security-Policy with a new nonce for every request, plus `X-Frame-Options`, `nosniff`, `Referrer-Policy`, `Permissions-Policy`, COOP and CORP headers, and HSTS in production.
- Cross-origin POST, PUT, PATCH and DELETE requests are rejected, and `Form<T>` only accepts submissions carrying the session's CSRF token.
- Request body size limit, request timeout, and overload protection that answers `503` instead of slowing down for everyone.
- Errors never leak internal details to visitors, and a panic in a handler becomes a `500` response.
- `Secret<T>` for values that must never be logged or serialized.

### Features

- `App` builder on axum with configuration from environment variables and graceful shutdown.
- Server-side sessions with hardened cookies, stored in memory or in the database.
- SQLite and Postgres support on sqlx 0.9, with tuned connection pools and fail-fast timeouts.
- Authentication: Argon2id password hashing, login and logout that rotate the session id and CSRF token, `AuthUser` and `Authorize` extractors, `Permission` checks, login throttling and safe redirects.
- Validation with rules written in plain Rust, `Valid<T>` and `Invalid<T>`, field-level errors with ARIA attributes, and `422` JSON responses for APIs.
- Auto-escaping HTML templates with maud.
- `TestClient`, which drives an app in tests like a browser.
- The `ironhold` command: `ironhold new` creates an app with sign up, log in and log out, and `ironhold dev` rebuilds and restarts it on every change.
- A tutorial that builds a blog, and the finished blog example.

### Known limitations

- The `html!` macro needs `maud` in your app's `Cargo.toml`.
- The login throttle is kept in memory, per server instance.
- Apps created by `ironhold new` use SQL checked at runtime, not at compile time.
