//! Middleware, Laravel-style: aliases, groups, parameters, priorities and terminable middleware
//! on top of Axum's middleware model.
//!
//! A [`Middleware`] receives the request and a [`MwNext`]; call [`MwNext::run`] to continue and
//! get the response, or return a response without calling it to stop the request.
//!
//! ```ignore
//! Application::builder()
//!     .middleware(SecurityHeaders)                         // global: every request
//!     .alias("role", RequireRole)                          // named: role:editor,admin
//!     .middleware_group("api", ["throttle:60,1", "auth"])  // a bundle of names
//!     .routes(routes! {
//!         prefix("/admin") {
//!             middleware(route_middleware(&["api", "role:admin"])) {
//!                 resource("/users", UserController);
//!             }
//!         }
//!     })
//! ```
//!
//! Global middleware runs outside the router, so it sees every request, including ones that
//! match no route. Route middleware runs only for routes inside its `middleware(..)` block.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use oxide_http::__private::axum::extract::{Request, State};
use oxide_http::__private::axum::http::{HeaderMap, Method, StatusCode};
use oxide_http::__private::axum::middleware::{self, Next};
use oxide_http::__private::axum::response::{IntoResponse, Response};
use oxide_http::__private::axum::routing::Route;
use oxide_http::OxideHttpError;
use tower::{Layer, Service};

/// The future a middleware returns.
pub type MiddlewareFuture<'a> = Pin<Box<dyn Future<Output = Response> + Send + 'a>>;

/// How many levels of groups-within-groups are followed before giving up.
const MAX_GROUP_DEPTH: usize = 8;

/// A step in handling a request. Implement it for a struct, or use [`middleware_fn`] for a
/// closure.
pub trait Middleware: Send + Sync + 'static {
    /// Handles the request. `params` are the values after the alias's colon: `role:editor,admin`
    /// passes `["editor", "admin"]`. Resolve services from the request's
    /// [`Scope`](crate::Scope) (`request.extensions().get::<Scope>()`).
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        params: &'a [String],
    ) -> MiddlewareFuture<'a>;

    /// Where this middleware sits when several apply: higher runs first (outermost). Equal
    /// priorities keep the order they were listed in. Defaults to 0.
    fn priority(&self) -> i32 {
        0
    }

    /// Whether [`terminate`](Self::terminate) should run.
    fn terminable(&self) -> bool {
        false
    }

    /// Work to do after the response has been produced, off the request path (for example
    /// flushing a session or recording metrics). It runs on a spawned task, so it cannot change
    /// the response.
    fn terminate<'a>(
        &'a self,
        _outcome: &'a Outcome,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(std::future::ready(()))
    }
}

/// What a terminable middleware is told about the finished request.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub method: Method,
    pub path: String,
    pub status: StatusCode,
    pub headers: HeaderMap,
}

/// The rest of the middleware chain, ending in the router (or the next layer).
pub struct MwNext {
    chain: Arc<Vec<Resolved>>,
    index: usize,
    inner: Next,
    /// How many steps have been entered so far, shared by the whole run: only those get
    /// `terminate`.
    reached: Arc<AtomicUsize>,
}

impl std::fmt::Debug for MwNext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MwNext")
            .field("remaining", &self.chain.len().saturating_sub(self.index))
            .finish_non_exhaustive()
    }
}

impl MwNext {
    /// Passes the request on and returns the response.
    pub async fn run(self, request: Request) -> Response {
        let step = self
            .chain
            .get(self.index)
            .map(|step| (Arc::clone(&step.middleware), Arc::clone(&step.params)));
        match step {
            Some((middleware, params)) => {
                self.reached.fetch_max(self.index + 1, Ordering::SeqCst);
                let next = Self {
                    chain: self.chain,
                    index: self.index + 1,
                    inner: self.inner,
                    reached: self.reached,
                };
                middleware.handle(request, next, &params).await
            }
            None => self.inner.run(request).await,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Resolved {
    middleware: Arc<dyn Middleware>,
    params: Arc<[String]>,
}

/// A middleware from a closure: `middleware_fn(|request, next, params| async move { .. })`.
pub fn middleware_fn<F, Fut>(f: F) -> FnMiddleware<F>
where
    F: Fn(Request, MwNext, Vec<String>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    FnMiddleware(f)
}

/// See [`middleware_fn`].
#[derive(Clone)]
pub struct FnMiddleware<F>(F);

impl<F> std::fmt::Debug for FnMiddleware<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FnMiddleware")
    }
}

impl<F, Fut> Middleware for FnMiddleware<F>
where
    F: Fn(Request, MwNext, Vec<String>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin((self.0)(request, next, params.to_vec()))
    }
}

