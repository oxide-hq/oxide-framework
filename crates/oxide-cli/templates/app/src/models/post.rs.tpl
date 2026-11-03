use oxide::prelude::*;
use serde::Serialize;

/// A blog post. `#[derive(Model)]` adds `Post::query()`, `find`, `create`, `save`, `delete`,
/// a typed query builder and route-model binding for `Bind<Post>`.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, Model)]
#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub published: bool,
}
