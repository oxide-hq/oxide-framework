//! # oxide-foundation
//!
//! Application bootstrap, Laravel-style: an [`Application`] built from configuration, service
//! providers and routes, on a typed dependency [`Container`].
//!
//! ```ignore
//! let app = Application::builder()
//!     .config(config)
//!     .provider(DatabaseProvider)
//!     .provider(MailProvider)
//!     .routes(routes::api())
//!     .build()
//!     .await?;
//! app.serve(tokio::net::TcpListener::bind("127.0.0.1:3000").await?).await?;
//! ```
//!
//! `#[derive(ServiceContainer)]` stays the zero-cost way to share services known at compile
//! time. The container here is for what is only known at boot: trait-object contracts,
//! services registered by packages, per-request services.
//!
//! # Request lifecycle
//!
//! 1. **Build.** `ApplicationBuilder::build` runs every provider's `register` (bindings only),
//!    then the builder's `bind` overrides, then checks that everything declared with `require`
//!    is bound.
//! 2. **Boot.** Each provider's `boot` runs in order, with the finished container available.
//! 3. **Routes.** Providers' routes and the builder's routes are merged, and the request-scope
//!    middleware is layered on top.
//! 4. **Per request.** The scope middleware starts a [`Scope`]; your own middleware and the
//!    router run; a handler's [`Inject<T>`](Inject) extractors resolve services (scoped ones
//!    are built once for the request); the response flows back out through the middleware.

mod application;
mod builtin;
mod container;
mod exceptions;
mod middleware;
mod provider;

pub use application::{Application, ApplicationBuilder};
pub use builtin::{
    Client, ClientInfo, ConvertEmptyStringsToNull, CorsConfig, Down, HandleCors,
    PreventRequestsDuringMaintenance, SecurityHeaders, SetCacheHeaders, TrimStrings, TrustHosts,
    TrustProxies, ValidatePostSize, down, is_down, up,
};
pub use container::{Bindings, Container, Inject, ResolveError, Scope, ServiceKey};
pub use exceptions::{
    DefaultExceptionHandler, ErrorContext, ExceptionHandler, RequestInfo, default_render,
    default_report, default_should_report,
};
pub use middleware::{
    FnMiddleware, Middleware, MiddlewareFuture, MwNext, Outcome, middleware_fn, route_middleware,
    route_middleware_without,
};
pub use provider::{App, BootError, BootFuture, ServiceProvider};

/// `use oxide_foundation::prelude::*;`
pub mod prelude {
    pub use crate::{
        App, Application, Bindings, BootError, BootFuture, Container, Inject, ResolveError, Scope,
        ServiceKey, ServiceProvider,
    };
}
