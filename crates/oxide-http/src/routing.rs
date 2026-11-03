//! Named routes, reverse URL generation and the [`OxideRouter`] wrapper.
//!
//! Routes are usually declared with the [`routes!`](crate::routes) DSL, which expands into
//! [`OxideRouter`] calls. Every route is recorded in a [`RouteRegistry`] so URLs can be generated
//! from route names:
//!
//! ```ignore
//! let url = route!("posts.show", post = 42); // "/api/v1/posts/42"
//! ```

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::fmt::Write as _;
use std::future::Future;
use std::sync::{Arc, LazyLock, RwLock};

use axum::Extension;
use axum::extract::{FromRequestParts, Request};
use axum::handler::Handler;
use axum::http::Method;
use axum::http::request::Parts;
use axum::response::IntoResponse;
use axum::routing::{MethodFilter, Route, on};
use tower::{Layer, Service};

use crate::error::OxideHttpError;

/// One registered route: its HTTP method, full path pattern and (optional) name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMetadata {
    /// The route name, e.g. `"posts.show"`.
    pub name: Option<String>,
    /// The HTTP method.
    pub method: Method,
    /// The full Axum path pattern, e.g. `"/api/v1/posts/{post}"`.
    pub path: String,
    /// The path parameters in order of appearance, e.g. `["post"]`.
    pub params: Vec<String>,
}

impl RouteMetadata {
    fn new(method: Method, path: String, name: Option<String>) -> Self {
        let params = path_params(&path).map(|p| p.name.to_owned()).collect();
        Self {
            name,
            method,
            path,
            params,
        }
    }
}

/// Every route known to a router, with an index of named routes.
#[derive(Debug, Clone, Default)]
pub struct RouteRegistry {
    routes: Vec<RouteMetadata>,
    by_name: HashMap<String, usize>,
}

impl RouteRegistry {
    /// All registered routes, in registration order (named and unnamed).
    pub fn routes(&self) -> &[RouteMetadata] {
        &self.routes
    }

    /// Looks up a named route.
    pub fn get(&self, name: &str) -> Option<&RouteMetadata> {
        self.by_name.get(name).map(|&index| &self.routes[index])
    }

    /// Registers a route.
    ///
    /// # Panics
    /// Panics if `meta` is named and the name is already registered.
    fn push(&mut self, meta: RouteMetadata) {
        if let Some(name) = &meta.name {
            assert!(
                !self.by_name.contains_key(name),
                "route name `{name}` is already registered"
            );
            self.by_name.insert(name.clone(), self.routes.len());
        }
        self.routes.push(meta);
    }

    /// Inserts or replaces a route (used by the process-wide registry, where re-publishing the
    /// same router — e.g. once per test — must not panic).
    fn upsert(&mut self, meta: RouteMetadata) {
        match meta.name.as_ref().and_then(|name| self.by_name.get(name)) {
            Some(&index) => self.routes[index] = meta,
            None => self.push(meta),
        }
    }

    /// Generates a URL for the named route.
    ///
    /// `{param}` placeholders are replaced with percent-encoded values; parameters that don't
    /// appear in the pattern are appended as a query string, like Laravel's `route()` helper.
    pub fn url(&self, name: &str, params: &[(&str, &str)]) -> Result<String, OxideHttpError> {
        let route = self
            .get(name)
            .ok_or_else(|| OxideHttpError::Internal(format!("route `{name}` is not defined")))?;
        build_url(route, params)
    }
}

/// An `axum::Router` that also records route names and patterns for URL generation.
///
/// Build one with [`routes!`](crate::routes) or the builder methods, then pass it to
/// [`axum::serve()`] (via `into()`/[`OxideRouter::into_axum`]) or to
/// [`TestClient::new`](crate::testing::TestClient::new).
pub struct OxideRouter<S = ()> {
    router: axum::Router<S>,
    registry: RouteRegistry,
}

