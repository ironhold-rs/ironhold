//! Secure-by-default building blocks for Ironhold.
//!
//! - [`harden`] wraps a router with security headers, a per-request CSP nonce,
//!   overload protection, cross-origin request protection, request body
//!   limits and timeouts.
//! - [`CspNonce`] is an extractor that gives handlers the nonce for inline
//!   `<script>`/`<style>` tags allowed by the Content-Security-Policy.
//! - [`Secret`] wraps sensitive values so they can't be logged or serialized
//!   by accident.

#![forbid(unsafe_code)]

mod config;
mod cross_origin;
mod headers;
mod nonce;
mod secret;

pub use config::SecurityConfig;
pub use nonce::CspNonce;
pub use secret::Secret;

use std::sync::Arc;

use axum::{
    BoxError, Router,
    error_handling::HandleErrorLayer,
    extract::DefaultBodyLimit,
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
};
use tower::{ServiceBuilder, limit::GlobalConcurrencyLimitLayer, load_shed::LoadShedLayer};
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

/// Applies Ironhold's security layers to `router`.
///
/// Layers, from outermost to innermost:
/// 1. Security headers and CSP nonce (outermost, so even 403/408/413/503
///    error responses carry the headers)
/// 2. Overload protection: a `503` as soon as `max_concurrent_requests` are
///    in flight
/// 3. Cross-origin protection: blocks state-changing requests from other
///    sites before the body is read
/// 4. Request body size limit
/// 5. Request timeout
pub fn harden<S>(router: Router<S>, config: &SecurityConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let policy = Arc::new(headers::HeaderPolicy::from(config));
    let trusted = Arc::new(cross_origin::TrustedOrigins(config.trusted_origins.clone()));

    router
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ))
        .layer(DefaultBodyLimit::max(config.max_body_bytes))
        .layer(RequestBodyLimitLayer::new(config.max_body_bytes))
        .layer(middleware::from_fn_with_state(
            trusted,
            cross_origin::cross_origin_protection,
        ))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(overloaded))
                .layer(LoadShedLayer::new())
                .layer(GlobalConcurrencyLimitLayer::new(
                    config.max_concurrent_requests,
                )),
        )
        .layer(middleware::from_fn_with_state(
            policy,
            headers::security_headers,
        ))
}

/// The response when the server is at `max_concurrent_requests`.
async fn overloaded(_: BoxError) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "1")],
        "Server busy. Please try again in a moment.",
    )
        .into_response()
}
