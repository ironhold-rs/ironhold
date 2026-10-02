# Ironhold architecture

Apps depend on the `ironhold` crate, which re-exports the `ironhold-*` crates described here.

## Goals

Ironhold is a full-stack Rust web framework. It aims to be fast, stable, reliable and secure by default, without being harder to use than Laravel or Rails. Speed, stability and reliability come before new features.

- Pages are rendered on the server and work without JavaScript.
- Interactive pages will come from live components rendered on the server, similar to Phoenix LiveView and Livewire. WebAssembly islands and an Inertia adapter (for React or Vue) are planned for apps that need more client-side code.
- An app should deploy as one binary with no other services: SQLite plus database-backed jobs and cache by default, Postgres when an app outgrows one server.
- Common web vulnerabilities (XSS, CSRF, SQL injection, broken access control, leaked secrets) should be prevented by types or by defaults, not by the developer remembering a checklist.

## Related projects

| Project | Focus |
|---|---|
| axum, Actix Web | HTTP layer. Ironhold is built on axum. |
| Loco | Rails-style framework on axum with SeaORM, jobs and mailers |
| Leptos, Dioxus | Rust UI frameworks with server rendering and WebAssembly |

Ironhold's focus is security enforced by types, compile-time checks wherever possible, and single-binary deploys.

## Design principles

1. Secure by default. Anything unsafe needs an explicit opt-out with a name that stands out in review, such as `raw_unchecked`.
2. Make wrong code fail to compile. Types guard escaping, CSRF, login, permissions and secrets.
3. Build on proven crates (`tokio`, `hyper`, `tower`, `axum`, `sqlx`) instead of rewriting them.
4. No `unsafe` code: every framework crate has `#![forbid(unsafe_code)]`.
5. No crash points. CI rejects `unwrap`, `expect` and `panic` in framework code, and a panic in a handler becomes a `500` response instead of a dropped connection.
6. Typed input only. Every request body is parsed into a concrete Rust type with a size limit. Nothing rebuilds arbitrary objects from client data.
7. Fast where it matters. The database is usually the bottleneck, so connection pools, timeouts and SQLite settings are tuned out of the box.
8. Little or no JavaScript by default.
9. Clear errors, a CLI, generators and docs, so people coming from Laravel, Rails or Next.js can be productive quickly.

## Crates

```
crates/
  ironhold            the crate apps depend on; re-exports the others
  ironhold-core       App builder, routing, config, errors, graceful shutdown
  ironhold-security   security headers, CSP nonces, cross-origin checks, limits, Secret<T>
  ironhold-session    server-side sessions (tower-sessions), CSRF tokens
  ironhold-html       auto-escaping templates (maud)
  ironhold-forms      CSRF-checked Form<T>
  ironhold-db         SQLite and Postgres on sqlx 0.9, database-backed sessions, Db extractor
  ironhold-auth       Argon2id passwords, login and logout, AuthUser, Authorize, LoginThrottle
  ironhold-cli        the `ironhold` command: `new` and `dev`
examples/
  hello               minimal app
```

Planned: `ironhold-model` (models, typed queries, generated migrations), `ironhold-live` (live components), `ironhold-jobs`, `ironhold-mail`, `ironhold-admin`, `ironhold-islands` and `ironhold-inertia`.

## Request handling

Layers run in this order for every request:

1. Security headers and the request's CSP nonce. This layer is outermost so even error responses carry the headers.
2. Overload protection: past 1024 requests in flight, an immediate `503` with `Retry-After`.
3. Cross-origin check for POST, PUT, PATCH and DELETE.
4. Request body size limit (2 MiB) and request timeout (30 s).
5. Request tracing, and turning handler panics into `500` responses.
6. Session loading.
7. The router and the handler's extractors: `Form<T>` (checks the CSRF token), `CsrfToken`, `Session`, `AuthUser<U>`, `Authorize<U, P>`, `Db`, `Path<T>`, `Query<T>` and so on.

## Security mechanisms

