use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{
            CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY,
            X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
        },
    },
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{CspNonce, SecurityConfig};

const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");
const CROSS_ORIGIN_OPENER_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-opener-policy");
const CROSS_ORIGIN_RESOURCE_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-resource-policy");

/// The header set derived from [`SecurityConfig`] once at startup.
pub(crate) struct HeaderPolicy {
    hsts: bool,
}

impl From<&SecurityConfig> for HeaderPolicy {
    fn from(config: &SecurityConfig) -> Self {
        Self { hsts: config.hsts }
    }
}

impl HeaderPolicy {
    fn csp(&self, nonce: &CspNonce) -> String {
        let mut csp = format!(
            "default-src 'self'; \
             script-src 'self' 'nonce-{nonce}'; \
             style-src 'self' 'nonce-{nonce}'; \
             img-src 'self' data:; \
             object-src 'none'; \
             base-uri 'none'; \
             frame-ancestors 'none'; \
             form-action 'self'"
        );
        if self.hsts {
            csp.push_str("; upgrade-insecure-requests");
        }
        csp
    }
}

/// Generates the request's CSP nonce, runs the handler, then adds security
/// headers. Headers the handler already set are left alone, so a route can
/// deliberately override one.
pub(crate) async fn security_headers(
    State(policy): State<Arc<HeaderPolicy>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Ok(nonce) = CspNonce::generate() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let csp = policy.csp(&nonce);
    request.extensions_mut().insert(nonce);

    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    if let Ok(value) = HeaderValue::from_str(&csp) {
        set_default(headers, CONTENT_SECURITY_POLICY, value);
    }
    set_default(
        headers,
        X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    set_default(headers, X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    set_default(
        headers,
        REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    set_default(
        headers,
        PERMISSIONS_POLICY,
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    set_default(
        headers,
        CROSS_ORIGIN_OPENER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    set_default(
        headers,
        CROSS_ORIGIN_RESOURCE_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    if policy.hsts {
        set_default(
            headers,
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=63072000; includeSubDomains"),
        );
    }

    response
}

fn set_default(headers: &mut HeaderMap, name: HeaderName, value: HeaderValue) {
    headers.entry(name).or_insert(value);
}
