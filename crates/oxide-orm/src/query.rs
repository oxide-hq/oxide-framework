//! A parameterized SQL query builder.
//!
//! Column names are `&'static str` (they come from `#[derive(Model)]` at compile time) and every
//! value is bound as a parameter (`?` on SQLite, `$n` on PostgreSQL), so user input never reaches
//! the SQL text.

use std::fmt::{self, Write as _};
use std::marker::PhantomData;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use sqlx::Arguments as _;
#[cfg(feature = "postgres")]
use sqlx::postgres::PgArguments;
use sqlx::sqlite::SqliteArguments;

use crate::database::{DatabasePool, Driver};
use crate::error::OrmError;
use crate::exec;
use crate::model::Model;

/// A value bound to a query parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Bool(bool),
    Text(String),
    Blob(Vec<u8>),
    DateTimeUtc(DateTime<Utc>),
    DateTime(NaiveDateTime),
    Date(NaiveDate),
}

macro_rules! sql_value_from {
    ($($ty:ty => $variant:ident $(as $cast:ty)?),* $(,)?) => {$(
        impl From<$ty> for SqlValue {
            fn from(value: $ty) -> Self {
                Self::$variant(value $(as $cast)?)
            }
        }
    )*};
}

sql_value_from! {
    i64 => Integer,
    i32 => Integer as i64,
    i16 => Integer as i64,
    i8 => Integer as i64,
    u32 => Integer as i64,
    u16 => Integer as i64,
    u8 => Integer as i64,
    f64 => Real,
    f32 => Real as f64,
    bool => Bool,
    String => Text,
    Vec<u8> => Blob,
    DateTime<Utc> => DateTimeUtc,
    NaiveDateTime => DateTime,
    NaiveDate => Date,
}

impl From<&str> for SqlValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl<T: Into<SqlValue>> From<Option<T>> for SqlValue {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

impl SqlValue {
    fn bind_sqlite(self, args: &mut SqliteArguments<'_>) -> Result<(), sqlx::Error> {
        let result = match self {
            Self::Null => args.add(None::<i64>),
            Self::Integer(v) => args.add(v),
            Self::Real(v) => args.add(v),
            Self::Bool(v) => args.add(v),
            Self::Text(v) => args.add(v),
            Self::Blob(v) => args.add(v),
            Self::DateTimeUtc(v) => args.add(v),
            Self::DateTime(v) => args.add(v),
            Self::Date(v) => args.add(v),
        };
        result.map_err(sqlx::Error::Encode)
    }

    #[cfg(feature = "postgres")]
    fn bind_postgres(self, args: &mut PgArguments) -> Result<(), sqlx::Error> {
        let result = match self {
            Self::Null => args.add(None::<i64>),
            Self::Integer(v) => args.add(v),
            Self::Real(v) => args.add(v),
            Self::Bool(v) => args.add(v),
            Self::Text(v) => args.add(v),
            Self::Blob(v) => args.add(v),
            Self::DateTimeUtc(v) => args.add(v),
            Self::DateTime(v) => args.add(v),
            Self::Date(v) => args.add(v),
        };
        result.map_err(sqlx::Error::Encode)
    }
}

/// Collects values into SQLite arguments.
pub(crate) fn sqlite_arguments(
    values: Vec<SqlValue>,
) -> Result<SqliteArguments<'static>, OrmError> {
    let mut args = SqliteArguments::default();
    for value in values {
        value.bind_sqlite(&mut args)?;
    }
    Ok(args)
}

/// Collects values into PostgreSQL arguments.
#[cfg(feature = "postgres")]
pub(crate) fn postgres_arguments(values: Vec<SqlValue>) -> Result<PgArguments, OrmError> {
    let mut args = PgArguments::default();
    for value in values {
        value.bind_postgres(&mut args)?;
    }
    Ok(args)
}

/// Builds SQL text and its bound values for one [`Driver`]'s placeholder syntax.
pub(crate) struct SqlWriter {
    sql: String,
    values: Vec<SqlValue>,
    driver: Driver,
}

