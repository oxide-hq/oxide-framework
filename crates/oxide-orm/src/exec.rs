//! Runs rendered SQL on whichever pool a [`DatabasePool`] holds.

use sqlx::Sqlite;
#[cfg(feature = "postgres")]
use sqlx::{Postgres, query::QueryAs};

use crate::database::DatabasePool;
use crate::error::OrmError;
use crate::model::Model;
#[cfg(feature = "postgres")]
use crate::query::postgres_arguments;
use crate::query::{SqlValue, sqlite_arguments};

#[cfg(feature = "postgres")]
fn postgres_query<M: Model>(
    sql: &str,
    values: Vec<SqlValue>,
) -> Result<QueryAs<'_, Postgres, M, sqlx::postgres::PgArguments>, OrmError> {
    Ok(sqlx::query_as_with::<Postgres, M, _>(
        sql,
        postgres_arguments(values)?,
    ))
}

pub(crate) async fn fetch_all<M: Model>(
    db: &DatabasePool,
    sql: &str,
    values: Vec<SqlValue>,
) -> Result<Vec<M>, OrmError> {
    Ok(match db {
        DatabasePool::Sqlite(pool) => {
            sqlx::query_as_with::<Sqlite, M, _>(sql, sqlite_arguments(values)?)
                .fetch_all(pool)
                .await?
        }
        #[cfg(feature = "postgres")]
        DatabasePool::Postgres(pool) => postgres_query(sql, values)?.fetch_all(pool).await?,
    })
}

pub(crate) async fn fetch_optional<M: Model>(
    db: &DatabasePool,
    sql: &str,
    values: Vec<SqlValue>,
) -> Result<Option<M>, OrmError> {
    Ok(match db {
        DatabasePool::Sqlite(pool) => {
            sqlx::query_as_with::<Sqlite, M, _>(sql, sqlite_arguments(values)?)
                .fetch_optional(pool)
                .await?
        }
        #[cfg(feature = "postgres")]
        DatabasePool::Postgres(pool) => postgres_query(sql, values)?.fetch_optional(pool).await?,
    })
}

pub(crate) async fn fetch_one<M: Model>(
    db: &DatabasePool,
    sql: &str,
    values: Vec<SqlValue>,
) -> Result<M, OrmError> {
    Ok(match db {
        DatabasePool::Sqlite(pool) => {
            sqlx::query_as_with::<Sqlite, M, _>(sql, sqlite_arguments(values)?)
                .fetch_one(pool)
                .await?
        }
        #[cfg(feature = "postgres")]
        DatabasePool::Postgres(pool) => postgres_query(sql, values)?.fetch_one(pool).await?,
    })
}

/// Runs a `SELECT COUNT(*) ...`.
pub(crate) async fn fetch_count(
    db: &DatabasePool,
    sql: &str,
    values: Vec<SqlValue>,
) -> Result<i64, OrmError> {
    Ok(match db {
        DatabasePool::Sqlite(pool) => {
            sqlx::query_scalar_with::<Sqlite, i64, _>(sql, sqlite_arguments(values)?)
                .fetch_one(pool)
                .await?
        }
        #[cfg(feature = "postgres")]
        DatabasePool::Postgres(pool) => {
            sqlx::query_scalar_with::<Postgres, i64, _>(sql, postgres_arguments(values)?)
                .fetch_one(pool)
                .await?
        }
    })
}

/// Runs a statement and returns the number of affected rows.
pub(crate) async fn execute(
    db: &DatabasePool,
    sql: &str,
    values: Vec<SqlValue>,
) -> Result<u64, OrmError> {
    Ok(match db {
        DatabasePool::Sqlite(pool) => sqlx::query_with::<Sqlite, _>(sql, sqlite_arguments(values)?)
            .execute(pool)
            .await?
            .rows_affected(),
        #[cfg(feature = "postgres")]
        DatabasePool::Postgres(pool) => {
            sqlx::query_with::<Postgres, _>(sql, postgres_arguments(values)?)
                .execute(pool)
                .await?
                .rows_affected()
        }
    })
}
