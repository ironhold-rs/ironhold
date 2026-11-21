# Changelog

Notable changes to Ironhold, newest first. Versions follow [semantic versioning](https://semver.org). Until 1.0, a new minor version (0.x) may include breaking changes; the notes for each release say what changed and how to update.

## Unreleased

Fixes from a security review of 0.1.0. Several protections could be defeated by a client sending requests in parallel or very slowly.

### Security

- The login throttle is now atomic. `LoginThrottle::begin` reserves an attempt before the password is checked, and attempts in progress count against the limit, so sending many guesses at once no longer gets them all checked (900 of 900 parallel guesses got through before). A cancelled attempt counts as a failure.
- The login throttle counts failures per account and client address instead of per account alone, so five wrong guesses from someone else can no longer lock a user out. IPv6 addresses are grouped by `/64`. See the README for the trade-off.
- Request bodies are read before a request takes one of the `max_concurrent_requests` slots, with their own deadline (`body_timeout`, 10 s, longer only while the client keeps sending at `min_body_rate`, 1 KiB/s) and a cap of `max_uploads_per_client` (32) uploads in progress per client address. Before, about 1,000 connections trickling a form body could make every other visitor get `503`.
- The server now enforces a header read timeout (`header_read_timeout`, 10 s), which also closes idle keep-alive connections, caps open connections (`max_connections`, 10,000), and stops waiting for open connections after `Config::shutdown_timeout` (10 s) on shutdown. Before, a slow or half-sent request could keep a connection open forever and stop the server from exiting.
- Password hashing keeps its slot until the hash finishes, even when the client hangs up. Before, cancelled logins freed their slot while the hash kept running, which pushed one test server from 7 MB to 2 GB of memory.
- Connections to a remote Postgres host now default to `sslmode=verify-full`, which checks the server's certificate and host name. `require`, used before, accepts any certificate. A URL or `PGSSLMODE` that sets `sslmode` is still respected, with a warning when it doesn't verify the server.
- The default in-memory session store drops expired sessions every 5 minutes and holds at most 100,000 (`DEFAULT_MAX_SESSIONS`), removing the session closest to expiry when full. Before, every visitor without a cookie added a session that was never freed.
- New `ironhold::db::QueryBuilder` builds queries at runtime from SQL literals and bound values only; runtime SQL needs `push_unchecked`. sqlx's own `QueryBuilder` accepts any string, so `clippy.toml` in this repository and in apps made by `ironhold new` forbids it. The README no longer says sqlx alone prevents SQL injection.
- The database session store keeps a SHA-256 hash of each session id instead of the id, so reading the table doesn't give away live sessions.
- `AuthUser` can no longer be built by hand, so a function taking one really needs a logged-in user.
- Argon2id parameters are set explicitly in code instead of taken from the library's defaults.
- Updated dependencies with RustSec advisories (rustls, rustls-webpki, webpki-roots, tokio, ctutils, libsqlite3-sys through sqlx 0.9.1). A weekly workflow now checks advisories.

### Added

- `ClientIp` extractor, and `SecurityConfig::trusted_proxies` with the `IRONHOLD_TRUSTED_PROXIES` variable, so the client address comes from `X-Forwarded-For` behind a reverse proxy. Handlers can also take `ConnectInfo<SocketAddr>` now.
- `App::serve_on` serves on an already bound listener until a given future completes.
- `DbConfig::statement_cache_capacity` (100 by default) is now set on every connection.
- `TestClient::with_client_addr` sends test requests from a given client address.
- The README and tutorial recommend a reverse proxy that buffers request bodies (Nginx does by default, Caddy doesn't).

### Fixed

- CI: a failing test run of the generated Postgres app no longer passes because its output goes through `tee`.

### Breaking changes

- `LoginThrottle::check`, `record_failure` and `record_success` are removed. Use `let attempt = throttle.begin(&email)?;` before checking the password, then `attempt.succeeded()` or `attempt.failed()`. The generated app and the blog example show the change.
- `AuthUser`'s field is private: replace `AuthUser(user)` patterns with `user.into_user()`, or keep using it through `Deref`.
- `Environment` and `Invalid<T>` are `#[non_exhaustive]`: a `match` on `Environment` needs a `_` arm, and `Invalid` can't be built with a struct literal outside Ironhold (its fields can still be read).
- `MemoryStore` (in `ironhold::MemoryStore` and `ironhold_session`) is now Ironhold's own bounded store instead of a re-export of `tower_sessions::MemoryStore`.
- The SQLite and Postgres session stores run a new migration that deletes existing sessions, because they were stored under the plain id. Everyone has to log in once after upgrading.
- Remote Postgres URLs without `sslmode` now need a server certificate that verifies. For a private certificate authority add `sslrootcert=/path/to/ca.pem`, or opt out explicitly with `sslmode=require`.
- Request bodies are read in full before the handler runs, so handlers can no longer stream a body as it arrives. `max_body_bytes` still caps the size.
- `SecurityConfig::request_timeout` now covers the handler only; reading the body has its own `body_timeout`.
- `App::serve` no longer uses `axum::serve`, and new `SecurityConfig` and `Config` fields set its limits. The timeouts above may cut off clients that were slower than them before.
- `LoginThrottle` limits now apply per account and client address. Requests without a client address (a router called directly, as in tests) are counted per account, as before.

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
