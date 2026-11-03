//! # oxide-orm
//!
//! An Eloquent-inspired ORM for Oxide, built on [sqlx](https://docs.rs/sqlx) (SQLite).
//!
//! `#[derive(Model)]` generates, at compile time:
//!
//! - static entry points — `Post::query()`, `find`, `find_or_fail`, `all`, `insert`/`create`;
//! - a typed query builder with `where_<field>`, `where_<field>_in`, `order_by_<field>_desc`, ...
//!   (plus `_contains`/`_starts_with`/`_ends_with` on strings and `_gt`/`_gte`/`_lt`/`_lte` on
//!   numbers), always with bound `?` parameters;
//! - active-record mutations — `post.save(&db)`, `post.delete(&db)`;
//! - a `RouteBindable` impl, so `Bind<Post>` resolves rows from any state holding a
//!   [`DatabasePool`] — no manual binding code.
//!
//! ```ignore
//! let posts = Post::query()
//!     .where_published(true)
//!     .where_title_contains("Oxide")
//!     .order_by_id_desc()
//!     .get(&db)
//!     .await?;
//! ```

// Lets `#[derive(Model)]`'s `::oxide_orm::...` paths resolve inside this crate too.
extern crate self as oxide_orm;

pub mod database;
pub mod error;
pub mod model;
pub mod query;

pub use database::DatabasePool;
pub use error::OrmError;
pub use model::Model;
pub use oxide_orm_macros::Model;
pub use query::{Direction, Op, Query, SqlValue};

/// Re-exports used by generated code. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use axum;
    pub use oxide_http;
}

/// `use oxide_orm::prelude::*;`
pub mod prelude {
    pub use crate::database::DatabasePool;
    pub use crate::error::OrmError;
    pub use crate::model::Model;
    pub use oxide_orm_macros::Model;
}

/// Generated method names that would collide are a compile error.
///
/// ```compile_fail
/// #[derive(sqlx::FromRow, oxide_orm::Model)]
/// struct Score {
///     id: i64,
///     score: i64,
///     score_gt: bool, // `where_score_gt` already exists for `score`
/// }
/// ```
///
/// Renaming the field (and keeping the column with `#[sqlx(rename)]`) resolves it:
///
/// ```
/// #[derive(sqlx::FromRow, oxide_orm::Model)]
/// struct Score {
///     id: i64,
///     score: i64,
///     #[sqlx(rename = "score_gt")]
///     above_threshold: bool,
/// }
/// ```
#[cfg(doctest)]
pub struct CollidingMethodNames;
