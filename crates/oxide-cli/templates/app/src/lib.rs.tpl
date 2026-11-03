//! {{name}}, an Oxide application.

pub mod context;
pub mod http;
pub mod models;
pub mod routes;

use oxide::prelude::*;
use sqlx::migrate::Migrator;

pub use context::AppContext;

/// The migrations in `database/migrations`, embedded at compile time.
pub static MIGRATOR: Migrator = sqlx::migrate!("./database/migrations");

/// Connects to `database_url`, applies pending migrations and builds the service container.
pub async fn boot(database_url: &str) -> Result<AppContext, OrmError> {
    let db = DatabasePool::connect(database_url).await?;
    db.migrate(&MIGRATOR).await?;
    Ok(AppContext::new(db))
}

/// The application, ready for `axum::serve` or `TestClient::new`.
pub fn build_app(ctx: AppContext) -> oxide::axum::Router {
    routes::api::router().with_state(ctx).into()
}
