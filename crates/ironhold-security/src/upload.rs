//! Reads request bodies before a request takes an in-flight slot.
//!
//! A request holds one of the `max_concurrent_requests` slots from the
//! moment it passes overload protection until its handler finishes. If the
//! handler had to wait for the body, a client sending one byte every few
//! seconds would hold a slot for the whole request timeout, and a thousand
//! such clients would make everyone else get `503`. So the body is read
//! here first, outside that cap, with its own deadline and a per-client
//! limit. Handlers then get the complete body straight away.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use axum::{
    body::{Body, Bytes, HttpBody},
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use tokio::time::Instant;

use crate::{ClientIp, SecurityConfig};

/// Limits for reading request bodies, shared by every request.
pub(crate) struct UploadLimits {
    max_bytes: usize,
    timeout: Duration,
    min_rate: u64,
    longest: Duration,
    per_client: usize,
    in_progress: Mutex<HashMap<IpAddr, usize>>,
}

impl From<&SecurityConfig> for UploadLimits {
    fn from(config: &SecurityConfig) -> Self {
        Self {
            max_bytes: config.max_body_bytes,
            timeout: config.body_timeout,
            min_rate: config.min_body_rate,
            longest: config.request_timeout.max(config.body_timeout),
            per_client: config.max_uploads_per_client,
            in_progress: Mutex::new(HashMap::new()),
        }
    }
}

impl UploadLimits {
    /// How long a body that has sent `received` bytes so far may take in
    /// total: `body_timeout`, or longer while the average rate stays at
    /// `min_body_rate`, but never longer than the request timeout.
    fn allowed_time(&self, received: usize) -> Duration {
        let at_min_rate = match self.min_rate {
            0 => Duration::ZERO,
            rate => Duration::from_millis((received as u64).saturating_mul(1000) / rate),
        };
        self.timeout.max(at_min_rate).min(self.longest)
    }

    /// Counts an upload against `client`, or `None` if it already has
    /// `max_uploads_per_client` in progress.
    fn claim(self: &Arc<Self>, client: IpAddr) -> Option<UploadSlot> {
        let mut in_progress = self.lock();
        let count = in_progress.entry(client).or_insert(0);
        if *count >= self.per_client {
            if *count == 0 {
                in_progress.remove(&client);
            }
            return None;
        }
        *count += 1;
        Some(UploadSlot {
            limits: Arc::clone(self),
            client,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, usize>> {
        // A panic can't leave a counter half-updated, so keep going.
        self.in_progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// One upload in progress for a client. Dropping it frees the slot, also
/// when the client disconnects mid-upload.
struct UploadSlot {
    limits: Arc<UploadLimits>,
    client: IpAddr,
}

impl Drop for UploadSlot {
    fn drop(&mut self) {
        let mut in_progress = self.limits.lock();
        if let Some(count) = in_progress.get_mut(&self.client) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                in_progress.remove(&self.client);
            }
        }
    }
}

enum Rejection {
    TooLarge,
    TooSlow,
    TooManyUploads,
    Unreadable,
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        match self {
            Self::TooLarge => {
                (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response()
            }
            Self::TooSlow => (
                StatusCode::REQUEST_TIMEOUT,
                "Request body took too long to arrive",
            )
                .into_response(),
            Self::TooManyUploads => (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "1")],
                "Too many uploads at once. Please try again in a moment.",
            )
                .into_response(),
            Self::Unreadable => (StatusCode::BAD_REQUEST, "Bad Request").into_response(),
        }
    }
}

/// Reads the whole body (up to `max_body_bytes`) before passing the
/// request on. Requests without a body go straight through.
pub(crate) async fn read_body_first(
    State(limits): State<Arc<UploadLimits>>,
    request: Request,
    next: Next,
) -> Response {
    if request.body().is_end_stream() {
        return next.run(request).await;
    }
    if content_length(request.headers()).is_some_and(|len| len > limits.max_bytes as u64) {
        return Rejection::TooLarge.into_response();
    }
    let slot = match request.extensions().get::<ClientIp>() {
        Some(client) => match limits.claim(client.limit_key()) {
            Some(slot) => Some(slot),
            None => return Rejection::TooManyUploads.into_response(),
        },
        None => None,
    };

    let (parts, body) = request.into_parts();
    let capacity = content_length(&parts.headers)
        .and_then(|len| usize::try_from(len).ok())
        .unwrap_or(0)
        .min(limits.max_bytes);
    let bytes = match read(body, &limits, capacity).await {
        Ok(bytes) => bytes,
        Err(rejection) => return rejection.into_response(),
    };
    drop(slot);
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

async fn read(mut body: Body, limits: &UploadLimits, capacity: usize) -> Result<Bytes, Rejection> {
    let start = Instant::now();
    let mut buffer: Vec<u8> = Vec::with_capacity(capacity);
    loop {
        let deadline = start + limits.allowed_time(buffer.len());
        let frame = match tokio::time::timeout_at(deadline, body.frame()).await {
            Err(_) => return Err(Rejection::TooSlow),
            Ok(None) => break,
            Ok(Some(Err(_))) => return Err(Rejection::Unreadable),
            Ok(Some(Ok(frame))) => frame,
        };
        // Trailers are dropped: nothing in Ironhold reads them.
        if let Ok(data) = frame.into_data() {
            if buffer.len().saturating_add(data.len()) > limits.max_bytes {
                return Err(Rejection::TooLarge);
            }
            buffer.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(buffer))
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_timing(timeout_ms: u64, min_rate: u64, longest_ms: u64) -> UploadLimits {
        let mut config = SecurityConfig::development();
        config.body_timeout = Duration::from_millis(timeout_ms);
        config.min_body_rate = min_rate;
        config.request_timeout = Duration::from_millis(longest_ms);
        UploadLimits::from(&config)
    }

    #[test]
    fn deadline_grows_only_at_the_minimum_rate() {
        let limits = with_timing(10_000, 1024, 30_000);
        assert_eq!(limits.allowed_time(0), Duration::from_secs(10));
        assert_eq!(limits.allowed_time(5 * 1024), Duration::from_secs(10));
        assert_eq!(limits.allowed_time(20 * 1024), Duration::from_secs(20));
        // Never past the request timeout.
        assert_eq!(limits.allowed_time(1024 * 1024), Duration::from_secs(30));

        let no_rate = with_timing(10_000, 0, 30_000);
        assert_eq!(no_rate.allowed_time(1024 * 1024), Duration::from_secs(10));
    }

    #[test]
    fn each_client_gets_a_few_upload_slots() {
        let mut config = SecurityConfig::development();
        config.max_uploads_per_client = 2;
        let limits = Arc::new(UploadLimits::from(&config));
        let a: IpAddr = "203.0.113.1".parse().unwrap();
        let b: IpAddr = "203.0.113.2".parse().unwrap();

        let first = limits.claim(a).unwrap();
        let _second = limits.claim(a).unwrap();
        assert!(limits.claim(a).is_none());
        assert!(limits.claim(b).is_some(), "other clients are unaffected");

        drop(first);
        assert!(
            limits.claim(a).is_some(),
            "a finished upload frees its slot"
        );
    }
}