| Threat | Mechanism |
|---|---|
| XSS | Templates escape every interpolated value. Raw HTML needs `raw_unchecked(...)`. |
| Script injection | Content-Security-Policy with a new nonce per request; inline scripts and styles need the nonce. |
| SQL injection | sqlx 0.9 only accepts SQL written as string literals (dynamic SQL needs an explicit `AssertSqlSafe`), and values are passed with `.bind()`. |
| CSRF | Cross-origin POST, PUT, PATCH and DELETE requests are rejected using `Sec-Fetch-Site`, falling back to comparing `Origin` with `Host`. `Form<T>` also requires the session's CSRF token. axum's unchecked `Form` isn't re-exported. |
| Broken access control | A handler that takes `AuthUser<U>` can't run for logged-out visitors, and `Authorize<U, P>` can't run unless policy `P` allows the user. |
| Session theft and fixation | Server-side sessions with random 128-bit IDs. Cookies are `HttpOnly` and `SameSite=Lax`, and `Secure` with the `__Host-` prefix in production. Login issues a new session ID and CSRF token. |
| Password storage | Argon2id with the OWASP-recommended parameters, pinned in code. Plaintext passwords are held in `Secret<String>`, which can't be logged or serialized. |
| Password guessing | 5 failed logins per account per 5 minutes, then `429`. Unknown accounts take as long to check as real ones. |
| Open redirects | `safe_redirect_path` only allows paths on the same site. |
| Clickjacking, MIME sniffing | `frame-ancestors 'none'`, `X-Frame-Options`, `nosniff`, COOP and CORP headers; HSTS in production. |
| Denial of service | Body size limits, request timeouts, a cap on requests in flight, and a database connection timeout. |
| Vulnerable dependencies | CI runs `cargo-deny` for security advisories, licences and sources. |

## Front end

1. Server-rendered HTML. Available now.
2. Live components (phase 2). A component keeps its state on the server; browser events reach it over a WebSocket or HTTP, and the server sends back HTML changes. Events are typed Rust enums with CSRF checks and size limits.
3. Islands and Inertia (phase 4). `#[island]` components compile to WebAssembly for client-heavy widgets such as editors and charts. Teams that prefer React or Vue can use the Inertia adapter and keep Ironhold's routing, auth and validation.

## Roadmap

| Phase | Scope |
|---|---|
| 0: Foundation (done) | Workspace, App builder, security headers, config, CI |
| 1: v0.1 | Done: sessions, cross-origin checks, CSRF-checked forms, overload protection, SQLite and Postgres support, database-backed sessions, auth, test client, `ironhold new`, `ironhold dev`. Remaining: form validation, a `blog` example, documentation |
| 2: v0.2 | Model layer and generators, live components, template hot reload |
| 3: v0.3 | Jobs, scheduler, mail, file storage, cache, real-time updates, admin panel, deployment command |
| 4: v0.4 | WebAssembly islands, Inertia adapter, OpenAPI for JSON APIs, translations |
| 1.0 | Stable API, external security audit, long-term support policy |

Published benchmarks will run on a small cloud server with the load generator on a separate machine, several runs per result, and the code in this repository.

## Decisions