/// The application's middleware: the global stack, aliases and groups.
#[derive(Default)]
pub(crate) struct MiddlewareRegistry {
    globals: Arc<Vec<Resolved>>,
    aliases: HashMap<String, Arc<dyn Middleware>>,
    groups: HashMap<String, Vec<String>>,
}

impl MiddlewareRegistry {
    pub(crate) fn new(
        globals: Vec<Arc<dyn Middleware>>,
        aliases: Vec<(String, Arc<dyn Middleware>)>,
        groups: Vec<(String, Vec<String>)>,
    ) -> Result<Self, String> {
        let mut registry = Self {
            globals: Arc::new(sorted(
                globals
                    .into_iter()
                    .map(|middleware| Resolved {
                        middleware,
                        params: Arc::from([]),
                    })
                    .collect(),
            )),
            aliases: aliases.into_iter().collect(),
            groups: groups.into_iter().collect(),
        };
        for name in registry.groups.keys() {
            if registry.aliases.contains_key(name) {
                return Err(format!("`{name}` is both a middleware alias and a group"));
            }
        }
        // Every group must resolve now, so a typo fails at start-up rather than on a request.
        let names: Vec<String> = registry.groups.keys().cloned().collect();
        for name in names {
            registry.expand(std::slice::from_ref(&name), &[], 0)?;
        }
        registry.groups.shrink_to_fit();
        Ok(registry)
    }

    fn expand(
        &self,
        names: &[String],
        without: &[String],
        depth: usize,
    ) -> Result<Vec<Resolved>, String> {
        if depth > MAX_GROUP_DEPTH {
            return Err(format!(
                "middleware groups nest more than {MAX_GROUP_DEPTH} deep (a loop?): {}",
                names.join(", ")
            ));
        }
        let mut out = Vec::new();
        for name in names {
            let (base, params) = match name.split_once(':') {
                Some((base, params)) => (
                    base,
                    params
                        .split(',')
                        .map(str::trim)
                        .filter(|param| !param.is_empty())
                        .map(str::to_owned)
                        .collect(),
                ),
                None => (name.as_str(), Vec::new()),
            };
            if without.iter().any(|excluded| excluded == base) {
                continue;
            }
            if let Some(middleware) = self.aliases.get(base) {
                out.push(Resolved {
                    middleware: Arc::clone(middleware),
                    params: params.into(),
                });
            } else if let Some(members) = self.groups.get(base) {
                out.extend(self.expand(members, without, depth + 1)?);
            } else {
                return Err(format!(
                    "unknown middleware `{base}`: register it with `alias` or `middleware_group`"
                ));
            }
        }
        Ok(out)
    }

    fn resolve(&self, names: &[String], without: &[String]) -> Result<Arc<Vec<Resolved>>, String> {
        self.expand(names, without, 0)
            .map(|chain| Arc::new(sorted(chain)))
    }
}

/// Highest priority first; ties keep their listed order (the sort is stable).
fn sorted(mut chain: Vec<Resolved>) -> Vec<Resolved> {
    chain.sort_by_key(|step| std::cmp::Reverse(step.middleware.priority()));
    chain
}

