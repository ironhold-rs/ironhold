//! Accepting connections and serving HTTP/1.1 on them, with the connection
//! limits from [`SecurityConfig`] and a deadline for graceful shutdown.
//!
//! This replaces `axum::serve`, which runs hyper without a timer. Without a
//! timer hyper can't enforce its header read timeout, so a client sending
//! headers one byte at a time (or nothing at all) could keep a connection
//! open forever and stop graceful shutdown from finishing.

use std::{
    convert::Infallible, future::Future, io, net::SocketAddr, pin::pin, sync::Arc, time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, Response},
};
use hyper::{body::Incoming, server::conn::http1};
use hyper_util::rt::{TokioIo, TokioTimer};
use ironhold_security::SecurityConfig;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    task::JoinSet,
};
use tower::ServiceExt;

/// What [`run`] needs from the app's configuration.
pub(crate) struct Limits {
    pub(crate) header_read_timeout: Duration,
    pub(crate) max_connections: usize,
    pub(crate) shutdown_timeout: Duration,
}

impl Limits {
    pub(crate) fn new(security: &SecurityConfig, shutdown_timeout: Duration) -> Self {
        Self {
            header_read_timeout: security.header_read_timeout,
            max_connections: security.max_connections,
            shutdown_timeout,
        }
    }
}

/// Serves `router` on `listener` until `shutdown` completes, then lets
/// requests in progress finish for up to `shutdown_timeout`.
pub(crate) async fn run<F>(listener: TcpListener, router: Router, limits: Limits, shutdown: F)
where
    F: Future<Output = ()>,
{
    let slots = Arc::new(Semaphore::new(limits.max_connections.max(1)));
    let (closing, close) = watch::channel(());
    let mut connections = JoinSet::new();
    let mut shutdown = pin!(shutdown);

    loop {
        // At the connection limit, stop accepting: new connections wait in
        // the operating system's queue instead of using more memory here.
        let slot = tokio::select! {
            slot = Arc::clone(&slots).acquire_owned() => match slot {
                Ok(slot) => slot,
                Err(_) => break,
            },
            () = &mut shutdown => break,
        };
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    handle_accept_error(error).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        // Forget connections that have finished, so the set stays small.
        while connections.try_join_next().is_some() {}
        connections.spawn(serve_connection(
            stream,
            peer,
            router.clone(),
            limits.header_read_timeout,
            close.clone(),
            slot,
        ));
    }

    drop(listener);
    tracing::info!("shutting down gracefully");
    // Idle connections close now; the others after their current request.
    let _ = closing.send(());
    let finished = tokio::time::timeout(limits.shutdown_timeout, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if finished.is_err() {
        tracing::warn!(
            connections = connections.len(),
            "closing connections still open after the shutdown timeout"
        );
        connections.shutdown().await;
    }
}

async fn serve_connection(
    stream: TcpStream,
    peer: SocketAddr,
    router: Router,
    header_read_timeout: Duration,
    mut close: watch::Receiver<()>,
    _slot: OwnedSemaphorePermit,
) {
    let _ = stream.set_nodelay(true);
    let service = hyper::service::service_fn(move |mut request: Request<Incoming>| {
        // Lets the security layer work out the `ClientIp`, and handlers
        // take `ConnectInfo<SocketAddr>`.
        request.extensions_mut().insert(ConnectInfo(peer));
        let router = router.clone();
        async move {
            let response: Result<Response<Body>, Infallible> =
                router.oneshot(request.map(Body::new)).await;
            response
        }
    });

    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        // Covers slow headers, and idle time between keep-alive requests.
        .header_read_timeout(header_read_timeout);
    let mut connection = pin!(
        builder
            .serve_connection(TokioIo::new(stream), service)
            .with_upgrades()
    );

    tokio::select! {
        result = connection.as_mut() => {
            if let Err(error) = result {
                tracing::debug!(%error, %peer, "connection ended with an error");
            }
        }
        _ = close.changed() => {
            connection.as_mut().graceful_shutdown();
            if let Err(error) = connection.await {
                tracing::debug!(%error, %peer, "connection ended with an error during shutdown");
            }
        }
    }
}

/// Like hyper and axum: errors about a single connection are ignored, and
/// anything else (usually running out of file descriptors) is logged, then
/// accepting pauses for a second so open connections can finish.
async fn handle_accept_error(error: io::Error) {
    if matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    ) {
        return;
    }
    tracing::error!(%error, "could not accept a connection");
    tokio::time::sleep(Duration::from_secs(1)).await;
}
