//! Core of the Ironhold web framework: the [`App`] builder, [`Config`],
//! the [`Error`] type, and serving with graceful shutdown.

#![forbid(unsafe_code)]

mod app;
mod config;
mod error;

pub use app::App;
pub use config::{Config, ConfigError, Environment};
pub use error::{Error, Result};

/// Routing, responses and HTTP types, re-exported from axum.
pub use axum::{Router, body, http, response, routing};

/// Request extractors.
///
/// axum's `Form` is deliberately not re-exported because it doesn't check
/// CSRF tokens. Use `ironhold::Form` instead.
pub mod extract {
    pub use axum::extract::{
        ConnectInfo, DefaultBodyLimit, Extension, FromRef, FromRequest, FromRequestParts, Json,
        MatchedPath, OriginalUri, Path, Query, RawQuery, Request, State, rejection,
    };
}