/// Runs `chain` around `inner`, then hands terminable middleware their [`Outcome`].
async fn run_chain(chain: Arc<Vec<Resolved>>, request: Request, inner: Next) -> Response {
    let terminable: Vec<(usize, Arc<dyn Middleware>)> = chain
        .iter()
        .enumerate()
        .filter(|(_, step)| step.middleware.terminable())
        .map(|(index, step)| (index, Arc::clone(&step.middleware)))
        .collect();
    let started = (!terminable.is_empty())
        .then(|| (request.method().clone(), request.uri().path().to_owned()));
    let reached = Arc::new(AtomicUsize::new(0));
    let response = MwNext {
        chain,
        index: 0,
        inner,
        reached: Arc::clone(&reached),
    }
    .run(request)
    .await;
    if let Some((method, path)) = started {
        // Only middleware the request actually reached are told how it ended: one that an
        // earlier middleware cut off never saw it.
        let reached = reached.load(Ordering::SeqCst);
        let terminable: Vec<Arc<dyn Middleware>> = terminable
            .into_iter()
            .filter(|(index, _)| *index < reached)
            .map(|(_, middleware)| middleware)
            .collect();
        if terminable.is_empty() {
            return response;
        }
        let outcome = Outcome {
            method,
            path,
            status: response.status(),
            headers: response.headers().clone(),
        };
        tokio::spawn(async move {
            for middleware in terminable {
                middleware.terminate(&outcome).await;
            }
        });
    }
    response
}

fn misconfigured(reason: &str) -> Response {
    // The detail is for the developer; the client only learns the request failed.
    oxide_log::tracing::error!("middleware misconfigured: {reason}");
    OxideHttpError::Internal("Server Error".to_owned()).into_response()
}

/// Makes the registry reachable from route middleware.
pub(crate) async fn registry_middleware(
    State(registry): State<Arc<MiddlewareRegistry>>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().insert(Arc::clone(&registry));
    next.run(request).await
}

/// The global stack.
pub(crate) async fn global_middleware(
    State(registry): State<Arc<MiddlewareRegistry>>,
    request: Request,
    next: Next,
) -> Response {
    run_chain(Arc::clone(&registry.globals), request, next).await
}

struct RouteSpec {
    names: Vec<String>,
    without: Vec<String>,
    /// The resolved chain and the registry (by address) it was resolved against, so the names
    /// are expanded once rather than on every request.
    resolved: RwLock<Option<(usize, Arc<Vec<Resolved>>)>>,
}

impl RouteSpec {
    fn chain(&self, registry: &Arc<MiddlewareRegistry>) -> Result<Arc<Vec<Resolved>>, String> {
        let registry_id = Arc::as_ptr(registry) as usize;
        if let Some((id, chain)) = self
            .resolved
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            && *id == registry_id
        {
            return Ok(Arc::clone(chain));
        }
        let chain = registry.resolve(&self.names, &self.without)?;
        *self
            .resolved
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some((registry_id, Arc::clone(&chain)));
        Ok(chain)
    }
}

async fn route_middleware_handler(
    State(spec): State<Arc<RouteSpec>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(registry) = request
        .extensions()
        .get::<Arc<MiddlewareRegistry>>()
        .cloned()
    else {
        return misconfigured(
            "route middleware needs an `Application` (no registry on the request)",
        );
    };
    match spec.chain(&registry) {
        Ok(chain) => run_chain(chain, request, next).await,
        Err(reason) => misconfigured(&reason),
    }
}

/// A layer for `routes!` that runs the named middleware, aliases or groups, in order (then by
/// priority), for the routes inside its block:
/// `middleware(route_middleware(&["auth", "role:admin"])) { .. }`. Names are resolved when a
/// request arrives, against the application's registrations, so a typo is a `500` and a log
/// line, not a build error.
pub fn route_middleware(
    names: &[&str],
) -> impl Layer<
    Route,
    Service = impl Service<
        Request,
        Response = Response,
        Error = Infallible,
        Future = impl Send + 'static,
    > + Clone
              + Send
              + Sync
              + 'static,
> + Clone
+ Send
+ Sync
+ 'static {
    route_middleware_without(names, &[])
}

/// Like [`route_middleware`], leaving out the named middleware wherever they appear in the
/// listed groups: `route_middleware_without(&["api"], &["throttle"])` is the `api` group minus
/// `throttle`.
pub fn route_middleware_without(
    names: &[&str],
    without: &[&str],
) -> impl Layer<
    Route,
    Service = impl Service<
        Request,
        Response = Response,
        Error = Infallible,
        Future = impl Send + 'static,
    > + Clone
              + Send
              + Sync
              + 'static,
> + Clone
+ Send
+ Sync
+ 'static {
    let spec = Arc::new(RouteSpec {
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        without: without.iter().map(|name| (*name).to_owned()).collect(),
        resolved: RwLock::new(None),
    });
    middleware::from_fn_with_state(spec, route_middleware_handler)
}
