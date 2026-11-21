//! The real server over TCP: header and idle timeouts, and shutdown.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use ironhold::prelude::*;
use ironhold::{Config, Environment};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};

struct Server {
    addr: std::net::SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<std::io::Result<()>>,
}

async fn start(config: Config) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let app = App::with_config(config)
        .route("/", get(|| async { "ok" }))
        .route("/ip", get(|ip: ClientIp| async move { ip.to_string() }));
    let task = tokio::spawn(app.serve_on(listener, async {
        let _ = stopped.await;
    }));
    Server {
        addr,
        stop: Some(stop),
        task,
    }
}

fn quick_timeouts() -> Config {
    let mut config = Config::new(Environment::Development);
    config.security.header_read_timeout = Duration::from_millis(300);
    config.shutdown_timeout = Duration::from_millis(500);
    config
}

/// Reads until the server closes the connection; `None` if it doesn't
/// within `limit`.
async fn read_until_closed(stream: &mut TcpStream, limit: Duration) -> Option<Vec<u8>> {
    let mut all = Vec::new();
    let mut buf = [0u8; 1024];
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        match tokio::time::timeout(left, stream.read(&mut buf)).await {
            Err(_) => return None,
            Ok(Ok(0)) | Ok(Err(_)) => return Some(all),
            Ok(Ok(n)) => all.extend_from_slice(&buf[..n]),
        }
    }
}

#[tokio::test]
async fn serves_requests_and_knows_the_client_address() {
    let server = start(quick_timeouts()).await;
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream
        .write_all(b"GET /ip HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let response = read_until_closed(&mut stream, Duration::from_secs(5))
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    assert!(text.ends_with("127.0.0.1"), "{text}");
}

#[tokio::test]
async fn slow_headers_are_cut_off() {
    let server = start(quick_timeouts()).await;
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nX-Slow: ")
        .await
        .unwrap();
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = stream.write_all(b"a").await;
    }
    // The headers never finished, so there's no response, just a close.
    let closed = read_until_closed(&mut stream, Duration::from_secs(3)).await;
    assert!(closed.is_some(), "the connection is still open");
    assert!(!String::from_utf8_lossy(&closed.unwrap()).contains("200 OK"));
}

#[tokio::test]
async fn idle_connections_are_closed() {
    let server = start(quick_timeouts()).await;
    let mut silent = TcpStream::connect(server.addr).await.unwrap();
    assert!(
        read_until_closed(&mut silent, Duration::from_secs(3))
            .await
            .is_some(),
        "a connection that never sent anything is still open"
    );

    // Also between keep-alive requests.
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let response = read_until_closed(&mut stream, Duration::from_secs(3)).await;
    assert!(
        response.is_some(),
        "an idle keep-alive connection is still open"
    );
    assert!(String::from_utf8_lossy(&response.unwrap()).starts_with("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn a_half_sent_request_does_not_block_shutdown() {
    let mut config = quick_timeouts();
    // Long enough that only the shutdown deadline can end the connection.
    config.security.header_read_timeout = Duration::from_secs(60);
    let mut server = start(config).await;
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nX-Slow: a")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let started = Instant::now();
    server.stop.take().unwrap().send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), &mut server.task)
        .await
        .expect("the server stopped within the shutdown timeout");
    result.unwrap().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(stream);
}

#[tokio::test]
async fn shutdown_lets_requests_in_progress_finish() {
    let mut config = quick_timeouts();
    config.shutdown_timeout = Duration::from_secs(5);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let app = App::with_config(config).route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_millis(400)).await;
            "done"
        }),
    );
    let server = tokio::spawn(app.serve_on(listener, async {
        let _ = stopped.await;
    }));

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.send(()).unwrap();
    let response = read_until_closed(&mut stream, Duration::from_secs(3))
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&response);
    assert!(
        text.starts_with("HTTP/1.1 200 OK") && text.ends_with("done"),
        "{text}"
    );
    server.await.unwrap().unwrap();
}
