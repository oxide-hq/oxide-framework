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
pub mod constraints;
pub mod container;
pub mod error;
pub mod input;
pub mod request;
pub mod request_ext;
pub mod response;
pub mod routing;
pub mod testing;
pub mod upload;

pub use binding::{Bind, BindIn, RouteBindable, ScopedBindable};
pub use constraints::{
    DomainParams, domain, where_alpha, where_alpha_numeric, where_fn, where_in, where_number,
    where_uuid,
};
pub use container::{ServiceContainer, facade_middleware};
pub use error::{DomainError, HandledError, OxideHttpError, OxideResult};
pub use input::Input;
pub use oxide_http_macros::*;
pub use request::{Authorize, FormRequest, Validated};
pub use request_ext::{RequestExt, negotiate};
pub use response::{
    Cookie, Responsable, ResponseExt, SameSite, permanent_redirect_to, redirect_to,
};
pub use routing::{OxideRouter, RouteMetadata, RouteRegistry, UrlGenerator, url_for};
pub use upload::{FileRules, UploadedFile, Uploads};

/// Re-exports used by generated code. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use axum;
    pub use tokio;
    pub use validator;
}

/// Everything you need for day-to-day Oxide HTTP code: `use oxide_http::prelude::*;`.
pub mod prelude {
    pub use crate::binding::{Bind, BindIn, RouteBindable, ScopedBindable};
    pub use crate::constraints::{
        DomainParams, domain, where_alpha, where_alpha_numeric, where_fn, where_in, where_number,
        where_uuid,
    };
    pub use crate::container::{ServiceContainer, facade_middleware};
    pub use crate::error::{DomainError, OxideHttpError, OxideResult};
    pub use crate::input::Input;
    pub use crate::request::{Authorize, FormRequest, Validated};
    pub use crate::request_ext::RequestExt;
    pub use crate::response::{
        self, Cookie, Responsable, ResponseExt, permanent_redirect_to, redirect_to,
    };
    pub use crate::route;
    pub use crate::routing::{OxideRouter, UrlGenerator, url_for};
    pub use crate::testing::{TestClient, TestResponse};
    pub use crate::upload::{FileRules, UploadedFile, Uploads};
    pub use oxide_http_macros::{FormRequest, Service, ServiceContainer, routes};
    pub use validator::Validate;
}
