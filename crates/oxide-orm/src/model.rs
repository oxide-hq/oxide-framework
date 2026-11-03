//! The [`Model`] trait, generic CRUD helpers used by `#[derive(Model)]`, and relations.

use std::fmt::Display;

#[cfg(feature = "postgres")]
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;

use crate::database::DatabasePool;
use crate::error::OrmError;
use crate::exec;
use crate::query::{Op, Query, SqlValue, SqlWriter};

/// Decodes rows from every enabled backend: SQLite, plus PostgreSQL with the `postgres` feature.
///
/// Implemented automatically for types deriving `sqlx::FromRow`, which is generic over the row
/// type. With `postgres` on, every field must decode from both (`i64` needs a `BIGINT` column).
#[cfg(not(feature = "postgres"))]
pub trait FromDatabaseRow: for<'r> sqlx::FromRow<'r, SqliteRow> {}
#[cfg(not(feature = "postgres"))]
impl<T: for<'r> sqlx::FromRow<'r, SqliteRow>> FromDatabaseRow for T {}

/// Decodes rows from every enabled backend: SQLite, plus PostgreSQL with the `postgres` feature.
///
/// Implemented automatically for types deriving `sqlx::FromRow`, which is generic over the row
/// type. With `postgres` on, every field must decode from both (`i64` needs a `BIGINT` column).
#[cfg(feature = "postgres")]
pub trait FromDatabaseRow:
    for<'r> sqlx::FromRow<'r, SqliteRow> + for<'r> sqlx::FromRow<'r, PgRow>
{
}
#[cfg(feature = "postgres")]
impl<T> FromDatabaseRow for T where
    T: for<'r> sqlx::FromRow<'r, SqliteRow> + for<'r> sqlx::FromRow<'r, PgRow>
{
}

/// A database-backed model. Implemented by `#[derive(Model)]`.
pub trait Model: Sized + Send + Unpin + FromDatabaseRow + 'static {
    /// The type name, used in `No query results for model [Post] 1` messages.
    const NAME: &'static str;
    /// The table name.
    const TABLE: &'static str;
    /// The primary key column.
    const PRIMARY_KEY: &'static str;
    /// Every column, primary key included, in field order.
    const COLUMNS: &'static [&'static str];

    /// The primary key type.
    type Key: Into<SqlValue> + Clone + Display + Send + Sync;
    /// The generated typed query builder (`PostQueryBuilder` for `Post`).
    type QueryBuilder: From<Query<Self>>;

    /// This row's primary key.
    fn key(&self) -> Self::Key;

    /// The value of `column`, or `None` if the model has no such column.
    fn value_of(&self, column: &str) -> Option<SqlValue>;

    /// `(column, value)` for every column except the primary key, in field order.
    fn column_values(&self) -> Vec<(&'static str, SqlValue)>;

    /// Rows of `R` whose `foreign_key` column points at this model (`post.has_many::<Comment>("post_id")`).
    fn has_many<R: Model>(&self, foreign_key: &'static str) -> R::QueryBuilder {
        R::QueryBuilder::from(Query::new().filter(foreign_key, Op::Eq, self.key()))
    }

    /// Like [`Model::has_many`]; call `.first(&db)` on the result for the single related row.
    fn has_one<R: Model>(&self, foreign_key: &'static str) -> R::QueryBuilder {
        self.has_many::<R>(foreign_key)
    }

    /// The `R` row this model's `foreign_key` column points at (`post.belongs_to::<User>("author_id")`).
    ///
    /// # Panics
    /// Panics if this model has no `foreign_key` column.
    fn belongs_to<R: Model>(&self, foreign_key: &'static str) -> R::QueryBuilder {
        let value = self
            .value_of(foreign_key)
            .unwrap_or_else(|| panic!("model `{}` has no column `{foreign_key}`", Self::NAME));
        R::QueryBuilder::from(Query::new().filter(R::PRIMARY_KEY, Op::Eq, value))
    }
}

/// Finds a row by primary key.
pub async fn find<M: Model>(db: &DatabasePool, key: M::Key) -> Result<Option<M>, OrmError> {
    Query::<M>::new()
        .filter(M::PRIMARY_KEY, Op::Eq, key)
        .first(db)
        .await
}

/// Finds a row by primary key or returns [`OrmError::NotFound`].
pub async fn find_or_fail<M: Model>(db: &DatabasePool, key: M::Key) -> Result<M, OrmError> {
    let id = key.to_string();
    find::<M>(db, key)
        .await?
        .ok_or_else(|| OrmError::not_found(M::NAME, Some(id)))
}

/// Inserts `values` and returns the stored row (`INSERT ... RETURNING *`).
pub async fn insert<M: Model>(
    db: &DatabasePool,
    values: Vec<(&'static str, SqlValue)>,
) -> Result<M, OrmError> {
    let mut sql = SqlWriter::new(db.driver(), "INSERT INTO ");
    sql.push_ident(M::TABLE);
    if values.is_empty() {
        sql.push(" DEFAULT VALUES");
    } else {
        sql.push(" (");
        for (index, (column, _)) in values.iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.push_ident(column);
        }
        sql.push(") VALUES (");
        for (index, (_, value)) in values.into_iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.bind(value);
        }
        sql.push(")");
    }
    sql.push(" RETURNING *");
    let (sql, values) = sql.finish();
    exec::fetch_one(db, &sql, values).await
}

/// Writes every non-key column of `model` back to its row.
pub async fn save<M: Model>(db: &DatabasePool, model: &M) -> Result<(), OrmError> {
    let columns = model.column_values();
    if columns.is_empty() {
        // Nothing to write, but saving a deleted row must still be a NotFound.
        let id = model.key().to_string();
        let exists = Query::<M>::new()
            .filter(M::PRIMARY_KEY, Op::Eq, model.key())
            .count(db)
            .await?
            > 0;
        return if exists {
            Ok(())
        } else {
            Err(OrmError::not_found(M::NAME, Some(id)))
        };
    }
    let mut sql = SqlWriter::new(db.driver(), "UPDATE ");
    sql.push_ident(M::TABLE);
    sql.push(" SET ");
    for (index, (column, value)) in columns.into_iter().enumerate() {
        if index > 0 {
            sql.push(", ");
        }
        sql.push_ident(column);
        sql.push(" = ");
        sql.bind(value);
    }
    sql.push(" WHERE ");
    sql.push_ident(M::PRIMARY_KEY);
    sql.push(" = ");
    let key = model.key();
    let id = key.to_string();
    sql.bind(key.into());

    let (sql, values) = sql.finish();
    if exec::execute(db, &sql, values).await? == 0 {
        return Err(OrmError::not_found(M::NAME, Some(id)));
    }
    Ok(())
}

/// Deletes `model`'s row.
pub async fn delete<M: Model>(db: &DatabasePool, model: &M) -> Result<(), OrmError> {
    let key = model.key();
    let id = key.to_string();
    let removed = Query::<M>::new()
        .filter(M::PRIMARY_KEY, Op::Eq, key)
        .delete(db)
        .await?;
    if removed == 0 {
        return Err(OrmError::not_found(M::NAME, Some(id)));
    }
    Ok(())
}
