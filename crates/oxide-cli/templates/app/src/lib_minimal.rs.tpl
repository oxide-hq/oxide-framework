//! {{name}}, an Oxide application.

pub mod config;
pub mod context;
pub mod http;
pub mod routes;

pub use context::AppContext;

/// The application, ready for `axum::serve` or `TestClient::new`.
pub fn build_app(ctx: AppContext) -> oxide::axum::Router {
    routes::api::router().with_state(ctx).into()
}
