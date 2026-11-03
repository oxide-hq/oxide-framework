//! # Oxide
//!
//! Laravel's developer ergonomics with compile-time safe Rust, on Axum and sqlx. This crate is
//! the single dependency an Oxide application needs: it re-exports configuration as [`config`] (`oxide-config`), the HTTP layer as
//! [`http`] (`oxide-http`) and the ORM as [`orm`] (`oxide-orm`), and gathers the everyday items
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
pub use oxide_http as http;
pub use oxide_orm as orm;

/// Axum, which Oxide builds on: `oxide::axum::serve`, `oxide::axum::Router`, Tower layers, ...
pub use axum;

pub use oxide_config::{Config, ConfigErrors, ConfigSection, Configuration, Environment};
pub use oxide_http::{
    Authorize, Bind, FormRequest, OxideHttpError, OxideResult, OxideRouter, RouteBindable,
    ServiceContainer, UrlGenerator, Validated, facade_middleware, route, routes, url_for,
};
pub use oxide_orm::{DatabasePool, Driver, Model, OrmError};

/// Everything you need for day-to-day Oxide code: `use oxide::prelude::*;`.
pub mod prelude {
    pub use axum::Json;
    pub use axum::http::StatusCode;
    pub use oxide_config::prelude::*;
    pub use oxide_http::prelude::*;
    pub use oxide_orm::prelude::*;
}