impl<S> fmt::Debug for OxideRouter<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OxideRouter")
            .field("routes", &self.registry.routes)
            .finish_non_exhaustive()
    }
}

impl<S> Default for OxideRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S> OxideRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            router: axum::Router::new(),
            registry: RouteRegistry::default(),
        }
    }

    /// Adds a route for `method` at `path`, optionally naming it.
    ///
    /// Registering several methods on the same path is fine; Axum merges them.
    ///
    /// # Panics
    /// Panics if `name` is already registered, if `method` is not routable (e.g. `CONNECT`), or
    /// if Axum rejects the route (duplicate method on the same path, invalid pattern).
    #[must_use]
    pub fn route_named<H, T>(
        mut self,
        method: Method,
        path: &str,
        handler: H,
        name: Option<&str>,
    ) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        let filter = MethodFilter::try_from(method.clone())
            .unwrap_or_else(|_| panic!("HTTP method `{method}` cannot be routed"));
        self.router = self.router.route(path, on(filter, handler));
        self.registry.push(RouteMetadata::new(
            method,
            path.to_owned(),
            name.map(str::to_owned),
        ));
        self
    }

    /// Nests `child` under `prefix`, prefixing the patterns of all its routes in the registry.
    ///
    /// # Panics
    /// Panics if a child route name is already registered, or if Axum rejects the nesting.
    #[must_use]
    pub fn nest_prefix(mut self, prefix: &str, child: OxideRouter<S>) -> Self {
        let trimmed = prefix.trim_end_matches('/');
        self.router = if trimmed.is_empty() {
            // Axum 0.8 refuses to nest at the root; merging is equivalent.
            self.router.merge(child.router)
        } else {
            self.router.nest(trimmed, child.router)
        };
        for meta in child.registry.routes {
            // Re-derive the params so ones in `prefix` (`/tenants/{tenant}`) are path params too.
            let path = join_paths(trimmed, &meta.path);
            self.registry
                .push(RouteMetadata::new(meta.method, path, meta.name));
        }
        self
    }

    /// Merges another router's routes (and registry entries) into this one.
    ///
    /// # Panics
    /// Panics if a route name is registered in both, or if Axum rejects the merge.
    #[must_use]
    pub fn merge(mut self, other: OxideRouter<S>) -> Self {
        self.router = self.router.merge(other.router);
        for meta in other.registry.routes {
            self.registry.push(meta);
        }
        self
    }

    /// Applies a Tower layer to every route added so far (forwards to [`axum::Router::layer`]).
    #[must_use]
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.router = self.router.layer(layer);
        self
    }

    /// Applies a layer that only runs when a route matched (forwards to
    /// [`axum::Router::route_layer`]).
    #[must_use]
    pub fn route_layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.router = self.router.route_layer(layer);
        self
    }

    /// The routes registered so far.
    pub fn registry(&self) -> &RouteRegistry {
        &self.registry
    }

    /// Provides the state and publishes the routes to the process-wide registry used by
    /// [`url_for`] and [`route!`](crate::route).
    ///
    /// The result can still be nested or merged; [`OxideRouter::into_axum`] attaches the final
    /// registry to requests.
    pub fn with_state<S2>(self, state: S) -> OxideRouter<S2> {
        publish(&self.registry);
        OxideRouter {
            router: self.router.with_state(state),
            registry: self.registry,
        }
    }

    /// Finalizes URL generation and returns the underlying `axum::Router`.
    ///
    /// The complete registry is published to the process-wide registry and attached to every
    /// request as an [`Extension`] (see [`UrlGenerator`]).
    pub fn into_axum(self) -> axum::Router<S> {
        let shared = publish(&self.registry);
        self.router.layer(Extension(shared))
    }
}

impl From<OxideRouter<()>> for axum::Router {
    fn from(router: OxideRouter<()>) -> Self {
        router.into_axum()
    }
}

/// Process-wide registry behind [`url_for`] and [`route!`](crate::route).
static GLOBAL_REGISTRY: LazyLock<RwLock<RouteRegistry>> = LazyLock::new(RwLock::default);

