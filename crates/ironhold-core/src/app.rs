use std::{any::Any, future::Future, io, pin::Pin};

use axum::{
    Extension, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::MethodRouter,
};
use ironhold_session::{MemoryStore, SessionConfig, SessionStore};
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};

use crate::{Config, Environment, Error};

/// Applies the session middleware. Boxed so `App` isn't generic over the
/// store type.
type SessionLayer = Box<dyn FnOnce(Router, &SessionConfig) -> Router + Send>;

/// A layer added with [`App::extension`], applied when the router is built.
type ExtensionLayer = Box<dyn FnOnce(Router) -> Router + Send>;

/// A task started by [`App::serve`].
type BackgroundTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The Ironhold application builder.
///
/// ```no_run
/// use ironhold_core::{App, routing::get};
///
/// # async fn run() -> std::io::Result<()> {
/// App::new()
///     .route("/", get(|| async { "Hello, Ironhold!" }))
///     .serve()
///     .await
/// # }
/// ```
///
/// Every app gets the security layer ([`ironhold_security::harden`]),
/// sessions, request tracing and a plain-text `404` fallback. There is no
/// way to build an `App` without them.
pub struct App {
    router: Router,
    config: Config,
    sessions: SessionLayer,
    memory_sessions: bool,
    extensions: Vec<ExtensionLayer>,
    background: Vec<BackgroundTask>,
}

impl App {
    /// Creates an app configured from environment variables
    /// (see [`Config::from_env`]).
    ///
    /// # Panics
    ///
    /// If the environment contains an invalid value. Refusing to start is
    /// safer than running with a configuration nobody asked for. Use
    /// [`App::with_config`] to handle the error yourself.
    #[expect(
        clippy::panic,
        reason = "startup only: refusing to run with invalid configuration is the safe choice"
    )]
    pub fn new() -> Self {
        let config = Config::from_env().unwrap_or_else(|error| panic!("{error}"));
        Self::with_config(config)
    }

    /// Creates an app with an explicit configuration.
    pub fn with_config(config: Config) -> Self {
        Self {
            router: Router::new(),
            config,
            sessions: session_layer(MemoryStore::default()),
            memory_sessions: true,
            extensions: Vec::new(),
            background: Vec::new(),
        }
    }

    /// Makes `value` available to every handler through its extractor (for
    /// example a database pool) or `Extension<T>`.
    pub fn extension<T>(mut self, value: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        self.extensions
            .push(Box::new(move |router| router.layer(Extension(value))));
        self
    }

    /// Runs `task` in the background once [`serve`](App::serve) has bound
    /// its address, for example cleaning up expired sessions. Tasks don't
    /// run when the app is only turned into a router for tests.
    pub fn spawn_on_serve<F>(mut self, task: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.background.push(Box::pin(task));
        self
    }

    /// Stores sessions in `store` instead of memory.
    ///
    /// The default in-memory store loses sessions on restart and isn't
    /// shared between server instances, so production apps should use a
    /// database-backed store.
    pub fn session_store<Store>(mut self, store: Store) -> Self
    where
        Store: SessionStore + Clone,
    {
        self.sessions = session_layer(store);
        self.memory_sessions = false;
        self
    }

    /// The app's configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Adds a route. Paths use `{param}` for captures, e.g. `/users/{id}`.
    pub fn route(mut self, path: &str, method_router: MethodRouter) -> Self {
        self.router = self.router.route(path, method_router);
        self
    }

    /// Merges a group of routes built elsewhere.
    pub fn merge(mut self, router: Router) -> Self {
        self.router = self.router.merge(router);
        self
    }

    /// Builds the final router with all Ironhold layers applied.
    ///
    /// [`serve`](App::serve) calls this for you. Use it directly in tests,
    /// e.g. with `tower::ServiceExt::oneshot`.
    pub fn into_router(self) -> Router {
        let mut router = self.router.fallback(|| async { Error::NotFound });
        for extension in self.extensions {
            router = extension(router);
        }
        let router = (self.sessions)(router, &self.config.session)
            .layer(CatchPanicLayer::custom(panic_response))
            .layer(TraceLayer::new_for_http());
        ironhold_security::harden(router, &self.config.security)
    }

    /// Binds to the configured address and serves until Ctrl+C or SIGTERM,
    /// letting in-flight requests finish before exiting.
    pub async fn serve(mut self) -> io::Result<()> {
        init_tracing();

        let addr = self.config.addr;
        let environment = self.config.environment;
        if environment == Environment::Production && self.memory_sessions {
            tracing::warn!(
                "sessions are stored in memory: they are lost on restart and not shared \
                 between instances. Set a persistent store with App::session_store"
            );
        }
        let background = std::mem::take(&mut self.background);
        let router = self.into_router();

        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!(
            ?environment,
            "Ironhold listening on http://{}",
            listener.local_addr()?
        );
        for task in background {
            tokio::spawn(task);
        }

        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// A panic in a handler is a bug, but it shouldn't drop the connection or
/// leak details. Log it and answer like any other internal error.
fn panic_response(panic: Box<dyn Any + Send + 'static>) -> Response {
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic payload");
    tracing::error!(panic = message, "handler panicked");
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
}

fn session_layer<Store>(store: Store) -> SessionLayer
where
    Store: SessionStore + Clone,
{
    Box::new(move |router, config| router.layer(ironhold_session::layer(store, config)))
}

/// Installs a log subscriber unless the app already installed its own.
/// Log levels come from `RUST_LOG`, defaulting to `info`.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to listen for Ctrl+C");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down gracefully");
}
