use std::{borrow::Cow, future::Future, marker::PhantomData, ops::Deref};

use axum::{
    extract::{FromRequestParts, OptionalFromRequestParts},
    http::{Method, header, request::Parts},
    response::{IntoResponse, Redirect, Response},
};
use ironhold_core::Error;
use ironhold_session::Session;
use serde::{Serialize, de::DeserializeOwned};

use crate::session::USER_KEY;

/// Where [`AuthUser`] sends logged-out visitors. Register a custom one with
/// `App::extension(AuthConfig::new("/sign-in"))`; the default is `/login`.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    login_path: Cow<'static, str>,
}

impl AuthConfig {
    /// Sends logged-out visitors to `login_path`.
    pub fn new(login_path: impl Into<Cow<'static, str>>) -> Self {
        Self {
            login_path: login_path.into(),
        }
    }

    /// The login page path.
    pub fn login_path(&self) -> &str {
        &self.login_path
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self::new("/login")
    }
}

/// How [`AuthUser`] turns the id stored in the session into your user type.
///
/// ```ignore
/// #[derive(Clone)]
/// struct User { id: i64, email: String }
///
/// impl LoadUser for User {
///     type Id = i64;
///
///     async fn load_user(id: i64, parts: &mut Parts) -> Result<Option<Self>> {
///         let db = SqliteDb::from_parts(parts)?;
///         let row: Option<(i64, String)> =
///             sqlx::query_as("SELECT id, email FROM users WHERE id = $1")
///                 .bind(id)
///                 .fetch_optional(&*db)
///                 .await?;
///         Ok(row.map(|(id, email)| User { id, email }))
///     }
/// }
/// ```
pub trait LoadUser: Clone + Send + Sync + 'static {
    /// The id stored in the session, usually `i64`.
    type Id: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Loads the user, or returns `None` if they no longer exist (they are
    /// then logged out). `parts` gives access to request extensions such as
    /// the database.
    fn load_user(
        id: Self::Id,
        parts: &mut Parts,
    ) -> impl Future<Output = Result<Option<Self>, Error>> + Send;
}

/// The logged-in user. A handler that takes `AuthUser<U>` only runs for
/// logged-in users:
///
/// - browsers asking for a page are redirected to the login page, with
///   `?next=` set so they come back afterwards
/// - other requests (forms, `fetch`, APIs) get `401 Unauthorized`
///
/// Take `Option<AuthUser<U>>` for pages that work either way.
#[derive(Debug, Clone)]
pub struct AuthUser<U>(pub U);

impl<U> Deref for AuthUser<U> {
    type Target = U;

    fn deref(&self) -> &U {
        &self.0
    }
}

impl<U: LoadUser> AuthUser<U> {
    /// `Ok(None)` when nobody is logged in.
    async fn load(parts: &mut Parts) -> Result<Option<Self>, Error> {
        let session = Session::from_request_parts(parts, &())
            .await
            .map_err(|(_, message)| Error::internal(message))?;
        let Some(id) = session.get::<U::Id>(USER_KEY).await? else {
            return Ok(None);
        };
        match U::load_user(id, parts).await? {
            Some(user) => Ok(Some(Self(user))),
            None => {
                // The account is gone (deleted or disabled): end the session.
                session.flush().await?;
                Ok(None)
            }
        }
    }
}

impl<U: LoadUser, S: Send + Sync> FromRequestParts<S> for AuthUser<U> {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match Self::load(parts).await {
            Ok(Some(user)) => Ok(user),
            Ok(None) => Err(not_logged_in(parts)),
            Err(error) => Err(error.into_response()),
        }
    }
}

impl<U: LoadUser, S: Send + Sync> OptionalFromRequestParts<S> for AuthUser<U> {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Self::load(parts).await.map_err(IntoResponse::into_response)
    }
}

/// A rule about which users may do something, checked before the handler
/// runs. See [`Authorize`].
pub trait Policy<U>: Send + Sync + 'static {
    /// Whether `user` passes this policy.
    fn allows(user: &U) -> bool;
}

/// The logged-in user, if policy `P` allows them. Otherwise logged-out
/// visitors are treated as for [`AuthUser`] and everyone else gets
/// `403 Forbidden`.
///
/// ```ignore
/// struct Admins;
/// impl Policy<User> for Admins {
///     fn allows(user: &User) -> bool { user.is_admin }
/// }
///
/// async fn admin_page(admin: Authorize<User, Admins>) -> Markup { ... }
/// ```
pub struct Authorize<U, P> {
    user: U,
    _policy: PhantomData<fn() -> P>,
}

impl<U, P> Authorize<U, P> {
    /// The authorized user.
    pub fn into_user(self) -> U {
        self.user
    }
}

impl<U, P> Deref for Authorize<U, P> {
    type Target = U;

    fn deref(&self) -> &U {
        &self.user
    }
}

impl<U, P, S> FromRequestParts<S> for Authorize<U, P>
where
    U: LoadUser,
    P: Policy<U>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthUser(user) =
            <AuthUser<U> as FromRequestParts<S>>::from_request_parts(parts, state).await?;
        if P::allows(&user) {
            Ok(Self {
                user,
                _policy: PhantomData,
            })
        } else {
            Err(Error::Forbidden.into_response())
        }
    }
}

/// A rule about what a user may do with a specific resource, such as
/// editing a post. Checked inside the handler once the resource is loaded:
///
/// ```ignore
/// struct EditPost;
/// impl Permission<User, Post> for EditPost {
///     fn allows(user: &User, post: &Post) -> bool { post.author_id == user.id }
/// }
///
/// EditPost::check(&user, &post)?; // 403 unless allowed
/// ```
pub trait Permission<U, R> {
    /// Whether `user` may act on `resource`.
    fn allows(user: &U, resource: &R) -> bool;

    /// `Err(Error::Forbidden)` unless [`allows`](Self::allows).
    fn check(user: &U, resource: &R) -> Result<(), Error> {
        if Self::allows(user, resource) {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}

/// Redirect page loads to the login page; answer everything else with 401.
fn not_logged_in(parts: &Parts) -> Response {
    let wants_page = parts.method == Method::GET
        && parts
            .headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|accept| accept.contains("text/html"));
    if !wants_page {
        return Error::Unauthorized.into_response();
    }

    let login_path = parts
        .extensions
        .get::<AuthConfig>()
        .map_or("/login", AuthConfig::login_path);
    let here = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    match serde_urlencoded::to_string([("next", here)]) {
        Ok(query) => Redirect::to(&format!("{login_path}?{query}")).into_response(),
        Err(_) => Redirect::to(login_path).into_response(),
    }
}