| Decision | Reason | Revisit |
|---|---|---|
| Build on axum 0.8 and tower-http | Fast, widely used, and its extractor model is easy for app developers | No plans |
| maud for templates | Checked at compile time, escapes by default, components are plain functions | Phase 2, when live components need our own `html!` macro. maud's macro also requires `maud` in the app's Cargo.toml, which our own macro will remove |
| CSP nonces from `getrandom` | 128 bits straight from the operating system | No plans |
| Listen on `127.0.0.1` by default | A development server is never exposed to the network by accident | No plans |
| Invalid configuration stops the app at startup | Safer than silently falling back to a default | No plans |
| tower-sessions for sessions | Maintained, built for axum, server-side so sessions can be revoked, pluggable stores | No plans |
| `SameSite=Lax`, not `Strict` | `Strict` logs people out when they follow a link from an email or another site. `Lax` still blocks cross-site POSTs, and CSRF has two other layers | No plans |
| Session cookie named `id` (`__Host-id` in production) | A generic name doesn't reveal the framework, and `__Host-` stops subdomains from overwriting the cookie | No plans |
| Cross-origin check based on Go 1.25's `CrossOriginProtection` | Stateless, covers every route including JSON, and needs no token | No plans |
| Minimum Rust version 1.94 | Required by sqlx 0.9. Earlier, 1.88 was needed for the patched `time` crate (RUSTSEC-2026-0009). Security fixes come before support for older compilers | When a dependency needs a newer compiler |
| sqlx 0.9 for database access | Async, supports SQLite and Postgres, compile-time checked queries for apps, and only accepts SQL literals. SeaORM 2.0 also builds on sqlx, so the model layer decision stays open | No plans |
| sqlx's `sqlite-bundled` feature instead of `sqlite` | The full feature also enables loading native SQLite extensions at runtime, which we don't need | When an app needs extensions |
| Framework tables tracked in `ironhold_migrations` | Kept apart from sqlx's `_sqlx_migrations`, so framework upgrades never clash with app migrations. On Postgres they run under an advisory lock so several instances can start at once | No plans |
| Remote Postgres requires TLS unless the URL sets `sslmode` | sqlx's default (`prefer`) falls back to plain text, which a network attacker can force | No plans |
| Cap of 1024 requests in flight, then `503` | When a server is overloaded, latency climbs for everyone. Rejecting extra work early keeps accepted requests fast | After the first published benchmark |
| Login checks in extractors, not middleware | A route can't skip the check through a middleware bypass (the kind of bug behind CVE-2025-29927). A handler without `AuthUser` is visibly public in code review | No plans |
| Argon2id parameters pinned in code | A library update can't change hashing silently. Verification reads parameters from each stored hash, so raising them later is safe | No plans |
| Login throttle kept in memory, per account | No setup, and it stops guessing against one account. It doesn't share counts between instances or limit per IP address yet | When the database-backed cache lands (phase 3) |
| Minimum password length 12, no composition rules | Length matters more than character classes, and composition rules push people towards predictable patterns | No plans |
| Generated apps use runtime-checked SQL | `query!` needs a database at compile time, which would break `ironhold new` followed by `ironhold dev` on a fresh machine. Values are still always bound | Phase 2, with the model layer |
| `ironhold dev` rebuilds, then restarts | If a build fails, the last working version keeps running, so the browser never hits a dead server mid-edit. Hot-patching is a later step | Phase 2 |
| `Error` converts from any error with `?` | Handlers stay short, and anything unexpected becomes a logged `500`. `Error` doesn't implement `std::error::Error`, the same approach as `anyhow` | No plans |
| Live components before WebAssembly islands | Server-driven pages need no Rust in the browser and ship far less code. Phoenix, which works this way, has been the most admired web framework in the Stack Overflow survey since 2023 | No plans |
| SQLite is the default for new apps; Postgres is fully supported | No database server to run, and queries skip a network round trip. Rails 8 and Laravel 11 made the same choice. Framework tables work on both, and CI tests both | No plans |
| Model layer chosen by prototype | Models shape day-to-day work. SeaORM 2.0 and our own derive on sqlx will be compared by lines of code, compile time and error messages | Start of phase 2 |

## Project policies

- Licensed under MIT or Apache-2.0, at your option.
- `CONTRIBUTING.md` covers setup, checks and the code of conduct; `SECURITY.md` covers private vulnerability reporting.
- The minimum supported Rust version is 1.94 and is tested in CI. Raising it is a minor-version change.
- Every public item is documented (CI fails otherwise), and examples in docs are compiled as tests where they can run on their own.

## Notes for contributors

Framework-level Rust (generic extractors, traits, procedural macros) is harder than application Rust. To keep the code approachable, phases 0 and 1 use axum's extractor model and no procedural macros. Procedural macros (`#[island]`, derives) come in phase 2, once the core APIs are stable.