/// Publishes a router's routes globally and returns the shared copy for the `Extension` layer.
fn publish(registry: &RouteRegistry) -> Arc<RouteRegistry> {
    let mut global = GLOBAL_REGISTRY
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for meta in &registry.routes {
        global.upsert(meta.clone());
    }
    Arc::new(registry.clone())
}

/// Generates a URL from the process-wide registry (populated when a router is finalized with
/// [`OxideRouter::with_state`], [`OxideRouter::into_axum`] or `Into<axum::Router>`).
///
/// Prefer [`route!`](crate::route) in application code, or the [`UrlGenerator`] extractor inside
/// handlers.
pub fn url_for(name: &str, params: &[(&str, &str)]) -> Result<String, OxideHttpError> {
    GLOBAL_REGISTRY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .url(name, params)
}

/// Generates the URL of a named route, panicking if the name or a parameter is missing.
///
/// ```ignore
/// route!("posts.index");              // "/api/v1/posts"
/// route!("posts.show", post = 42);    // "/api/v1/posts/42"
/// route!("posts.index", page = 2);    // "/api/v1/posts?page=2"
/// ```
#[macro_export]
macro_rules! route {
    ($name:expr $(,)?) => {
        $crate::routing::url_for($name, &[])
            .unwrap_or_else(|error| ::core::panic!("route!({:?}): {}", $name, error))
    };
    ($name:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let values = [$(::std::string::ToString::to_string(&$value)),+];
        let keys = [$(::core::stringify!($key)),+];
        let params: ::std::vec::Vec<(&str, &str)> = keys
            .iter()
            .zip(values.iter())
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        $crate::routing::url_for($name, &params)
            .unwrap_or_else(|error| ::core::panic!("route!({:?}): {}", $name, error))
    }};
}

/// Extractor giving handlers access to the router's named routes.
///
/// ```ignore
/// async fn store(urls: UrlGenerator, /* ... */) -> impl IntoResponse {
///     let location = urls.url("posts.show", &[("post", &id.to_string())])?;
///     // ...
/// }
/// ```
#[derive(Debug, Clone)]
pub struct UrlGenerator(Arc<RouteRegistry>);

impl UrlGenerator {
    /// Generates a URL for the named route; see [`RouteRegistry::url`].
    pub fn url(&self, name: &str, params: &[(&str, &str)]) -> Result<String, OxideHttpError> {
        self.0.url(name, params)
    }

    /// The router's registry.
    pub fn registry(&self) -> &RouteRegistry {
        &self.0
    }
}

impl<S> FromRequestParts<S> for UrlGenerator
where
    S: Send + Sync,
{
    type Rejection = OxideHttpError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let generator = parts
            .extensions
            .get::<Arc<RouteRegistry>>()
            .cloned()
            .map(Self)
            .ok_or_else(|| {
                OxideHttpError::Internal(
                    "UrlGenerator requires an OxideRouter finalized with `with_state`/`into_axum`"
                        .into(),
                )
            });
        std::future::ready(generator)
    }
}

/// A `{name}` or `{*name}` placeholder in a path pattern.
struct PathParam<'a> {
    name: &'a str,
    wildcard: bool,
    /// Byte range of the whole placeholder, braces included.
    span: std::ops::Range<usize>,
}

fn path_params(path: &str) -> impl Iterator<Item = PathParam<'_>> {
    let mut rest = 0;
    std::iter::from_fn(move || {
        let start = rest + path[rest..].find('{')?;
        let end = start + path[start..].find('}')?;
        rest = end + 1;
        let inner = &path[start + 1..end];
        let (name, wildcard) = match inner.strip_prefix('*') {
            Some(name) => (name, true),
            None => (inner, false),
        };
        Some(PathParam {
            name,
            wildcard,
            span: start..end + 1,
        })
    })
}

