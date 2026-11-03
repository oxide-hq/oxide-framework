//! # Oxide
//!
//! Laravel's developer ergonomics with compile-time safe Rust, on Axum and sqlx. This crate is
//! the single dependency an Oxide application needs: it re-exports configuration as [`config`]
//! (`oxide-config`), the application bootstrap as [`foundation`] (`oxide-foundation`), the HTTP layer as [`http`] (`oxide-http`), the ORM as [`orm`] (`oxide-orm`)
//! and the helpers as [`support`] (`oxide-support`), and gathers the everyday items
//! into one prelude.
//!
//! ```ignore
//! use oxide::prelude::*;
//!
//! #[derive(Clone, Debug, ServiceContainer)]
//! pub struct AppContext {
//!     pub db: DatabasePool,
//! }
//!
//! let router: OxideRouter<AppContext> = routes! {
//!     prefix("/api/v1") {
//!         resource("/posts", PostController);
//!     }
//! };
//! ```
//!
//! The derive macros generate `::oxide::config::...` / `::oxide::http::...` / `::oxide::orm::...` paths when your crate
//! depends on `oxide`, so you never need `oxide-http` or `oxide-orm` directly. Enable the
//! `postgres` feature for PostgreSQL.

// Lets the derive macros' `::oxide::...` paths resolve inside this crate too.
extern crate self as oxide;

pub use oxide_config as config;
pub use oxide_foundation as foundation;
pub use oxide_http as http;
pub use oxide_log as log;
pub use oxide_orm as orm;
pub use oxide_support as support;

/// Axum, which Oxide builds on: `oxide::axum::serve`, `oxide::axum::Router`, Tower layers, ...
pub use axum;

pub use oxide_config::{Config, ConfigErrors, ConfigSection, Configuration, Environment};
pub use oxide_foundation::{
    Application, Bindings, BootError, ConvertEmptyStringsToNull, CorsConfig, ExceptionHandler,
    HandleCors, Inject, Middleware, MwNext, SecurityHeaders, ServiceProvider, SetCacheHeaders,
    TrimStrings, TrustHosts, ValidatePostSize, middleware_fn, route_middleware,
};
pub use oxide_http::{
    Authorize, Bind, FormRequest, OxideHttpError, OxideResult, OxideRouter, RouteBindable,
    ServiceContainer, UrlGenerator, Validated, facade_middleware, route, routes, url_for,
};
pub use oxide_log::{Context, Log, Logger, LoggingConfig, tracing};
pub use oxide_orm::{DatabasePool, Driver, Model, OrmError};
pub use oxide_support::{Arr, CollectionExt, Number, Str, Stringable, Tap};

/// Everything you need for day-to-day Oxide code: `use oxide::prelude::*;`.
pub mod prelude {
    pub use axum::Json;
    pub use axum::http::StatusCode;
    pub use oxide_config::prelude::*;
    pub use oxide_foundation::{
        Application, Bindings, BootError, ExceptionHandler, Inject, Middleware, MwNext,
        ServiceProvider, middleware_fn, route_middleware,
    };
    pub use oxide_http::prelude::*;
    pub use oxide_log::prelude::*;
    pub use oxide_orm::prelude::*;
    pub use oxide_support::prelude::*;
}
