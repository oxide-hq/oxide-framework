//! The ORM suite against a real PostgreSQL server.
//!
//! Set `OXIDE_TEST_POSTGRES_URL` (e.g. `postgres://postgres:oxide@localhost:55432/postgres`);
//! every test gets its own schema. Without it the tests are skipped, except under CI, where a
//! missing server is a failure.

#![cfg(feature = "postgres")]
// Axum handlers are async by contract even when they don't await.
#![allow(clippy::unused_async, clippy::unused_async_trait_impl)]

mod suite;

use oxide_orm::prelude::*;
use suite::{Post, backend_tests};

async fn database() -> Option<DatabasePool> {
    let Ok(url) = std::env::var("OXIDE_TEST_POSTGRES_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "OXIDE_TEST_POSTGRES_URL must be set in CI"
        );
        eprintln!("skipping: set OXIDE_TEST_POSTGRES_URL to run the PostgreSQL suite");
        return None;
    };
    Some(suite::fresh_database(&url).await)
}

backend_tests!();

#[test]
fn generated_sql_uses_numbered_placeholders() {
    let (sql, values) = Post::query()
        .where_title_contains("x' OR 1=1 --")
        .where_published(true)
        .order_by_id_desc()
        .limit(5)
        .into_query()
        .to_sql_for(Driver::Postgres);
    assert_eq!(
        sql,
        r#"SELECT * FROM "posts" WHERE "title" LIKE $1 ESCAPE '\' AND "published" = $2 ORDER BY "id" DESC LIMIT $3"#
    );
    assert_eq!(values.len(), 3);
}

#[tokio::test]
async fn pools_report_their_driver() {
    let Some(db) = database().await else { return };
    assert_eq!(db.driver(), Driver::Postgres);
    assert!(db.as_postgres().is_some());
    assert!(db.as_sqlite().is_none());
}