impl SqlWriter {
    pub(crate) fn new(driver: Driver, sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            values: Vec::new(),
            driver,
        }
    }

    pub(crate) fn push(&mut self, sql: &str) {
        self.sql.push_str(sql);
    }

    pub(crate) fn push_ident(&mut self, name: &str) {
        self.sql.push_str(&ident(name));
    }

    /// Writes a placeholder (`?` or `$n`) bound to `value`.
    ///
    /// `NULL` is written as a literal instead: PostgreSQL types every parameter, and a `NULL`
    /// bound as one type is rejected by columns of another. The literal is a constant, so no
    /// input reaches the SQL text.
    pub(crate) fn bind(&mut self, value: SqlValue) {
        if value == SqlValue::Null {
            self.sql.push_str("NULL");
            return;
        }
        self.values.push(value);
        match self.driver {
            Driver::Sqlite => self.sql.push('?'),
            Driver::Postgres => {
                let _ = write!(self.sql, "${}", self.values.len());
            }
        }
    }

    pub(crate) fn finish(self) -> (String, Vec<SqlValue>) {
        (self.sql, self.values)
    }
}

/// A comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    /// `LIKE ... ESCAPE '\'`; build patterns with [`like_pattern`].
    Like,
}

impl Op {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Ne => "!=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Like => "LIKE",
        }
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

/// Escapes `%`, `_` and `\` in `text` and wraps it for a `LIKE ... ESCAPE '\'` match.
pub fn like_pattern(prefix: &str, text: &str, suffix: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + prefix.len() + suffix.len());
    pattern.push_str(prefix);
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push_str(suffix);
    pattern
}

#[derive(Debug, Clone)]
enum Condition {
    Compare {
        column: &'static str,
        op: Op,
        value: SqlValue,
    },
    In {
        column: &'static str,
        values: Vec<SqlValue>,
    },
    Null {
        column: &'static str,
        negated: bool,
    },
}

/// A query against `M`'s table. `#[derive(Model)]` wraps it in a typed builder with
/// `where_<field>` methods; this type holds the shared logic.
pub struct Query<M> {
    conditions: Vec<Condition>,
    order: Vec<(&'static str, Direction)>,
    limit: Option<u32>,
    offset: Option<u32>,
    model: PhantomData<fn() -> M>,
}

impl<M> fmt::Debug for Query<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Query")
            .field("conditions", &self.conditions)
            .field("order", &self.order)
            .field("limit", &self.limit)
            .field("offset", &self.offset)
            .finish()
    }
}

impl<M> Clone for Query<M> {
    fn clone(&self) -> Self {
        Self {
            conditions: self.conditions.clone(),
            order: self.order.clone(),
            limit: self.limit,
            offset: self.offset,
            model: PhantomData,
        }
    }
}

impl<M: Model> Default for Query<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Model> Query<M> {
    pub fn new() -> Self {
        Self {
            conditions: Vec::new(),
            order: Vec::new(),
            limit: None,
            offset: None,
            model: PhantomData,
        }
    }

    /// `column <op> ?`. A `Null` value with `Eq`/`Ne` becomes `IS NULL` / `IS NOT NULL`.
    #[must_use]
    pub fn filter(mut self, column: &'static str, op: Op, value: impl Into<SqlValue>) -> Self {
        let value = value.into();
        let condition = match (op, &value) {
            (Op::Eq, SqlValue::Null) => Condition::Null {
                column,
                negated: false,
            },
            (Op::Ne, SqlValue::Null) => Condition::Null {
                column,
                negated: true,
            },
            _ => Condition::Compare { column, op, value },
        };
        self.conditions.push(condition);
        self
    }

