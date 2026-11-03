//! {{name}}, an Oxide application.

pub mod config;
pub mod context;
pub mod http;
pub mod routes;

use oxide::prelude::*;

pub use context::AppContext;

/// The application, ready for `axum::serve` or `TestClient::new`: your routes plus the
/// framework's request id, logging context and error handling. Add your own middleware here with
/// `.middleware(..)` or `.alias(..)` (`oxide make:middleware`).
pub async fn build_app(ctx: AppContext) -> Result<oxide::axum::Router, BootError> {
    let debug = ctx.config.get::<config::AppConfig>().is_some_and(|app| app.debug);
    let application = Application::builder()
        .config(ctx.config.clone())
        .debug(debug)
        .routes(routes::api::router().with_state(ctx))
        .build()
        .await?;
    Ok(application.into_router())
}