fn build_url(route: &RouteMetadata, params: &[(&str, &str)]) -> Result<String, OxideHttpError> {
    let route_name = route.name.as_deref().unwrap_or_default();
    let mut url = String::with_capacity(route.path.len());
    let mut last = 0;
    for param in path_params(&route.path) {
        let value = params
            .iter()
            .find(|(key, _)| *key == param.name)
            .map(|(_, value)| *value)
            .ok_or_else(|| {
                OxideHttpError::Internal(format!(
                    "missing parameter `{}` for route `{route_name}` ({})",
                    param.name, route.path
                ))
            })?;
        url.push_str(&route.path[last..param.span.start]);
        encode_into(&mut url, value, param.wildcard);
        last = param.span.end;
    }
    url.push_str(&route.path[last..]);

    let mut separator = '?';
    for (key, value) in params {
        if !route.params.iter().any(|p| p == key) {
            url.push(separator);
            encode_into(&mut url, key, false);
            url.push('=');
            encode_into(&mut url, value, false);
            separator = '&';
        }
    }
    Ok(url)
}

/// Percent-encodes everything except RFC 3986 unreserved characters (and `/` in wildcards).
fn encode_into(out: &mut String, value: &str, keep_slash: bool) {
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            b'/' if keep_slash => out.push('/'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
}

/// Joins a route prefix and a child path the way `axum::Router::nest` does.
fn join_paths(prefix: &str, path: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    match path {
        "" | "/" if prefix.is_empty() => "/".to_owned(),
        "" | "/" => prefix.to_owned(),
        _ => format!("{prefix}{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> RouteRegistry {
        let mut registry = RouteRegistry::default();
        registry.push(RouteMetadata::new(
            Method::GET,
            "/api/posts/{post}/comments/{comment}".into(),
            Some("comments.show".into()),
        ));
        registry.push(RouteMetadata::new(
            Method::GET,
            "/files/{*path}".into(),
            Some("files".into()),
        ));
        registry.push(RouteMetadata::new(
            Method::GET,
            "/posts".into(),
            Some("posts.index".into()),
        ));
        registry
    }

    #[test]
    fn interpolates_params_in_order() {
        let url = registry()
            .url("comments.show", &[("comment", "7"), ("post", "42")])
            .unwrap();
        assert_eq!(url, "/api/posts/42/comments/7");
    }

    #[test]
    fn extra_params_become_query_string() {
        let url = registry()
            .url("posts.index", &[("page", "2"), ("q", "rust web")])
            .unwrap();
        assert_eq!(url, "/posts?page=2&q=rust%20web");
    }

    #[test]
    fn encodes_values_but_keeps_slashes_in_wildcards() {
        let registry = registry();
        assert_eq!(
            registry.url("files", &[("path", "a b/c.txt")]).unwrap(),
            "/files/a%20b/c.txt"
        );
        assert_eq!(
            registry
                .url("comments.show", &[("post", "a/b"), ("comment", "1")])
                .unwrap(),
            "/api/posts/a%2Fb/comments/1"
        );
    }

    #[test]
    fn missing_route_or_param_is_an_error() {
        let registry = registry();
        assert!(registry.url("nope", &[]).is_err());
        let error = registry.url("comments.show", &[("post", "1")]).unwrap_err();
        assert!(error.to_string().contains("missing parameter `comment`"));
    }

    #[test]
    fn records_params() {
        assert_eq!(
            registry().get("comments.show").unwrap().params,
            vec!["post".to_owned(), "comment".to_owned()]
        );
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn duplicate_names_panic() {
        let mut registry = registry();
        registry.push(RouteMetadata::new(
            Method::POST,
            "/x".into(),
            Some("posts.index".into()),
        ));
    }

    #[test]
    fn joins_like_nest() {
        assert_eq!(join_paths("/api/", "/posts"), "/api/posts");
        assert_eq!(join_paths("/api", "/"), "/api");
        assert_eq!(join_paths("", "/"), "/");
    }
}
