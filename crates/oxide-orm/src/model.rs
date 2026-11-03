//! The [`Model`] trait, generic CRUD helpers used by `#[derive(Model)]`, and relations.

use std::fmt::{Display, Write as _};

use sqlx::Sqlite;
use sqlx::sqlite::SqliteRow;

use crate::database::DatabasePool;
use crate::error::OrmError;
use crate::query::{Op, Query, SqlValue, arguments, ident};

/// A database-backed model. Implemented by `#[derive(Model)]`.
pub trait Model: Sized + Send + Unpin + for<'r> sqlx::FromRow<'r, SqliteRow> + 'static {
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
    let (columns, values): (Vec<_>, Vec<_>) = values.into_iter().unzip();
    let sql = if columns.is_empty() {
        format!("INSERT INTO {} DEFAULT VALUES RETURNING *", ident(M::TABLE))
    } else {
        let columns = columns
            .iter()
            .map(|c| ident(c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = vec!["?"; values.len()].join(", ");
        format!(
            "INSERT INTO {} ({columns}) VALUES ({placeholders}) RETURNING *",
            ident(M::TABLE)
        )
    };
    Ok(
        sqlx::query_as_with::<Sqlite, M, _>(&sql, arguments(values)?)
            .fetch_one(db.pool())
            .await?,
    )
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
    let mut sql = format!("UPDATE {} SET ", ident(M::TABLE));
    let mut values = Vec::with_capacity(columns.len() + 1);
    for (index, (column, value)) in columns.into_iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        let _ = write!(sql, "{} = ?", ident(column));
        values.push(value);
    }
    let _ = write!(sql, " WHERE {} = ?", ident(M::PRIMARY_KEY));
    let key = model.key();
    let id = key.to_string();
    values.push(key.into());

    let affected = sqlx::query_with::<Sqlite, _>(&sql, arguments(values)?)
        .execute(db.pool())
        .await?
        .rows_affected();
    if affected == 0 {
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
