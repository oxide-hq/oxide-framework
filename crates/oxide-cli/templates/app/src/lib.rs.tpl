//! {{name}}, an Oxide application.

pub mod config;
pub mod context;
pub mod http;
pub mod models;
pub mod routes;

use oxide::prelude::*;
use sqlx::migrate::Migrator;

pub use context::AppContext;

/// The migrations in `database/migrations`, embedded at compile time.
pub static MIGRATOR: Migrator = sqlx::migrate!("./database/migrations");

/// Connects to the configured database, applies pending migrations and builds the container.
pub async fn boot(config: Configuration) -> Result<AppContext, Box<dyn std::error::Error>> {
    let database = config.require::<config::DatabaseConfig>()?;
    let db = DatabasePool::connect(&database.url).await?;
    db.migrate(&MIGRATOR).await?;
    Ok(AppContext::new(db, config))
}

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
