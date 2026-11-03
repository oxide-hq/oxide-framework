//! The [`DatabasePool`] service.

use std::str::FromStr;
#[cfg(feature = "postgres")]
use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
#[cfg(feature = "postgres")]
use sqlx::{PgPool, postgres::PgConnectOptions};

use crate::error::OrmError;

/// A database backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Sqlite,
    /// Requires the `postgres` feature to connect.
    Postgres,
}

impl Driver {
    /// The driver a connection URL names: `sqlite:` or `postgres://` / `postgresql://`.
    pub fn from_url(url: &str) -> Result<Self, OrmError> {
        if url.starts_with("sqlite:") {
            Ok(Self::Sqlite)
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Ok(Self::Postgres)
        } else {
            Err(configuration(format!(
                "unsupported database URL `{}`; expected `sqlite:` or `postgres://`",
                redact(url)
            )))
        }
    }
}

/// A connection pool, injectable as a service.
///
/// Put it in a `#[derive(ServiceContainer)]` state struct and handlers can take `db: DatabasePool`
/// directly; `#[derive(Model)]` types use it for route-model binding. SQLite is always available;
/// PostgreSQL needs the `postgres` feature.
#[derive(Clone, Debug, oxide_http::Service)]
#[non_exhaustive]
pub enum DatabasePool {
    Sqlite(SqlitePool),
    #[cfg(feature = "postgres")]
    Postgres(PgPool),
}

impl DatabasePool {
    /// Connects to `url`, e.g. `sqlite://database/app.sqlite?mode=rwc` or
    /// `postgres://user:pass@localhost/app`. The scheme picks the driver.
    pub async fn connect(url: &str) -> Result<Self, OrmError> {
        match Driver::from_url(url)? {
            Driver::Sqlite => {
                let options = SqliteConnectOptions::from_str(url)?.foreign_keys(true);
                Ok(Self::Sqlite(
                    SqlitePoolOptions::new().connect_with(options).await?,
                ))
            }
            #[cfg(feature = "postgres")]
            Driver::Postgres => Self::connect_postgres(url).await,
            #[cfg(not(feature = "postgres"))]
            Driver::Postgres => Err(postgres_disabled()),
        }
    }

    #[cfg(feature = "postgres")]
    async fn connect_postgres(url: &str) -> Result<Self, OrmError> {
        Ok(Self::Postgres(PgPool::connect(url).await?))
    }

    /// A fresh in-memory SQLite database, ideal for tests.
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
        Ok(Self::Sqlite(pool))
    }

    /// An empty, isolated database for one test, like Laravel's `RefreshDatabase`.
    ///
    /// A `sqlite:` URL gives a new in-memory database. A `postgres://` URL creates a uniquely
    /// named `oxide_test_*` schema on that server and returns a pool whose connections use it,
    /// so tests can run in parallel against one server. The schemas are left behind for you to
    /// inspect; point tests at a throwaway database.
    pub async fn connect_fresh(url: &str) -> Result<Self, OrmError> {
        match Driver::from_url(url)? {
            Driver::Sqlite => Self::connect_in_memory().await,
            #[cfg(feature = "postgres")]
            Driver::Postgres => Self::connect_fresh_postgres(url).await,
            #[cfg(not(feature = "postgres"))]
            Driver::Postgres => Err(postgres_disabled()),
        }
    }

    #[cfg(feature = "postgres")]
    async fn connect_fresh_postgres(url: &str) -> Result<Self, OrmError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let schema = format!(
            "oxide_test_{}_{nanos}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );

        let admin = PgPool::connect(url).await?;
        sqlx::raw_sql(&format!("CREATE SCHEMA \"{schema}\""))
            .execute(&admin)
            .await?;
        admin.close().await;

        let options = PgConnectOptions::from_str(url)?.options([("search_path", schema.as_str())]);
        Ok(Self::Postgres(PgPool::connect_with(options).await?))
    }

    /// The backend this pool talks to.
    pub fn driver(&self) -> Driver {
        match self {
            Self::Sqlite(_) => Driver::Sqlite,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => Driver::Postgres,
        }
    }

    /// Runs one or more `;`-separated SQL statements (schema setup, seeds, one-off scripts).
    pub async fn execute_schema(&self, sql: &str) -> Result<(), OrmError> {
        match self {
            Self::Sqlite(pool) => {
                sqlx::raw_sql(sql).execute(pool).await?;
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => {
                sqlx::raw_sql(sql).execute(pool).await?;
            }
        }
        Ok(())
    }

    /// Applies pending migrations, recording them in `_sqlx_migrations`.
    ///
    /// Build the migrator with `sqlx::migrate!("./database/migrations")`, which embeds the
    /// `NNNN_name.sql` files at compile time. Already-applied migrations are skipped.
    pub async fn migrate(&self, migrator: &Migrator) -> Result<(), OrmError> {
        match self {
            Self::Sqlite(pool) => migrator.run(pool).await?,
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => migrator.run(pool).await?,
        }
        Ok(())
    }

    /// The SQLite pool, for queries the builder doesn't cover.
    pub fn as_sqlite(&self) -> Option<&SqlitePool> {
        match self {
            Self::Sqlite(pool) => Some(pool),
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => None,
        }
    }

    /// The PostgreSQL pool, for queries the builder doesn't cover.
    #[cfg(feature = "postgres")]
    pub fn as_postgres(&self) -> Option<&PgPool> {
        match self {
            Self::Postgres(pool) => Some(pool),
            Self::Sqlite(_) => None,
        }
    }
}

fn configuration(message: String) -> OrmError {
    OrmError::Database(sqlx::Error::Configuration(message.into()))
}

#[cfg(not(feature = "postgres"))]
fn postgres_disabled() -> OrmError {
    configuration("PostgreSQL support needs the `postgres` feature of oxide-orm".into())
}

/// The URL without credentials, for error messages.
fn redact(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => {
            format!("{}://***{}", &url[..scheme], &url[at..])
        }
        _ => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drivers_come_from_the_url_scheme() {
        assert_eq!(Driver::from_url("sqlite::memory:").unwrap(), Driver::Sqlite);
        assert_eq!(
            Driver::from_url("postgres://localhost/app").unwrap(),
            Driver::Postgres
        );
        assert_eq!(
            Driver::from_url("postgresql://localhost/app").unwrap(),
            Driver::Postgres
        );
        let error = Driver::from_url("mysql://root:secret@localhost/app").unwrap_err();
        assert!(error.to_string().contains("mysql://***@localhost/app"));
        assert!(!error.to_string().contains("secret"));
    }
}
