//! Service container support: state-backed services and task-local facades.
//!
//! Derive [`ServiceContainer`](crate::ServiceContainer) on your application state and
//! [`Service`](crate::Service) on each service type. Handlers can then take services directly as
//! arguments, and — inside [`facade_middleware`] — code anywhere in the request's task can reach
//! the container statically via [`ServiceContainer::current`] / [`ServiceContainer::resolve`].

use std::future::Future;

use axum::extract::{FromRef, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use tokio::task::futures::TaskLocalFuture;

/// An application state struct whose fields are injectable services.
///
/// Implemented by `#[derive(ServiceContainer)]`; you normally don't implement it by hand.
pub trait ServiceContainer: Clone + Send + Sync + 'static {
    /// Runs `future` with `self` installed as the task-local container.
    fn scope<F>(self, future: F) -> TaskLocalFuture<Self, F>
    where
        F: Future;

    /// The container for the current task, if one is in scope.
    fn try_current() -> Option<Self>;

    /// The container for the current task.
    ///
    /// # Panics
    /// Panics when called outside [`ServiceContainer::scope`] (e.g. outside [`facade_middleware`]).
    #[track_caller]
    fn current() -> Self {
        Self::try_current().unwrap_or_else(|| {
            panic!(
                "no `{}` in scope: facades are only available inside `ServiceContainer::scope` \
                 (did you add `facade_middleware` to the router?)",
                std::any::type_name::<Self>()
            )
        })
    }

    /// Resolves a single service from the task-local container (a "facade" lookup).
    ///
    /// # Panics
    /// Panics when no container is in scope; see [`ServiceContainer::current`].
    #[track_caller]
    fn resolve<T>() -> T
    where
        T: FromRef<Self>,
    {
        T::from_ref(&Self::current())
    }
}

/// Middleware that installs the router state as the task-local container for each request.
///
/// ```ignore
/// let app = Router::new()
///     .route("/", get(handler))
///     .layer(axum::middleware::from_fn_with_state(ctx.clone(), facade_middleware::<AppContext>))
///     .with_state(ctx);
/// ```
pub async fn facade_middleware<C>(
    State(container): State<C>,
    request: Request,
    next: Next,
) -> Response
where
    C: ServiceContainer,
{
    container.scope(next.run(request)).await
}
