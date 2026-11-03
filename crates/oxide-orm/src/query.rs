//! A parameterized SQL query builder.
//!
//! Column names are `&'static str` (they come from `#[derive(Model)]` at compile time) and every
//! value is bound as a `?` parameter, so user input never reaches the SQL text.

use std::fmt::{self, Write as _};
use std::marker::PhantomData;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use sqlx::sqlite::SqliteArguments;
use sqlx::{Arguments as _, Sqlite};

use crate::database::DatabasePool;
use crate::error::OrmError;
use crate::model::Model;

/// A value bound to a `?` placeholder.
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
    fn bind(self, args: &mut SqliteArguments<'_>) -> Result<(), sqlx::Error> {
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

/// Collects values into sqlx arguments.
pub(crate) fn arguments(values: Vec<SqlValue>) -> Result<SqliteArguments<'static>, OrmError> {
    let mut args = SqliteArguments::default();
    for value in values {
        value.bind(&mut args)?;
    }
    Ok(args)
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

    /// The `SELECT` statement and its bound values, for debugging or custom execution.
    pub fn to_sql(&self) -> (String, Vec<SqlValue>) {
        let mut sql = format!("SELECT * FROM {}", ident(M::TABLE));
        let mut values = Vec::new();
        self.push_where(&mut sql, &mut values);
        self.push_order_limit(&mut sql, &mut values);
        (sql, values)
    }

    /// Runs the query.
    pub async fn get(self, db: &DatabasePool) -> Result<Vec<M>, OrmError> {
        let (sql, values) = self.to_sql();
        Ok(
            sqlx::query_as_with::<Sqlite, M, _>(&sql, arguments(values)?)
                .fetch_all(db.pool())
                .await?,
        )
    }

    /// The first matching row.
    pub async fn first(self, db: &DatabasePool) -> Result<Option<M>, OrmError> {
        let (sql, values) = self.limit(1).to_sql();
        Ok(
            sqlx::query_as_with::<Sqlite, M, _>(&sql, arguments(values)?)
                .fetch_optional(db.pool())
                .await?,
        )
    }

    /// The first matching row, or [`OrmError::NotFound`] (a `404` over HTTP).
    pub async fn first_or_fail(self, db: &DatabasePool) -> Result<M, OrmError> {
        self.first(db)
            .await?
            .ok_or_else(|| OrmError::not_found(M::NAME, None))
    }

    /// Counts matching rows (respecting `limit`/`offset` when set).
    pub async fn count(self, db: &DatabasePool) -> Result<i64, OrmError> {
        let mut values = Vec::new();
        let sql = if self.limit.is_some() || self.offset.is_some() {
            let mut inner = format!("SELECT 1 FROM {}", ident(M::TABLE));
            self.push_where(&mut inner, &mut values);
            self.push_order_limit(&mut inner, &mut values);
            format!("SELECT COUNT(*) FROM ({inner})")
        } else {
            let mut sql = format!("SELECT COUNT(*) FROM {}", ident(M::TABLE));
            self.push_where(&mut sql, &mut values);
            sql
        };
        Ok(
            sqlx::query_scalar_with::<Sqlite, i64, _>(&sql, arguments(values)?)
                .fetch_one(db.pool())
                .await?,
        )
    }

    /// Deletes matching rows and returns how many were removed.
    pub async fn delete(self, db: &DatabasePool) -> Result<u64, OrmError> {
        let mut values = Vec::new();
        let mut sql = format!("DELETE FROM {}", ident(M::TABLE));
        if self.limit.is_some() || self.offset.is_some() || !self.order.is_empty() {
            // SQLite's DELETE ... LIMIT is a compile-time option; select the keys instead.
            let pk = ident(M::PRIMARY_KEY);
            let mut inner = format!("SELECT {pk} FROM {}", ident(M::TABLE));
            self.push_where(&mut inner, &mut values);
            self.push_order_limit(&mut inner, &mut values);
            let _ = write!(sql, " WHERE {pk} IN ({inner})");
        } else {
            self.push_where(&mut sql, &mut values);
        }
        Ok(sqlx::query_with::<Sqlite, _>(&sql, arguments(values)?)
            .execute(db.pool())
            .await?
            .rows_affected())
    }

    fn push_where(&self, sql: &mut String, values: &mut Vec<SqlValue>) {
        for (index, condition) in self.conditions.iter().enumerate() {
            sql.push_str(if index == 0 { " WHERE " } else { " AND " });
            match condition {
                Condition::Compare { column, op, value } => {
                    let _ = write!(sql, "{} {} ?", ident(column), op.as_sql());
                    if *op == Op::Like {
                        sql.push_str(" ESCAPE '\\'");
                    }
                    values.push(value.clone());
                }
                Condition::In {
                    column,
                    values: list,
                } if list.is_empty() => {
                    let _ = write!(sql, "0 = 1 /* {} IN () */", ident(column));
                }
                Condition::In {
                    column,
                    values: list,
                } => {
                    let placeholders = vec!["?"; list.len()].join(", ");
                    let _ = write!(sql, "{} IN ({placeholders})", ident(column));
                    values.extend(list.iter().cloned());
                }
                Condition::Null { column, negated } => {
                    let not = if *negated { " NOT" } else { "" };
                    let _ = write!(sql, "{} IS{not} NULL", ident(column));
                }
            }
        }
    }

    fn push_order_limit(&self, sql: &mut String, values: &mut Vec<SqlValue>) {
        for (index, (column, direction)) in self.order.iter().enumerate() {
            sql.push_str(if index == 0 { " ORDER BY " } else { ", " });
            let dir = match direction {
                Direction::Asc => "ASC",
                Direction::Desc => "DESC",
            };
            let _ = write!(sql, "{} {dir}", ident(column));
        }
        match (self.limit, self.offset) {
            (None, None) => {}
            (limit, offset) => {
                // SQLite needs a LIMIT before OFFSET; -1 means "no limit".
                sql.push_str(" LIMIT ?");
                values.push(limit.map_or(SqlValue::Integer(-1), SqlValue::from));
                if let Some(offset) = offset {
                    sql.push_str(" OFFSET ?");
                    values.push(SqlValue::from(offset));
                }
            }
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