    /// `column IN (?, ...)`; an empty list matches nothing.
    #[must_use]
    pub fn filter_in<V: Into<SqlValue>>(
        mut self,
        column: &'static str,
        values: impl IntoIterator<Item = V>,
    ) -> Self {
        self.conditions.push(Condition::In {
            column,
            values: values.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// `column IS NULL` (or `IS NOT NULL` when `negated`).
    #[must_use]
    pub fn filter_null(mut self, column: &'static str, negated: bool) -> Self {
        self.conditions.push(Condition::Null { column, negated });
        self
    }

    #[must_use]
    pub fn order_by(mut self, column: &'static str, direction: Direction) -> Self {
        self.order.push((column, direction));
        self
    }

    #[must_use]
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    #[must_use]
    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = Some(offset);
        self
    }

    /// The `SELECT` statement and its bound values, in SQLite syntax (`?` placeholders).
    pub fn to_sql(&self) -> (String, Vec<SqlValue>) {
        self.to_sql_for(Driver::Sqlite)
    }

    /// The `SELECT` statement and its bound values for `driver`'s SQL dialect.
    pub fn to_sql_for(&self, driver: Driver) -> (String, Vec<SqlValue>) {
        self.select(driver, "*").finish()
    }

    /// Runs the query.
    pub async fn get(self, db: &DatabasePool) -> Result<Vec<M>, OrmError> {
        let (sql, values) = self.to_sql_for(db.driver());
        exec::fetch_all(db, &sql, values).await
    }

    /// The first matching row.
    pub async fn first(self, db: &DatabasePool) -> Result<Option<M>, OrmError> {
        let (sql, values) = self.limit(1).to_sql_for(db.driver());
        exec::fetch_optional(db, &sql, values).await
    }

    /// The first matching row, or [`OrmError::NotFound`] (a `404` over HTTP).
    pub async fn first_or_fail(self, db: &DatabasePool) -> Result<M, OrmError> {
        self.first(db)
            .await?
            .ok_or_else(|| OrmError::not_found(M::NAME, None))
    }

    /// Counts matching rows (respecting `limit`/`offset` when set).
    pub async fn count(self, db: &DatabasePool) -> Result<i64, OrmError> {
        let (sql, values) = self.count_sql(db.driver());
        exec::fetch_count(db, &sql, values).await
    }

    /// Deletes matching rows and returns how many were removed.
    pub async fn delete(self, db: &DatabasePool) -> Result<u64, OrmError> {
        let (sql, values) = self.delete_sql(db.driver());
        exec::execute(db, &sql, values).await
    }

    fn select(&self, driver: Driver, columns: &str) -> SqlWriter {
        let mut sql = SqlWriter::new(driver, format!("SELECT {columns} FROM "));
        sql.push_ident(M::TABLE);
        self.push_where(&mut sql);
        self.push_order_limit(&mut sql);
        sql
    }

    fn is_windowed(&self) -> bool {
        self.limit.is_some() || self.offset.is_some()
    }

    pub(crate) fn count_sql(&self, driver: Driver) -> (String, Vec<SqlValue>) {
        if self.is_windowed() {
            // The window applies to rows, so count over a subquery (aliased for PostgreSQL).
            let (inner, values) = self.select(driver, "1").finish();
            return (format!("SELECT COUNT(*) FROM ({inner}) AS sub"), values);
        }
        let mut sql = SqlWriter::new(driver, "SELECT COUNT(*) FROM ");
        sql.push_ident(M::TABLE);
        self.push_where(&mut sql);
        sql.finish()
    }

    pub(crate) fn delete_sql(&self, driver: Driver) -> (String, Vec<SqlValue>) {
        let mut sql = SqlWriter::new(driver, "DELETE FROM ");
        sql.push_ident(M::TABLE);
        if self.is_windowed() || !self.order.is_empty() {
            // Neither SQLite (by default) nor PostgreSQL has DELETE ... LIMIT; select the keys.
            sql.push(" WHERE ");
            sql.push_ident(M::PRIMARY_KEY);
            sql.push(" IN (SELECT ");
            sql.push_ident(M::PRIMARY_KEY);
            sql.push(" FROM ");
            sql.push_ident(M::TABLE);
            self.push_where(&mut sql);
            self.push_order_limit(&mut sql);
            sql.push(")");
        } else {
            self.push_where(&mut sql);
        }
        sql.finish()
    }

    fn push_where(&self, sql: &mut SqlWriter) {
        for (index, condition) in self.conditions.iter().enumerate() {
            sql.push(if index == 0 { " WHERE " } else { " AND " });
            match condition {
                Condition::Compare { column, op, value } => {
                    sql.push_ident(column);
                    sql.push(" ");
                    sql.push(op.as_sql());
                    sql.push(" ");
                    sql.bind(value.clone());
                    if *op == Op::Like {
                        sql.push(" ESCAPE '\\'");
                    }
                }
                // `x IN ()` is never true. The column is not echoed into a comment, where
                // quoting would not protect it.
                Condition::In { values, .. } if values.is_empty() => sql.push("0 = 1"),
                Condition::In { column, values } => {
                    sql.push_ident(column);
                    sql.push(" IN (");
                    for (index, value) in values.iter().enumerate() {
                        if index > 0 {
                            sql.push(", ");
                        }
                        sql.bind(value.clone());
                    }
                    sql.push(")");
                }
                Condition::Null { column, negated } => {
                    sql.push_ident(column);
                    sql.push(if *negated { " IS NOT NULL" } else { " IS NULL" });
                }
            }
        }
    }

    fn push_order_limit(&self, sql: &mut SqlWriter) {
        for (index, (column, direction)) in self.order.iter().enumerate() {
            sql.push(if index == 0 { " ORDER BY " } else { ", " });
            sql.push_ident(column);
            sql.push(match direction {
                Direction::Asc => " ASC",
                Direction::Desc => " DESC",
            });
        }
        match (self.limit, sql.driver) {
            (Some(limit), _) => {
                sql.push(" LIMIT ");
                sql.bind(SqlValue::from(limit));
            }
            // SQLite needs a LIMIT before OFFSET; -1 means "no limit".
            (None, Driver::Sqlite) if self.offset.is_some() => {
                sql.push(" LIMIT ");
                sql.bind(SqlValue::Integer(-1));
            }
            (None, _) => {}
        }
        if let Some(offset) = self.offset {
            sql.push(" OFFSET ");
            sql.bind(SqlValue::from(offset));
        }
    }
}

/// Quotes an SQL identifier.
pub(crate) fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, sqlx::FromRow, crate::Model)]
    #[allow(dead_code)]
    struct Podcast {
        id: i64,
        title: String,
        subscribers: Option<i64>,
    }

    fn sample() -> Query<Podcast> {
        Query::new()
            .filter("title", Op::Like, like_pattern("%", "rust", "%"))
            .filter_in("id", [1_i64, 2, 3])
            .filter("subscribers", Op::Gte, 100_i64)
            .order_by("id", Direction::Desc)
            .limit(10)
            .offset(20)
    }

    #[test]
    fn sqlite_uses_question_mark_placeholders() {
        let (sql, values) = sample().to_sql();
        assert_eq!(
            sql,
            r#"SELECT * FROM "podcasts" WHERE "title" LIKE ? ESCAPE '\' AND "id" IN (?, ?, ?) AND "subscribers" >= ? ORDER BY "id" DESC LIMIT ? OFFSET ?"#
        );
        assert_eq!(values.len(), 7);
    }

    #[test]
    fn postgres_numbers_its_placeholders() {
        let (sql, values) = sample().to_sql_for(Driver::Postgres);
        assert_eq!(
            sql,
            r#"SELECT * FROM "podcasts" WHERE "title" LIKE $1 ESCAPE '\' AND "id" IN ($2, $3, $4) AND "subscribers" >= $5 ORDER BY "id" DESC LIMIT $6 OFFSET $7"#
        );
        assert_eq!(values[5], SqlValue::Integer(10));
        assert_eq!(values[6], SqlValue::Integer(20));
    }

    #[test]
    fn offset_without_limit_follows_the_dialect() {
        let query = Query::<Podcast>::new().offset(5);
        assert_eq!(
            query.to_sql().0,
            r#"SELECT * FROM "podcasts" LIMIT ? OFFSET ?"#
        );
        assert_eq!(
            query.to_sql_for(Driver::Postgres).0,
            r#"SELECT * FROM "podcasts" OFFSET $1"#
        );
    }

    #[test]
    fn null_values_are_literals() {
        let (sql, values) = Query::<Podcast>::new()
            .filter("subscribers", Op::Gt, None::<i64>)
            .filter("id", Op::Eq, 1_i64)
            .to_sql_for(Driver::Postgres);
        assert_eq!(
            sql,
            r#"SELECT * FROM "podcasts" WHERE "subscribers" > NULL AND "id" = $1"#
        );
        assert_eq!(values, [SqlValue::Integer(1)]);
    }

    #[test]
    fn windowed_counts_and_deletes_use_subqueries() {
        let query = Query::<Podcast>::new()
            .filter("id", Op::Gt, 1_i64)
            .order_by("id", Direction::Asc)
            .limit(2);
        assert_eq!(
            query.count_sql(Driver::Postgres).0,
            r#"SELECT COUNT(*) FROM (SELECT 1 FROM "podcasts" WHERE "id" > $1 ORDER BY "id" ASC LIMIT $2) AS sub"#
        );
        assert_eq!(
            query.delete_sql(Driver::Postgres).0,
            r#"DELETE FROM "podcasts" WHERE "id" IN (SELECT "id" FROM "podcasts" WHERE "id" > $1 ORDER BY "id" ASC LIMIT $2)"#
        );
        assert_eq!(
            Query::<Podcast>::new().count_sql(Driver::Sqlite).0,
            r#"SELECT COUNT(*) FROM "podcasts""#
        );
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_pattern("%", "50%_off\\", "%"), "%50\\%\\_off\\\\%");
        assert_eq!(like_pattern("", "Oxide", "%"), "Oxide%");
    }

    #[test]
    fn idents_are_quoted() {
        assert_eq!(ident("posts"), "\"posts\"");
        assert_eq!(ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn option_values_map_to_null() {
        assert_eq!(SqlValue::from(None::<i64>), SqlValue::Null);
        assert_eq!(SqlValue::from(Some("x")), SqlValue::Text("x".into()));
    }
}
