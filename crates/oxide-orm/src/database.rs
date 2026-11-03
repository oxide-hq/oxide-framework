//! The [`DatabasePool`] service.

use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::error::OrmError;

/// A SQLite connection pool, injectable as a service.
///
/// Put it in a `#[derive(ServiceContainer)]` state struct and handlers can take `db: DatabasePool`
/// directly; `#[derive(Model)]` types use it for route-model binding.
#[derive(Clone, Debug, oxide_http::Service)]
pub struct DatabasePool(pub SqlitePool);

impl DatabasePool {
    /// Connects to `url`, e.g. `sqlite://app.db?mode=rwc`.
    pub async fn connect(url: &str) -> Result<Self, OrmError> {
        let options = SqliteConnectOptions::from_str(url)?.foreign_keys(true);
        Ok(Self(SqlitePoolOptions::new().connect_with(options).await?))
    }

    /// A fresh in-memory database, ideal for tests.
    ///
    /// SQLite gives every connection its own in-memory database, so the pool holds exactly one
    /// connection that is never recycled.
    pub async fn connect_in_memory() -> Result<Self, OrmError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await?;
        Ok(Self(pool))
    }

    /// Runs one or more `;`-separated SQL statements (schema setup, migrations, seeds).
    pub async fn execute_schema(&self, sql: &str) -> Result<(), OrmError> {
        sqlx::raw_sql(sql).execute(&self.0).await?;
        Ok(())
    }

    /// The underlying sqlx pool, for queries the builder doesn't cover.
    pub fn pool(&self) -> &SqlitePool {
        &self.0
    }
}
