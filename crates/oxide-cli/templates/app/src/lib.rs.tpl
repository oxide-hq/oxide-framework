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

/// The application, ready for `axum::serve` or `TestClient::new`.
pub fn build_app(ctx: AppContext) -> oxide::axum::Router {
    routes::api::router().with_state(ctx).into()
}
