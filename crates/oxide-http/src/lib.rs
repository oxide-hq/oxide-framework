//! # oxide-http
//!
//! Laravel-style HTTP ergonomics for [Axum](https://docs.rs/axum), checked at compile time:
//!
//! - **Service container** — `#[derive(ServiceContainer)]` on your state, `#[derive(Service)]` on
//!   each service, then take services directly as handler arguments. Optional task-local facades.
//! - **Form requests** — `#[derive(FormRequest)]` + [`Validated<T>`]: deserialize, authorize, validate.
//! - **Route-model binding** — implement [`RouteBindable`] and extract [`Bind<T>`].
//! - **Routing** — the [`routes!`] DSL (prefixes, middleware, `resource(...)`), named routes and
//!   [`route!`] URL generation via [`OxideRouter`].
//! - **Laravel error contract** — [`OxideHttpError`] renders 400/403/404/422/500 JSON like Laravel.
//! - **Fluent testing** — [`TestClient`](testing::TestClient) with chained assertions.
//!
//! ```ignore
//! use oxide_http::prelude::*;
//! ```

// Lets the derive macros' `::oxide_http::...` paths resolve inside this crate too.
extern crate self as oxide_http;

pub mod binding;
pub mod container;
pub mod error;
pub mod request;
pub mod routing;
pub mod testing;

pub use binding::{Bind, RouteBindable};
pub use container::{ServiceContainer, facade_middleware};
pub use error::{OxideHttpError, OxideResult};
pub use oxide_http_macros::*;
pub use request::{Authorize, FormRequest, Validated};
pub use routing::{OxideRouter, RouteMetadata, RouteRegistry, UrlGenerator, url_for};

/// Re-exports used by generated code. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use axum;
    pub use tokio;
    pub use validator;
}

/// Everything you need for day-to-day Oxide HTTP code: `use oxide_http::prelude::*;`.
pub mod prelude {
    pub use crate::binding::{Bind, RouteBindable};
    pub use crate::container::{ServiceContainer, facade_middleware};
    pub use crate::error::{OxideHttpError, OxideResult};
    pub use crate::request::{Authorize, FormRequest, Validated};
    pub use crate::route;
    pub use crate::routing::{OxideRouter, UrlGenerator, url_for};
    pub use crate::testing::{TestClient, TestResponse};
    pub use oxide_http_macros::{FormRequest, Service, ServiceContainer, routes};
    pub use validator::Validate;
}
