//! Secure-by-default building blocks for Ironhold.
//!
//! - [`harden`] wraps a router with security headers, a per-request CSP nonce,
//!   cross-origin request protection, request body limits and timeouts, and
//!   overload protection.
//! - [`CspNonce`] is an extractor that gives handlers the nonce for inline
//!   `<script>`/`<style>` tags allowed by the Content-Security-Policy.
//! - [`ClientIp`] is the client's address, taking trusted reverse proxies
//!   into account.
//! - [`Secret`] wraps sensitive values so they can't be logged or serialized
//!   by accident.

#![forbid(unsafe_code)]

mod client_ip;
mod config;
mod cross_origin;
mod headers;
mod nonce;
mod secret;
mod upload;

pub use client_ip::{ClientIp, InvalidIpRange, IpRange};
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
use tower_http::timeout::TimeoutLayer;

/// Applies Ironhold's security layers to `router`.
///
/// Layers, from outermost to innermost:
/// 1. Security headers and CSP nonce (outermost, so even 403/408/413/429/503
///    error responses carry the headers)
/// 2. Working out the [`ClientIp`], taking trusted proxies into account
/// 3. Cross-origin protection: blocks state-changing requests from other
///    sites before the body is read
/// 4. Reading the request body, with a size limit, its own deadline and a
///    cap on uploads per client. This happens before overload protection,
///    so slow uploads can't take up the slots other visitors need
/// 5. Overload protection: a `503` as soon as `max_concurrent_requests` are
///    in flight
/// 6. Request timeout
pub fn harden<S>(router: Router<S>, config: &SecurityConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let policy = Arc::new(headers::HeaderPolicy::from(config));
    let trusted_origins = Arc::new(cross_origin::TrustedOrigins(config.trusted_origins.clone()));
    let trusted_proxies: Arc<[IpRange]> = config.trusted_proxies.clone().into();
    let uploads = Arc::new(upload::UploadLimits::from(config));

    router
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ))
        .layer(DefaultBodyLimit::max(config.max_body_bytes))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(overloaded))
                .layer(LoadShedLayer::new())
                .layer(GlobalConcurrencyLimitLayer::new(
                    config.max_concurrent_requests,
                )),
        )
        .layer(middleware::from_fn_with_state(
            uploads,
            upload::read_body_first,
        ))
        .layer(middleware::from_fn_with_state(
            trusted_origins,
            cross_origin::cross_origin_protection,
        ))
        .layer(middleware::from_fn_with_state(
            trusted_proxies,
            client_ip::resolve_client_ip,
        ))
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
