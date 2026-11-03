//! Phase 3: Eloquent-style queries plus the full HTTP + SQLite lifecycle through `Bind<Post>`.
//!
//! The cases live in `suite/` and run here against an in-memory SQLite database.

// Axum handlers are async by contract even when they don't await.
#![allow(clippy::unused_async, clippy::unused_async_trait_impl)]

mod suite;

use oxide_orm::prelude::*;
use suite::{Post, backend_tests};

#[allow(clippy::unnecessary_wraps)]
async fn database() -> Option<DatabasePool> {
    Some(suite::fresh_database("sqlite::memory:").await)
}

backend_tests!();

#[test]
fn generated_sql_binds_every_value() {
    let (sql, values) = Post::query()
        .where_title_contains("x' OR 1=1 --")
        .where_published(true)
        .order_by_id_desc()
        .limit(5)
        .to_sql();
    assert_eq!(
        sql,
        r#"SELECT * FROM "posts" WHERE "title" LIKE ? ESCAPE '\' AND "published" = ? ORDER BY "id" DESC LIMIT ?"#
    );
    assert_eq!(values.len(), 3);

    let (sql, values) = Post::query().where_id_in(Vec::new()).to_sql();
    assert_eq!(sql, r#"SELECT * FROM "posts" WHERE 0 = 1"#);
    assert_eq!(values.len(), 0);
}
